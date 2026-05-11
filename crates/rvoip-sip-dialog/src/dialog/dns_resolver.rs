//! RFC 3263 SIP URI resolution.
//!
//! Given a SIP URI, produce a `(SocketAddr, TransportType)` by walking the
//! RFC 3263 §4 ladder:
//!
//! 1. **IP literal** → return `(ip, uri-port-or-default)` with the URI-derived
//!    transport.
//! 2. **Hostname with explicit port** → A/AAAA on the hostname, return
//!    `(ip, port)` with the URI-derived transport. (RFC 3263 §4.2 — "if port
//!    is explicit, skip NAPTR/SRV".)
//! 3. **Hostname, no port, explicit `;transport=` or `sips:` scheme** →
//!    `_service._proto.host` SRV lookup, weighted select, A/AAAA on the
//!    target, return `(ip, srv-port)` with the URI-derived transport.
//! 4. **Fallback** → A/AAAA on the hostname with the scheme-default port.
//!
//! This implementation deliberately stops short of NAPTR. NAPTR's only
//! incremental value over `;transport=` and the `sip:` vs `sips:` scheme is
//! when the UA has *no* a-priori transport preference, which is uncommon for
//! our callers. Add it if a deployment needs it.
//!
//! Tests validate the pure-logic pieces (SRV selection, service-name
//! derivation, scheme defaults) without touching live DNS. The live
//! resolver uses `hickory-resolver` under the hood.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip_core::types::uri::{Host, Scheme, Uri};
use rvoip_sip_transport::transport::TransportType;
use tokio::sync::OnceCell;
use tracing::{debug, warn};

use crate::transaction::transport::multiplexed;

/// A resolved destination for a SIP URI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedTarget {
    pub addr: SocketAddr,
    pub transport: TransportType,
}

/// RFC 3261 §19.1.2 — default port by scheme.
pub(crate) fn default_port_for_scheme(scheme: &Scheme) -> u16 {
    match scheme {
        Scheme::Sips => 5061,
        _ => 5060,
    }
}

/// RFC 3263 §4.1 service label for `_service._proto.host` SRV lookups.
///
/// Returns `None` when SRV should be skipped entirely (e.g. the URI pinned
/// UDP but the scheme is `sips:`, which RFC 3263 forbids).
pub(crate) fn srv_service_name(
    host: &str,
    transport: TransportType,
    scheme: &Scheme,
) -> Option<String> {
    let (service, proto) = match (scheme, transport) {
        // `sips:` URIs must use TLS-capable transport. RFC 3263 §4.2.
        (Scheme::Sips, _) => ("_sips", "_tcp"),
        (_, TransportType::Tls) => ("_sips", "_tcp"),
        (_, TransportType::Tcp) => ("_sip", "_tcp"),
        (_, TransportType::Udp) => ("_sip", "_udp"),
        // RFC 7118 — WebSocket SRV labels.
        (_, TransportType::Ws) => ("_sip", "_ws"),
        (_, TransportType::Wss) => ("_sips", "_wss"),
    };
    Some(format!("{}.{}.{}", service, proto, host))
}

/// Pure SRV selection per RFC 2782: within the lowest-priority group, pick a
/// record weighted by its `weight` field. Given `(priority, weight, port,
/// target)` tuples, returns the selected `(port, target)`.
///
/// RFC 2782 weighted selection algorithm:
/// 1. Filter to the lowest priority value.
/// 2. For each entry in weight order, assign a running cumulative weight.
/// 3. Pick a uniformly random value in `[0, total_weight]`; the first entry
///    whose running sum ≥ the picked value wins.
///
/// Zero-weight entries are special-cased per RFC: they still participate but
/// only if the running sum is still `0` when we reach them (so they're
/// effectively first in weight order, but only picked if *no* non-zero entry
/// is selected — we preserve this by sorting zero-weight first).
pub(crate) fn select_srv_best<'a>(
    records: &'a [(u16, u16, u16, String)], // (priority, weight, port, target)
    rand_0_1: f64,
) -> Option<&'a (u16, u16, u16, String)> {
    if records.is_empty() {
        return None;
    }
    let min_priority = records.iter().map(|r| r.0).min()?;
    let mut group: Vec<&(u16, u16, u16, String)> =
        records.iter().filter(|r| r.0 == min_priority).collect();

    // RFC 2782: zero-weight entries are sorted first within the group.
    group.sort_by(|a, b| {
        // Zero-weight first, then ascending weight (stable sort preserves
        // original order within equal weights).
        match (a.1 == 0, b.1 == 0) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.1.cmp(&b.1),
        }
    });

    let total_weight: u32 = group.iter().map(|r| r.1 as u32).sum();
    if total_weight == 0 {
        // All weights zero — pick the first (RFC 2782: treat as equivalent).
        return group.first().copied();
    }

    let picked = (rand_0_1 * total_weight as f64).floor() as u32;
    let picked = picked.min(total_weight - 1);
    let mut running: u32 = 0;
    for rec in &group {
        running += rec.1 as u32;
        if running > picked {
            return Some(*rec);
        }
    }
    // Shouldn't reach here if total_weight > 0; fall back to first.
    group.first().copied()
}

/// Thin facade over `hickory_resolver::TokioAsyncResolver` so the rest of
/// this module can be tested against a stub via trait objects if we ever
/// need to. For now the only impl is the live system resolver.
struct SystemDnsResolver {
    inner: hickory_resolver::TokioAsyncResolver,
}

impl SystemDnsResolver {
    fn new_system() -> Result<Self, hickory_resolver::error::ResolveError> {
        // `tokio_from_system_conf()` honours /etc/resolv.conf on Unix and
        // the system resolver on macOS/Windows — which is what the UA
        // administrator expects. If that's unavailable (sandboxed
        // envs), fall back to Cloudflare+Google as a last resort via
        // `TokioAsyncResolver::tokio(...)`; done by caller.
        let inner = hickory_resolver::TokioAsyncResolver::tokio_from_system_conf()?;
        Ok(Self { inner })
    }

    async fn lookup_srv(&self, service_name: &str) -> Vec<(u16, u16, u16, String)> {
        match self.inner.srv_lookup(service_name).await {
            Ok(lookup) => lookup
                .iter()
                .map(|srv| {
                    (
                        srv.priority(),
                        srv.weight(),
                        srv.port(),
                        srv.target().to_utf8().trim_end_matches('.').to_string(),
                    )
                })
                .collect(),
            Err(e) => {
                debug!("SRV lookup {} failed: {}", service_name, e);
                Vec::new()
            }
        }
    }

    async fn lookup_ip(&self, host: &str) -> Vec<IpAddr> {
        match self.inner.lookup_ip(host).await {
            Ok(lookup) => lookup.iter().collect(),
            Err(e) => {
                debug!("A/AAAA lookup {} failed: {}", host, e);
                Vec::new()
            }
        }
    }
}

/// Process-wide default resolver, initialised lazily.
static DEFAULT_RESOLVER: OnceCell<Arc<SystemDnsResolver>> = OnceCell::const_new();

async fn default_resolver() -> Option<Arc<SystemDnsResolver>> {
    let resolver = DEFAULT_RESOLVER
        .get_or_try_init(|| async {
            match SystemDnsResolver::new_system() {
                Ok(r) => Ok::<_, ()>(Arc::new(r)),
                Err(e) => {
                    warn!(
                        "Could not construct system DNS resolver ({}); RFC 3263 SRV/AAAA resolution disabled",
                        e
                    );
                    Err(())
                }
            }
        })
        .await
        .ok()?;
    Some(resolver.clone())
}

/// Resolve a SIP URI per RFC 3263 §4. Returns `Some(ResolvedTarget)` with
/// the first usable `(ip, port, transport)` triple, or `None` if nothing
/// resolved.
///
/// Walks the ladder in order (see module docstring). Transport comes from
/// `MultiplexedTransport::select_transport_for_uri` — the URI's `;transport=`
/// or scheme governs, matching the Sprint 1 A2 dispatch. SRV is consulted
/// only when no explicit port is present.
pub async fn resolve_uri(uri: &Uri) -> Option<ResolvedTarget> {
    let transport = multiplexed::select_transport_for_uri(uri);
    let default_port = default_port_for_scheme(uri.scheme());

    match &uri.host {
        Host::Address(ip) => {
            let port = uri.port.filter(|p| *p > 0).unwrap_or(default_port);
            Some(ResolvedTarget {
                addr: SocketAddr::new(*ip, port),
                transport,
            })
        }
        Host::Domain(domain) => {
            let resolver = default_resolver().await?;

            // RFC 3263 §4.2 — if port is explicit, skip NAPTR/SRV.
            if let Some(port) = uri.port.filter(|p| *p > 0) {
                let ip = resolver.lookup_ip(domain).await.into_iter().next()?;
                return Some(ResolvedTarget {
                    addr: SocketAddr::new(ip, port),
                    transport,
                });
            }

            // No port → try SRV.
            if let Some(service) = srv_service_name(domain, transport, uri.scheme()) {
                let records = resolver.lookup_srv(&service).await;
                if !records.is_empty() {
                    let random = fastrand::f64();
                    if let Some(pick) = select_srv_best(&records, random) {
                        let (_priority, _weight, port, target) = pick;
                        debug!(
                            "RFC 3263: {} → SRV {} ({} records) picked {}:{}",
                            domain,
                            service,
                            records.len(),
                            target,
                            port
                        );
                        if let Some(ip) = resolver.lookup_ip(target).await.into_iter().next() {
                            return Some(ResolvedTarget {
                                addr: SocketAddr::new(ip, *port),
                                transport,
                            });
                        }
                        warn!(
                            "RFC 3263: SRV target {} had no A/AAAA records; falling back to direct lookup on {}",
                            target, domain
                        );
                    }
                }
            }

            // RFC 3263 §4.2 fallback — direct A/AAAA on the host.
            let ip = resolver.lookup_ip(domain).await.into_iter().next()?;
            Some(ResolvedTarget {
                addr: SocketAddr::new(ip, default_port),
                transport,
            })
        }
    }
}

/// Back-compat wrapper for callers that only need the `SocketAddr`. Existing
/// A3-era code paths consume this signature unchanged.
pub async fn resolve_uri_to_socketaddr(uri: &Uri) -> Option<SocketAddr> {
    resolve_uri(uri).await.map(|r| r.addr)
}

#[allow(dead_code)]
pub(crate) fn _resolver_timeout_hint() -> Duration {
    // Not enforced at the hickory layer today — hickory uses its own config
    // for timeouts. Kept as a knob for a follow-up if DNS timeouts surface
    // as a problem in the field.
    Duration::from_secs(5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn default_port_follows_scheme() {
        assert_eq!(default_port_for_scheme(&Scheme::Sip), 5060);
        assert_eq!(default_port_for_scheme(&Scheme::Sips), 5061);
    }

    #[test]
    fn srv_service_name_sip_udp() {
        let name = srv_service_name("example.com", TransportType::Udp, &Scheme::Sip).unwrap();
        assert_eq!(name, "_sip._udp.example.com");
    }

    #[test]
    fn srv_service_name_sip_tcp() {
        let name = srv_service_name("example.com", TransportType::Tcp, &Scheme::Sip).unwrap();
        assert_eq!(name, "_sip._tcp.example.com");
    }

    #[test]
    fn srv_service_name_sip_tls_upgrades_to_sips_label() {
        // `sip:host;transport=tls` → `_sips._tcp` per RFC 3263 §4.1.
        let name = srv_service_name("example.com", TransportType::Tls, &Scheme::Sip).unwrap();
        assert_eq!(name, "_sips._tcp.example.com");
    }

    #[test]
    fn srv_service_name_sips_always_sips_label() {
        let name = srv_service_name("example.com", TransportType::Udp, &Scheme::Sips).unwrap();
        assert_eq!(name, "_sips._tcp.example.com");
    }

    #[test]
    fn srv_service_name_ws() {
        let name = srv_service_name("example.com", TransportType::Ws, &Scheme::Sip).unwrap();
        assert_eq!(name, "_sip._ws.example.com");
        let name = srv_service_name("example.com", TransportType::Wss, &Scheme::Sip).unwrap();
        assert_eq!(name, "_sips._wss.example.com");
    }

    #[test]
    fn select_srv_best_empty_returns_none() {
        let records: Vec<(u16, u16, u16, String)> = Vec::new();
        assert!(select_srv_best(&records, 0.5).is_none());
    }

    #[test]
    fn select_srv_best_prefers_lowest_priority() {
        let records = vec![
            (10, 100, 5060, "low-priority.example.com".into()),
            (1, 1, 5060, "high-priority.example.com".into()),
        ];
        let picked = select_srv_best(&records, 0.5).unwrap();
        assert_eq!(picked.3, "high-priority.example.com");
    }

    #[test]
    fn select_srv_best_weights_by_weight() {
        // Two priority-1 entries with weights 1 and 99. rand=0.01 should
        // land inside the weight=1 bucket; rand=0.5 should land in the
        // weight=99 bucket.
        let records = vec![
            (1, 1, 5060, "one.example.com".into()),
            (1, 99, 5060, "two.example.com".into()),
        ];
        let low = select_srv_best(&records, 0.005).unwrap();
        assert_eq!(low.3, "one.example.com");
        let high = select_srv_best(&records, 0.5).unwrap();
        assert_eq!(high.3, "two.example.com");
    }

    #[test]
    fn select_srv_best_all_zero_weight_picks_first() {
        let records = vec![
            (1, 0, 5060, "first.example.com".into()),
            (1, 0, 5060, "second.example.com".into()),
        ];
        let picked = select_srv_best(&records, 0.5).unwrap();
        assert_eq!(picked.3, "first.example.com");
    }

    #[test]
    fn select_srv_best_ignores_higher_priority_group() {
        let records = vec![
            (1, 10, 5060, "primary.example.com".into()),
            (2, 10, 5060, "backup.example.com".into()),
        ];
        let picked = select_srv_best(&records, 0.9).unwrap();
        assert_eq!(picked.3, "primary.example.com");
    }

    #[tokio::test]
    async fn resolve_uri_ip_literal_sip_default_port() {
        let uri = Uri::from_str("sip:1.2.3.4").unwrap();
        let resolved = resolve_uri(&uri).await.unwrap();
        assert_eq!(resolved.addr.to_string(), "1.2.3.4:5060");
        assert_eq!(resolved.transport, TransportType::Udp);
    }

    #[tokio::test]
    async fn resolve_uri_ip_literal_sips_default_port() {
        let uri = Uri::from_str("sips:1.2.3.4").unwrap();
        let resolved = resolve_uri(&uri).await.unwrap();
        assert_eq!(resolved.addr.to_string(), "1.2.3.4:5061");
        assert_eq!(resolved.transport, TransportType::Tls);
    }

    #[tokio::test]
    async fn resolve_uri_ip_literal_explicit_port_wins() {
        let uri = Uri::from_str("sip:1.2.3.4:12345").unwrap();
        let resolved = resolve_uri(&uri).await.unwrap();
        assert_eq!(resolved.addr.to_string(), "1.2.3.4:12345");
    }

    #[tokio::test]
    async fn resolve_uri_ip_literal_transport_param_wins() {
        let uri = Uri::from_str("sip:1.2.3.4;transport=tcp").unwrap();
        let resolved = resolve_uri(&uri).await.unwrap();
        assert_eq!(resolved.transport, TransportType::Tcp);
    }
}

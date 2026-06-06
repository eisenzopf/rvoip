//! Reference [`Resolver`] implementation backed by `hickory-resolver`.
//!
//! Walks the full RFC 3263 §4 ladder
//! IP-literal/explicit-port short-circuits → NAPTR (§4.1) →
//! fallback SRV chain (§4.2) → A/AAAA (§4.2 last resort).
//!
//! The implementation maps `hickory_resolver::error::ResolveError` to
//! [`ResolverError::Dns`] at the boundary so the dialog crate (and other
//! consumers) never need a transitive dep on hickory's error types.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::net::{DnsError, NetError};
use hickory_resolver::proto::rr::{RData, RecordType};
use hickory_resolver::TokioResolver;
use rvoip_sip_core::types::uri::{Host, Scheme};
use rvoip_sip_core::Uri;
use tracing::{debug, trace, warn};

use crate::resolver::srv::{
    default_port_for_scheme, expand_srv_priority_group, fallback_srv_chain, map_naptr_service,
    srv_service_name,
};
use crate::resolver::{select_transport_for_uri, ResolvedTarget, Resolver, ResolverError};
use crate::transport::TransportType;

/// `HickoryResolver` runs the full RFC 3263 §4 ladder against an
/// underlying `TokioAsyncResolver`. Construct with
/// [`HickoryResolver::new_system`] for production (`/etc/resolv.conf`
/// or the OS resolver) or [`HickoryResolver::with_resolver`] for tests
/// that need to point at a fixture DNS server.
pub struct HickoryResolver {
    inner: Arc<TokioResolver>,
}

impl std::fmt::Debug for HickoryResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HickoryResolver").finish_non_exhaustive()
    }
}

impl HickoryResolver {
    /// Build a resolver from the host system's DNS configuration
    /// (`/etc/resolv.conf` on Unix, OS-supplied resolver elsewhere).
    /// Forces EDNS0 on so large NAPTR responses don't get truncated.
    ///
    /// Returns `Err` if the system DNS config can't be loaded — typical
    /// in sandboxed CI without a resolv.conf. The dialog layer catches
    /// this and falls back to IP-literal-only resolution.
    pub fn new_system() -> Result<Self, ResolverError> {
        let (config, mut opts) = hickory_resolver::system_conf::read_system_conf()
            .map_err(|e| ResolverError::Dns(format!("system DNS config: {}", e)))?;
        opts.edns0 = true;
        let inner = build_tokio_resolver(config, opts)?;
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    /// Like [`new_system`](Self::new_system) but resilient to a slow or hung
    /// system DNS configuration read.
    ///
    /// `read_system_conf` can block for many seconds on a misconfigured or
    /// slow host (observed ~14s on macOS where a configured resolver is
    /// unreachable). This runs the config read on a blocking thread, caps it
    /// at `timeout`, and on timeout/failure falls back to hickory's default
    /// resolver config so the first resolution can never stall the caller.
    /// Always returns a usable resolver.
    pub async fn new_system_resilient(timeout: Duration) -> Self {
        let read = tokio::task::spawn_blocking(hickory_resolver::system_conf::read_system_conf);
        let (config, mut opts) = match tokio::time::timeout(timeout, read).await {
            Ok(Ok(Ok((config, opts)))) => (config, opts),
            Ok(Ok(Err(e))) => {
                warn!("system DNS config read failed ({e}); using default resolver config");
                (ResolverConfig::default(), ResolverOpts::default())
            }
            Ok(Err(join_err)) => {
                warn!("system DNS config read task failed ({join_err}); using default resolver config");
                (ResolverConfig::default(), ResolverOpts::default())
            }
            Err(_) => {
                warn!("system DNS config read exceeded {timeout:?}; using default resolver config");
                (ResolverConfig::default(), ResolverOpts::default())
            }
        };
        opts.edns0 = true;
        let inner = build_tokio_resolver(config, opts).unwrap_or_else(|e| {
            warn!("resolver build failed ({e}); rebuilding from default config");
            let mut default_opts = ResolverOpts::default();
            default_opts.edns0 = true;
            build_tokio_resolver(ResolverConfig::default(), default_opts)
                .expect("default Hickory resolver config must build")
        });
        Self {
            inner: Arc::new(inner),
        }
    }

    /// Build a resolver from explicit config + opts. Used by tests
    /// pointing at a local `hickory-server` fixture.
    pub fn with_resolver(config: ResolverConfig, mut opts: ResolverOpts) -> Self {
        opts.edns0 = true;
        let inner = build_tokio_resolver(config, opts)
            .expect("explicit Hickory resolver config should build");
        Self {
            inner: Arc::new(inner),
        }
    }

    /// Adopt an externally-built `TokioAsyncResolver`. Provided for the
    /// rare case a caller wants full control of hickory's config.
    pub fn from_tokio(inner: TokioResolver) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }

    async fn lookup_ip(&self, host: &str) -> Result<Vec<(IpAddr, Option<Instant>)>, ResolverError> {
        match self.inner.lookup_ip(host).await {
            Ok(lookup) => {
                let ttl_deadline = lookup_ttl_deadline(lookup.valid_until());
                Ok(lookup.iter().map(|ip| (ip, ttl_deadline)).collect())
            }
            Err(e) => {
                if is_no_records_error(&e) {
                    return Ok(Vec::new());
                }
                Err(map_resolve_err(e))
            }
        }
    }

    async fn lookup_srv(&self, name: &str) -> Result<Vec<(u16, u16, u16, String)>, ResolverError> {
        match self.inner.srv_lookup(name).await {
            Ok(lookup) => Ok(lookup
                .answers()
                .iter()
                .filter_map(|record| {
                    let RData::SRV(srv) = &record.data else {
                        return None;
                    };
                    Some((
                        srv.priority,
                        srv.weight,
                        srv.port,
                        srv.target.to_utf8().trim_end_matches('.').to_string(),
                    ))
                })
                .collect()),
            Err(e) => {
                if is_no_records_error(&e) {
                    return Ok(Vec::new());
                }
                Err(map_resolve_err(e))
            }
        }
    }

    async fn lookup_naptr(&self, host: &str) -> Result<Vec<NaptrRecord>, ResolverError> {
        // hickory's high-level `naptr_lookup` only exists in some
        // builds; use the generic `lookup()` with RecordType::NAPTR so
        // this compiles against the stable 0.24 surface.
        let lookup = match self.inner.lookup(host, RecordType::NAPTR).await {
            Ok(l) => l,
            Err(e) if is_no_records_error(&e) => return Ok(Vec::new()),
            Err(e) => return Err(map_resolve_err(e)),
        };

        let mut out = Vec::new();
        for record in lookup.answers() {
            let RData::NAPTR(rdata_naptr) = &record.data else {
                continue;
            };
            let flags = std::str::from_utf8(&rdata_naptr.flags)
                .unwrap_or("")
                .to_string();
            let service = std::str::from_utf8(&rdata_naptr.services)
                .unwrap_or("")
                .to_string();
            // RFC 3263 SIP NAPTRs use empty regexp + non-empty replacement.
            let replacement = rdata_naptr.replacement.to_utf8();
            let replacement = replacement.trim_end_matches('.').to_string();
            out.push(NaptrRecord {
                order: rdata_naptr.order,
                preference: rdata_naptr.preference,
                flags,
                service,
                replacement,
            });
        }
        Ok(out)
    }

    /// Resolve a host that is known to be a domain (not an IP literal).
    /// Returns the ordered candidate list per RFC 3263.
    async fn resolve_domain(
        &self,
        host: &str,
        uri: &Uri,
    ) -> Result<Vec<ResolvedTarget>, ResolverError> {
        // (2) Explicit port → A/AAAA only, skip NAPTR/SRV. (RFC 3263 §4.2)
        if let Some(port) = uri.port.filter(|p| *p > 0) {
            let transport = select_transport_for_uri(uri);
            // sips:host:port + transport=udp is still forbidden.
            if matches!(uri.scheme(), Scheme::Sips) && matches!(transport, TransportType::Udp) {
                return Err(ResolverError::Forbidden(
                    "sips: scheme cannot use transport=udp",
                ));
            }
            let ips = self.lookup_ip(host).await?;
            return Ok(ips
                .into_iter()
                .map(|(ip, expires)| ResolvedTarget {
                    addr: SocketAddr::new(ip, port),
                    transport,
                    expires,
                })
                .collect());
        }

        // (3) Has ;transport= or scheme is sips: → skip NAPTR, SRV-only.
        let transport_pinned = uri.transport().is_some() || matches!(uri.scheme(), Scheme::Sips);
        let transport_from_uri = select_transport_for_uri(uri);

        if transport_pinned {
            let Some(service) = srv_service_name(host, transport_from_uri, uri.scheme()) else {
                // sips: + transport=udp → forbidden.
                return Err(ResolverError::Forbidden(
                    "sips: scheme cannot use transport=udp",
                ));
            };
            let srv_records = self.lookup_srv(&service).await?;
            if !srv_records.is_empty() {
                let candidates = self
                    .expand_srv_records(&srv_records, transport_from_uri)
                    .await?;
                if !candidates.is_empty() {
                    return Ok(candidates);
                }
            }
            // Fallback to A/AAAA on host with scheme-default port.
            return self
                .a_fallback(
                    host,
                    transport_from_uri,
                    default_port_for_scheme(uri.scheme()),
                )
                .await;
        }

        // (4) Full NAPTR ladder.
        let naptr_records = self.lookup_naptr(host).await?;
        if !naptr_records.is_empty() {
            let mut usable: Vec<(u16, u16, TransportType, String)> = naptr_records
                .into_iter()
                .filter(|n| n.flags.eq_ignore_ascii_case("s"))
                .filter_map(|n| {
                    let transport = map_naptr_service(&n.service)?;
                    Some((n.order, n.preference, transport, n.replacement))
                })
                .collect();
            usable.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

            let mut out = Vec::new();
            for (_order, _pref, transport, replacement) in usable {
                let srv_records = self.lookup_srv(&replacement).await?;
                if !srv_records.is_empty() {
                    out.extend(self.expand_srv_records(&srv_records, transport).await?);
                }
            }
            if !out.is_empty() {
                return Ok(out);
            }
            debug!(
                "RFC 3263: NAPTR for {} returned records but no usable SRV; falling through",
                host
            );
        }

        // (5) NAPTR-less fallback: probe the well-known SRV chain.
        for (transport, label) in fallback_srv_chain(host) {
            let srv_records = self.lookup_srv(&label).await?;
            if !srv_records.is_empty() {
                let candidates = self.expand_srv_records(&srv_records, transport).await?;
                if !candidates.is_empty() {
                    return Ok(candidates);
                }
            }
        }

        // (6) Last-resort A/AAAA.
        self.a_fallback(
            host,
            transport_from_uri,
            default_port_for_scheme(uri.scheme()),
        )
        .await
    }

    async fn a_fallback(
        &self,
        host: &str,
        transport: TransportType,
        port: u16,
    ) -> Result<Vec<ResolvedTarget>, ResolverError> {
        let ips = self.lookup_ip(host).await?;
        if ips.is_empty() {
            return Err(ResolverError::NoCandidates);
        }
        Ok(ips
            .into_iter()
            .map(|(ip, expires)| ResolvedTarget {
                addr: SocketAddr::new(ip, port),
                transport,
                expires,
            })
            .collect())
    }

    async fn expand_srv_records(
        &self,
        records: &[(u16, u16, u16, String)],
        transport: TransportType,
    ) -> Result<Vec<ResolvedTarget>, ResolverError> {
        let ordered = expand_srv_priority_group(records, fastrand::f64);
        let mut out = Vec::with_capacity(ordered.len());
        for (_prio, _weight, port, target) in ordered {
            let ips = self.lookup_ip(&target).await?;
            if ips.is_empty() {
                trace!("RFC 3263: SRV target {} produced no A/AAAA records", target);
                continue;
            }
            for (ip, expires) in ips {
                out.push(ResolvedTarget {
                    addr: SocketAddr::new(ip, port),
                    transport,
                    expires,
                });
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl Resolver for HickoryResolver {
    async fn resolve(&self, uri: &Uri) -> Result<Vec<ResolvedTarget>, ResolverError> {
        // (1) IP literal short-circuit — no DNS.
        let transport = select_transport_for_uri(uri);
        let default_port = default_port_for_scheme(uri.scheme());

        match &uri.host {
            Host::Address(ip) => {
                // Validate sips: + transport=udp combo even for IP literals.
                if matches!(uri.scheme(), Scheme::Sips) && matches!(transport, TransportType::Udp) {
                    return Err(ResolverError::Forbidden(
                        "sips: scheme cannot use transport=udp",
                    ));
                }
                let port = uri.port.filter(|p| *p > 0).unwrap_or(default_port);
                Ok(vec![ResolvedTarget::immediate(
                    SocketAddr::new(*ip, port),
                    transport,
                )])
            }
            Host::Domain(domain) => {
                // A request URI whose host is empty, or a bare all-numeric
                // label like `sip:600`, is almost always a misaddressed
                // extension — the caller meant `sip:600@registrar`. Neither is
                // DNS-resolvable; fail early with a routing hint instead of a
                // cryptic NAPTR/SRV/A ladder error. (All-numeric single labels
                // are not valid hostnames, so this won't reject real domains
                // like `sip:pbx` or `sip:host.example`.)
                if domain.is_empty() {
                    return Err(ResolverError::InvalidHost(
                        "request URI has no host".to_string(),
                    ));
                }
                if domain.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(ResolverError::InvalidHost(format!(
                        "request URI host `{domain}` is not routable; address it \
                         to the registrar, e.g. `sip:{domain}@your-pbx`"
                    )));
                }
                self.resolve_domain(domain, uri).await
            }
        }
    }
}

/// Internal NAPTR record shape — translated from hickory's RData.
struct NaptrRecord {
    order: u16,
    preference: u16,
    flags: String,
    service: String,
    replacement: String,
}

fn lookup_ttl_deadline(valid_until: Instant) -> Option<Instant> {
    let now = Instant::now();
    if valid_until <= now {
        return None;
    }
    // Hickory occasionally surfaces effectively-infinite deadlines for
    // static / synthesised records. Anything beyond 30 days is treated
    // as "no useful TTL".
    if valid_until - now > Duration::from_secs(60 * 60 * 24 * 30) {
        return None;
    }
    Some(valid_until)
}

fn build_tokio_resolver(
    config: ResolverConfig,
    opts: ResolverOpts,
) -> Result<TokioResolver, ResolverError> {
    let mut builder = TokioResolver::builder_with_config(config, TokioRuntimeProvider::default());
    *builder.options_mut() = opts;
    builder
        .build()
        .map_err(|e| ResolverError::Dns(format!("hickory resolver config: {}", e)))
}

fn map_resolve_err(e: NetError) -> ResolverError {
    warn!("hickory resolve error: {}", e);
    ResolverError::Dns(format!("{}", e))
}

fn is_no_records_error(e: &NetError) -> bool {
    matches!(e, NetError::Dns(DnsError::NoRecordsFound(_)))
}

#[cfg(test)]
mod tests {
    //! Tests that exercise the parts of `HickoryResolver` that don't
    //! require a DNS server — IP-literal short-circuits and the
    //! `sips:` / `transport=udp` rejection. Full NAPTR/SRV/A coverage
    //! against a live hickory client lives in
    //! `crates/sip/rvoip-sip-transport/tests/resolver_hickory_e2e.rs`.

    use super::*;
    use hickory_resolver::config::{NameServerConfig, ResolverConfig};
    use std::str::FromStr;

    fn empty_resolver() -> HickoryResolver {
        // Configure a resolver pointed at a port that nothing answers on.
        // We never actually issue DNS queries against it for the
        // IP-literal / forbidden tests — the short-circuit paths return
        // before hitting hickory.
        let ns = vec![NameServerConfig::udp("127.0.0.1".parse().unwrap())];
        let config = ResolverConfig::from_parts(None, vec![], ns);
        HickoryResolver::with_resolver(config, ResolverOpts::default())
    }

    #[tokio::test]
    async fn ip_literal_sip_default_port() {
        let resolver = empty_resolver();
        let uri = Uri::from_str("sip:1.2.3.4").unwrap();
        let candidates = resolver.resolve(&uri).await.unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].addr.to_string(), "1.2.3.4:5060");
        assert_eq!(candidates[0].transport, TransportType::Udp);
        assert!(candidates[0].expires.is_none());
    }

    #[tokio::test]
    async fn ip_literal_sips_default_port() {
        let resolver = empty_resolver();
        let uri = Uri::from_str("sips:1.2.3.4").unwrap();
        let candidates = resolver.resolve(&uri).await.unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].addr.to_string(), "1.2.3.4:5061");
        assert_eq!(candidates[0].transport, TransportType::Tls);
    }

    #[tokio::test]
    async fn ip_literal_explicit_port_wins() {
        let resolver = empty_resolver();
        let uri = Uri::from_str("sip:1.2.3.4:12345").unwrap();
        let candidates = resolver.resolve(&uri).await.unwrap();
        assert_eq!(candidates[0].addr.to_string(), "1.2.3.4:12345");
    }

    #[tokio::test]
    async fn ip_literal_transport_param_wins() {
        let resolver = empty_resolver();
        let uri = Uri::from_str("sip:1.2.3.4;transport=tcp").unwrap();
        let candidates = resolver.resolve(&uri).await.unwrap();
        assert_eq!(candidates[0].transport, TransportType::Tcp);
    }

    #[tokio::test]
    async fn sips_with_transport_udp_is_forbidden_for_ip_literal() {
        let resolver = empty_resolver();
        // `sips:` pinned to UDP. Even though `select_transport_for_uri`
        // returns TLS for `sips:` by default, an explicit `transport=udp`
        // overrides it and the resolver must reject.
        let uri = Uri::from_str("sips:1.2.3.4;transport=udp").unwrap();
        let err = resolver.resolve(&uri).await.unwrap_err();
        assert!(matches!(err, ResolverError::Forbidden(_)));
    }

    #[tokio::test]
    async fn bare_extension_host_is_invalid() {
        // `sip:600` parses with host `600` — a misaddressed extension, not a
        // routable host. The all-numeric check returns InvalidHost *before* any
        // DNS query (so this passes against the dead-port resolver), turning a
        // cryptic NAPTR/SRV/A failure into a clear routing error.
        let resolver = empty_resolver();
        let uri = Uri::from_str("sip:600").unwrap();
        let err = resolver.resolve(&uri).await.unwrap_err();
        assert!(matches!(err, ResolverError::InvalidHost(_)));
    }
}

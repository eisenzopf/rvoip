//! Pure RFC 3263 / RFC 2782 helpers. No I/O, no DNS client deps. Any
//! [`crate::resolver::Resolver`] impl can build on these.

use rvoip_sip_core::types::uri::Scheme;

use crate::transport::TransportType;

/// RFC 3261 §19.1.2 — default port by scheme.
pub fn default_port_for_scheme(scheme: &Scheme) -> u16 {
    match scheme {
        Scheme::Sips => 5061,
        _ => 5060,
    }
}

/// RFC 3263 §4.1 service label for `_service._proto.host` SRV lookups.
///
/// Returns `None` when the (scheme, transport) combination is invalid
/// per RFC 3263 §4.2 — specifically `sips:` paired with `;transport=udp`
/// (TLS-capable transport is mandatory for the SIPS scheme).
///
/// Examples:
/// - `(Sip, Udp)` → `_sip._udp.example.com`
/// - `(Sip, Tcp)` → `_sip._tcp.example.com`
/// - `(Sip, Tls)` → `_sips._tcp.example.com` (TLS travels over TCP transport)
/// - `(Sips, Tls/Tcp)` → `_sips._tcp.example.com`
/// - `(Sip, Ws)` → `_sip._ws.example.com`   (RFC 7118)
/// - `(Sip, Wss)` → `_sips._wss.example.com` (RFC 7118)
/// - `(Sips, Udp)` → `None`  (forbidden by RFC 3263 §4.2)
pub fn srv_service_name(host: &str, transport: TransportType, scheme: &Scheme) -> Option<String> {
    // sips: pinned to a TLS-capable transport. UDP is forbidden — caller
    // should map this to ResolverError::Forbidden, not silently coerce.
    if matches!(scheme, Scheme::Sips) && matches!(transport, TransportType::Udp) {
        return None;
    }

    let (service, proto) = match (scheme, transport) {
        // `sips:` URIs travel over TLS-over-TCP regardless of any
        // ;transport= hint other than the WebSocket variants below.
        (Scheme::Sips, TransportType::Ws) => ("_sip", "_ws"),
        (Scheme::Sips, TransportType::Wss) => ("_sips", "_wss"),
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

/// Pure SRV selection per RFC 2782: within the lowest-priority group,
/// pick a record weighted by its `weight` field. Given
/// `(priority, weight, port, target)` tuples, returns the selected
/// `(priority, weight, port, target)`.
///
/// RFC 2782 weighted selection:
/// 1. Filter to the lowest priority value.
/// 2. For each entry in weight order, assign a running cumulative weight.
/// 3. Pick a uniformly random value in `[0, total_weight)`; the first
///    entry whose running sum > the picked value wins.
///
/// Zero-weight entries are special-cased per RFC: they participate but
/// are sorted first within the group so they're only picked when no
/// non-zero entry would win the dice roll.
pub fn select_srv_best<'a>(
    records: &'a [(u16, u16, u16, String)],
    rand_0_1: f64,
) -> Option<&'a (u16, u16, u16, String)> {
    if records.is_empty() {
        return None;
    }
    let min_priority = records.iter().map(|r| r.0).min()?;
    let mut group: Vec<&(u16, u16, u16, String)> =
        records.iter().filter(|r| r.0 == min_priority).collect();

    group.sort_by(|a, b| match (a.1 == 0, b.1 == 0) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.1.cmp(&b.1),
    });

    let total_weight: u32 = group.iter().map(|r| r.1 as u32).sum();
    if total_weight == 0 {
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
    group.first().copied()
}

/// Walk an entire priority group in weighted-random order, returning a
/// `Vec` of all records within the lowest priority. Used by resolvers
/// that need to surface every candidate inside the top priority for
/// RFC 3263 §4.3 failover, not just the single winner of one dice roll.
///
/// `rand_fn` is invoked once per remaining entry. Tests can pin it to a
/// deterministic sequence; production passes `fastrand::f64`.
pub fn expand_srv_priority_group(
    records: &[(u16, u16, u16, String)],
    mut rand_fn: impl FnMut() -> f64,
) -> Vec<(u16, u16, u16, String)> {
    if records.is_empty() {
        return Vec::new();
    }
    let Some(min_priority) = records.iter().map(|r| r.0).min() else {
        return Vec::new();
    };
    let mut group: Vec<(u16, u16, u16, String)> = records
        .iter()
        .filter(|r| r.0 == min_priority)
        .cloned()
        .collect();
    let mut ordered = Vec::with_capacity(group.len());
    while !group.is_empty() {
        let pick = rand_fn();
        let chosen_idx = {
            // Re-run RFC 2782 weighted selection against the current
            // working set, then remove the winner.
            group.sort_by(|a, b| match (a.1 == 0, b.1 == 0) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.1.cmp(&b.1),
            });
            let total: u32 = group.iter().map(|r| r.1 as u32).sum();
            if total == 0 {
                0
            } else {
                let picked = (pick * total as f64).floor() as u32;
                let picked = picked.min(total.saturating_sub(1));
                let mut running: u32 = 0;
                let mut found = 0;
                for (i, rec) in group.iter().enumerate() {
                    running += rec.1 as u32;
                    if running > picked {
                        found = i;
                        break;
                    }
                }
                found
            }
        };
        ordered.push(group.remove(chosen_idx));
    }
    // After the lowest-priority group, append the rest in (priority,
    // weight) order so callers still have something to try when the top
    // group all fails.
    let mut rest: Vec<(u16, u16, u16, String)> = records
        .iter()
        .filter(|r| r.0 != min_priority)
        .cloned()
        .collect();
    rest.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    ordered.extend(rest);
    ordered
}

/// RFC 3263 §4.1 NAPTR service tokens we recognise. Tokens not in this
/// set (e.g. `SIP+D2S` SCTP, exotic vendor strings) are dropped — the
/// caller falls through to the SRV-only chain.
///
/// Returns the SIP transport flavour and the SRV protocol label that the
/// NAPTR replacement is expected to target. Matches the labels in
/// [`srv_service_name`].
pub fn map_naptr_service(service: &str) -> Option<TransportType> {
    match service.trim().to_ascii_uppercase().as_str() {
        "SIP+D2U" => Some(TransportType::Udp),
        "SIP+D2T" => Some(TransportType::Tcp),
        "SIPS+D2T" => Some(TransportType::Tls),
        "SIP+D2W" => Some(TransportType::Ws),
        "SIPS+D2W" => Some(TransportType::Wss),
        _ => None,
    }
}

/// The well-known SRV service labels in RFC 3263 §4.2 fallback order
/// (most-preferred first). Used when NAPTR is empty / unusable and the
/// resolver needs to probe SRV directly.
pub fn fallback_srv_chain(host: &str) -> [(TransportType, String); 3] {
    [
        (TransportType::Tls, format!("_sips._tcp.{}", host)),
        (TransportType::Tcp, format!("_sip._tcp.{}", host)),
        (TransportType::Udp, format!("_sip._udp.{}", host)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::types::uri::Scheme;

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
        let name = srv_service_name("example.com", TransportType::Tls, &Scheme::Sip).unwrap();
        assert_eq!(name, "_sips._tcp.example.com");
    }

    #[test]
    fn srv_service_name_sips_with_tls_uses_sips_tcp() {
        let name = srv_service_name("example.com", TransportType::Tls, &Scheme::Sips).unwrap();
        assert_eq!(name, "_sips._tcp.example.com");
    }

    #[test]
    fn srv_service_name_sips_udp_is_forbidden() {
        // RFC 3263 §4.2: sips: + transport=udp is invalid.
        let name = srv_service_name("example.com", TransportType::Udp, &Scheme::Sips);
        assert!(name.is_none());
    }

    #[test]
    fn srv_service_name_ws_variants() {
        let plain = srv_service_name("example.com", TransportType::Ws, &Scheme::Sip).unwrap();
        assert_eq!(plain, "_sip._ws.example.com");
        let secure = srv_service_name("example.com", TransportType::Wss, &Scheme::Sip).unwrap();
        assert_eq!(secure, "_sips._wss.example.com");
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

    #[test]
    fn expand_srv_priority_group_emits_every_entry() {
        let records = vec![
            (1, 50, 5060, "a.example.com".into()),
            (1, 50, 5060, "b.example.com".into()),
            (2, 10, 5060, "backup.example.com".into()),
        ];
        let mut seq = [0.1, 0.9].into_iter().cycle();
        let out = expand_srv_priority_group(&records, || seq.next().unwrap());
        assert_eq!(out.len(), 3);
        // Backup comes last (higher priority value).
        assert_eq!(out.last().unwrap().3, "backup.example.com");
        // The two priority-1 entries appear before the backup.
        let prio_one: Vec<&String> = out.iter().take(2).map(|r| &r.3).collect();
        assert!(prio_one.iter().any(|s| s.as_str() == "a.example.com"));
        assert!(prio_one.iter().any(|s| s.as_str() == "b.example.com"));
    }

    #[test]
    fn map_naptr_service_recognises_all_sip_tokens() {
        assert_eq!(map_naptr_service("SIP+D2U"), Some(TransportType::Udp));
        assert_eq!(map_naptr_service("SIP+D2T"), Some(TransportType::Tcp));
        assert_eq!(map_naptr_service("SIPS+D2T"), Some(TransportType::Tls));
        assert_eq!(map_naptr_service("SIP+D2W"), Some(TransportType::Ws));
        assert_eq!(map_naptr_service("SIPS+D2W"), Some(TransportType::Wss));
        // Case-insensitive (per RFC 3263 §4.1 NAPTR services tokens are
        // ASCII case-insensitive).
        assert_eq!(map_naptr_service("sip+d2u"), Some(TransportType::Udp));
        // SCTP and unknown tokens are dropped.
        assert_eq!(map_naptr_service("SIP+D2S"), None);
        assert_eq!(map_naptr_service("garbage"), None);
    }

    #[test]
    fn fallback_srv_chain_orders_sips_first() {
        let chain = fallback_srv_chain("example.com");
        assert_eq!(chain[0].0, TransportType::Tls);
        assert_eq!(chain[0].1, "_sips._tcp.example.com");
        assert_eq!(chain[1].0, TransportType::Tcp);
        assert_eq!(chain[1].1, "_sip._tcp.example.com");
        assert_eq!(chain[2].0, TransportType::Udp);
        assert_eq!(chain[2].1, "_sip._udp.example.com");
    }
}

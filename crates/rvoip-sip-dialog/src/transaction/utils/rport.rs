//! RFC 3581 §4 — server-side `received=` / `rport=` Via stamping.
//!
//! When a UAC sends a request through NAT, the source IP/port the
//! request *appears* to come from differs from the address baked into
//! the top `Via` (which carries the UAC's own internal address). RFC
//! 3261 §18.2.1 already requires servers to add `received=<src_ip>`
//! when the Via host disagrees with the packet's source IP.
//! RFC 3581 extends this: when the UAC includes `;rport` (with no
//! value) on the top Via, the server MUST echo back `rport=<src_port>`
//! plus `received=<src_ip>` regardless of whether the Via host matches.
//!
//! These parameters let the UAC discover its public-facing address
//! and direct subsequent responses / dialog targeting through the same
//! NAT binding.
//!
//! ## Use
//!
//! ```ignore
//! use rvoip_sip_dialog::transaction::utils::rport::stamp_received_rport;
//! // After response_from_request, before serialization:
//! if let Some(via_header) = response.first_via_mut() {
//!     stamp_received_rport(via_header, source_addr);
//! }
//! ```
//!
//! The helper is conservative: it only stamps when the inbound Via
//! actually carried `;rport` (the RFC 3581 opt-in marker) OR when the
//! source IP differs from the Via host (the RFC 3261 §18.2.1
//! always-stamp condition).

use rvoip_sip_core::types::via::Via;
use std::net::SocketAddr;

/// Stamp the top `Via` header with RFC 3581 `received=` / `rport=`
/// parameters reflecting the inbound packet's source address.
///
/// Behaviour:
/// - **Always** sets `received=<source.ip>`. RFC 3261 §18.2.1 requires
///   this when the Via `sent-by` host differs from the source IP; we
///   set it unconditionally because (a) the comparison is expensive
///   for hostnames that would require DNS, and (b) setting it when
///   IPs match is harmless — the UAC ignores it.
/// - **Sets `rport=<source.port>` only when the inbound Via carried
///   `;rport`** (with or without a value). Per RFC 3581 §4, `rport`
///   on responses is opt-in; setting it when the UAC didn't ask for
///   it would surprise legacy stacks that key dialog state on the
///   absence of the parameter.
///
/// Returns `true` when at least one parameter was added or replaced.
pub fn stamp_received_rport(via: &mut Via, source: SocketAddr) -> bool {
    let inbound_rport_present = via.rport().is_some();
    let mut changed = false;

    via.set_received(source.ip());
    changed = true;

    if inbound_rport_present {
        via.set_rport(Some(source.port()));
    }

    changed
}

/// Convenience wrapper that locates the top `Via` header on an
/// outbound `Response` and applies [`stamp_received_rport`] to it.
///
/// No-op when the response has no `Via` header (malformed; the
/// transport will reject it anyway). The mutation is performed
/// in-place on the `TypedHeader::Via` entry already stored in
/// `response.headers`.
pub fn stamp_response_via_with_source(
    response: &mut rvoip_sip_core::Response,
    source: SocketAddr,
) -> bool {
    use rvoip_sip_core::types::TypedHeader;
    for header in response.headers.iter_mut() {
        if let TypedHeader::Via(via) = header {
            return stamp_received_rport(via, source);
        }
    }
    false
}

#[cfg(test)]
mod response_wrapper_tests {
    use super::*;
    use rvoip_sip_core::types::param::Param;
    use rvoip_sip_core::types::{StatusCode, TypedHeader};
    use rvoip_sip_core::Response;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn response_with_via_carrying_rport() -> Response {
        let via = Via::new(
            "SIP", "2.0", "UDP",
            "10.0.0.5", Some(5060),
            vec![Param::branch("z9hG4bKtest"), Param::Rport(None)],
        )
        .expect("valid Via");
        Response::new(StatusCode::Ok).with_header(TypedHeader::Via(via))
    }

    #[test]
    fn wrapper_stamps_top_via_on_response() {
        let mut response = response_with_via_carrying_rport();
        let source = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)), 33000);

        assert!(stamp_response_via_with_source(&mut response, source));

        let via = response.first_via().expect("via present");
        assert_eq!(via.received(), Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7))));
        assert_eq!(via.rport(), Some(Some(33000)));
    }

    #[test]
    fn wrapper_is_noop_when_no_via_present() {
        let mut response = Response::new(StatusCode::Ok);
        let source = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)), 5060);
        assert!(!stamp_response_via_with_source(&mut response, source));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn make_via_with_rport_flag() -> Via {
        // Mimic what a UAC behind NAT puts on the wire:
        // `Via: SIP/2.0/UDP 10.0.0.5:5060;branch=z9hG4bKtest;rport`
        Via::new(
            "SIP", "2.0", "UDP",
            "10.0.0.5", Some(5060),
            vec![
                rvoip_sip_core::types::param::Param::branch("z9hG4bKtest"),
                rvoip_sip_core::types::param::Param::Rport(None),
            ],
        )
        .expect("valid Via")
    }

    fn make_via_without_rport() -> Via {
        Via::new(
            "SIP", "2.0", "UDP",
            "10.0.0.5", Some(5060),
            vec![rvoip_sip_core::types::param::Param::branch("z9hG4bKtest")],
        )
        .expect("valid Via")
    }

    #[test]
    fn stamps_received_and_rport_when_inbound_via_had_rport() {
        let mut via = make_via_with_rport_flag();
        let source = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)), 33000);

        assert!(stamp_received_rport(&mut via, source));

        assert_eq!(via.received(), Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7))));
        // RFC 3581: `;rport` with no value becomes `;rport=<src_port>`.
        assert_eq!(via.rport(), Some(Some(33000)));
    }

    #[test]
    fn stamps_only_received_when_inbound_via_had_no_rport() {
        let mut via = make_via_without_rport();
        let source = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)), 33000);

        assert!(stamp_received_rport(&mut via, source));

        assert_eq!(via.received(), Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7))));
        // No rport on inbound → no rport on response (RFC 3581 opt-in).
        assert_eq!(via.rport(), None);
    }

    #[test]
    fn idempotent_when_called_twice() {
        let mut via = make_via_with_rport_flag();
        let source = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)), 33000);

        stamp_received_rport(&mut via, source);
        stamp_received_rport(&mut via, source);

        // No duplicate parameters.
        let received_count = via
            .headers()
            .iter()
            .flat_map(|h| &h.params)
            .filter(|p| matches!(p, rvoip_sip_core::types::param::Param::Received(_)))
            .count();
        assert_eq!(received_count, 1);
    }

    #[test]
    fn second_call_with_different_source_overwrites() {
        let mut via = make_via_with_rport_flag();
        let first = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)), 33000);
        let second = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)), 44000);

        stamp_received_rport(&mut via, first);
        stamp_received_rport(&mut via, second);

        assert_eq!(via.received(), Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9))));
        assert_eq!(via.rport(), Some(Some(44000)));
    }

    #[test]
    fn stamps_ipv6_source_correctly() {
        use std::net::Ipv6Addr;
        let mut via = make_via_with_rport_flag();
        let source = SocketAddr::new(
            IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
            44000,
        );

        stamp_received_rport(&mut via, source);

        match via.received() {
            Some(IpAddr::V6(addr)) => {
                assert_eq!(addr, Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));
            }
            other => panic!("expected IPv6 received, got {:?}", other),
        }
        assert_eq!(via.rport(), Some(Some(44000)));
    }
}

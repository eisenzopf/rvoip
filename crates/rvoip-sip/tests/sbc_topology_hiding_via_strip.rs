//! Phase 8 acceptance — SBC topology hiding helpers.
//!
//! Demonstrates the wire-form effect of [`strip_via_below_top`] and
//! [`strip_record_route_below_self`] on an inbound INVITE that
//! traversed two upstream proxies before reaching the SBC. After the
//! helpers run, the downstream peer should see only the SBC's own
//! Via and Record-Route — upstream hop identities are hidden.
//!
//! These helpers are used by code that mutates the inbound Request
//! in-place before forwarding (proxy-style on top of
//! `Transport::send_message_raw`). The default B2BUA pattern in this
//! codebase — `coord.invite(...)` + `with_headers_from(&call, ...)`
//! + `send()` — builds a *fresh* outbound INVITE with the SBC's own
//! Via stamped from scratch, so it never needs to strip in the
//! first place. The helpers cover the "forward existing Request"
//! shape that Phase 8.5 stateless-proxy work will lean on.

use rvoip_sip::adapters::{strip_record_route_below_self, strip_via_below_top};
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::address::Address;
use rvoip_sip_core::types::param::Param;
use rvoip_sip_core::types::record_route::{RecordRoute, RecordRouteEntry};
use rvoip_sip_core::types::via::Via;
use rvoip_sip_core::types::TypedHeader;
use rvoip_sip_core::Method;

fn upstream_invite_three_vias_two_proxy_record_routes() -> rvoip_sip_core::Request {
    SimpleRequestBuilder::new(Method::Invite, "sip:bob@downstream.example.com")
        .unwrap()
        .from("Alice", "sip:alice@uac.example.com", Some("alicetag"))
        .to("Bob", "sip:bob@downstream.example.com", None)
        .call_id("topology-hiding-test")
        .cseq(1)
        // Top Via — SBC (the immediate sender of the inbound the SBC
        // received). In production this is what the SBC stamped on
        // its own listener-side egress.
        .header(TypedHeader::Via(
            Via::new(
                "SIP", "2.0", "UDP",
                "sbc.example.com",
                Some(5060),
                vec![Param::branch("z9hG4bKsbc")],
            )
            .unwrap(),
        ))
        // Second Via — internal proxy.
        .header(TypedHeader::Via(
            Via::new(
                "SIP", "2.0", "UDP",
                "proxy.internal.example.com",
                Some(5060),
                vec![Param::branch("z9hG4bKproxy")],
            )
            .unwrap(),
        ))
        // Third Via — the UAC.
        .header(TypedHeader::Via(
            Via::new(
                "SIP", "2.0", "UDP",
                "10.0.0.5",
                Some(5060),
                vec![Param::branch("z9hG4bKuac")],
            )
            .unwrap(),
        ))
        // Record-Route stack — SBC inserted last (so it's first in
        // the header list per RFC 3261 §16.6), then two upstream
        // proxies whose entries the SBC wants to hide.
        .header(TypedHeader::RecordRoute(RecordRoute::new(vec![
            RecordRouteEntry::new(Address::new(
                "sip:sbc.example.com;lr".parse().unwrap(),
            )),
        ])))
        .header(TypedHeader::RecordRoute(RecordRoute::new(vec![
            RecordRouteEntry::new(Address::new(
                "sip:proxy.internal.example.com;lr".parse().unwrap(),
            )),
            RecordRouteEntry::new(Address::new(
                "sip:edge.internal.example.com;lr".parse().unwrap(),
            )),
        ])))
        .build()
}

#[test]
fn strip_via_below_top_keeps_only_topmost_via() {
    let mut request = upstream_invite_three_vias_two_proxy_record_routes();

    let before = request
        .headers
        .iter()
        .filter(|h| matches!(h, TypedHeader::Via(_)))
        .count();
    assert_eq!(before, 3, "fixture should start with three Via headers");

    let removed = strip_via_below_top(&mut request);
    assert_eq!(removed, 2, "should have removed two of three Via headers");

    let after = request
        .headers
        .iter()
        .filter(|h| matches!(h, TypedHeader::Via(_)))
        .count();
    assert_eq!(after, 1, "exactly one Via should remain");

    // The remaining Via must be the topmost (SBC's own).
    let remaining_via = request.first_via().expect("via present");
    let host = remaining_via
        .headers()
        .first()
        .expect("inner header")
        .sent_by_host
        .to_string();
    assert_eq!(
        host, "sbc.example.com",
        "remaining Via should be SBC's own (the topmost)"
    );
}

#[test]
fn strip_record_route_below_self_keeps_only_sbc_entries() {
    let mut request = upstream_invite_three_vias_two_proxy_record_routes();

    // Count Record-Route entries across all RR headers.
    let count_entries = |req: &rvoip_sip_core::Request| -> usize {
        req.headers
            .iter()
            .filter_map(|h| {
                if let TypedHeader::RecordRoute(rr) = h {
                    Some(rr.0.len())
                } else {
                    None
                }
            })
            .sum()
    };

    assert_eq!(
        count_entries(&request),
        3,
        "fixture should start with 1 SBC + 2 upstream Record-Route entries"
    );

    let removed = strip_record_route_below_self(&mut request, "sbc.example.com");
    assert_eq!(removed, 2, "should have removed the two upstream entries");

    assert_eq!(
        count_entries(&request),
        1,
        "exactly one Record-Route entry should remain (SBC's own)"
    );

    // Confirm the remaining entry is the SBC's.
    let remaining = request
        .headers
        .iter()
        .find_map(|h| {
            if let TypedHeader::RecordRoute(rr) = h {
                rr.0.first().cloned()
            } else {
                None
            }
        })
        .expect("RR entry present");
    let host = remaining.0.uri.host.to_string();
    assert_eq!(host, "sbc.example.com");
}

#[test]
fn strip_record_route_drops_empty_headers() {
    // After stripping all entries from the upstream-only RR header,
    // the helper should remove the now-empty header entirely so the
    // wire form doesn't carry `Record-Route: ` (which some parsers
    // reject).
    let mut request = upstream_invite_three_vias_two_proxy_record_routes();

    let before_headers = request
        .headers
        .iter()
        .filter(|h| matches!(h, TypedHeader::RecordRoute(_)))
        .count();
    assert_eq!(before_headers, 2, "fixture has two RR header entries");

    strip_record_route_below_self(&mut request, "sbc.example.com");

    let after_headers = request
        .headers
        .iter()
        .filter(|h| matches!(h, TypedHeader::RecordRoute(_)))
        .count();
    assert_eq!(
        after_headers, 1,
        "upstream-only RR header should have been dropped entirely"
    );
}

#[test]
fn strip_helpers_combined_yield_topology_hidden_wire_form() {
    let mut request = upstream_invite_three_vias_two_proxy_record_routes();

    strip_via_below_top(&mut request);
    strip_record_route_below_self(&mut request, "sbc.example.com");

    let wire = request.to_string();

    // Positive assertions — SBC's own identity survives.
    assert!(wire.contains("sbc.example.com"), "SBC Via/RR missing: {}", wire);

    // Negative assertions — upstream identities are hidden.
    assert!(
        !wire.contains("proxy.internal.example.com"),
        "internal proxy host leaked: {}",
        wire
    );
    assert!(
        !wire.contains("edge.internal.example.com"),
        "edge proxy host leaked: {}",
        wire
    );
    assert!(
        !wire.contains("10.0.0.5"),
        "UAC internal IP leaked: {}",
        wire
    );
}

#[test]
fn strip_via_is_noop_when_only_one_via_present() {
    let mut request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@server.example.com")
        .unwrap()
        .from("Alice", "sip:alice@uac.example.com", Some("alicetag"))
        .to("Bob", "sip:bob@server.example.com", None)
        .call_id("single-via")
        .cseq(1)
        .header(TypedHeader::Via(
            Via::new(
                "SIP", "2.0", "UDP",
                "uac.example.com",
                Some(5060),
                vec![Param::branch("z9hG4bKsingle")],
            )
            .unwrap(),
        ))
        .build();

    let removed = strip_via_below_top(&mut request);
    assert_eq!(removed, 0);

    let via_count = request
        .headers
        .iter()
        .filter(|h| matches!(h, TypedHeader::Via(_)))
        .count();
    assert_eq!(via_count, 1);
}

#[test]
fn strip_record_route_self_match_is_case_insensitive() {
    let mut request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@server.example.com")
        .unwrap()
        .from("Alice", "sip:alice@uac.example.com", Some("alicetag"))
        .to("Bob", "sip:bob@server.example.com", None)
        .call_id("case-insensitive-test")
        .cseq(1)
        .header(TypedHeader::RecordRoute(RecordRoute::new(vec![
            RecordRouteEntry::new(Address::new(
                "sip:SBC.Example.Com;lr".parse().unwrap(),
            )),
            RecordRouteEntry::new(Address::new(
                "sip:proxy.upstream.example.com;lr".parse().unwrap(),
            )),
        ])))
        .build();

    // Pass self_host in lowercase; SBC entry uses mixed case.
    let removed = strip_record_route_below_self(&mut request, "sbc.example.com");
    assert_eq!(removed, 1, "upstream proxy entry should be removed");

    let remaining_entries: Vec<String> = request
        .headers
        .iter()
        .filter_map(|h| {
            if let TypedHeader::RecordRoute(rr) = h {
                Some(rr.0.iter().map(|e| e.0.uri.host.to_string()).collect::<Vec<_>>())
            } else {
                None
            }
        })
        .flatten()
        .collect();
    assert_eq!(remaining_entries.len(), 1);
    // Original case preserved.
    assert!(
        remaining_entries[0].eq_ignore_ascii_case("SBC.Example.Com"),
        "remaining entry should be SBC's, got: {:?}",
        remaining_entries
    );
}

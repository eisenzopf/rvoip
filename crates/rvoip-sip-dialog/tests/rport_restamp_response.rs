//! RFC 3581 §4 acceptance test — server-side `received=` / `rport=`
//! restamping on outbound responses.
//!
//! Simulates a NAT'd UAC: the inbound INVITE's top `Via` carries the
//! UAC's INTERNAL address (`10.0.0.5:5060`) and `;rport` (no value),
//! but the packet arrives at the server with a different source IP/port
//! (`198.51.100.7:33000` — the NAT-translated address).
//!
//! Per RFC 3581 §4, the server's response MUST echo back the
//! observed source as `received=198.51.100.7;rport=33000` on the top
//! Via, so the UAC can:
//! - Discover its public-facing address (NAT learning).
//! - Re-use the same NAT binding for subsequent in-dialog messages.
//!
//! The test exercises the same code path the production
//! `ServerInviteTransaction::send_response` / `ServerNonInviteTransaction::send_response`
//! follow: build the response via `response_from_request` (canonical
//! header carry-over per RFC 3261 §8.2.6.2), then apply the
//! `stamp_response_via_with_source` helper.

use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::types::param::Param;
use rvoip_sip_core::types::via::Via;
use rvoip_sip_core::types::TypedHeader;
use rvoip_sip_core::{Method, StatusCode};
use rvoip_sip_dialog::transaction::utils::stamp_response_via_with_source;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// Build an INVITE whose top Via carries `;rport` (the RFC 3581 opt-in
/// marker) plus the UAC's internal address (the address the UAC THINKS
/// it has, not the post-NAT address).
fn nat_uac_invite() -> rvoip_sip_core::Request {
    SimpleRequestBuilder::new(Method::Invite, "sip:bob@server.example.com")
        .unwrap()
        .from("Alice", "sip:alice@uac.example.com", Some("alicetag"))
        .to("Bob", "sip:bob@server.example.com", None)
        .call_id("rport-restamp-test")
        .cseq(1)
        .header(TypedHeader::Via(
            Via::new(
                "SIP",
                "2.0",
                "UDP",
                // UAC's INTERNAL address — what's in the Via sent-by.
                "10.0.0.5",
                Some(5060),
                vec![
                    Param::branch("z9hG4bKnat-test"),
                    // RFC 3581 §3 — the bare `rport` flag (no value)
                    // is the UAC's request "please echo back the
                    // observed source port".
                    Param::Rport(None),
                ],
            )
            .unwrap(),
        ))
        .build()
}

#[test]
fn response_via_gets_received_and_rport_when_inbound_via_had_rport_flag() {
    let request = nat_uac_invite();

    // NAT-translated source address — what the server's socket
    // recvfrom() reports.
    let nat_source = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)), 33000);

    // Build a 200 OK exactly as `ServerInviteTransaction::send_response`
    // would (canonical RFC 3261 §8.2.6.2 header copy).
    let mut response =
        SimpleResponseBuilder::response_from_request(&request, StatusCode::Ok, Some("OK")).build();

    // Apply the stamp helper that the server transaction calls
    // before handing the response to the transport.
    let stamped = stamp_response_via_with_source(&mut response, nat_source);
    assert!(stamped, "stamp helper should report a change");

    let top_via = response.first_via().expect("Via copied from request");
    assert_eq!(
        top_via.received(),
        Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7))),
        "received= should reflect the NAT-translated source IP"
    );
    assert_eq!(
        top_via.rport(),
        Some(Some(33000)),
        "rport= should reflect the NAT-translated source port"
    );

    // Wire-form smoke check: the serialized response must contain
    // the new params. This is what the peer actually reads off the
    // wire.
    let wire = response.to_string();
    assert!(
        wire.contains("received=198.51.100.7"),
        "wire form missing received=: {}",
        wire
    );
    assert!(
        wire.contains("rport=33000"),
        "wire form missing rport=: {}",
        wire
    );
    // Original branch must survive.
    assert!(
        wire.contains("branch=z9hG4bKnat-test"),
        "wire form missing original branch: {}",
        wire
    );
}

#[test]
fn response_via_only_gets_received_when_inbound_via_had_no_rport() {
    // Inbound INVITE WITHOUT `;rport` — pre-RFC 3581 UAC.
    // RFC 3261 §18.2.1 still requires `received=` when source IP
    // differs from Via host, but we MUST NOT add `rport=` (that
    // would surprise legacy stacks that key state on its absence).
    let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@server.example.com")
        .unwrap()
        .from("Alice", "sip:alice@uac.example.com", Some("alicetag"))
        .to("Bob", "sip:bob@server.example.com", None)
        .call_id("rport-restamp-no-rport")
        .cseq(1)
        .header(TypedHeader::Via(
            Via::new(
                "SIP",
                "2.0",
                "UDP",
                "10.0.0.5",
                Some(5060),
                vec![Param::branch("z9hG4bKno-rport")],
            )
            .unwrap(),
        ))
        .build();

    let nat_source = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)), 33000);
    let mut response =
        SimpleResponseBuilder::response_from_request(&request, StatusCode::Ok, Some("OK")).build();
    stamp_response_via_with_source(&mut response, nat_source);

    let top_via = response.first_via().expect("Via copied from request");
    assert_eq!(
        top_via.received(),
        Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)))
    );
    assert_eq!(
        top_via.rport(),
        None,
        "rport= must NOT be added when UAC did not request it (RFC 3581 opt-in)"
    );

    let wire = response.to_string();
    assert!(wire.contains("received=198.51.100.7"));
    assert!(
        !wire.contains("rport="),
        "wire form leaked rport=: {}",
        wire
    );
}

#[test]
fn ipv6_nat_source_is_stamped_correctly() {
    use std::net::Ipv6Addr;

    let request = nat_uac_invite();
    let nat_source = SocketAddr::new(
        IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
        44000,
    );

    let mut response =
        SimpleResponseBuilder::response_from_request(&request, StatusCode::Ok, Some("OK")).build();
    stamp_response_via_with_source(&mut response, nat_source);

    let top_via = response.first_via().expect("Via");
    match top_via.received() {
        Some(IpAddr::V6(addr)) => {
            assert_eq!(addr, Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));
        }
        other => panic!("expected IPv6 received, got {:?}", other),
    }
    assert_eq!(top_via.rport(), Some(Some(44000)));
}

#[test]
fn second_via_in_chain_is_not_modified() {
    // Multi-hop scenario: the request has TWO Via headers (the UAC's
    // and an upstream proxy's). Per RFC 3261 / 3581, only the TOP
    // Via gets the received/rport treatment — the response is sent
    // back UP the Via chain and each hop pops its own entry.
    let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@server.example.com")
        .unwrap()
        .from("Alice", "sip:alice@uac.example.com", Some("alicetag"))
        .to("Bob", "sip:bob@server.example.com", None)
        .call_id("rport-multi-hop")
        .cseq(1)
        // Top Via — proxy (most recent hop)
        .header(TypedHeader::Via(
            Via::new(
                "SIP",
                "2.0",
                "UDP",
                "proxy.example.com",
                Some(5060),
                vec![Param::branch("z9hG4bKproxy"), Param::Rport(None)],
            )
            .unwrap(),
        ))
        // Second Via — UAC (older hop, untouched by this server)
        .header(TypedHeader::Via(
            Via::new(
                "SIP",
                "2.0",
                "UDP",
                "10.0.0.5",
                Some(5060),
                vec![Param::branch("z9hG4bKuac")],
            )
            .unwrap(),
        ))
        .build();

    let nat_source = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)), 33000);
    let mut response =
        SimpleResponseBuilder::response_from_request(&request, StatusCode::Ok, Some("OK")).build();
    stamp_response_via_with_source(&mut response, nat_source);

    // Top Via (proxy) — stamped.
    let top_via = response.first_via().expect("top Via");
    assert_eq!(
        top_via.received(),
        Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)))
    );
    assert_eq!(top_via.rport(), Some(Some(33000)));

    // Second Via (UAC's) — UNTOUCHED. Wire form check is the
    // tightest way to verify this — we want to see the original
    // string and no spurious received=/rport= leaking onto it.
    let wire = response.to_string();
    // Count occurrences of received= — should be exactly 1 (only
    // the top Via).
    let received_count = wire.matches("received=").count();
    assert_eq!(
        received_count, 1,
        "second Via leaked received=; full wire form: {}",
        wire
    );
    let rport_eq_count = wire.matches("rport=33000").count();
    assert_eq!(
        rport_eq_count, 1,
        "second Via leaked rport=33000; full wire form: {}",
        wire
    );
}

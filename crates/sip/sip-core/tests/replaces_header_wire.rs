//! RFC 3891 `Replaces` — the header as it actually arrives on the wire.
//!
//! The unit tests in `types::replaces` and `parser::headers::replaces` cover
//! the value grammar. This file covers the step that sits between them and the
//! dialog layer: a full INVITE parsed from bytes has to hand back a typed
//! `Replaces`, because that is the call the UAS makes when deciding whether to
//! replace a dialog.
//!
//! What this replaces: the dialog layer used to read the header with
//! `header_str.split(':').nth(1)`, which takes everything after the first
//! colon. A Call-ID carrying a port is cut in the wrong place by that, and the
//! `port_in_call_id_survives_the_wire` case below is the one that used to fail.

use rvoip_sip_core::parse_message;
use rvoip_sip_core::types::replaces::Replaces;
use rvoip_sip_core::Message;

fn invite_with_replaces(replaces_value: &str) -> Vec<u8> {
    format!(
        "INVITE sip:charlie@example.test SIP/2.0\r\n\
         Via: SIP/2.0/UDP 192.168.0.10:5060;branch=z9hG4bK-1\r\n\
         Max-Forwards: 70\r\n\
         From: <sip:alice@example.test>;tag=alice-tag\r\n\
         To: <sip:charlie@example.test>\r\n\
         Call-ID: new-call-id@example.test\r\n\
         CSeq: 1 INVITE\r\n\
         Replaces: {}\r\n\
         Contact: <sip:alice@192.168.0.10:5060>\r\n\
         Content-Length: 0\r\n\
         \r\n",
        replaces_value
    )
    .into_bytes()
}

fn parse_invite(replaces_value: &str) -> rvoip_sip_core::Request {
    match parse_message(&invite_with_replaces(replaces_value)).expect("parse INVITE") {
        Message::Request(request) => request,
        Message::Response(_) => panic!("expected a request"),
    }
}

#[test]
fn typed_replaces_is_reachable_from_a_parsed_invite() {
    let request = parse_invite("consult-call-id@example.test;to-tag=charlie-tag;from-tag=bob-tag");

    let replaces = request
        .typed_header::<Replaces>()
        .expect("Replaces must be reachable as a typed header");

    assert_eq!(replaces.call_id, "consult-call-id@example.test");
    assert_eq!(replaces.to_tag, "charlie-tag");
    assert_eq!(replaces.from_tag, "bob-tag");
    assert!(!replaces.early_only);
}

/// The regression that motivated the typed header. `split(':')` on
/// `Replaces: cid@host:5060;...` yields `cid@host`, silently dropping the port
/// and producing a Call-ID that matches no dialog.
#[test]
fn port_in_call_id_survives_the_wire() {
    let request = parse_invite("call-abc@192.168.0.1:5060;to-tag=t1;from-tag=f1");

    let replaces = request.typed_header::<Replaces>().expect("typed Replaces");
    assert_eq!(replaces.call_id, "call-abc@192.168.0.1:5060");

    let naive = "call-abc@192.168.0.1:5060;to-tag=t1;from-tag=f1"
        .split(':')
        .nth(1);
    assert_ne!(
        naive.map(str::to_string),
        Some(replaces.call_id.clone()),
        "the old split(':') parse must not agree with the typed parse here, \
         otherwise this regression is not being exercised"
    );
}

/// RFC 3891 Section 3: the to-tag is the *receiver's* local tag and the
/// from-tag is the remote one. Pinned at the wire level because reversing it
/// builds a well formed key that matches nothing.
#[test]
fn tags_map_to_local_then_remote_for_the_receiver() {
    let request = parse_invite("consult-cid;to-tag=charlie-local;from-tag=bob-remote");
    let replaces = request.typed_header::<Replaces>().expect("typed Replaces");

    assert_eq!(
        replaces.as_local_remote_tags(),
        ("charlie-local", "bob-remote")
    );
}

#[test]
fn early_only_flag_survives_the_wire() {
    let request = parse_invite("consult-cid;to-tag=t1;from-tag=f1;early-only");
    let replaces = request.typed_header::<Replaces>().expect("typed Replaces");
    assert!(
        replaces.early_only,
        "early-only drives the 486 Busy Here branch of RFC 3891 Section 3"
    );
}

/// A Replaces value missing a mandatory tag must not surface as a usable
/// header. This is what lets the UAS answer 400 Bad Request.
#[test]
fn a_value_missing_a_mandatory_tag_does_not_become_typed() {
    for malformed in [
        "consult-cid;to-tag=t1",
        "consult-cid;from-tag=f1",
        "consult-cid",
    ] {
        let request = parse_invite(malformed);
        assert!(
            request.typed_header::<Replaces>().is_none(),
            "{:?} is malformed under RFC 3891 Section 6.1 and must not parse",
            malformed
        );
    }
}

#[test]
fn replaces_round_trips_through_reserialisation() {
    let request = parse_invite("consult-cid@host.test;to-tag=t1;from-tag=f1;early-only");
    let reparsed = match parse_message(&request.to_bytes()).expect("reparse serialised INVITE") {
        Message::Request(request) => request,
        Message::Response(_) => panic!("expected a request"),
    };

    assert_eq!(
        reparsed.typed_header::<Replaces>(),
        request.typed_header::<Replaces>()
    );
}

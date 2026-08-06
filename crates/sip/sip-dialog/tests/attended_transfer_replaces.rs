//! RFC 3891 `Replaces` — the half of attended transfer that is missing.
//!
//! These tests define the target before the implementation exists, so "done"
//! is not decided by whoever writes the code. Several of them fail today, on
//! purpose. Each one says in its own assertion what it means when it fails.
//!
//! # Where `Replaces` actually travels
//!
//! Attended transfer between Alice, Bob and Charlie:
//!
//! 1. Alice ↔ Bob, the call being transferred.
//! 2. Bob ↔ Charlie, the consultation call.
//! 3. Bob REFERs Alice, with the consultation dialog carried as a **URI
//!    parameter inside `Refer-To`** — `<sip:charlie@host?Replaces=...>` — not
//!    as a header on the REFER.
//! 4. Alice INVITEs Charlie with a **`Replaces` header** on that INVITE.
//! 5. Charlie matches it to his dialog with Bob, accepts Alice, and BYEs Bob.
//!
//! Step 5 is where the transfer actually happens, and it happens on the
//! transfer *target*, in an inbound INVITE. Looking for `Replaces` on the
//! inbound REFER — which is what `manager/protocol_handlers.rs` does today —
//! inspects a header that a conforming peer never sends.
//!
//! # Blind transfer
//!
//! The blind cases are here as regression, not as new coverage. Blind transfer
//! works today, end to end, and none of this may break it: a REFER without
//! `Replaces` must keep behaving exactly as it does now.

use rvoip_sip_core::builder::headers::ReferToExt;
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::refer_to::ReferTo;
use rvoip_sip_core::Method;
use rvoip_sip_dialog::manager::utils::DialogUtils;

/// The three fields RFC 3891 §3 matches a dialog on.
#[derive(Debug, PartialEq, Eq)]
struct ReplacesTarget {
    call_id: String,
    to_tag: String,
    from_tag: String,
}

/// Parse a `Replaces` value: `call-id;to-tag=<tag>;from-tag=<tag>`.
///
/// Deliberately not `split(':')`. A Call-ID routinely contains `@host`, and a
/// host may carry `:port`, so splitting on the first colon truncates it. The
/// separator that matters is `;`.
fn parse_replaces(value: &str) -> Option<ReplacesTarget> {
    let mut parts = value.split(';');
    let call_id = parts.next()?.trim().to_string();
    if call_id.is_empty() {
        return None;
    }

    let (mut to_tag, mut from_tag) = (None, None);
    for part in parts {
        let (key, tag) = part.split_once('=')?;
        match key.trim().to_ascii_lowercase().as_str() {
            "to-tag" => to_tag = Some(tag.trim().to_string()),
            "from-tag" => from_tag = Some(tag.trim().to_string()),
            _ => {}
        }
    }

    Some(ReplacesTarget {
        call_id,
        to_tag: to_tag?,
        from_tag: from_tag?,
    })
}

// ── The wire format ───────────────────────────────────────────────────────────

/// What `sip-core` builds for an attended `Refer-To` must be readable back out
/// of the parsed URI.
///
/// This is the round trip rvoip needs against itself: it already emits the
/// escaped URI parameter correctly, so failing to read it back would mean the
/// library cannot interpret its own REFER.
#[test]
fn replaces_survives_a_refer_to_round_trip_through_the_uri() {
    let call_id = "consultation-1@bob.example.test";
    let refer = SimpleRequestBuilder::new(Method::Refer, "sip:alice@example.test")
        .expect("builder")
        .from("bob", "sip:bob@example.test", Some("bobtag"))
        .to("alice", "sip:alice@example.test", Some("alicetag"))
        .call_id("transfer-1")
        .cseq(2)
        .refer_to_attended_transfer(
            "sip:charlie@example.test",
            call_id,
            "charlietag",
            "bobtag-consult",
        )
        .build();

    let refer_to = refer
        .typed_header::<ReferTo>()
        .expect("the REFER carries a Refer-To");

    // RFC 3515 §2.1 / RFC 3891 §2.1: the dialog to replace rides as an
    // embedded URI header, which the parser exposes on the URI itself.
    let embedded = refer_to
        .uri()
        .headers
        .iter()
        .find(|(name, _): &(&String, &String)| name.eq_ignore_ascii_case("replaces"))
        .map(|(_, value)| value.clone())
        .expect(
            "Refer-To carried no embedded Replaces. rvoip builds this parameter itself, so \
             failing to read it back means it cannot interpret its own attended REFER.",
        );

    let target = parse_replaces(&embedded).expect("embedded Replaces must parse");
    assert_eq!(
        target,
        ReplacesTarget {
            call_id: call_id.to_string(),
            to_tag: "charlietag".to_string(),
            from_tag: "bobtag-consult".to_string(),
        }
    );
}

/// A Call-ID with a port survives parsing.
///
/// `split(':')` — what `protocol_handlers.rs` uses today — truncates this one
/// at the port, producing a Call-ID that matches no dialog. The failure would
/// be a transfer that silently does nothing.
#[test]
fn a_call_id_containing_a_colon_is_not_truncated() {
    let target = parse_replaces("call-9@host.example.test:5060;to-tag=a;from-tag=b")
        .expect("value must parse");
    assert_eq!(target.call_id, "call-9@host.example.test:5060");
    assert_eq!(target.to_tag, "a");
    assert_eq!(target.from_tag, "b");
}

/// Both tags are mandatory (RFC 3891 §3). A value missing either identifies no
/// dialog and must be rejected rather than matched loosely.
#[test]
fn a_replaces_missing_either_tag_is_rejected() {
    assert!(parse_replaces("call-1@host;to-tag=a").is_none());
    assert!(parse_replaces("call-1@host;from-tag=b").is_none());
    assert!(parse_replaces("call-1@host").is_none());
    assert!(parse_replaces("").is_none());
}

/// Tag order is not significant.
#[test]
fn tag_order_does_not_matter() {
    let a = parse_replaces("c@h;to-tag=x;from-tag=y").expect("parse");
    let b = parse_replaces("c@h;from-tag=y;to-tag=x").expect("parse");
    assert_eq!(a, b);
}

// ── Blind transfer: regression ────────────────────────────────────────────────

/// A blind REFER carries no `Replaces`, and must not acquire one.
///
/// Blind transfer works today. This is the guard that says so while the
/// attended path is built beside it.
#[test]
fn a_blind_refer_carries_no_replaces() {
    let refer = SimpleRequestBuilder::new(Method::Refer, "sip:alice@example.test")
        .expect("builder")
        .from("bob", "sip:bob@example.test", Some("bobtag"))
        .to("alice", "sip:alice@example.test", Some("alicetag"))
        .call_id("blind-1")
        .cseq(2)
        .refer_to_uri("sip:charlie@example.test")
        .build();

    let refer_to = refer
        .typed_header::<ReferTo>()
        .expect("the REFER carries a Refer-To");

    assert!(
        !refer_to
            .uri()
            .headers
            .iter()
            .any(|(name, _): (&String, &String)| name.eq_ignore_ascii_case("replaces")),
        "a blind REFER must not carry a Replaces parameter"
    );
    assert_eq!(refer_to.uri().to_string(), "sip:charlie@example.test");
}

/// The blind target survives the round trip unchanged.
///
/// E.164 targets are deliberately absent here and covered separately below:
/// they trip over a `sip-core` serialisation issue that has nothing to do with
/// transfer, and folding them in would make this regression test fail for a
/// reason it does not describe.
#[test]
fn a_blind_refer_to_target_round_trips_unchanged() {
    for target in ["sip:charlie@example.test", "sip:charlie@192.0.2.10:5060"] {
        let refer = SimpleRequestBuilder::new(Method::Refer, "sip:alice@example.test")
            .expect("builder")
            .from("bob", "sip:bob@example.test", Some("bobtag"))
            .to("alice", "sip:alice@example.test", Some("alicetag"))
            .call_id("blind-2")
            .cseq(2)
            .refer_to_uri(target)
            .build();

        let refer_to = refer.typed_header::<ReferTo>().expect("Refer-To");
        assert_eq!(
            refer_to.uri().to_string(),
            target,
            "blind Refer-To target was altered in transit"
        );
    }
}

/// An E.164 blind-transfer target keeps its `+` in the parsed user part.
///
/// This is the half that works. `sip-core`'s parser implements RFC 3261's
/// `user-unreserved`, which includes `+`, so the value survives parsing intact.
#[test]
fn an_e164_blind_target_parses_with_its_plus_intact() {
    let refer = SimpleRequestBuilder::new(Method::Refer, "sip:alice@example.test")
        .expect("builder")
        .from("bob", "sip:bob@example.test", Some("bobtag"))
        .to("alice", "sip:alice@example.test", Some("alicetag"))
        .call_id("blind-e164")
        .cseq(2)
        .refer_to_uri("sip:+15551234567@gateway.example.test")
        .build();

    let refer_to = refer.typed_header::<ReferTo>().expect("Refer-To");
    assert_eq!(
        refer_to.uri().user.as_deref(),
        Some("+15551234567"),
        "the parser must keep `+` unescaped: RFC 3261 puts it in user-unreserved"
    );
}

/// Serialising that same URI keeps the `+` unescaped, so the target survives
/// the round trip a blind transfer to an E.164 number depends on.
///
/// This test used to assert the opposite, pinning the serialiser's behaviour
/// while the RFC question was still open. RFC 3261 §19.1.4 settled it:
/// "Characters other than those in the `reserved` set (see RFC 2396) are
/// equivalent to their `"%" HEX HEX` encoding", and RFC 2396's reserved set
/// contains `+`. So `%2B` is a *different* URI, not a synonym, and escaping it
/// is an interop bug rather than cosmetics.
///
/// It lives here because blind transfer to an E.164 target is exactly where
/// the mismatch bites, and losing that connection would make the bug look
/// academic.
#[test]
fn e164_serialisation_keeps_the_plus_unescaped() {
    let refer = SimpleRequestBuilder::new(Method::Refer, "sip:alice@example.test")
        .expect("builder")
        .from("bob", "sip:bob@example.test", Some("bobtag"))
        .to("alice", "sip:alice@example.test", Some("alicetag"))
        .call_id("blind-e164-wire")
        .cseq(2)
        .refer_to_uri("sip:+15551234567@gateway.example.test")
        .build();

    let rendered = refer
        .typed_header::<ReferTo>()
        .expect("Refer-To")
        .uri()
        .to_string();
    assert_eq!(
        rendered, "sip:+15551234567@gateway.example.test",
        "a gateway comparing the E.164 target literally will not match %2B"
    );
}

// ── Matching the dialog: the direction convention ─────────────────────────────

/// The tags in `Replaces` map onto the receiver's dialog index without
/// inversion: `to-tag` is the receiver's *local* tag, `from-tag` its *remote*.
///
/// This is the single easiest thing to get backwards, and getting it backwards
/// is invisible in a one-legged test: both tags exist, the key is well formed,
/// and it simply matches nothing. The transfer then fails as "no such dialog"
/// rather than as a bug.
///
/// The convention follows from the producer already in the tree.
/// `DialogIdentity::to_replaces_value` emits
/// `call-id;to-tag=<remote>;from-tag=<local>` **from the sender's viewpoint**,
/// so the sender's remote party is the receiver itself. Charlie, holding the
/// Bob↔Charlie dialog, therefore finds it at
/// `call_id : to-tag (his own) : from-tag (Bob's)`.
#[test]
fn replaces_tags_map_onto_the_receivers_dialog_key() {
    // Bob's consultation dialog with Charlie, as Charlie has it indexed.
    let call_id = "consultation-1@bob.example.test";
    let charlie_local_tag = "charlietag";
    let bob_remote_tag = "bobtag-consult";
    let charlies_index_key =
        DialogUtils::create_lookup_key(call_id, charlie_local_tag, bob_remote_tag);

    // What Bob puts on the wire, mirroring DialogIdentity::to_replaces_value.
    let on_the_wire = format!(
        "{};to-tag={};from-tag={}",
        call_id, charlie_local_tag, bob_remote_tag
    );
    let target = parse_replaces(&on_the_wire).expect("wire value must parse");

    let looked_up =
        DialogUtils::create_lookup_key(&target.call_id, &target.to_tag, &target.from_tag);

    assert_eq!(
        looked_up, charlies_index_key,
        "a Replaces from the peer must address the receiver's own dialog key: to-tag is the \
         receiver's local tag, from-tag the remote one. Swapping them yields a well-formed \
         key that matches nothing, so the transfer fails as 'no such dialog'."
    );
}

/// Swapping the tags must NOT find the dialog.
///
/// The companion to the test above: it proves that one is asserting a real
/// constraint rather than a formatting coincidence.
#[test]
fn reversed_replaces_tags_do_not_address_the_same_dialog() {
    let call_id = "consultation-1@bob.example.test";
    let correct = DialogUtils::create_lookup_key(call_id, "charlietag", "bobtag-consult");
    let reversed = DialogUtils::create_lookup_key(call_id, "bobtag-consult", "charlietag");
    assert_ne!(
        correct, reversed,
        "the dialog key must be direction-sensitive, or Replaces matching cannot be validated"
    );
}

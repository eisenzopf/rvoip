//! Unit tests for the UAC side of RFC 3262 (PRACK / 100rel).
//!
//! Covers the pure functions that drive auto-PRACK:
//! - `detect_reliable_provisional` — detect `Require: 100rel` + `RSeq` on 18x
//! - `inject_100rel_policy` — add Supported/Require 100rel header on outgoing INVITE
//! - `prack_for_dialog` — build a PRACK request with a valid `RAck` header
//!
//! Full end-to-end auto-PRACK behaviour (dialog lookup + transaction send) is
//! covered by integration tests in session-core-v3.

use rvoip_dialog_core::api::config::RelUsage;
use rvoip_dialog_core::manager::transaction_integration::{
    detect_reliable_provisional, inject_100rel_policy,
};
use rvoip_dialog_core::transaction::dialog::prack_for_dialog;
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::{HeaderName, Method, RSeq, Require, Supported, TypedHeader};
use rvoip_sip_core::types::rack::RAck;
use rvoip_sip_core::{Response, StatusCode, Version};
use std::net::SocketAddr;

fn make_response(status: StatusCode, headers: Vec<TypedHeader>) -> Response {
    let mut r = Response::new(status);
    r.version = Version::default();
    r.reason = Some(status.reason_phrase().to_string());
    for h in headers {
        r.headers.push(h);
    }
    r
}

#[test]
fn detect_requires_both_require_and_rseq() {
    // Reliable 183: Require: 100rel + RSeq: 42
    let r = make_response(
        StatusCode::SessionProgress,
        vec![
            TypedHeader::Require(Require::with_tag("100rel")),
            TypedHeader::RSeq(RSeq::new(42)),
        ],
    );
    assert_eq!(detect_reliable_provisional(&r), Some(42));
}

#[test]
fn detect_returns_none_without_require_100rel() {
    // 183 with RSeq but no Require: 100rel — not a reliable provisional.
    let r = make_response(
        StatusCode::SessionProgress,
        vec![TypedHeader::RSeq(RSeq::new(7))],
    );
    assert_eq!(detect_reliable_provisional(&r), None);
}

#[test]
fn detect_returns_none_without_rseq() {
    // Require: 100rel present but no RSeq — malformed; skip auto-PRACK.
    let r = make_response(
        StatusCode::SessionProgress,
        vec![TypedHeader::Require(Require::with_tag("100rel"))],
    );
    assert_eq!(detect_reliable_provisional(&r), None);
}

#[test]
fn detect_returns_none_for_unrelated_require_tags() {
    let r = make_response(
        StatusCode::SessionProgress,
        vec![
            TypedHeader::Require(Require::with_tag("timer")),
            TypedHeader::RSeq(RSeq::new(3)),
        ],
    );
    assert_eq!(detect_reliable_provisional(&r), None);
}

#[test]
fn inject_supported_adds_100rel_when_absent() {
    let mut req = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
        .unwrap()
        .build();
    inject_100rel_policy(&mut req, RelUsage::Supported);
    let sup = req
        .headers
        .iter()
        .find_map(|h| if let TypedHeader::Supported(s) = h { Some(s) } else { None })
        .expect("Supported header should be present");
    assert!(sup.option_tags.iter().any(|t| t == "100rel"));
}

#[test]
fn inject_supported_appends_to_existing() {
    let mut req = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
        .unwrap()
        .header(TypedHeader::Supported(Supported::new(vec!["timer".to_string()])))
        .build();
    inject_100rel_policy(&mut req, RelUsage::Supported);

    // Only one Supported header, with both tags.
    let sups: Vec<_> = req
        .headers
        .iter()
        .filter_map(|h| if let TypedHeader::Supported(s) = h { Some(s) } else { None })
        .collect();
    assert_eq!(sups.len(), 1);
    assert!(sups[0].option_tags.iter().any(|t| t == "timer"));
    assert!(sups[0].option_tags.iter().any(|t| t == "100rel"));
}

#[test]
fn inject_required_adds_require_header() {
    let mut req = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
        .unwrap()
        .build();
    inject_100rel_policy(&mut req, RelUsage::Required);
    let reqs: Vec<_> = req
        .headers
        .iter()
        .filter_map(|h| if let TypedHeader::Require(r) = h { Some(r) } else { None })
        .collect();
    assert_eq!(reqs.len(), 1);
    assert!(reqs[0].requires("100rel"));
}

#[test]
fn inject_not_supported_is_noop() {
    let mut req = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
        .unwrap()
        .build();
    let header_count_before = req.headers.len();
    inject_100rel_policy(&mut req, RelUsage::NotSupported);
    assert_eq!(req.headers.len(), header_count_before);
    assert!(req
        .headers
        .iter()
        .all(|h| !matches!(h, TypedHeader::Require(_) | TypedHeader::Supported(_))));
}

#[test]
fn inject_does_not_duplicate_100rel_in_supported() {
    let mut req = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
        .unwrap()
        .header(TypedHeader::Supported(Supported::new(vec!["100rel".to_string()])))
        .build();
    inject_100rel_policy(&mut req, RelUsage::Supported);
    let sup = req
        .headers
        .iter()
        .find_map(|h| if let TypedHeader::Supported(s) = h { Some(s) } else { None })
        .unwrap();
    assert_eq!(sup.option_tags.iter().filter(|t| *t == "100rel").count(), 1);
}

#[test]
fn prack_for_dialog_builds_valid_request() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    let prack = prack_for_dialog(
        "call-xyz",
        "sip:alice@example.com",
        "alice-tag",
        "sip:bob@example.com",
        "bob-tag",
        /* rseq */ 7,
        /* invite_cseq */ 101,
        /* prack_cseq */ 102,
        local_addr,
        None,
    )
    .expect("prack_for_dialog should build a PRACK");

    assert_eq!(prack.method(), Method::Prack);
    assert_eq!(prack.call_id().unwrap().value(), "call-xyz");
    assert_eq!(prack.from().unwrap().tag().unwrap(), "alice-tag");
    assert_eq!(prack.to().unwrap().tag().unwrap(), "bob-tag");
    assert_eq!(prack.cseq().unwrap().seq, 102);

    let rack = prack
        .header(&HeaderName::RAck)
        .expect("RAck header must be present");
    match rack {
        TypedHeader::RAck(r) => {
            assert_eq!(r.rseq, 7);
            assert_eq!(r.cseq, 101);
            assert_eq!(r.method, Method::Invite);
        }
        other => panic!("Expected TypedHeader::RAck, got {:?}", other),
    }
}

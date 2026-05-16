//! SIP_API_DESIGN_2 §10 verification #11 — B2BUA carry-through litmus test.
//!
//! Exercises the §11.2 example end-to-end at the wire layer:
//!
//! 1. Build a synthetic upstream INVITE carrying both application
//!    headers (`History-Info`, `Diversion`, `Privacy`, `P-Asserted-Identity`,
//!    `X-Customer-ID`) and stack-managed topology (`Via`, `Call-ID`,
//!    `CSeq`, `Max-Forwards`).
//! 2. Drive `coord.invite(...).with_headers_from(&inbound, app_names)?`
//!    on alice (the B2BUA's outbound leg) → strip `Privacy` → rewrite
//!    `P-Asserted-Identity`.
//! 3. Send to bob (downstream peer); read the inbound trace on bob.
//! 4. Assert on the wire:
//!    - Application carry-through headers (`History-Info`, `Diversion`,
//!      `X-Customer-ID`) survived.
//!    - Privacy is stripped.
//!    - P-Asserted-Identity carries the rewritten value, not the
//!      synthetic upstream value.
//!    - Upstream topology values (`Via`, `Call-ID`, `CSeq`,
//!      `Max-Forwards`) do NOT appear — dialog-core stamps fresh
//!      values per §12.3 automatic topology hiding.
//! 5. Assert on the carry-through report:
//!    - Every stack-managed name requested for carry-through landed in
//!      `report.skipped` with `ViolationReason::StackManaged`.
//!    - Every application name landed in `report.copied`.
//!
//! Two flavors live side by side:
//!
//! - `b2bua_carry_through_runs_strip_and_rewrite_end_to_end` — uses a
//!   hand-built `SipHeaderView` so the policy filtering / strip / raw
//!   override / wire output contract can be asserted without booting a
//!   third coordinator. Fast-running smoke test.
//! - `b2bua_carry_through_drives_real_incoming_call` — boots three
//!   coordinators (alice = upstream UAC, b2bua = middle, bob =
//!   downstream UAS). The b2bua's `CallHandler` reads the real
//!   `IncomingCall` from alice, drives `with_headers_from(&incoming,
//!   ...)` on the outbound leg to bob, and the test asserts bob's
//!   inbound wire trace. This exercises the full §11.2 flow now that
//!   `IncomingCall::raw_request()` round-trips correctly (the
//!   `rvoip-sip-dialog/src/events/adapter.rs:282` reserialization fix
//!   landed in the 2026-05-13 session).

use std::time::Duration;

use rvoip_sip::api::headers::options::{SipRequestOptions, ViolationReason};
use rvoip_sip::api::headers::view::SipHeaderView;
use rvoip_sip::api::unified::UnifiedCoordinator;
use rvoip_sip::HeaderName;
use rvoip_sip_core::types::call_id::CallId as CallIdHdr;
use rvoip_sip_core::types::headers::{HeaderValue, TypedHeader};
use rvoip_sip_core::types::max_forwards::MaxForwards;
use rvoip_sip_core::types::{CSeq, Method, Request, Uri};

mod support;

use support::{receiver_config, wait_for_inbound_method};

const PAIR_B2BUA: (u16, u16) = (16400, 16410);

// PAI values use alphanumeric users (no `+`) so the URI-percent-encoding
// the parser applies to E.164-style `+` doesn't trip the wire string
// match — the rewrite contract is what's under test, not the parser's
// encoding rules.
const UPSTREAM_PAI: &str = "<sip:upstream-trunk@upstream.example>";
const REWRITTEN_PAI: &str = "<sip:b2bua-rewritten@b2bua.example>";
const HISTORY_INFO: &str = "<sip:reception@upstream.example>;index=1";
const DIVERSION: &str = "<sip:menu@upstream.example>;reason=no-answer";
const CUSTOMER_ID: &str = "cust-7142";
const PRIVACY_VALUE: &str = "id;header";
const UPSTREAM_CALL_ID: &str = "upstream-call-id@upstream.example";
const UPSTREAM_VIA: &str = "SIP/2.0/UDP upstream.example:5060;branch=z9hG4bK-upstream-leg";

/// Synthetic stand-in for a real `IncomingCall::raw_request()`. The
/// production `SipHeaderView` impl on `IncomingCall` would be wrapped
/// over this once the adapter.rs:282 reserialization fix lands.
struct UpstreamView(Request);

impl SipHeaderView for UpstreamView {
    fn header(&self, name: &HeaderName) -> Option<&TypedHeader> {
        self.0.header(name)
    }
    fn headers_named<'a>(
        &'a self,
        name: &HeaderName,
    ) -> Box<dyn Iterator<Item = &'a TypedHeader> + 'a> {
        let n = name.clone();
        Box::new(self.0.headers.iter().filter(move |h| h.name() == n))
    }
    fn headers<'a>(&'a self) -> Box<dyn Iterator<Item = &'a TypedHeader> + 'a> {
        Box::new(self.0.headers.iter())
    }
    fn header_names(&self) -> Vec<HeaderName> {
        let mut seen = Vec::new();
        for h in &self.0.headers {
            let n = h.name();
            if !seen.contains(&n) {
                seen.push(n);
            }
        }
        seen
    }
}

fn build_synthetic_upstream_invite() -> UpstreamView {
    let uri: Uri = "sip:bob@b2bua.example".parse().expect("uri parse");
    let mut req = Request::new(Method::Invite, uri);
    req.headers.push(TypedHeader::CallId(CallIdHdr::new(UPSTREAM_CALL_ID)));
    req.headers
        .push(TypedHeader::CSeq(CSeq::new(101, Method::Invite)));
    req.headers
        .push(TypedHeader::MaxForwards(MaxForwards::new(70)));
    req.headers.push(TypedHeader::Other(
        HeaderName::Via,
        HeaderValue::Raw(UPSTREAM_VIA.as_bytes().to_vec()),
    ));
    // The application-controlled headers the B2BUA wants to carry through.
    req.headers.push(TypedHeader::Other(
        HeaderName::Other("History-Info".to_string()),
        HeaderValue::Raw(HISTORY_INFO.as_bytes().to_vec()),
    ));
    req.headers.push(TypedHeader::Other(
        HeaderName::Other("Diversion".to_string()),
        HeaderValue::Raw(DIVERSION.as_bytes().to_vec()),
    ));
    req.headers.push(TypedHeader::Other(
        HeaderName::Other("X-Customer-ID".to_string()),
        HeaderValue::Raw(CUSTOMER_ID.as_bytes().to_vec()),
    ));
    // Privacy will be requested for carry-through, then stripped.
    req.headers.push(TypedHeader::Other(
        HeaderName::Other("Privacy".to_string()),
        HeaderValue::Raw(PRIVACY_VALUE.as_bytes().to_vec()),
    ));
    // PAI present on inbound; the B2BUA will REWRITE it (not carry it).
    req.headers.push(TypedHeader::Other(
        HeaderName::Other("P-Asserted-Identity".to_string()),
        HeaderValue::Raw(UPSTREAM_PAI.as_bytes().to_vec()),
    ));
    UpstreamView(req)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b2bua_carry_through_runs_strip_and_rewrite_end_to_end() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_B2BUA;

    let bob = UnifiedCoordinator::new(receiver_config("bob", bob_port))
        .await
        .expect("bob coordinator");
    let mut bob_events = bob.events().await.expect("bob events");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let alice = UnifiedCoordinator::new(receiver_config("alice", alice_port))
        .await
        .expect("alice coordinator");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let upstream = build_synthetic_upstream_invite();

    // §11.2 litmus shape. Ask carry-through for the three application
    // names AND every stack-managed name we want surfaced as skipped.
    let names = vec![
        HeaderName::Other("History-Info".to_string()),
        HeaderName::Other("Diversion".to_string()),
        HeaderName::Other("X-Customer-ID".to_string()),
        HeaderName::Other("Privacy".to_string()),
        // Topology — these MUST land in report.skipped.
        HeaderName::Via,
        HeaderName::CallId,
        HeaderName::CSeq,
        HeaderName::MaxForwards,
    ];

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    let (builder, report) = alice
        .invite(
            Some(format!("sip:alice@127.0.0.1:{}", alice_port)),
            target.clone(),
        )
        .with_headers_from(&upstream, &names)
        .expect("with_headers_from must succeed");

    // Every stack-managed name we asked for must surface as skipped.
    for must_skip in [
        HeaderName::Via,
        HeaderName::CallId,
        HeaderName::CSeq,
        HeaderName::MaxForwards,
    ] {
        assert!(
            report
                .skipped
                .iter()
                .any(|(n, r)| n == &must_skip && r == &ViolationReason::StackManaged),
            "{must_skip:?} must be skipped as StackManaged; report.skipped = {:?}",
            report.skipped
        );
    }

    // Every application name must be copied.
    for must_copy in [
        HeaderName::Other("History-Info".to_string()),
        HeaderName::Other("Diversion".to_string()),
        HeaderName::Other("X-Customer-ID".to_string()),
        HeaderName::Other("Privacy".to_string()),
    ] {
        assert!(
            report.copied.contains(&must_copy),
            "{must_copy:?} must be copied through; report.copied = {:?}",
            report.copied
        );
    }

    // Strip Privacy and rewrite PAI per the §11.2 / §11.3 trust-boundary
    // pattern.
    let builder = builder
        .strip_header(&HeaderName::Other("Privacy".to_string()))
        .with_raw_header(
            HeaderName::Other("P-Asserted-Identity".to_string()),
            REWRITTEN_PAI,
        )
        .expect("with_raw_header on PAI");

    // Sanity-check the staged slice before dispatch: no Privacy, no
    // topology, PAI present once with the rewritten value.
    let staged: Vec<(HeaderName, String)> = builder
        .staged_headers()
        .iter()
        .map(|h| (h.name(), h.to_string()))
        .collect();
    assert!(
        !staged
            .iter()
            .any(|(n, _)| n == &HeaderName::Other("Privacy".to_string())),
        "Privacy must be stripped from staged; staged = {staged:?}"
    );
    for forbidden in [
        HeaderName::Via,
        HeaderName::CallId,
        HeaderName::CSeq,
        HeaderName::MaxForwards,
    ] {
        assert!(
            !staged.iter().any(|(n, _)| n == &forbidden),
            "{forbidden:?} must never reach staged; staged = {staged:?}"
        );
    }
    let pai_staged: Vec<&String> = staged
        .iter()
        .filter(|(n, _)| n == &HeaderName::Other("P-Asserted-Identity".to_string()))
        .map(|(_, v)| v)
        .collect();
    assert_eq!(
        pai_staged.len(),
        1,
        "exactly one staged PAI expected; got {pai_staged:?}"
    );
    assert!(
        pai_staged[0].contains(REWRITTEN_PAI),
        "staged PAI must carry rewritten value; got {:?}",
        pai_staged[0]
    );

    // Dispatch — bob's inbound trace captures the wire output.
    let _call_id = builder.send().await.expect("invite().send()");

    let trace = wait_for_inbound_method(&mut bob_events, "INVITE", Duration::from_secs(8))
        .await
        .expect("bob did not see inbound INVITE trace");
    let wire = &trace.raw_message;

    // Application carry-through visible on the wire.
    assert!(
        wire.contains(HISTORY_INFO),
        "History-Info must reach the wire; wire =\n{wire}"
    );
    assert!(
        wire.contains(DIVERSION),
        "Diversion must reach the wire; wire =\n{wire}"
    );
    assert!(
        wire.contains(CUSTOMER_ID),
        "X-Customer-ID must reach the wire; wire =\n{wire}"
    );

    // Privacy stripped.
    assert!(
        !wire.contains(PRIVACY_VALUE),
        "Privacy value must NOT reach the wire; wire =\n{wire}"
    );

    // PAI rewritten, not carried.
    assert!(
        wire.contains(REWRITTEN_PAI),
        "rewritten PAI must be on the wire; wire =\n{wire}"
    );
    assert!(
        !wire.contains(UPSTREAM_PAI),
        "upstream PAI value must NOT leak to the wire; wire =\n{wire}"
    );

    // Topology hidden — dialog-core stamps fresh values.
    assert!(
        !wire.contains(UPSTREAM_CALL_ID),
        "upstream Call-ID must NOT leak (§12.3 topology hiding); wire =\n{wire}"
    );
    assert!(
        !wire.contains("z9hG4bK-upstream-leg"),
        "upstream Via branch must NOT leak; wire =\n{wire}"
    );
    assert!(
        !wire.contains("upstream.example:5060"),
        "upstream Via sent-by must NOT leak; wire =\n{wire}"
    );

    bob.terminate_current_session().await.ok();
    alice.terminate_current_session().await.ok();
    tokio::time::sleep(Duration::from_millis(150)).await;
}

// ─────────────────────────────────────────────────────────────────────
// Real-IncomingCall flow — three coordinators, no synthetic stand-in.
// ─────────────────────────────────────────────────────────────────────

use rvoip_sip::api::callback_peer::CallbackPeer;
use rvoip_sip::api::unified::Config;

use support::B2buaCarryThrough;

const PAIR_B2BUA_E2E_ALICE_PORT: u16 = 16420;
const PAIR_B2BUA_E2E_MIDDLE_PORT: u16 = 16430;
const PAIR_B2BUA_E2E_BOB_PORT: u16 = 16440;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b2bua_carry_through_drives_real_incoming_call() {
    let _ = tracing_subscriber::fmt::try_init();

    // bob (downstream UAS) — captures the outbound INVITE wire from
    // the b2bua middle.
    let bob = UnifiedCoordinator::new(receiver_config("bob-e2e", PAIR_B2BUA_E2E_BOB_PORT))
        .await
        .expect("bob coordinator");
    let mut bob_events = bob.events().await.expect("bob events");
    tokio::time::sleep(Duration::from_millis(150)).await;

    // b2bua (middle) — its CallHandler reads each IncomingCall and
    // drives the outbound INVITE using `with_headers_from(&call, ...)`.
    // The same coordinator is used for inbound and outbound because the
    // §11.2 carry-through API only requires that the SipHeaderView
    // source and the outbound builder share the same process — not the
    // same coordinator. Using one coord keeps the test boot fast.
    let middle_cfg = receiver_config("b2bua-e2e", PAIR_B2BUA_E2E_MIDDLE_PORT);
    let outbound_coord = UnifiedCoordinator::new(middle_cfg.clone())
        .await
        .expect("middle outbound coord");
    tokio::time::sleep(Duration::from_millis(100)).await;

    let handler = B2buaCarryThrough {
        outbound_coord: outbound_coord.clone(),
        outbound_target: format!("sip:bob@127.0.0.1:{}", PAIR_B2BUA_E2E_BOB_PORT),
        outbound_from: format!("sip:b2bua@127.0.0.1:{}", PAIR_B2BUA_E2E_MIDDLE_PORT),
        carry_names: vec![
            HeaderName::Other("History-Info".to_string()),
            HeaderName::Other("Diversion".to_string()),
            HeaderName::Other("X-Customer-ID".to_string()),
            HeaderName::Other("Privacy".to_string()),
        ],
        strip_names: vec![HeaderName::Other("Privacy".to_string())],
        rewrites: vec![(
            HeaderName::Other("P-Asserted-Identity".to_string()),
            REWRITTEN_PAI.to_string(),
        )],
    };

    // Boot a second middle coord wrapped as a CallbackPeer that uses
    // `handler` so its inbound channel runs the B2BUA logic. The
    // `outbound_coord` field on the handler is the SAME coord, so the
    // outbound INVITE goes out through the same UDP socket — this is
    // exactly the in-process B2BUA shape from §11.2.
    let middle_peer_cfg = Config::local("b2bua-e2e-peer", PAIR_B2BUA_E2E_MIDDLE_PORT + 1);
    let middle_peer = CallbackPeer::new(handler, middle_peer_cfg)
        .await
        .expect("middle CallbackPeer");
    let middle_shutdown = middle_peer.shutdown_handle();
    let middle_task = tokio::spawn(async move {
        let _ = middle_peer.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    // alice (upstream UAC) — sends an INVITE to the b2bua's peer port
    // with the same application headers + Privacy + stack-managed names
    // the §11.2 example uses.
    let alice = UnifiedCoordinator::new(receiver_config(
        "alice-e2e",
        PAIR_B2BUA_E2E_ALICE_PORT,
    ))
    .await
    .expect("alice coordinator");
    let mut alice_events = alice.events().await.expect("alice events");
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Stage the upstream headers on alice's INVITE — these are the
    // application headers the b2bua will carry through.
    let target = format!(
        "sip:bob@127.0.0.1:{}",
        PAIR_B2BUA_E2E_MIDDLE_PORT + 1
    );
    let call_id = alice
        .invite(
            Some(format!(
                "sip:alice@127.0.0.1:{}",
                PAIR_B2BUA_E2E_ALICE_PORT
            )),
            target.clone(),
        )
        .with_raw_header(
            HeaderName::Other("History-Info".to_string()),
            HISTORY_INFO,
        )
        .expect("History-Info raw header")
        .with_raw_header(HeaderName::Other("Diversion".to_string()), DIVERSION)
        .expect("Diversion raw header")
        .with_raw_header(HeaderName::Other("X-Customer-ID".to_string()), CUSTOMER_ID)
        .expect("X-Customer-ID raw header")
        .with_raw_header(HeaderName::Other("Privacy".to_string()), PRIVACY_VALUE)
        .expect("Privacy raw header")
        .with_raw_header(
            HeaderName::Other("P-Asserted-Identity".to_string()),
            UPSTREAM_PAI,
        )
        .expect("upstream PAI raw header")
        .send()
        .await
        .expect("alice invite send");

    // Wait for bob's inbound INVITE trace. The b2bua handler dispatches
    // the outbound leg synchronously inside `on_incoming_call`, so the
    // outbound INVITE lands shortly after alice's INVITE reaches the
    // middle.
    let trace = wait_for_inbound_method(&mut bob_events, "INVITE", Duration::from_secs(10))
        .await
        .expect("bob did not see inbound B2BUA INVITE");
    let wire = &trace.raw_message;

    // Application carry-through visible on the wire (b2bua middle re-emitted).
    assert!(
        wire.contains(HISTORY_INFO),
        "History-Info must reach bob via b2bua; wire =\n{wire}"
    );
    assert!(
        wire.contains(DIVERSION),
        "Diversion must reach bob via b2bua; wire =\n{wire}"
    );
    assert!(
        wire.contains(CUSTOMER_ID),
        "X-Customer-ID must reach bob via b2bua; wire =\n{wire}"
    );

    // Privacy stripped on the outbound leg.
    assert!(
        !wire.contains(PRIVACY_VALUE),
        "Privacy value must NOT reach bob (stripped at b2bua); wire =\n{wire}"
    );

    // PAI rewritten, not carried.
    assert!(
        wire.contains(REWRITTEN_PAI),
        "rewritten PAI must appear on bob's wire; wire =\n{wire}"
    );
    assert!(
        !wire.contains(UPSTREAM_PAI),
        "upstream PAI must NOT leak past b2bua; wire =\n{wire}"
    );

    // Drain the failure / cancellation event on alice — the b2bua
    // rejects the inbound leg with 503 once the outbound dispatch
    // settles. Don't fail the test if it never arrives within the
    // budget; the wire-side assertions above are what's load-bearing.
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match alice_events.next().await {
                Some(rvoip_sip::Event::CallFailed { call_id: id, .. })
                | Some(rvoip_sip::Event::CallEnded { call_id: id, .. })
                    if id == call_id =>
                {
                    return;
                }
                Some(_) => continue,
                None => return,
            }
        }
    })
    .await;

    middle_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), middle_task).await;
    let _ = bob.terminate_current_session().await;
    let _ = alice.terminate_current_session().await;
    let _ = outbound_coord.terminate_current_session().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
}

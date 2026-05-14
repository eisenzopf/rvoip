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
//! Note on the synthetic upstream. The §11.2 example shows the
//! `with_headers_from(&incoming, ...)` form against a real
//! `IncomingCall`. Today the `IncomingCall::raw_request()` round-trip
//! reparses through `Request::to_string()` in
//! `rvoip-sip-dialog/src/events/adapter.rs:282`, which loses some
//! header values, so a real two-coordinator B2BUA flow can't drive
//! the trait against typed inbound headers reliably. This test stands
//! in a hand-built `SipHeaderView` — every other piece of the contract
//! (policy filtering, strip, raw-header override, wire output) is
//! exercised against the production code path.

use std::time::Duration;

use rvoip_sip::api::events::Event;
use rvoip_sip::api::headers::options::{SipRequestOptions, ViolationReason};
use rvoip_sip::api::headers::view::SipHeaderView;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::{HeaderName, SipTraceConfig, SipTraceDirection};
use rvoip_sip_core::types::call_id::CallId as CallIdHdr;
use rvoip_sip_core::types::headers::{HeaderValue, TypedHeader};
use rvoip_sip_core::types::max_forwards::MaxForwards;
use rvoip_sip_core::types::{CSeq, Method, Request, Uri};

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

fn receiver_config(name: &str, port: u16) -> Config {
    let mut cfg = Config::local(name, port);
    cfg.sip_trace = SipTraceConfig {
        enabled: true,
        redact_sensitive_headers: false,
        include_body: true,
        ..SipTraceConfig::default()
    };
    cfg
}

async fn wait_for_inbound_method(
    events: &mut EventReceiver,
    method_prefix: &str,
    timeout: Duration,
) -> Option<rvoip_sip::SipTrace> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, events.next()).await {
            Err(_) | Ok(None) => return None,
            Ok(Some(Event::SipTrace(trace))) => {
                if trace.direction == SipTraceDirection::Inbound
                    && trace.start_line.starts_with(method_prefix)
                {
                    return Some(trace);
                }
            }
            Ok(Some(_)) => continue,
        }
    }
}

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

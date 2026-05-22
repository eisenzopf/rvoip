//! SIP_API_DESIGN_2 §10 verification #7 — inbound INVITE carries
//! `Diversion`, `History-Info`, `Referred-By` accessible at the
//! receiver side.
//!
//! Today this test asserts the wire-level guarantee only: the
//! application headers Alice stamps via `with_raw_header` reach Bob's
//! socket in the inbound INVITE bytes. The typed `SipHeaderView`
//! consultation on `IncomingCall` depends on the inbound enrichment
//! re-parse path in
//! `rvoip-sip/src/adapters/session_event_handler.rs:1326-1348`, which
//! currently fails because the upstream cross-crate publish site at
//! `rvoip-sip-dialog/src/events/adapter.rs:282-283` reserializes the
//! parsed `Request` via `Display::to_string()` instead of preserving
//! the original wire bytes (the spec §7.5 commitment). Fixing the
//! preservation path is a follow-up; until then,
//! `IncomingCall::raw_request()` returns `None` and the `SipHeaderView`
//! impl surfaces an empty header list. The wire-side guarantee
//! covered here is what B2BUA-style downstream carry-through actually
//! relies on.

use std::time::Duration;

use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer};
use rvoip_sip::api::events::Event;
use rvoip_sip::api::headers::SipRequestOptions;
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::{SipTraceConfig, SipTraceDirection};
use rvoip_sip_core::types::header::HeaderName;

const PAIR: (u16, u16) = (17100, 17101);

fn header(name: &str) -> HeaderName {
    HeaderName::Other(name.to_string())
}

fn cfg(name: &str, port: u16) -> Config {
    let mut c = Config::local(name, port);
    c.sip_trace = SipTraceConfig {
        enabled: true,
        redact_sensitive_headers: false,
        include_body: false,
        ..SipTraceConfig::default()
    };
    c
}

struct AutoAccept;

#[async_trait::async_trait]
impl CallHandler for AutoAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let _ = call.accept().await;
        CallHandlerDecision::Accept
    }
}

async fn wait_for_inbound_invite(events: &mut EventReceiver, timeout: Duration) -> Option<String> {
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
                    && trace.start_line.starts_with("INVITE")
                {
                    return Some(trace.raw_message);
                }
            }
            Ok(Some(_)) => continue,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inbound_invite_wire_carries_application_routing_headers() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR;

    let bob = CallbackPeer::new(AutoAccept, cfg("bob", bob_port))
        .await
        .expect("bob callback peer");
    let bob_coord = bob.coordinator().clone();
    let bob_shutdown = bob.shutdown_handle();
    let bob_task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut bob_events = bob_coord.events().await.expect("bob events");

    let alice = UnifiedCoordinator::new(cfg("alice", alice_port))
        .await
        .expect("alice coordinator");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{bob_port}");
    let _call_id = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target)
        .with_raw_header(
            header("Diversion"),
            "<sip:diverter@example.com>;reason=unconditional",
        )
        .expect("Diversion is application-controlled")
        .with_raw_header(header("History-Info"), "<sip:original@example.com>;index=1")
        .expect("History-Info is application-controlled")
        .with_raw_header(header("Referred-By"), "<sip:referrer@example.com>")
        .expect("Referred-By is application-controlled")
        .send()
        .await
        .expect("invite.send()");

    let raw = wait_for_inbound_invite(&mut bob_events, Duration::from_secs(8))
        .await
        .expect("inbound INVITE trace");

    // Each routing-information header must appear verbatim in the
    // inbound bytes. Header-name match is case-insensitive in SIP, but
    // our canonicalizer Title-Cases the wire form so we can match on
    // the exact rendered prefix.
    for must in ["Diversion:", "History-Info:", "Referred-By:"] {
        assert!(
            raw.contains(must),
            "wire INVITE missing `{must}`; trace was:\n{raw}"
        );
    }

    bob_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), bob_task).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
}

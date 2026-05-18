//! SIP_API_DESIGN_2 §10 verification #30 — `Config.auto_emit_extra_headers`
//! is applied to internally-emitted teardown messages without
//! application code.
//!
//! Today the test exercises the BYE side: Alice configures
//! `auto_emit_extra_headers = [X-AutoEmit: trace]`, establishes a
//! call with Bob, and then calls `coord.hangup(...)` (the legacy
//! teardown path, which routes through `Action::SendBYE` rather than
//! `Action::SendBYEWithOptions`). That action consults
//! `dialog_adapter.auto_emit_extra_headers` when the
//! `pending_bye_options` stash is empty, builds a synthetic
//! `ByeRequestOptions`, and dispatches via the same wire path as
//! application-staged BYEs.

use std::time::Duration;

use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer};
use rvoip_sip::api::events::Event;
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::{SipTraceConfig, SipTraceDirection};
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderValue, TypedHeader};

const PAIR: (u16, u16) = (17600, 17601);
const AUTO_HEADER_NAME: &str = "X-AutoEmit";
const AUTO_HEADER_VALUE: &str = "operator-trace";

fn cfg_with_auto_emit(name: &str, port: u16) -> Config {
    let mut c = Config::local(name, port);
    c.sip_trace = SipTraceConfig {
        enabled: true,
        redact_sensitive_headers: false,
        include_body: false,
        ..SipTraceConfig::default()
    };
    c.auto_emit_extra_headers = vec![TypedHeader::Other(
        HeaderName::Other(AUTO_HEADER_NAME.to_string()),
        HeaderValue::Raw(AUTO_HEADER_VALUE.as_bytes().to_vec()),
    )];
    c
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

async fn wait_for_inbound_method(
    events: &mut EventReceiver,
    method_prefix: &str,
    timeout: Duration,
) -> Option<String> {
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
                    return Some(trace.raw_message);
                }
            }
            Ok(Some(_)) => continue,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn config_auto_emit_extra_headers_stamps_legacy_bye() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR;

    // Bob auto-accepts; no auto-emit headers on his side.
    let bob = CallbackPeer::new(AutoAccept, cfg("bob-ae", bob_port))
        .await
        .expect("bob");
    let bob_coord = bob.coordinator().clone();
    let bob_shutdown = bob.shutdown_handle();
    let bob_task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut bob_events = bob_coord.events().await.expect("bob events");

    // Alice carries the auto-emit header.
    let alice = UnifiedCoordinator::new(cfg_with_auto_emit("alice-ae", alice_port))
        .await
        .expect("alice");
    let mut alice_events = alice.events().await.expect("alice events");
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Establish a call.
    let target = format!("sip:bob@127.0.0.1:{bob_port}");
    let session_id = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), &target)
        .send()
        .await
        .expect("invite");

    // Wait for Alice to see CallAnswered before hanging up.
    let mut answered = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while !answered && tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), alice_events.next()).await {
            Ok(Some(Event::CallAnswered { call_id, .. })) if call_id == session_id => {
                answered = true;
            }
            _ => {}
        }
    }
    assert!(answered, "alice never saw CallAnswered");

    // Drain any inbound INVITE bob has logged so the next inbound
    // trace we wait on is unambiguously the BYE.
    let _ = wait_for_inbound_method(&mut bob_events, "INVITE", Duration::from_millis(500)).await;

    // Trigger BYE via the legacy hangup path (no pending_bye_options
    // staged — exactly the path the auto-emit fallback targets).
    alice.hangup(&session_id).await.expect("hangup");

    let bye_trace = wait_for_inbound_method(&mut bob_events, "BYE", Duration::from_secs(8))
        .await
        .expect("bob did not see inbound BYE trace");

    assert!(
        bye_trace.contains(AUTO_HEADER_NAME),
        "BYE on wire missing `{AUTO_HEADER_NAME}`; trace:\n{bye_trace}"
    );
    assert!(
        bye_trace.contains(AUTO_HEADER_VALUE),
        "BYE on wire missing auto-emit value `{AUTO_HEADER_VALUE}`; trace:\n{bye_trace}"
    );

    bob_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), bob_task).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
}

//! SIP_API_DESIGN_2 §10 verification #23 — stash lifecycle.
//!
//! The §7.3 invariants this file covers:
//!
//! - **(a) Set-once, consumed-once at final response.** After a
//!   successful `.send().await` on an in-dialog method, the per-method
//!   `pending_<method>_options` slot is cleared, so a *subsequent*
//!   request on the same session and method does NOT carry header
//!   residue from the previous send.
//!
//! - **(b) Conflict guard on single in-flight per (session, method).**
//!   Concurrent staging on the same slot returns
//!   `SessionError::Conflict { method }`. This sub-case is covered by
//!   `sip_api_design_2_section_10_skeletons::conflict_guard_integration`;
//!   referenced here for completeness.
//!
//! - **(c) Different methods are independent.** Simultaneous `.info()`
//!   and `.notify()` on the same session use distinct stash slots
//!   (`pending_info_options` vs `pending_notify_options`) and both
//!   succeed.
//!
//! The (a) and (c) sub-cases run end-to-end against a real
//! INVITE → 200 OK → ACK dialog: alice originates, bob auto-accepts
//! via `CallHandler`, then alice issues mid-dialog requests while
//! bob's inbound trace stream captures the wire.

use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::callback_peer::{
    CallHandler, CallHandlerDecision, CallbackPeer, ShutdownHandle,
};
use rvoip_sip::api::events::Event;
use rvoip_sip::api::headers::SipRequestOptions;
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::{HeaderName, SipTraceConfig, SipTraceDirection};

const PAIR_STASH_REUSE: (u16, u16) = (16500, 16510);
const PAIR_STASH_INDEPENDENT: (u16, u16) = (16520, 16530);

const TRACE_HEADER_NAME: &str = "X-Stash-Trace";
const TRACE_HEADER_VALUE: &str = "first-only";
const SMOKE_HEADER_NAME: &str = "X-Test";
const SMOKE_INFO_VALUE: &str = "info-side";
const SMOKE_NOTIFY_VALUE: &str = "notify-side";

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

struct AutoAccept;
#[async_trait::async_trait]
impl CallHandler for AutoAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let _ = call.accept().await;
        CallHandlerDecision::Accept
    }
}

async fn boot_callback_receiver(
    port: u16,
    name: &str,
) -> (
    Arc<UnifiedCoordinator>,
    tokio::task::JoinHandle<()>,
    ShutdownHandle,
) {
    let bob = CallbackPeer::new(AutoAccept, receiver_config(name, port))
        .await
        .expect("callback peer");
    let coord = bob.coordinator().clone();
    let shutdown = bob.shutdown_handle();
    let task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    (coord, task, shutdown)
}

async fn wait_for_call_answered(
    events: &mut EventReceiver,
    target_call_id: &rvoip_sip::api::events::CallId,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return false;
        }
        match tokio::time::timeout(remaining, events.next()).await {
            Err(_) | Ok(None) => return false,
            Ok(Some(Event::CallAnswered { call_id, .. })) if &call_id == target_call_id => {
                return true;
            }
            Ok(Some(_)) => continue,
        }
    }
}

async fn next_inbound(
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

/// §10 #23 sub-case (a) — after a successful `.send()`, the
/// `pending_info_options` slot is cleared. A subsequent INFO on the
/// same session that omits the trace header MUST NOT carry residue.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stash_clears_between_successive_in_dialog_sends() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_STASH_REUSE;

    let (bob, bob_task, bob_shutdown) = boot_callback_receiver(bob_port, "bob").await;
    let mut bob_events = bob.events().await.expect("bob events");

    let alice = UnifiedCoordinator::new(receiver_config("alice", alice_port))
        .await
        .expect("alice coordinator");
    let mut alice_events = alice.events().await.expect("alice events");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    let call_id = alice
        .invite(
            Some(format!("sip:alice@127.0.0.1:{}", alice_port)),
            target,
        )
        .send()
        .await
        .expect("invite().send()");

    assert!(
        wait_for_call_answered(&mut alice_events, &call_id, Duration::from_secs(10)).await,
        "alice did not see CallAnswered"
    );
    // Drain bob's INVITE so the first INFO assertion doesn't match it.
    let _ = next_inbound(&mut bob_events, "INVITE", Duration::from_secs(2)).await;

    // First INFO: stage with the trace header.
    alice
        .info(&call_id, "application/dtmf-relay")
        .with_body("Signal=1\r\nDuration=160\r\n")
        .with_raw_header(
            HeaderName::Other(TRACE_HEADER_NAME.to_string()),
            TRACE_HEADER_VALUE,
        )
        .expect("with_raw_header on first INFO")
        .send()
        .await
        .expect("first info().send()");

    let first = next_inbound(&mut bob_events, "INFO", Duration::from_secs(10))
        .await
        .expect("bob did not see first INFO trace");
    assert!(
        first.raw_message.contains(TRACE_HEADER_NAME),
        "first INFO must carry trace header; wire =\n{}",
        first.raw_message
    );
    assert!(
        first.raw_message.contains(TRACE_HEADER_VALUE),
        "first INFO trace value must be on the wire; wire =\n{}",
        first.raw_message
    );

    // Second INFO: NO trace header. If the stash leaked, the second
    // INFO would carry the same X-Stash-Trace value.
    alice
        .info(&call_id, "application/dtmf-relay")
        .with_body("Signal=2\r\nDuration=160\r\n")
        .send()
        .await
        .expect("second info().send()");

    let second = next_inbound(&mut bob_events, "INFO", Duration::from_secs(10))
        .await
        .expect("bob did not see second INFO trace");
    assert!(
        !second.raw_message.contains(TRACE_HEADER_NAME),
        "stash residue leak: trace header MUST NOT appear on second INFO; wire =\n{}",
        second.raw_message
    );
    assert!(
        !second.raw_message.contains(TRACE_HEADER_VALUE),
        "stash residue leak: trace value MUST NOT appear on second INFO; wire =\n{}",
        second.raw_message
    );
    // And the second INFO is structurally valid — body distinguishes
    // it from the first.
    assert!(
        second.raw_message.contains("Signal=2"),
        "second INFO body should carry Signal=2; wire =\n{}",
        second.raw_message
    );

    let _ = alice.bye(&call_id).send().await;
    bob_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), bob_task).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
}

/// §10 #23 sub-case (c) — `pending_info_options` and
/// `pending_notify_options` are independent slots. Two concurrent
/// `.send()` futures on the same session, one INFO and one NOTIFY,
/// must both succeed without `SessionError::Conflict`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stash_slots_are_independent_across_methods() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_STASH_INDEPENDENT;

    let (bob, bob_task, bob_shutdown) = boot_callback_receiver(bob_port, "bob").await;
    let mut bob_events = bob.events().await.expect("bob events");

    let alice = UnifiedCoordinator::new(receiver_config("alice", alice_port))
        .await
        .expect("alice coordinator");
    let mut alice_events = alice.events().await.expect("alice events");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{}", bob_port);
    let call_id = alice
        .invite(
            Some(format!("sip:alice@127.0.0.1:{}", alice_port)),
            target,
        )
        .send()
        .await
        .expect("invite().send()");

    assert!(
        wait_for_call_answered(&mut alice_events, &call_id, Duration::from_secs(10)).await,
        "alice did not see CallAnswered"
    );
    let _ = next_inbound(&mut bob_events, "INVITE", Duration::from_secs(2)).await;

    // Launch INFO and NOTIFY simultaneously. Different stash slots →
    // no Conflict, both must succeed.
    let info_fut = {
        let alice = alice.clone();
        let cid = call_id.clone();
        async move {
            alice
                .info(&cid, "application/dtmf-relay")
                .with_body("Signal=1\r\nDuration=160\r\n")
                .with_raw_header(
                    HeaderName::Other(SMOKE_HEADER_NAME.to_string()),
                    SMOKE_INFO_VALUE,
                )
                .expect("with_raw_header on INFO")
                .send()
                .await
        }
    };
    let notify_fut = {
        let alice = alice.clone();
        let cid = call_id.clone();
        async move {
            alice
                .notify(&cid, "presence")
                .with_subscription_state("active;expires=3600")
                .with_raw_header(
                    HeaderName::Other(SMOKE_HEADER_NAME.to_string()),
                    SMOKE_NOTIFY_VALUE,
                )
                .expect("with_raw_header on NOTIFY")
                .send()
                .await
        }
    };

    let (info_res, notify_res) = tokio::join!(info_fut, notify_fut);
    info_res.expect("concurrent INFO must succeed (independent slot)");
    notify_res.expect("concurrent NOTIFY must succeed (independent slot)");

    // Collect both inbound traces — they may arrive in either order.
    let mut saw_info = false;
    let mut saw_notify = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while (!saw_info || !saw_notify) && tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, bob_events.next()).await {
            Err(_) | Ok(None) => break,
            Ok(Some(Event::SipTrace(trace))) => {
                if trace.direction != SipTraceDirection::Inbound {
                    continue;
                }
                if trace.start_line.starts_with("INFO")
                    && trace.raw_message.contains(SMOKE_INFO_VALUE)
                {
                    saw_info = true;
                }
                if trace.start_line.starts_with("NOTIFY")
                    && trace.raw_message.contains(SMOKE_NOTIFY_VALUE)
                {
                    saw_notify = true;
                }
            }
            Ok(Some(_)) => continue,
        }
    }

    assert!(saw_info, "INFO with its smoke value did not arrive on the wire");
    assert!(
        saw_notify,
        "NOTIFY with its smoke value did not arrive on the wire"
    );

    let _ = alice.bye(&call_id).send().await;
    bob_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), bob_task).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
}

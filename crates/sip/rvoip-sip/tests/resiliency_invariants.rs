//! Runtime resiliency invariants for high-risk SIP teardown paths.
//!
//! These tests use the perf-only retained-object snapshot because the stable
//! public API should not expose internal maps and transaction runners. They
//! are a focused complement to the longer soak gate: fast enough for targeted
//! regression runs, strict enough to catch retained sessions, dialogs, media
//! receivers, transaction runners, and cleanup work.

#![cfg(feature = "perf-tests")]

use std::time::Duration;

use rvoip_sip::api::events::{CallId, Event};
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::CallState;

mod support;

use support::{
    assert_no_watchdog_fallback, assert_pair_released, assert_single_endpoint_released,
    boot_ringing_uas, boot_unified_caller_with_config, establish_call, receiver_config,
    watchdog_counters,
};

const BYE_ALICE_PORT: u16 = 16800;
const BYE_BOB_PORT: u16 = 16810;
const CANCEL_ALICE_PORT: u16 = 16820;
const CANCEL_BOB_PORT: u16 = 16830;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn established_bye_releases_all_runtime_owners_without_watchdog_fallback() {
    let _ = tracing_subscriber::fmt::try_init();
    let mut call = establish_call(BYE_ALICE_PORT, BYE_BOB_PORT).await;
    let watchdog_before = watchdog_counters(&call.alice).await;

    call.alice
        .bye(&call.call_id)
        .send()
        .await
        .expect("BYE send");
    wait_for_terminal_for_call(
        &mut call.alice_events,
        &call.call_id,
        Duration::from_secs(5),
    )
    .await
    .expect("alice terminal event");
    let _ = wait_for_any_terminal(&mut call.bob_events, Duration::from_secs(5)).await;

    assert_pair_released(
        "established BYE",
        &call.alice,
        &call.bob.coord,
        Duration::from_secs(40),
    )
    .await;
    let watchdog_after = watchdog_counters(&call.alice).await;
    assert_no_watchdog_fallback(watchdog_before, watchdog_after);

    let alice = call.alice;
    let bob = call.bob;
    alice.shutdown();
    bob.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pre_answer_cancel_releases_all_runtime_owners_without_watchdog_fallback() {
    let _ = tracing_subscriber::fmt::try_init();
    let ringing_uas = boot_ringing_uas(CANCEL_BOB_PORT, Duration::from_millis(100)).await;
    let alice = boot_unified_caller_with_config(
        receiver_config("cancel-alice", CANCEL_ALICE_PORT)
            .with_cleanup_diagnostics(true)
            .with_setup_teardown_timeout_secs(10),
    )
    .await;
    let watchdog_before = watchdog_counters(&alice).await;

    let call_id = alice
        .invite(
            Some(format!("sip:alice@127.0.0.1:{CANCEL_ALICE_PORT}")),
            format!("sip:bob@127.0.0.1:{CANCEL_BOB_PORT}"),
        )
        .send()
        .await
        .expect("INVITE send");
    let handle = alice.session(&call_id);
    wait_for_state(&handle, CallState::Ringing, Duration::from_secs(5))
        .await
        .expect("call reaches Ringing before CANCEL");

    let terminal = handle
        .hangup_and_wait(Some(Duration::from_secs(8)))
        .await
        .expect("CANCEL terminal event");
    assert_eq!(terminal, "Cancelled");
    ringing_uas
        .wait_for(|request| request.method == "CANCEL", Duration::from_secs(2))
        .await
        .expect("CANCEL reached raw UAS");

    // The INVITE client transaction absorbs possible 487 retransmissions via
    // Timer D. Keep this focused test below one minute while still requiring
    // true transaction-runner cleanup.
    assert_single_endpoint_released("pre-answer CANCEL", &alice, Duration::from_secs(40)).await;
    let watchdog_after = watchdog_counters(&alice).await;
    assert_no_watchdog_fallback(watchdog_before, watchdog_after);

    alice.shutdown();
    ringing_uas.shutdown();
}

async fn wait_for_terminal_for_call(
    events: &mut EventReceiver,
    call_id: &CallId,
    timeout: Duration,
) -> Option<Event> {
    wait_for_terminal(events, timeout, |event| event.call_id() == Some(call_id)).await
}

async fn wait_for_any_terminal(events: &mut EventReceiver, timeout: Duration) -> Option<Event> {
    wait_for_terminal(events, timeout, |_| true).await
}

async fn wait_for_terminal<F>(
    events: &mut EventReceiver,
    timeout: Duration,
    predicate: F,
) -> Option<Event>
where
    F: Fn(&Event) -> bool,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, events.next()).await {
            Err(_) | Ok(None) => return None,
            Ok(Some(event)) if is_terminal(&event) && predicate(&event) => return Some(event),
            Ok(Some(_)) => continue,
        }
    }
}

fn is_terminal(event: &Event) -> bool {
    matches!(
        event,
        Event::CallEnded { .. } | Event::CallFailed { .. } | Event::CallCancelled { .. }
    )
}

async fn wait_for_state(
    handle: &rvoip_sip::api::handle::SessionHandle,
    expected: CallState,
    timeout: Duration,
) -> Option<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if matches!(handle.state().await, Ok(state) if state == expected) {
            return Some(());
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

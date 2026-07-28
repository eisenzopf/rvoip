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
//! INVITE → 200 OK → ACK dialog established via the shared
//! `tests/support/` harness.

use std::time::Duration;

use rvoip_sip::api::events::Event;
use rvoip_sip::api::headers::SipRequestOptions;
use rvoip_sip::{CallState, HeaderName, SessionError, SipTraceDirection, StreamPeer};
use rvoip_sip_core::Method;

mod support;

use support::{
    boot_unified_caller, establish_call, establish_call_with_handler, receiver_config,
    wait_for_call_answered, wait_for_inbound_method, AutoAcceptUnsupportedInfo,
};

const PAIR_STASH_REUSE: (u16, u16) = (16500, 16510);
const PAIR_STASH_INDEPENDENT: (u16, u16) = (16520, 16530);
const PAIR_INFO_REJECTION: (u16, u16) = (16540, 16550);
const PAIR_INFO_DEFAULT: (u16, u16) = (16560, 16570);
const PAIR_INFO_STREAM_DROP: (u16, u16) = (16580, 16590);
const PAIR_INFO_NO_OWNER: (u16, u16) = (16600, 16610);

const TRACE_HEADER_NAME: &str = "X-Stash-Trace";
const TRACE_HEADER_VALUE: &str = "first-only";
const SMOKE_HEADER_NAME: &str = "X-Test";
const SMOKE_INFO_VALUE: &str = "info-side";
const SMOKE_NOTIFY_VALUE: &str = "notify-side";

fn wire_headers(raw_message: &str) -> impl Iterator<Item = (&str, &str)> {
    raw_message
        .lines()
        .skip(1)
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name, value.trim()))
}

fn wire_header_values<'a>(raw_message: &'a str, expected_name: &str) -> Vec<&'a str> {
    wire_headers(raw_message)
        .filter_map(|(name, value)| name.eq_ignore_ascii_case(expected_name).then_some(value))
        .collect()
}

fn wire_via_branch(via: &str) -> Option<&str> {
    via.split(';').find_map(|parameter| {
        let (name, value) = parameter.trim().split_once('=')?;
        name.eq_ignore_ascii_case("branch").then_some(value)
    })
}

fn wire_body(raw_message: &str) -> &str {
    raw_message
        .split_once("\r\n\r\n")
        .or_else(|| raw_message.split_once("\n\n"))
        .map_or("", |(_, body)| body)
}

fn assert_verbatim_packet(trace: &rvoip_sip::SipTrace) {
    assert!(
        !trace.redacted,
        "wire-contract assertions require explicit development trace passthrough"
    );
    assert!(
        !trace.truncated,
        "wire-contract assertions require a complete SIP packet"
    );
}

async fn wait_for_inbound_response(
    events: &mut rvoip_sip::api::stream_peer::EventReceiver,
    status: u16,
    method: &str,
    timeout: Duration,
) -> rvoip_sip::SipTrace {
    tokio::time::timeout(timeout, async {
        loop {
            match events.next().await {
                Some(Event::SipTrace(trace))
                    if trace.direction == SipTraceDirection::Inbound
                        && trace.start_line.starts_with(&format!("SIP/2.0 {status}"))
                        && wire_header_values(&trace.raw_message, "CSeq")
                            .iter()
                            .any(|value| value.ends_with(&format!(" {method}"))) =>
                {
                    break trace;
                }
                Some(_) => continue,
                None => panic!("event stream closed before exact response"),
            }
        }
    })
    .await
    .expect("peer did not send an exact terminal response")
}

async fn assert_no_duplicate_inbound_response(
    events: &mut rvoip_sip::api::stream_peer::EventReceiver,
    status: u16,
    method: &str,
) {
    let duplicate = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            match events.next().await {
                Some(Event::SipTrace(trace))
                    if trace.direction == SipTraceDirection::Inbound
                        && trace.start_line.starts_with(&format!("SIP/2.0 {status}"))
                        && wire_header_values(&trace.raw_message, "CSeq")
                            .iter()
                            .any(|value| value.ends_with(&format!(" {method}"))) =>
                {
                    break;
                }
                Some(_) => continue,
                None => break,
            }
        }
    })
    .await;
    assert!(
        duplicate.is_err(),
        "received a duplicate {status} response for {method}"
    );
}

/// §10 #23 sub-case (a) — `.send()` returns after the exact first transport
/// write, while the same-method slot remains owned until the terminal
/// transaction event. After that event, a subsequent INFO that omits the trace
/// header MUST NOT carry residue.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stash_clears_between_successive_in_dialog_sends() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_STASH_REUSE;
    let mut call = establish_call(alice_port, bob_port).await;

    // First INFO: stage with the trace header.
    call.alice
        .info(&call.call_id, "application/dtmf-relay")
        .with_body("Signal=1\r\nDuration=160\r\n")
        .with_raw_header(
            HeaderName::Other(TRACE_HEADER_NAME.to_string()),
            TRACE_HEADER_VALUE,
        )
        .expect("with_raw_header on first INFO")
        .send()
        .await
        .expect("first info().send()");

    let first = wait_for_inbound_method(&mut call.bob_events, "INFO", Duration::from_secs(10))
        .await
        .expect("bob did not see first INFO trace");
    assert_verbatim_packet(&first);
    assert_eq!(
        wire_header_values(&first.raw_message, TRACE_HEADER_NAME),
        vec![TRACE_HEADER_VALUE],
        "first INFO must carry exactly one trace header; wire =\n{}",
        first.raw_message,
    );

    // Bob's test handler authors a 200 on the exact INFO server transaction.
    // Assert that the response copied this request's Via and CSeq rather than
    // being routed to the retained INVITE or another in-dialog transaction.
    let first_via = wire_header_values(&first.raw_message, "Via");
    let response =
        wait_for_inbound_response(&mut call.alice_events, 200, "INFO", Duration::from_secs(10))
            .await;
    assert_verbatim_packet(&response);
    let response_via = wire_header_values(&response.raw_message, "Via");
    assert_eq!(
        response_via.len(),
        first_via.len(),
        "INFO response must preserve the exact request Via count"
    );
    let response_branch = response_via
        .first()
        .and_then(|value| wire_via_branch(value))
        .expect("INFO response Via must carry a branch");
    let request_branch = first_via
        .first()
        .and_then(|value| wire_via_branch(value))
        .expect("INFO request Via must carry a branch");
    assert_eq!(
        response_branch, request_branch,
        "INFO response must correlate to the exact request Via branch"
    );
    assert_eq!(
        wire_header_values(&response.raw_message, "CSeq"),
        wire_header_values(&first.raw_message, "CSeq"),
        "INFO response must correlate to the exact request CSeq"
    );

    // Second INFO: NO trace header. If the stash leaked, the second
    // INFO would carry the same X-Stash-Trace value.
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match call
                .alice
                .info(&call.call_id, "application/dtmf-relay")
                .with_body("Signal=2\r\nDuration=160\r\n")
                .send()
                .await
            {
                Ok(()) => break,
                Err(SessionError::Conflict {
                    method: Method::Info,
                }) => tokio::time::sleep(Duration::from_millis(10)).await,
                Err(error) => panic!("second INFO failed unexpectedly: {error}"),
            }
        }
    })
    .await
    .expect("first INFO terminal response did not release the exact request slot");

    assert_eq!(
        call.alice
            .get_state(&call.call_id)
            .await
            .expect("caller state after exact INFO response"),
        CallState::Active,
        "an in-dialog INFO response must not transition the surrounding call"
    );

    let second = wait_for_inbound_method(&mut call.bob_events, "INFO", Duration::from_secs(10))
        .await
        .expect("bob did not see second INFO trace");
    assert_verbatim_packet(&second);
    assert!(
        wire_header_values(&second.raw_message, TRACE_HEADER_NAME).is_empty(),
        "stash residue leak: trace header MUST NOT appear on second INFO; wire =\n{}",
        second.raw_message,
    );
    assert!(
        wire_body(&second.raw_message).contains("Signal=2"),
        "second INFO body should carry Signal=2; wire =\n{}",
        second.raw_message
    );

    call.teardown().await;
}

/// A non-2xx INFO response is transaction-local and must not run the INVITE
/// rejection state machine or terminate the established call.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn in_dialog_info_4xx_response_keeps_call_active() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_INFO_REJECTION;
    let mut call = establish_call(alice_port, bob_port).await;

    call.alice
        .info(&call.call_id, "application/dtmf-relay")
        .with_body("Signal=4\r\nResponse=488\r\n")
        .send()
        .await
        .expect("send INFO that test UAS rejects");
    let response =
        wait_for_inbound_response(&mut call.alice_events, 488, "INFO", Duration::from_secs(10))
            .await;
    assert_verbatim_packet(&response);
    assert_eq!(
        wire_header_values(&response.raw_message, "X-Exact-Info-Response"),
        vec!["transaction-local"],
        "exact INFO rejection must preserve staged response headers"
    );

    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match call
                .alice
                .info(&call.call_id, "application/dtmf-relay")
                .with_body("Signal=5\r\nDuration=160\r\n")
                .send()
                .await
            {
                Ok(()) => break,
                Err(SessionError::Conflict {
                    method: Method::Info,
                }) => tokio::time::sleep(Duration::from_millis(10)).await,
                Err(error) => panic!("INFO after 488 failed unexpectedly: {error}"),
            }
        }
    })
    .await
    .expect("488 did not release the exact INFO request slot");
    assert_eq!(
        call.alice
            .get_state(&call.call_id)
            .await
            .expect("caller state after INFO 488"),
        CallState::Active
    );
    let bob_sessions = call.bob.coord.list_sessions().await;
    assert_eq!(bob_sessions.len(), 1, "expected one Bob-side session");
    assert_eq!(
        bob_sessions[0].state,
        CallState::Active,
        "exact INFO rejection must not terminate the UAS dialog"
    );

    call.teardown().await;
}

/// A callback handler that does not implement INFO still resolves the exact
/// transaction once with 501, and the surrounding dialog remains active.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn default_info_handler_sends_one_exact_501_and_releases_slot() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_INFO_DEFAULT;
    let mut call =
        establish_call_with_handler(AutoAcceptUnsupportedInfo, alice_port, bob_port).await;

    call.alice
        .info(&call.call_id, "application/dtmf-relay")
        .with_body("Signal=6\r\nDuration=160\r\n")
        .send()
        .await
        .expect("send unsupported INFO");
    let response =
        wait_for_inbound_response(&mut call.alice_events, 501, "INFO", Duration::from_secs(10))
            .await;
    assert_verbatim_packet(&response);
    assert_no_duplicate_inbound_response(&mut call.alice_events, 501, "INFO").await;

    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match call
                .alice
                .info(&call.call_id, "application/dtmf-relay")
                .with_body("Signal=7\r\nDuration=160\r\n")
                .send()
                .await
            {
                Ok(()) => break,
                Err(SessionError::Conflict {
                    method: Method::Info,
                }) => tokio::time::sleep(Duration::from_millis(10)).await,
                Err(error) => panic!("INFO after default 501 failed unexpectedly: {error}"),
            }
        }
    })
    .await
    .expect("default 501 did not release the exact INFO request slot");
    let _ = wait_for_inbound_response(&mut call.alice_events, 501, "INFO", Duration::from_secs(10))
        .await;

    assert_eq!(
        call.alice
            .get_state(&call.call_id)
            .await
            .expect("caller state after default INFO response"),
        CallState::Active
    );
    assert_eq!(
        call.bob.coord.list_sessions().await[0].state,
        CallState::Active
    );

    call.teardown().await;
}

/// Dropping a response-bearing StreamPeer event resolves its exact response
/// obligation through the coordinator-owned task supervisor.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discarded_stream_info_event_sends_exact_501() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_INFO_STREAM_DROP;
    let mut bob = StreamPeer::with_config(receiver_config("bob-stream", bob_port))
        .await
        .expect("stream peer");
    let bob_coord = bob.coordinator().clone();
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let bob_task = tokio::spawn(async move {
        let incoming = bob.wait_for_incoming().await.expect("stream incoming call");
        incoming.accept().await.expect("accept stream call");
        loop {
            match bob.next_event().await {
                Some(Event::InfoReceived { .. }) => break,
                Some(_) => continue,
                None => panic!("stream event owner closed before INFO"),
            }
        }
        let _ = dropped_tx.send(());
        let _ = shutdown_rx.await;
        bob.shutdown().await.expect("shutdown stream peer");
    });

    let alice = boot_unified_caller(alice_port, "alice-stream").await;
    let mut alice_events = alice.events().await.expect("alice events");
    let call_id = alice
        .invite(
            Some(format!("sip:alice@127.0.0.1:{alice_port}")),
            format!("sip:bob@127.0.0.1:{bob_port}"),
        )
        .send()
        .await
        .expect("stream test INVITE");
    assert!(
        wait_for_call_answered(&mut alice_events, &call_id, Duration::from_secs(10)).await,
        "stream call was not answered"
    );

    alice
        .info(&call_id, "application/dtmf-relay")
        .with_body("Signal=8\r\nDuration=160\r\n")
        .send()
        .await
        .expect("send INFO to stream owner");
    dropped_rx.await.expect("stream owner dropped INFO");
    let response =
        wait_for_inbound_response(&mut alice_events, 501, "INFO", Duration::from_secs(10)).await;
    assert_verbatim_packet(&response);
    assert_eq!(
        alice
            .get_state(&call_id)
            .await
            .expect("caller state after stream default response"),
        CallState::Active
    );
    assert_eq!(bob_coord.list_sessions().await[0].state, CallState::Active);

    let _ = alice.bye(&call_id).send().await;
    let _ = shutdown_tx.send(());
    tokio::time::timeout(Duration::from_secs(3), bob_task)
        .await
        .expect("stream peer task shutdown")
        .expect("stream peer task panicked");
}

/// A coordinator with no claimed control-event owner rejects INFO promptly
/// with exact 503 instead of retaining an unanswered server transaction.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missing_info_control_owner_sends_exact_503_and_releases_slot() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_INFO_NO_OWNER;
    let bob = rvoip_sip::UnifiedCoordinator::new(
        receiver_config("bob-no-info-owner", bob_port).with_fast_auto_accept_incoming_calls(true),
    )
    .await
    .expect("no-owner Bob coordinator");
    let mut bob_observations = bob.events().await.expect("Bob public observations");
    let alice = boot_unified_caller(alice_port, "alice-no-info-owner").await;
    let mut alice_events = alice.events().await.expect("alice events");
    let call_id = alice
        .invite(
            Some(format!("sip:alice@127.0.0.1:{alice_port}")),
            format!("sip:bob@127.0.0.1:{bob_port}"),
        )
        .send()
        .await
        .expect("no-owner test INVITE");
    assert!(
        wait_for_call_answered(&mut alice_events, &call_id, Duration::from_secs(10)).await,
        "no-owner call was not answered"
    );

    alice
        .info(&call_id, "application/dtmf-relay")
        .with_body("Signal=9\r\nDuration=160\r\n")
        .send()
        .await
        .expect("send INFO without control owner");
    let response =
        wait_for_inbound_response(&mut alice_events, 503, "INFO", Duration::from_secs(10)).await;
    assert_verbatim_packet(&response);
    let observed_request = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(Event::InfoReceived { request, .. }) = bob_observations.next().await {
                break request;
            }
        }
    })
    .await
    .expect("no-owner INFO remained publicly observable");
    assert!(
        observed_request.respond(200).is_err(),
        "public INFO observation retained exact response capability"
    );
    let duplicate_observation = tokio::time::timeout(Duration::from_millis(250), async {
        loop {
            if matches!(
                bob_observations.next().await,
                Some(Event::InfoReceived { .. })
            ) {
                break;
            }
        }
    })
    .await;
    assert!(
        duplicate_observation.is_err(),
        "no-owner INFO was published more than once"
    );

    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match alice
                .info(&call_id, "application/dtmf-relay")
                .with_body("Signal=0\r\nDuration=160\r\n")
                .send()
                .await
            {
                Ok(()) => break,
                Err(SessionError::Conflict {
                    method: Method::Info,
                }) => tokio::time::sleep(Duration::from_millis(10)).await,
                Err(error) => panic!("INFO after no-owner 503 failed: {error}"),
            }
        }
    })
    .await
    .expect("no-owner 503 did not release the exact INFO slot");
    let _ =
        wait_for_inbound_response(&mut alice_events, 503, "INFO", Duration::from_secs(10)).await;
    assert_eq!(
        alice
            .get_state(&call_id)
            .await
            .expect("caller state after no-owner INFO"),
        CallState::Active
    );
    assert_eq!(bob.list_sessions().await[0].state, CallState::Active);

    let _ = alice.bye(&call_id).send().await;
    bob.shutdown_gracefully(Some(Duration::ZERO))
        .await
        .expect("shutdown no-owner Bob");
}

/// §10 #23 sub-case (c) — `pending_info_options` and
/// `pending_notify_options` are independent slots. Two concurrent
/// `.send()` futures on the same session, one INFO and one NOTIFY,
/// must both succeed without `SessionError::Conflict`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stash_slots_are_independent_across_methods() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR_STASH_INDEPENDENT;
    let mut call = establish_call(alice_port, bob_port).await;

    // Launch INFO and NOTIFY simultaneously. Different stash slots →
    // no Conflict, both must succeed.
    let info_fut = {
        let alice = call.alice.clone();
        let cid = call.call_id.clone();
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
        let alice = call.alice.clone();
        let cid = call.call_id.clone();
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
        match tokio::time::timeout(remaining, call.bob_events.next()).await {
            Err(_) | Ok(None) => break,
            Ok(Some(Event::SipTrace(trace))) => {
                if trace.direction != SipTraceDirection::Inbound {
                    continue;
                }
                if trace.start_line.starts_with("INFO") {
                    assert_verbatim_packet(&trace);
                    saw_info = wire_header_values(&trace.raw_message, SMOKE_HEADER_NAME)
                        == [SMOKE_INFO_VALUE];
                }
                if trace.start_line.starts_with("NOTIFY") {
                    assert_verbatim_packet(&trace);
                    saw_notify = wire_header_values(&trace.raw_message, SMOKE_HEADER_NAME)
                        == [SMOKE_NOTIFY_VALUE];
                }
            }
            Ok(Some(_)) => continue,
        }
    }

    assert!(
        saw_info,
        "INFO with its smoke value did not arrive on the wire"
    );
    assert!(
        saw_notify,
        "NOTIFY with its smoke value did not arrive on the wire"
    );

    call.teardown().await;
}

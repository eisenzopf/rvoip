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
use rvoip_sip::{HeaderName, SipTraceDirection};

mod support;

use support::{establish_call, wait_for_inbound_method};

const PAIR_STASH_REUSE: (u16, u16) = (16500, 16510);
const PAIR_STASH_INDEPENDENT: (u16, u16) = (16520, 16530);

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

/// §10 #23 sub-case (a) — after a successful `.send()`, the
/// `pending_info_options` slot is cleared. A subsequent INFO on the
/// same session that omits the trace header MUST NOT carry residue.
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

    // Second INFO: NO trace header. If the stash leaked, the second
    // INFO would carry the same X-Stash-Trace value.
    call.alice
        .info(&call.call_id, "application/dtmf-relay")
        .with_body("Signal=2\r\nDuration=160\r\n")
        .send()
        .await
        .expect("second info().send()");

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

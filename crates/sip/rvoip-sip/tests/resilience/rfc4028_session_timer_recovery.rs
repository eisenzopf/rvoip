//! RFC 4028 session-timer resilience stubs.

#[path = "support.rs"]
mod support;

use support::{document_stub, ResilienceLayer::RvoipSipApi, ResilienceStub};

#[test]
#[ignore = "stub: add retained-owner assertions to existing session timer success flow"]
fn refresher_update_success_keeps_dialog_alive_without_resource_growth() {
    document_stub(ResilienceStub {
        id: "RFC 4028 successful session refresh recovery",
        layer: RvoipSipApi,
        existing_coverage: "session_timer_integration.rs covers negotiated refresh behavior; state tables cover UPDATE refresh while Active.",
        target: "Run repeated UPDATE refreshes for several intervals and assert the call stays Active, lifecycle entries stay bounded, and post-BYE retained owners drain to zero.",
        next_hardening: "Extend rvoip-sip integration tests with shortened session intervals and perf diagnostic retention assertions.",
    });
}

#[test]
#[ignore = "stub: existing failure test should grow into retained-owner invariant"]
fn refresher_failure_emits_session_refresh_failed_and_releases_resources() {
    document_stub(ResilienceStub {
        id: "RFC 4028 §10 refresh failure recovery",
        layer: RvoipSipApi,
        existing_coverage: "session_timer_failure_integration.rs already asserts SessionRefreshFailed when UPDATE/re-INVITE refresh fails.",
        target: "After refresh failure, assert one terminal app event, Reason cause=408 on BYE when sent, zero active audio receivers, zero transactions, and zero retained rvoip-sip owners.",
        next_hardening: "Add perf diagnostic assertions to the existing multi-binary failure test or convert it to an in-process fixture with shortened transaction timers.",
    });
}

#[test]
#[ignore = "stub: extend existing 422 retry coverage with repeated min-se handling"]
fn interval_too_small_422_retries_once_and_bounds_state() {
    document_stub(ResilienceStub {
        id: "RFC 4028 §6 422 Session Interval Too Small recovery",
        layer: RvoipSipApi,
        existing_coverage: "session_422_retry.rs covers retry wiring for Min-SE negotiation.",
        target: "Force 422 with Min-SE, assert exactly one corrected retry, no duplicate sessions, and terminal cleanup if the retry also fails.",
        next_hardening: "Add raw UAS responses that can emit sequential 422/final outcomes and assert retry counters through rvoip-sip diagnostics.",
    });
}

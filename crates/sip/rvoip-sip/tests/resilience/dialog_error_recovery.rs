//! Dialog error and overload recovery stubs.

#[path = "support.rs"]
mod support;

use support::{
    document_stub,
    ResilienceLayer::{LowerLibraryHardening, MixedRvoipSipAndLower, RvoipSipApi},
    ResilienceStub,
};

#[test]
#[ignore = "stub: add 481 stale-dialog fixture"]
fn stale_dialog_481_terminates_or_reconciles_without_leaking() {
    document_stub(ResilienceStub {
        id: "RFC 3261 481 Call/Transaction Does Not Exist recovery",
        layer: RvoipSipApi,
        existing_coverage: "Dialog routing handles 481 at lower layers; rvoip-sip has generic final response and teardown cleanup coverage.",
        target: "Receive 481 for an in-dialog request and assert rvoip-sip reconciles the session to a terminal or stable state, emits one event, and drains all retained owners.",
        next_hardening: "Add a raw peer that returns 481 to BYE/re-INVITE/UPDATE and define the rvoip-sip public event contract for each method.",
    });
}

#[test]
#[ignore = "stub: needs timeout-to-408 API contract per request type"]
fn request_timeout_408_releases_resources_and_preserves_retry_metadata() {
    document_stub(ResilienceStub {
        id: "RFC 3261 408 Request Timeout recovery",
        layer: MixedRvoipSipAndLower,
        existing_coverage: "Session timer failure maps refresh failure to SessionRefreshFailed; transaction timeout behavior lives in sip-dialog.",
        target: "Force 408/transaction timeout for setup and mid-dialog requests, assert the correct rvoip-sip terminal or retry event and zero retained transactions after cleanup.",
        next_hardening: "Use lower-layer shortened timers plus rvoip-sip event/cleanup assertions; document which API methods retry versus terminate.",
    });
}

#[test]
#[ignore = "stub: add Retry-After preserving overload fixture"]
fn overload_503_retry_after_is_surfaced_and_backoff_is_respected() {
    document_stub(ResilienceStub {
        id: "RFC 3261 503 Service Unavailable / Retry-After recovery",
        layer: RvoipSipApi,
        existing_coverage: "session_event_handler preserves response metadata and server admission can reject with 503.",
        target: "Return 503 with Retry-After and assert rvoip-sip surfaces retry metadata, avoids immediate retry storms, and releases setup resources.",
        next_hardening: "Add raw UAS/proxy responses carrying Retry-After and assert public events plus retry/backoff counters.",
    });
}

#[test]
#[ignore = "stub: forked early-dialog routing belongs below rvoip-sip first"]
fn forked_dialog_late_final_response_routes_to_the_correct_session() {
    document_stub(ResilienceStub {
        id: "RFC 3261 forked early-dialog recovery",
        layer: LowerLibraryHardening,
        existing_coverage: "sip-dialog has response routing and early-dialog lookup code; rvoip-sip has redirect and normal call setup tests.",
        target: "Simulate multiple early dialogs, late 2xx/non-2xx responses, and CANCEL/BYE cleanup so rvoip-sip exposes one chosen session and drains abandoned forks.",
        next_hardening: "Harden and expose forked response routing diagnostics in sip-dialog before adding rvoip-sip API assertions.",
    });
}

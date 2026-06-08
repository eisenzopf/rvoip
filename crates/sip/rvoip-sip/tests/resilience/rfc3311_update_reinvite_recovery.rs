//! RFC 3311 UPDATE and RFC 3261 re-INVITE resilience stubs.

#[path = "support.rs"]
mod support;

use support::{
    document_stub,
    ResilienceLayer::{MixedRvoipSipAndLower, RvoipSipApi},
    ResilienceStub,
};

#[test]
#[ignore = "stub: extend existing glare test with resource-retention assertions"]
fn simultaneous_update_and_reinvite_glare_rolls_back_media_state() {
    document_stub(ResilienceStub {
        id: "RFC 3311 UPDATE / RFC 3261 §14 glare rollback",
        layer: MixedRvoipSipAndLower,
        existing_coverage: "glare_retry_integration.rs covers simultaneous hold re-INVITEs converging to OnHold; state tables include UPDATE/session-refresh handling.",
        target: "Race UPDATE against re-INVITE while media is active, assert 491/backoff behavior, media rollback until success, one final negotiated state, and zero retained owners after BYE.",
        next_hardening: "Use rvoip-sip for API/state/media assertions; add lower-layer hooks only where transaction ordering must be forced deterministically.",
    });
}

#[test]
#[ignore = "stub: needs explicit failed re-INVITE rollback fixture"]
fn failed_reinvite_preserves_previous_session_description() {
    document_stub(ResilienceStub {
        id: "RFC 3261 §14 failed re-INVITE rollback",
        layer: RvoipSipApi,
        existing_coverage: "state table and session_event_handler contain re-INVITE non-2xx rollback logic.",
        target: "Reject hold/resume re-INVITE with 4xx/5xx/timeout and assert rvoip-sip preserves previous media direction, SDP version, and call state.",
        next_hardening: "Add a raw UAS or two-coordinator fixture that rejects mid-dialog re-INVITEs and exposes final media/session state through rvoip-sip.",
    });
}

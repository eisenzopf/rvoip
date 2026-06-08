//! RFC 5626 outbound flow recovery stubs.

#[path = "support.rs"]
mod support;

use support::{
    document_stub,
    ResilienceLayer::{LowerLibraryHardening, MixedRvoipSipAndLower},
    ResilienceStub,
};

#[test]
#[ignore = "stub: requires deterministic outbound flow failure injection"]
fn outbound_flow_failure_refreshes_registration_without_storming() {
    document_stub(ResilienceStub {
        id: "RFC 5626 §4.4.1 outbound flow recovery",
        layer: MixedRvoipSipAndLower,
        existing_coverage: "session_event_handler tracks OutboundFlowFailed refresh throttling; no end-to-end flow-failure test exists in rvoip-sip.",
        target: "Inject repeated flow-failure events for the same AoR and assert rvoip-sip refreshes registration once per throttle window, surfaces stable events, and does not leak refresh tasks.",
        next_hardening: "Use lower-library flow failure events as the trigger; implement rvoip-sip assertions around registration refresh throttling and task cleanup.",
    });
}

#[test]
#[ignore = "stub: requires CRLF/STUN keepalive support decision below rvoip-sip"]
fn keepalive_failure_marks_flow_failed_and_preserves_active_dialogs_until_timeout() {
    document_stub(ResilienceStub {
        id: "RFC 5626 keepalive failure behavior",
        layer: LowerLibraryHardening,
        existing_coverage: "rvoip-sip handles flow-failure events but does not own keepalive packet generation/detection.",
        target: "Lose outbound keepalive responses, mark the flow failed, recover registration if possible, and ensure active dialogs either migrate or terminate with cleanup.",
        next_hardening: "Decide and implement keepalive detection in sip-transport/sip-dialog before rvoip-sip can assert high-level recovery behavior.",
    });
}

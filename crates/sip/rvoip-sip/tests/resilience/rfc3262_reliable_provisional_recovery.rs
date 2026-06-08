//! RFC 3262 reliable-provisional resilience stubs.

#[path = "support.rs"]
mod support;

use support::{document_stub, ResilienceLayer::RvoipSipApi, ResilienceStub};

#[test]
#[ignore = "stub: build on existing PRACK integration with retransmitted reliable 183"]
fn retransmitted_reliable_provisional_is_pracked_once_and_does_not_duplicate_events() {
    document_stub(ResilienceStub {
        id: "RFC 3262 reliable 1xx retransmission recovery",
        layer: RvoipSipApi,
        existing_coverage: "prack_integration.rs covers 420 policy mismatch and the positive reliable 183 -> PRACK -> 200 flow.",
        target: "Retransmit the same reliable 183 multiple times and assert rvoip-sip sends one PRACK transaction per RSeq, emits one progress/early-media event, and drains all state after call teardown.",
        next_hardening: "Extend the raw/proxy UAS test harness to replay reliable 183 responses with stable RSeq/CSeq and observe PRACK idempotency at rvoip-sip.",
    });
}

#[test]
#[ignore = "stub: requires reliable provisional timeout observability"]
fn missing_prack_times_out_and_releases_uas_resources() {
    document_stub(ResilienceStub {
        id: "RFC 3262 UAS PRACK timeout recovery",
        layer: RvoipSipApi,
        existing_coverage: "reliable_provisional_bridge.rs and prack_integration.rs cover the successful bridge path.",
        target: "Have a UAS send reliable 183, suppress the UAC PRACK, then assert the UAS terminates the early dialog and releases session/media/dialog resources.",
        next_hardening: "Expose or configure shortened reliable-provisional timers so the timeout path can run quickly and deterministically in rvoip-sip tests.",
    });
}

//! RFC 3261 transaction/timer resilience stubs.
//!
//! These are intentionally ignored placeholders. They document the target
//! scenarios and ownership before we add lower-layer fault injection hooks.

#[path = "support.rs"]
mod support;

use support::{document_stub, ResilienceLayer::LowerLibraryHardening, ResilienceStub};

#[test]
#[ignore = "stub: needs deterministic UDP loss/duplicate/reorder injection in sip-transport/sip-dialog"]
fn invite_transaction_survives_loss_duplicates_and_reordering() {
    document_stub(ResilienceStub {
        id: "RFC 3261 §17 INVITE transaction retransmission recovery",
        layer: LowerLibraryHardening,
        existing_coverage: "sip-dialog transaction unit tests cover timers and retransmit behavior; rvoip-sip has teardown invariants for normal BYE/CANCEL flows.",
        target: "Drop, duplicate, and reorder INVITE/1xx/2xx/ACK/BYE datagrams while asserting the rvoip-sip session reaches one terminal event and drains all retained owners.",
        next_hardening: "Add deterministic packet fault injection below rvoip-sip, then assert rvoip-sip only observes stable API events and zero post-drain retention.",
    });
}

#[test]
#[ignore = "stub: requires shortened transaction timers and controllable message loss"]
fn non_invite_transaction_retransmit_cache_does_not_leak() {
    document_stub(ResilienceStub {
        id: "RFC 3261 §17.2.2 non-INVITE retransmission recovery",
        layer: LowerLibraryHardening,
        existing_coverage: "sip-dialog has non-INVITE retransmit tests; rvoip-sip has OPTIONS timeout and generic response coverage.",
        target: "Exercise duplicate OPTIONS/INFO/UPDATE requests and final-response retransmits, then verify no transaction cache, runner, dialog adapter, or lifecycle entries remain after Timer J cleanup.",
        next_hardening: "Expose a test-only transport harness that can replay duplicate non-INVITE requests against rvoip-sip without relying on sleeps.",
    });
}

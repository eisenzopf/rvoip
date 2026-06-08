//! RFC 3581 rport/NAT path resilience stubs.

#[path = "support.rs"]
mod support;

use support::{document_stub, ResilienceLayer::LowerLibraryHardening, ResilienceStub};

#[test]
#[ignore = "stub: requires transport-level source-port rewrite fixture"]
fn rport_response_routing_survives_source_port_rewrite() {
    document_stub(ResilienceStub {
        id: "RFC 3581 rport response routing",
        layer: LowerLibraryHardening,
        existing_coverage: "sip-dialog transaction code stamps received/rport on server responses; rvoip-sip has no NAT/source-port rewrite fixture.",
        target: "Send requests from a rewritten source port with Via rport and assert responses route to the observed source while rvoip-sip sees one coherent session.",
        next_hardening: "Build a lower-level UDP proxy/NAT test harness or transport fixture, then bind rvoip-sip assertions to successful call setup and cleanup.",
    });
}

#[test]
#[ignore = "stub: requires NAT pinhole expiration simulation"]
fn nat_binding_refresh_or_failure_releases_dialog_cleanly() {
    document_stub(ResilienceStub {
        id: "RFC 3581 NAT binding resilience",
        layer: LowerLibraryHardening,
        existing_coverage: "No direct rvoip-sip NAT binding expiration test found.",
        target: "Expire a simulated NAT mapping mid-dialog and assert retry/timeout behavior produces one terminal event and no retained media/RTP/session resources.",
        next_hardening: "Add transport/proxy controls for source address changes and blackholing; rvoip-sip should only own terminal event and cleanup assertions.",
    });
}

//! RFC 3263 DNS/transport failover resilience stubs.

#[path = "support.rs"]
mod support;

use support::{document_stub, ResilienceLayer::LowerLibraryHardening, ResilienceStub};

#[test]
#[ignore = "stub: needs injectable DNS resolver and multi-target transport harness"]
fn srv_naptr_failover_retries_next_target_without_duplicate_call_events() {
    document_stub(ResilienceStub {
        id: "RFC 3263 SRV/NAPTR failover recovery",
        layer: LowerLibraryHardening,
        existing_coverage: "rvoip-sip has per-method/per-leg outbound proxy routing and perf_transport_recovery coverage, but no deterministic DNS SRV/NAPTR failover harness.",
        target: "Return multiple DNS targets, force the first target to timeout or refuse transport, and assert rvoip-sip reaches one successful call with one public call id and no leaked failed candidate state.",
        next_hardening: "Add injectable resolver/transport candidate controls in sip-transport or sip-dialog, then bind rvoip-sip assertions to API events and retention counters.",
    });
}

#[test]
#[ignore = "stub: requires transport failover observability below rvoip-sip"]
fn transport_switch_from_udp_to_tcp_preserves_dialog_identity() {
    document_stub(ResilienceStub {
        id: "RFC 3263 transport fallback recovery",
        layer: LowerLibraryHardening,
        existing_coverage: "Transport recovery perf tests exist, but not RFC 3263 candidate ordering and dialog identity checks.",
        target: "Force UDP candidate failure followed by TCP/TLS success and verify Route/Via/Contact handling does not create duplicate rvoip-sip sessions.",
        next_hardening: "Expose selected transport candidate and failure reason as test diagnostics from lower SIP libraries.",
    });
}

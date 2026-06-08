//! PBX/proxy interop recovery stubs.

#[path = "support.rs"]
mod support;

use support::{
    document_stub,
    ResilienceLayer::{ExternalInterop, MixedRvoipSipAndLower},
    ResilienceStub,
};

#[test]
#[ignore = "stub: extend local PBX matrix with restart/drop recovery scenarios"]
fn asterisk_and_freeswitch_restart_mid_call_recover_or_cleanup() {
    document_stub(ResilienceStub {
        id: "Asterisk/FreeSWITCH mid-call recovery interop",
        layer: ExternalInterop,
        existing_coverage: "beta_gate.sh can run local Asterisk and FreeSWITCH PBX matrices; existing checks focus on interoperability, not restart recovery.",
        target: "Bring each PBX up one at a time, establish calls, restart/drop the PBX mid-call, and assert rvoip-sip emits terminal/recovery events and drains media/session/dialog state.",
        next_hardening: "Add docker-controlled PBX restart/drop scenarios under the existing interop harness; fixes may land in rvoip-sip or lower libraries depending on observed failure.",
    });
}

#[test]
#[ignore = "stub: requires Kamailio/OpenSIPS docker harness decision"]
fn kamailio_opensips_proxy_failover_preserves_dialog_route_set_or_cleans_up() {
    document_stub(ResilienceStub {
        id: "Kamailio/OpenSIPS proxy failover recovery",
        layer: MixedRvoipSipAndLower,
        existing_coverage: "Current beta gate de-scopes Kamailio/OpenSIPS but has SIPp and PBX interop infrastructure.",
        target: "Route calls through a proxy pair, fail the active proxy, and assert route-set handling either recovers correctly or terminates with zero retained owners.",
        next_hardening: "Create dockerized proxy fixtures and inspect failures; route-set, Record-Route, DNS, and transport fixes likely belong in sip-dialog/sip-transport.",
    });
}

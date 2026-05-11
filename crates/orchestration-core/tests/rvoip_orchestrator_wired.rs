//! CARVE_PLAN step 9: prove the rvoip_core::Orchestrator is wired up
//! alongside the existing UnifiedCoordinator path. When orchestration-core's
//! builder is given a session config, build() should auto-construct a
//! SipAdapter, register it with a fresh rvoip_core::Orchestrator, and stash
//! that handle on the Orchestrator.

use rvoip_orchestration_core::Orchestrator;
use rvoip_sip::api::unified::Config as SessionConfig;

#[tokio::test]
async fn rvoip_orchestrator_handle_present_when_coordinator_provided() {
    // Use a local lab profile bound to a free port to avoid collisions when
    // the test runs concurrently with other crates' SIP integration tests.
    let session_config = SessionConfig::local("rvoip-step9-test", 0);

    let orchestrator = Orchestrator::builder()
        .with_session_config(session_config)
        .build()
        .await
        .expect("orchestrator builds");

    let rvoip_orch = orchestrator
        .rvoip_orchestrator()
        .expect("rvoip_orchestrator handle present when coordinator was provided");

    // SipAdapter was registered for Transport::Sip — fetching it should succeed.
    let adapter = rvoip_orch
        .adapter(rvoip_core::Transport::Sip)
        .expect("SipAdapter registered for Sip transport");
    assert_eq!(adapter.transport(), rvoip_core::Transport::Sip);
    assert_eq!(adapter.kind(), rvoip_core::AdapterKind::Interop);

    // The cross-transport event bus is alive — subscribers can attach.
    let _events = rvoip_orch.subscribe_events();
}

#[tokio::test]
async fn rvoip_orchestrator_handle_absent_when_no_coordinator() {
    let orchestrator = Orchestrator::builder()
        .build()
        .await
        .expect("orchestrator builds without coordinator");
    assert!(orchestrator.rvoip_orchestrator().is_none());
    assert!(orchestrator.coordinator().is_none());
}

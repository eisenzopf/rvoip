//! P6 — multi-adapter dispatch + tenant quota + capacity scheduler.

use rvoip_core::config::{Config, TenantQuotas};
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::error::RvoipError;
use rvoip_core::events::Event;
use rvoip_core::ids::TenantId;
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::session::SessionMedium;
use std::collections::HashMap;
use std::time::Duration;

#[tokio::test]
async fn tenant_quota_rejects_exceeding_start_session() {
    let orch = Orchestrator::new(Config::default());
    let tenant = TenantId::new();
    orch.set_tenant_quotas(
        tenant.clone(),
        TenantQuotas {
            max_concurrent_sessions: Some(1),
            ..Default::default()
        },
    )
    .expect("set_tenant_quotas");

    let cid = orch
        .open_conversation(
            tenant.clone(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .unwrap();
    // First session goes through; we have to join to make it Active.
    let sid = orch
        .start_session(cid.clone(), SessionMedium::Voice, vec![])
        .await
        .unwrap();
    orch.join_session(
        sid,
        rvoip_core::ids::ParticipantId::new(),
        rvoip_core::participant::ParticipantKind::Human,
        rvoip_core::participant::ParticipantRole::Customer,
    )
    .await
    .unwrap();

    // Second start_session must hit the quota.
    match orch.start_session(cid, SessionMedium::Voice, vec![]).await {
        Err(RvoipError::AdmissionRejected(_)) => {}
        other => panic!("expected AdmissionRejected, got {other:?}"),
    }
}

#[tokio::test]
async fn capacity_scheduler_emits_capacity_report() {
    let mut cfg = Config::default();
    cfg.capacity_report_interval = Some(Duration::from_millis(50));
    let orch = Orchestrator::new(cfg);
    let mut events = orch.subscribe_events();
    orch.spawn_capacity_scheduler();

    let ev = tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            if let Ok(Event::CapacityReport { .. }) = events.recv().await {
                return Ok::<(), ()>(());
            }
        }
    })
    .await;
    assert!(ev.is_ok(), "CapacityReport should fire within 500ms");
}

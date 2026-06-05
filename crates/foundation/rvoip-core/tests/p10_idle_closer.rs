//! P10 — `spawn_idle_closer` drives Ephemeral Conversations to
//! `Closed` after their `idle_close_secs` elapses with no Active
//! Sessions.

use rvoip_core::config::Config;
use rvoip_core::conversation::{ConversationPolicy, ConversationState};
use rvoip_core::events::Event;
use rvoip_core::ids::TenantId;
use rvoip_core::orchestrator::Orchestrator;
use std::collections::HashMap;
use std::time::Duration;

#[tokio::test]
async fn idle_closer_force_closes_ephemeral_after_ttl() {
    let orch = Orchestrator::new(Config::default());
    // 1-second idle TTL so the test runs in well under 10s.
    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::Ephemeral { idle_close_secs: 1 },
            HashMap::new(),
        )
        .await
        .unwrap();

    let mut events = orch.subscribe_events();
    // Drive the closer ticking every 200ms — short enough that the
    // close lands within 2s of the TTL expiring.
    orch.spawn_idle_closer(Duration::from_millis(200));

    // Wait up to 4s for ConversationClosed.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut saw_close = false;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(300), events.recv()).await {
            Ok(Ok(Event::ConversationClosed {
                conversation_id, ..
            })) if conversation_id == cid => {
                saw_close = true;
                break;
            }
            _ => continue,
        }
    }
    assert!(
        saw_close,
        "idle_closer must close an Ephemeral conv past TTL"
    );

    let conv = orch.conversation(&cid).expect("present");
    assert_eq!(conv.read().unwrap().state, ConversationState::Closed);
}

#[tokio::test]
async fn idle_closer_leaves_persistent_conversations_alone() {
    let orch = Orchestrator::new(Config::default());
    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::Persistent,
            HashMap::new(),
        )
        .await
        .unwrap();
    orch.spawn_idle_closer(Duration::from_millis(50));

    // Wait well past any reasonable TTL — should still be Open.
    tokio::time::sleep(Duration::from_millis(800)).await;
    let conv = orch.conversation(&cid).expect("present");
    assert_eq!(conv.read().unwrap().state, ConversationState::Open);
}

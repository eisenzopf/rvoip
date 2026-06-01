//! P1 acceptance — Conversation lifecycle through the Orchestrator.
//!
//! Exercises `open_conversation` / `close_conversation` with the
//! state-machine and event-emission contracts the GAP_PLAN promises:
//! - Closing an already-Closed Conversation is idempotent.
//! - Close rejects with `InvalidState` when active Sessions exist and
//!   `force=false`; succeeds with `force=true` (ending those Sessions
//!   along the way).
//! - `ConversationOpened` and `ConversationClosed` fire on the
//!   broadcast bus with the expected `conversation_id`.

use rvoip_core::adapter::EndReason;
use rvoip_core::config::Config;
use rvoip_core::conversation::{ConversationPolicy, ConversationState};
use rvoip_core::error::RvoipError;
use rvoip_core::events::Event;
use rvoip_core::ids::TenantId;
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::session::SessionMedium;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::broadcast::Receiver;

async fn next_event(rx: &mut Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("event channel timed out")
        .expect("event channel closed")
}

#[tokio::test]
async fn open_and_close_emits_opened_and_closed() {
    let orch = Orchestrator::new(Config::default());
    let mut events = orch.subscribe_events();

    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .expect("open");

    match next_event(&mut events).await {
        Event::ConversationOpened { conversation_id, .. } => {
            assert_eq!(conversation_id, cid);
        }
        other => panic!("expected ConversationOpened, got {other:?}"),
    }

    let conv = orch.conversation(&cid).expect("conversation present");
    assert_eq!(
        conv.read().unwrap().state,
        ConversationState::Open,
        "newly-opened Conversation should be Open"
    );

    orch.close_conversation(cid.clone(), false)
        .await
        .expect("close");

    match next_event(&mut events).await {
        Event::ConversationClosed { conversation_id, .. } => {
            assert_eq!(conversation_id, cid);
        }
        other => panic!("expected ConversationClosed, got {other:?}"),
    }

    let conv = orch.conversation(&cid).expect("conversation still tracked");
    let c = conv.read().unwrap();
    assert_eq!(c.state, ConversationState::Closed);
    assert!(c.closed_at.is_some(), "closed_at populated");
}

#[tokio::test]
async fn close_is_idempotent_on_already_closed_conversation() {
    let orch = Orchestrator::new(Config::default());
    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .expect("open");
    orch.close_conversation(cid.clone(), false)
        .await
        .expect("first close");
    // Second close — no error, no panic.
    orch.close_conversation(cid, false)
        .await
        .expect("second close is a no-op");
}

#[tokio::test]
async fn close_rejects_when_active_sessions_exist_without_force() {
    let orch = Orchestrator::new(Config::default());
    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .expect("open");
    let _sid = orch
        .start_session(cid.clone(), SessionMedium::Voice, vec![])
        .await
        .expect("start_session");

    let err = orch
        .close_conversation(cid, false)
        .await
        .expect_err("close without force must fail");
    match err {
        RvoipError::InvalidState(_) => {}
        other => panic!("expected InvalidState, got {other:?}"),
    }
}

#[tokio::test]
async fn close_with_force_ends_active_sessions_then_closes() {
    let orch = Orchestrator::new(Config::default());
    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .expect("open");
    let sid = orch
        .start_session(cid.clone(), SessionMedium::Voice, vec![])
        .await
        .expect("start_session");

    // Drain the open/start events so we can assert the close path
    // cleanly.
    let mut events = orch.subscribe_events();

    orch.close_conversation(cid.clone(), true)
        .await
        .expect("force-close");

    // Order: SessionEnded (from the in-flight end_session) then
    // ConversationClosed.
    let mut saw_session_ended = false;
    let mut saw_conversation_closed = false;
    for _ in 0..2 {
        match next_event(&mut events).await {
            Event::SessionEnded { session_id, .. } => {
                assert_eq!(session_id, sid);
                saw_session_ended = true;
            }
            Event::ConversationClosed { conversation_id, .. } => {
                assert_eq!(conversation_id, cid);
                saw_conversation_closed = true;
            }
            other => panic!("unexpected event during force-close: {other:?}"),
        }
    }
    assert!(saw_session_ended && saw_conversation_closed);
}

#[tokio::test]
async fn close_unknown_conversation_returns_not_found() {
    let orch = Orchestrator::new(Config::default());
    // ConversationId::new() is a fresh UUID-based id — not registered.
    let fake = rvoip_core::ids::ConversationId::new();
    match orch.close_conversation(fake, false).await {
        Err(RvoipError::ConversationNotFound(_)) => {}
        other => panic!("expected ConversationNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn open_conversation_seeds_last_activity_at_to_opened_at() {
    let orch = Orchestrator::new(Config::default());
    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .expect("open");
    let conv = orch.conversation(&cid).expect("present");
    let c = conv.read().unwrap();
    assert_eq!(
        c.opened_at, c.last_activity_at,
        "last_activity_at should equal opened_at at open"
    );
}

#[tokio::test]
async fn start_session_bumps_last_activity_at() {
    let orch = Orchestrator::new(Config::default());
    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .expect("open");
    let opened_activity = orch
        .conversation(&cid)
        .unwrap()
        .read()
        .unwrap()
        .last_activity_at;

    // chrono::Utc::now() has microsecond resolution; sleep > 1µs to
    // guarantee a strictly greater timestamp.
    tokio::time::sleep(Duration::from_millis(2)).await;

    let _sid = orch
        .start_session(cid.clone(), SessionMedium::Voice, vec![])
        .await
        .expect("start_session");

    let new_activity = orch
        .conversation(&cid)
        .unwrap()
        .read()
        .unwrap()
        .last_activity_at;
    assert!(
        new_activity > opened_activity,
        "start_session must bump last_activity_at: opened={opened_activity:?} new={new_activity:?}"
    );
}

#[tokio::test]
async fn end_session_normal_after_force_close_does_not_panic() {
    // Edge case from the force-close path: end_session ran during
    // close, then a stale caller calls end_session again.
    let orch = Orchestrator::new(Config::default());
    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .expect("open");
    let sid = orch
        .start_session(cid.clone(), SessionMedium::Voice, vec![])
        .await
        .expect("start_session");
    orch.close_conversation(cid, true).await.expect("force close");
    // Now end_session on the already-ended session should be a no-op.
    orch.end_session(sid, EndReason::Normal)
        .await
        .expect("idempotent end_session");
}

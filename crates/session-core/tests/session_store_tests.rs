//! SessionStore CRUD and Index Tests
//!
//! Tests the DashMap-based SessionStore: create, get, update, remove,
//! index lookups, and concurrent access.

use rvoip_session_core::internals::{
    SessionId, Role,
};
use rvoip_session_core::session_store::SessionStore;
use rvoip_session_core::state_table::types::{DialogId, MediaSessionId};
use rvoip_session_core::types::CallState;

// ── Basic CRUD ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_create_and_get_session() {
    let store = SessionStore::new();
    let id = SessionId::new();

    let created = store.create_session(id.clone(), Role::UAC, false).await;
    assert!(created.is_ok());

    let fetched = store.get_session(&id).await;
    assert!(fetched.is_ok());
    let s = fetched.unwrap();
    assert_eq!(s.session_id, id);
    assert!(matches!(s.call_state, CallState::Idle));
}

#[tokio::test]
async fn test_create_duplicate_fails() {
    let store = SessionStore::new();
    let id = SessionId::new();

    assert!(store.create_session(id.clone(), Role::UAC, false).await.is_ok());
    assert!(store.create_session(id.clone(), Role::UAC, false).await.is_err());
}

#[tokio::test]
async fn test_get_nonexistent_returns_error() {
    let store = SessionStore::new();
    let id = SessionId::new();
    assert!(store.get_session(&id).await.is_err());
}

#[tokio::test]
async fn test_create_with_history() {
    let store = SessionStore::new();
    let id = SessionId::new();
    let created = store.create_session(id.clone(), Role::UAC, true).await.unwrap();
    assert!(created.history.is_some());
}

// ── Update ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_update_session_state() {
    let store = SessionStore::new();
    let id = SessionId::new();
    let mut s = store.create_session(id.clone(), Role::UAC, false).await.unwrap();

    s.call_state = CallState::Active;
    s.dialog_established = true;
    store.update_session(s).await.unwrap();

    let fetched = store.get_session(&id).await.unwrap();
    assert!(matches!(fetched.call_state, CallState::Active));
    assert!(fetched.dialog_established);
}

// ── Remove ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_remove_session() {
    let store = SessionStore::new();
    let id = SessionId::new();
    store.create_session(id.clone(), Role::UAC, false).await.unwrap();

    assert!(store.remove_session(&id).await.is_ok());
    assert!(store.get_session(&id).await.is_err());
}

#[tokio::test]
async fn test_remove_nonexistent_fails() {
    let store = SessionStore::new();
    let id = SessionId::new();
    assert!(store.remove_session(&id).await.is_err());
}

// ── List / count ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_get_all_sessions() {
    let store = SessionStore::new();
    let id1 = SessionId::new();
    let id2 = SessionId::new();
    store.create_session(id1, Role::UAC, false).await.unwrap();
    store.create_session(id2, Role::UAS, false).await.unwrap();

    let all = store.get_all_sessions().await;
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn test_has_session() {
    let store = SessionStore::new();
    assert!(!store.has_session().await);

    let id = SessionId::new();
    store.create_session(id, Role::UAC, false).await.unwrap();
    assert!(store.has_session().await);
}

#[tokio::test]
async fn test_clear() {
    let store = SessionStore::new();
    store.create_session(SessionId::new(), Role::UAC, false).await.unwrap();
    store.create_session(SessionId::new(), Role::UAS, false).await.unwrap();

    store.clear().await;
    assert!(!store.has_session().await);
    assert_eq!(store.get_all_sessions().await.len(), 0);
}

// ── Index lookups ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_dialog_id_index() {
    let store = SessionStore::new();
    let sid = SessionId::new();
    let did = DialogId(uuid::Uuid::new_v4());

    let mut s = store.create_session(sid.clone(), Role::UAC, false).await.unwrap();
    s.dialog_id = Some(did.clone());
    store.update_session(s).await.unwrap();

    let found = store.find_by_dialog(&did).await;
    assert!(found.is_some());
    assert_eq!(found.unwrap().session_id, sid);
}

#[tokio::test]
async fn test_dialog_id_index_not_found() {
    let store = SessionStore::new();
    let did = DialogId(uuid::Uuid::new_v4());
    assert!(store.find_by_dialog(&did).await.is_none());
}

#[tokio::test]
async fn test_call_id_index() {
    let store = SessionStore::new();
    let sid = SessionId::new();
    let cid: String = "call-abc-123".into();

    let mut s = store.create_session(sid.clone(), Role::UAC, false).await.unwrap();
    s.call_id = Some(cid.clone());
    store.update_session(s).await.unwrap();

    let found = store.find_by_call_id(&cid).await;
    assert!(found.is_some());
    assert_eq!(found.unwrap().session_id, sid);
}

#[tokio::test]
async fn test_media_id_index() {
    let store = SessionStore::new();
    let sid = SessionId::new();
    let mid = MediaSessionId("media-xyz".into());

    let mut s = store.create_session(sid.clone(), Role::UAC, false).await.unwrap();
    s.media_session_id = Some(mid.clone());
    store.update_session(s).await.unwrap();

    let found = store.find_by_media_id(&mid).await;
    assert!(found.is_some());
    assert_eq!(found.unwrap().session_id, sid);
}

#[tokio::test]
async fn test_index_updated_on_id_change() {
    let store = SessionStore::new();
    let sid = SessionId::new();
    let did1 = DialogId(uuid::Uuid::new_v4());
    let did2 = DialogId(uuid::Uuid::new_v4());

    let mut s = store.create_session(sid.clone(), Role::UAC, false).await.unwrap();
    s.dialog_id = Some(did1.clone());
    store.update_session(s.clone()).await.unwrap();

    // Now change the dialog ID
    s.dialog_id = Some(did2.clone());
    // We need to re-fetch because update_session compares old vs new
    let mut s2 = store.get_session(&sid).await.unwrap();
    s2.dialog_id = Some(did2.clone());
    store.update_session(s2).await.unwrap();

    // Old dialog ID should no longer resolve
    assert!(store.find_by_dialog(&did1).await.is_none());
    // New dialog ID should work
    assert!(store.find_by_dialog(&did2).await.is_some());
}

#[tokio::test]
async fn test_remove_cleans_indexes() {
    let store = SessionStore::new();
    let sid = SessionId::new();
    let did = DialogId(uuid::Uuid::new_v4());
    let mid = MediaSessionId("m1".into());
    let cid: String = "c1".into();

    let mut s = store.create_session(sid.clone(), Role::UAC, false).await.unwrap();
    s.dialog_id = Some(did.clone());
    s.media_session_id = Some(mid.clone());
    s.call_id = Some(cid.clone());
    store.update_session(s).await.unwrap();

    store.remove_session(&sid).await.unwrap();

    assert!(store.find_by_dialog(&did).await.is_none());
    assert!(store.find_by_media_id(&mid).await.is_none());
    assert!(store.find_by_call_id(&cid).await.is_none());
}

// ── Stats ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_get_stats_empty() {
    let store = SessionStore::new();
    let stats = store.get_stats().await;
    assert_eq!(stats.total, 0);
}

#[tokio::test]
async fn test_get_stats_counts_states() {
    let store = SessionStore::new();

    // Create an idle session
    store.create_session(SessionId::new(), Role::UAC, false).await.unwrap();

    // Create an active session
    let id2 = SessionId::new();
    let mut s2 = store.create_session(id2.clone(), Role::UAC, false).await.unwrap();
    s2.call_state = CallState::Active;
    store.update_session(s2).await.unwrap();

    // Create a terminated session
    let id3 = SessionId::new();
    let mut s3 = store.create_session(id3.clone(), Role::UAS, false).await.unwrap();
    s3.call_state = CallState::Terminated;
    store.update_session(s3).await.unwrap();

    let stats = store.get_stats().await;
    assert_eq!(stats.total, 3);
    assert_eq!(stats.idle, 1);
    assert_eq!(stats.active, 1);
    assert_eq!(stats.terminated, 1);
}

// ── Concurrent access ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_concurrent_create_and_read() {
    use std::sync::Arc;

    let store = Arc::new(SessionStore::new());
    let mut handles = Vec::new();

    // Spawn 10 tasks that each create a session
    for _ in 0..10 {
        let store = Arc::clone(&store);
        handles.push(tokio::spawn(async move {
            let id = SessionId::new();
            store.create_session(id.clone(), Role::UAC, false).await.unwrap();
            id
        }));
    }

    let ids: Vec<SessionId> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // All 10 should be retrievable
    for id in &ids {
        assert!(store.get_session(id).await.is_ok());
    }
    assert_eq!(store.get_all_sessions().await.len(), 10);
}

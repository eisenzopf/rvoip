//! SessionStore CRUD and Index Tests
//!
//! Tests the DashMap-based SessionStore: create, get, update, remove,
//! index lookups, and concurrent access.

use rvoip_sip::internals::{Role, SessionId};
use rvoip_sip::session_store::SessionStore;
use rvoip_sip::state_table::types::{DialogId, MediaSessionId};
use rvoip_sip::types::CallState;

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

    assert!(store
        .create_session(id.clone(), Role::UAC, false)
        .await
        .is_ok());
    assert!(store
        .create_session(id.clone(), Role::UAC, false)
        .await
        .is_err());
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
    let created = store
        .create_session(id.clone(), Role::UAC, true)
        .await
        .unwrap();
    assert!(created.history.is_some());
}

// ── Update ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_update_session_state() {
    let store = SessionStore::new();
    let id = SessionId::new();
    let mut s = store
        .create_session(id.clone(), Role::UAC, false)
        .await
        .unwrap();

    s.call_state = CallState::Active;
    s.dialog_established = true;
    store.update_session(s).await.unwrap();

    let fetched = store.get_session(&id).await.unwrap();
    assert!(matches!(fetched.call_state, CallState::Active));
    assert!(fetched.dialog_established);
}

#[tokio::test]
async fn snapshot_revisions_are_immutable_and_support_optimistic_updates() {
    let store = SessionStore::new();
    let id = SessionId::new();
    store
        .create_session(id.clone(), Role::UAC, false)
        .await
        .unwrap();

    let first = store.get_session_snapshot(&id).await.unwrap();
    store
        .update_session_snapshot_with(&first, |session| {
            session.call_state = CallState::Active;
        })
        .await
        .unwrap();
    let second = store.get_session_snapshot(&id).await.unwrap();

    assert!(matches!(first.call_state, CallState::Idle));
    assert!(matches!(second.call_state, CallState::Active));
    assert!(second.revision() > first.revision());
    assert!(store
        .update_session_snapshot_with(&first, |session| {
            session.dialog_established = true;
        })
        .await
        .unwrap_err()
        .to_string()
        .contains("revision is stale"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_updates_on_different_sessions_do_not_share_a_global_lock() {
    use std::sync::Arc;
    use std::time::Duration;

    let store = Arc::new(SessionStore::new());
    let first_id = SessionId::new();
    let second_id = SessionId::new();
    store
        .create_session(first_id.clone(), Role::UAC, false)
        .await
        .unwrap();
    store
        .create_session(second_id.clone(), Role::UAC, false)
        .await
        .unwrap();

    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let blocked_store = Arc::clone(&store);
    let blocked_id = first_id.clone();
    let blocked = tokio::task::spawn_blocking(move || {
        futures::executor::block_on(blocked_store.update_session_with(&blocked_id, |session| {
            let _ = entered_tx.send(());
            release_rx.recv().expect("release first session update");
            session.dialog_established = true;
        }))
    });
    entered_rx.await.expect("first session update entered");

    let independent_update = tokio::time::timeout(
        Duration::from_secs(1),
        store.update_session_with(&second_id, |session| {
            session.media_session_ready = true;
        }),
    )
    .await;

    release_tx.send(()).unwrap();
    blocked.await.unwrap().unwrap();
    independent_update
        .expect("independent session update must not wait for the first cell")
        .unwrap();
    assert!(
        store
            .get_session(&first_id)
            .await
            .unwrap()
            .dialog_established
    );
    assert!(
        store
            .get_session(&second_id)
            .await
            .unwrap()
            .media_session_ready
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn closure_updates_serialize_per_session_without_lost_revisions() {
    use std::sync::Arc;

    let store = Arc::new(SessionStore::new());
    let id = SessionId::new();
    store
        .create_session(id.clone(), Role::UAC, false)
        .await
        .unwrap();

    let mut tasks = Vec::new();
    for _ in 0..64 {
        let store = Arc::clone(&store);
        let id = id.clone();
        tasks.push(tokio::spawn(async move {
            store
                .update_session_with(&id, |session| {
                    session.registration_cseq += 1;
                })
                .await
                .unwrap();
        }));
    }
    futures::future::join_all(tasks)
        .await
        .into_iter()
        .for_each(|result| result.unwrap());

    let snapshot = store.get_session_snapshot(&id).await.unwrap();
    assert_eq!(snapshot.registration_cseq, 64);
    assert_eq!(snapshot.revision(), 65);
}

// ── Remove ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_remove_session() {
    let store = SessionStore::new();
    let id = SessionId::new();
    store
        .create_session(id.clone(), Role::UAC, false)
        .await
        .unwrap();

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
    store
        .create_session(SessionId::new(), Role::UAC, false)
        .await
        .unwrap();
    store
        .create_session(SessionId::new(), Role::UAS, false)
        .await
        .unwrap();

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

    let mut s = store
        .create_session(sid.clone(), Role::UAC, false)
        .await
        .unwrap();
    s.dialog_id = Some(did);
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

    let mut s = store
        .create_session(sid.clone(), Role::UAC, false)
        .await
        .unwrap();
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
    let mid = MediaSessionId::new("media-xyz");

    let mut s = store
        .create_session(sid.clone(), Role::UAC, false)
        .await
        .unwrap();
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

    let mut s = store
        .create_session(sid.clone(), Role::UAC, false)
        .await
        .unwrap();
    s.dialog_id = Some(did1);
    store.update_session(s.clone()).await.unwrap();

    // Now change the dialog ID
    s.dialog_id = Some(did2);
    // We need to re-fetch because update_session compares old vs new
    let mut s2 = store.get_session(&sid).await.unwrap();
    s2.dialog_id = Some(did2);
    store.update_session(s2).await.unwrap();

    // Old dialog ID should no longer resolve
    assert!(store.find_by_dialog(&did1).await.is_none());
    // New dialog ID should work
    assert!(store.find_by_dialog(&did2).await.is_some());
}

#[tokio::test]
async fn index_collision_rejects_without_mutating_either_session() {
    let store = SessionStore::new();
    let first_id = SessionId::new();
    let second_id = SessionId::new();
    let dialog_id = DialogId(uuid::Uuid::new_v4());

    let mut first = store
        .create_session(first_id.clone(), Role::UAC, false)
        .await
        .unwrap();
    first.dialog_id = Some(dialog_id);
    store.update_session(first).await.unwrap();

    let mut second = store
        .create_session(second_id.clone(), Role::UAS, false)
        .await
        .unwrap();
    second.dialog_id = Some(dialog_id);
    assert!(store.update_session(second).await.is_err());

    assert_eq!(
        store.find_by_dialog(&dialog_id).await.unwrap().session_id,
        first_id
    );
    assert!(store
        .get_session(&second_id)
        .await
        .unwrap()
        .dialog_id
        .is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_same_derived_index_claims_have_exactly_one_owner() {
    use std::sync::Arc;

    let store = Arc::new(SessionStore::new());
    let first_id = SessionId::new();
    let second_id = SessionId::new();
    let dialog_id = DialogId(uuid::Uuid::new_v4());
    store
        .create_session(first_id.clone(), Role::UAC, false)
        .await
        .unwrap();
    store
        .create_session(second_id.clone(), Role::UAS, false)
        .await
        .unwrap();

    let start = Arc::new(tokio::sync::Barrier::new(3));
    let mut claims = Vec::new();
    for session_id in [first_id.clone(), second_id.clone()] {
        let store = Arc::clone(&store);
        let start = Arc::clone(&start);
        claims.push(tokio::spawn(async move {
            start.wait().await;
            let result = store
                .update_session_with(&session_id, |session| {
                    session.dialog_id = Some(dialog_id);
                })
                .await;
            (session_id, result)
        }));
    }

    start.wait().await;
    let outcomes = futures::future::join_all(claims)
        .await
        .into_iter()
        .map(|outcome| outcome.unwrap())
        .collect::<Vec<_>>();
    let winners = outcomes
        .iter()
        .filter_map(|(session_id, result)| result.is_ok().then_some(session_id))
        .collect::<Vec<_>>();
    assert_eq!(winners.len(), 1, "exactly one concurrent claim must win");
    assert_eq!(
        outcomes
            .iter()
            .filter(|(_, result)| result.is_err())
            .count(),
        1,
        "the competing claim must fail"
    );

    let winner = (*winners[0]).clone();
    assert_eq!(
        store.find_by_dialog(&dialog_id).await.unwrap().session_id,
        winner,
        "the derived index must resolve to the successful claimant"
    );
    for session_id in [&first_id, &second_id] {
        let session = store.get_session(session_id).await.unwrap();
        assert_eq!(
            session.dialog_id,
            (session_id == &winner).then_some(dialog_id),
            "only the winning session state may contain the claimed identifier"
        );
    }
}

#[tokio::test]
async fn later_index_collision_rolls_back_every_index_and_session_field() {
    let store = SessionStore::new();
    let changing_id = SessionId::new();
    let collision_owner_id = SessionId::new();

    let old_dialog = DialogId(uuid::Uuid::new_v4());
    let old_media = MediaSessionId::new("rollback-old-media");
    let old_call: String = "rollback-old-call".into();
    let new_dialog = DialogId(uuid::Uuid::new_v4());
    let new_media = MediaSessionId::new("rollback-new-media");
    let colliding_call: String = "rollback-colliding-call".into();

    let mut changing = store
        .create_session(changing_id.clone(), Role::UAC, false)
        .await
        .unwrap();
    changing.dialog_id = Some(old_dialog);
    changing.media_session_id = Some(old_media.clone());
    changing.call_id = Some(old_call.clone());
    store.update_session(changing).await.unwrap();

    let mut collision_owner = store
        .create_session(collision_owner_id.clone(), Role::UAS, false)
        .await
        .unwrap();
    collision_owner.call_id = Some(colliding_call.clone());
    store.update_session(collision_owner).await.unwrap();

    let before = store.get_session_snapshot(&changing_id).await.unwrap();
    let error = store
        .update_session_with(&changing_id, |session| {
            session.dialog_id = Some(new_dialog);
            session.media_session_id = Some(new_media.clone());
            // Call-ID ownership is validated after dialog and media ownership.
            session.call_id = Some(colliding_call.clone());
        })
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("call identifier is already owned"),
        "the deliberately later Call-ID collision must reject the update: {error}"
    );

    let after = store.get_session_snapshot(&changing_id).await.unwrap();
    assert_eq!(after.revision(), before.revision());
    assert_eq!(after.dialog_id, Some(old_dialog));
    assert_eq!(after.media_session_id.as_ref(), Some(&old_media));
    assert_eq!(after.call_id.as_ref(), Some(&old_call));

    assert_eq!(
        store.find_by_dialog(&old_dialog).await.unwrap().session_id,
        changing_id
    );
    assert_eq!(
        store.find_by_media_id(&old_media).await.unwrap().session_id,
        changing_id
    );
    assert_eq!(
        store.find_by_call_id(&old_call).await.unwrap().session_id,
        changing_id
    );
    assert!(store.find_by_dialog(&new_dialog).await.is_none());
    assert!(store.find_by_media_id(&new_media).await.is_none());
    assert_eq!(
        store
            .find_by_call_id(&colliding_call)
            .await
            .unwrap()
            .session_id,
        collision_owner_id
    );
}

#[tokio::test]
async fn test_remove_cleans_indexes() {
    let store = SessionStore::new();
    let sid = SessionId::new();
    let did = DialogId(uuid::Uuid::new_v4());
    let mid = MediaSessionId::new("m1");
    let cid: String = "c1".into();

    let mut s = store
        .create_session(sid.clone(), Role::UAC, false)
        .await
        .unwrap();
    s.dialog_id = Some(did);
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
    store
        .create_session(SessionId::new(), Role::UAC, false)
        .await
        .unwrap();

    // Create an active session
    let id2 = SessionId::new();
    let mut s2 = store
        .create_session(id2.clone(), Role::UAC, false)
        .await
        .unwrap();
    s2.call_state = CallState::Active;
    store.update_session(s2).await.unwrap();

    // Create a terminated session
    let id3 = SessionId::new();
    let mut s3 = store
        .create_session(id3.clone(), Role::UAS, false)
        .await
        .unwrap();
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
            store
                .create_session(id.clone(), Role::UAC, false)
                .await
                .unwrap();
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

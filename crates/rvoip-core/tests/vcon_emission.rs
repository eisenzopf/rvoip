//! P3 acceptance — `VconReady` fires on `SessionEnded` and the
//! resulting handle resolves to bytes whose sha256 matches.

use rvoip_core::adapter::EndReason;
use rvoip_core::config::Config;
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::events::Event;
use rvoip_core::ids::{ParticipantId, TenantId};
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::participant::{ParticipantKind, ParticipantRole};
use rvoip_core::session::SessionMedium;
use std::collections::HashMap;
use std::time::Duration;

async fn next_matching<F>(rx: &mut tokio::sync::broadcast::Receiver<Event>, mut f: F) -> Event
where
    F: FnMut(&Event) -> bool,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            panic!("timed out waiting for matching event");
        }
        let remaining = deadline - now;
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(ev)) if f(&ev) => return ev,
            Ok(Ok(_)) => continue,
            Ok(Err(_)) => panic!("channel closed"),
            Err(_) => panic!("timed out"),
        }
    }
}

#[tokio::test]
async fn ending_a_session_emits_vcon_ready_with_resolvable_handle() {
    let orch = Orchestrator::new(Config::default());
    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .unwrap();
    let sid = orch
        .start_session(cid, SessionMedium::Voice, vec![])
        .await
        .unwrap();
    // Two participants → vCon should snapshot both as parties.
    orch.join_session(
        sid.clone(),
        ParticipantId::new(),
        ParticipantKind::Human,
        ParticipantRole::Customer,
    )
    .await
    .unwrap();
    orch.join_session(
        sid.clone(),
        ParticipantId::new(),
        ParticipantKind::Ai,
        ParticipantRole::Agent,
    )
    .await
    .unwrap();

    let mut events = orch.subscribe_events();

    orch.end_session(sid.clone(), EndReason::Normal)
        .await
        .unwrap();

    // VconReady arrives via a tokio::spawn so it may come after
    // SessionEnded on the broadcast bus.
    let ev = next_matching(&mut events, |e| matches!(e, Event::VconReady { .. })).await;
    let (sid_back, handle) = match ev {
        Event::VconReady {
            session_id, handle, ..
        } => (session_id, handle),
        _ => unreachable!(),
    };
    assert_eq!(sid_back, sid);
    assert!(handle.url.starts_with("memory:vcon/"));
    assert!(handle.content_hash.starts_with("sha256:"));

    // Verify the bytes resolve and hash-match.
    let store = orch.config.vcon_store.clone();
    let bytes = store.get(&handle).await.unwrap().expect("bytes resolve");
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(&bytes);
    let digest = h.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest.iter() {
        hex.push_str(&format!("{:02x}", b));
    }
    assert_eq!(handle.content_hash, format!("sha256:{}", hex));
}

#[tokio::test]
async fn session_vcon_handle_exposes_live_builder_during_session() {
    let orch = Orchestrator::new(Config::default());
    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .unwrap();
    let sid = orch
        .start_session(cid, SessionMedium::Voice, vec![])
        .await
        .unwrap();
    let pid = ParticipantId::new();
    orch.join_session(
        sid.clone(),
        pid.clone(),
        ParticipantKind::Human,
        ParticipantRole::Customer,
    )
    .await
    .unwrap();

    let handle = orch.session_vcon_handle(&sid).expect("present");
    let snap = handle.snapshot();
    assert_eq!(snap.parties.len(), 1);
    assert_eq!(snap.parties[0].participant_id, pid);
}

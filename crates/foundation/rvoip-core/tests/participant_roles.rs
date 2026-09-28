//! UP-2 — Participant role verbs through the Orchestrator.
//!
//! `set_participant_role` is the primitive; `take_over` / `hand_off` demote
//! every other Session `Agent` to `Observer` and promote `to` to `Agent`.
//! Connections are never moved.

use rvoip_core::config::Config;
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::error::RvoipError;
use rvoip_core::events::Event;
use rvoip_core::ids::{ConnectionId, ParticipantId, TenantId};
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::participant::{ParticipantKind, ParticipantRole};
use rvoip_core::session::{ConnectionRef, SessionMedium};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::broadcast::Receiver;

async fn next_event(rx: &mut Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("event channel timed out")
        .expect("event channel closed")
}

async fn drain_until_idle(rx: &mut Receiver<Event>) {
    while tokio::time::timeout(Duration::from_millis(50), rx.recv())
        .await
        .ok()
        .and_then(Result::ok)
        .is_some()
    {}
}

fn role_of(
    orch: &Orchestrator,
    cid: &rvoip_core::ids::ConversationId,
    pid: &ParticipantId,
) -> ParticipantRole {
    orch.conversation(cid)
        .expect("conversation present")
        .read()
        .unwrap()
        .participants
        .iter()
        .find(|p| p.id == *pid)
        .expect("participant present")
        .role
        .clone()
}

fn agents_in_session(orch: &Orchestrator, sid: &rvoip_core::ids::SessionId) -> Vec<ParticipantId> {
    let session = orch.session(sid).expect("session present");
    let session = session.read().unwrap();
    let conv = orch
        .conversation(&session.conversation_id)
        .expect("conversation present");
    let conv = conv.read().unwrap();
    conv.participants
        .iter()
        .filter(|p| session.participants.contains(&p.id) && p.role == ParticipantRole::Agent)
        .map(|p| p.id.clone())
        .collect()
}

#[tokio::test]
async fn take_over_demotes_ai_agent_and_promotes_human() {
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

    let customer = ParticipantId::new();
    let ai = ParticipantId::new();
    let human = ParticipantId::new();
    let customer_conn = ConnectionId::new();

    orch.join_session(
        sid.clone(),
        customer.clone(),
        ParticipantKind::Human,
        ParticipantRole::Customer,
    )
    .await
    .expect("join customer");
    orch.join_session(
        sid.clone(),
        ai.clone(),
        ParticipantKind::Ai,
        ParticipantRole::Agent,
    )
    .await
    .expect("join ai");

    {
        let session = orch.session(&sid).expect("session");
        let mut session = session.write().unwrap();
        session.connections.insert(
            customer_conn.clone(),
            ConnectionRef {
                id: customer_conn.clone(),
                participant_id: customer.clone(),
            },
        );
    }

    let mut events = orch.subscribe_events();
    drain_until_idle(&mut events).await;

    orch.take_over(sid.clone(), human.clone(), ParticipantKind::Human)
        .await
        .expect("take_over");

    assert_eq!(role_of(&orch, &cid, &ai), ParticipantRole::Observer);
    assert_eq!(role_of(&orch, &cid, &human), ParticipantRole::Agent);
    assert_eq!(role_of(&orch, &cid, &customer), ParticipantRole::Customer);

    let remaining_agents = agents_in_session(&orch, &sid);
    assert_eq!(remaining_agents, vec![human.clone()]);

    {
        let session = orch.session(&sid).expect("session");
        let session = session.read().unwrap();
        let conn = session
            .connections
            .get(&customer_conn)
            .expect("customer connection unchanged");
        assert_eq!(conn.participant_id, customer);
        assert_eq!(session.connections.len(), 1);
        assert!(session.participants.contains(&human));
    }

    let mut role_changes = Vec::new();
    let mut joined_human = false;
    loop {
        match tokio::time::timeout(Duration::from_millis(200), next_event(&mut events)).await {
            Ok(Event::ParticipantRoleChanged {
                participant_id,
                from,
                to,
                session_id,
                conversation_id,
                ..
            }) => {
                assert_ne!(from, to, "role-change events must not fire when from == to");
                assert_eq!(conversation_id, cid);
                assert_eq!(session_id.as_ref(), Some(&sid));
                role_changes.push((participant_id, from, to));
            }
            Ok(Event::ParticipantJoined { participant_id, .. }) => {
                assert_eq!(participant_id, human);
                joined_human = true;
            }
            Ok(other) => panic!("unexpected event {other:?}"),
            Err(_) => break,
        }
    }

    assert!(joined_human, "take_over must join a missing participant");
    assert_eq!(
        role_changes,
        vec![(ai, ParticipantRole::Agent, ParticipantRole::Observer)]
    );
}

#[tokio::test]
async fn set_participant_role_is_noop_when_unchanged() {
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
    let pid = ParticipantId::new();
    orch.join_session(
        sid,
        pid.clone(),
        ParticipantKind::Human,
        ParticipantRole::Customer,
    )
    .await
    .expect("join");

    let mut events = orch.subscribe_events();
    drain_until_idle(&mut events).await;

    orch.set_participant_role(pid.clone(), ParticipantRole::Customer)
        .await
        .expect("same role");

    assert!(
        tokio::time::timeout(Duration::from_millis(150), events.recv())
            .await
            .ok()
            .and_then(Result::ok)
            .is_none(),
        "identical role must not emit ParticipantRoleChanged"
    );
}

#[tokio::test]
async fn set_participant_role_unknown_id_is_not_found() {
    let orch = Orchestrator::new(Config::default());
    let err = orch
        .set_participant_role(ParticipantId::new(), ParticipantRole::Agent)
        .await
        .expect_err("unknown participant");
    assert!(matches!(err, RvoipError::ParticipantNotFound(_)));
}

#[tokio::test]
async fn hand_off_from_non_agent_is_rejected() {
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
        .start_session(cid, SessionMedium::Voice, vec![])
        .await
        .expect("start_session");
    let customer = ParticipantId::new();
    let human = ParticipantId::new();
    orch.join_session(
        sid.clone(),
        customer.clone(),
        ParticipantKind::Human,
        ParticipantRole::Customer,
    )
    .await
    .expect("join customer");

    let err = orch
        .hand_off(sid, customer, human, ParticipantKind::Human)
        .await
        .expect_err("customer cannot hand off");
    assert!(matches!(err, RvoipError::InvalidState(_)));
}

#[tokio::test]
async fn hand_off_demotes_from_and_promotes_to() {
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
    let from = ParticipantId::new();
    let to = ParticipantId::new();
    orch.join_session(
        sid.clone(),
        from.clone(),
        ParticipantKind::Ai,
        ParticipantRole::Agent,
    )
    .await
    .expect("join from");
    orch.join_session(
        sid.clone(),
        to.clone(),
        ParticipantKind::Human,
        ParticipantRole::Observer,
    )
    .await
    .expect("join to");

    orch.hand_off(
        sid.clone(),
        from.clone(),
        to.clone(),
        ParticipantKind::Human,
    )
    .await
    .expect("hand_off");

    assert_eq!(role_of(&orch, &cid, &from), ParticipantRole::Observer);
    assert_eq!(role_of(&orch, &cid, &to), ParticipantRole::Agent);
    assert_eq!(agents_in_session(&orch, &sid), vec![to]);
}

//! P1 acceptance — Session lifecycle through the Orchestrator.
//!
//! Covers:
//! - `start_session` / `end_session` / `join_session` / `leave_session`
//!   and the events they emit.
//! - State-machine enforcement: start_session rejected on a Closed
//!   Conversation; join_session rejected on Ended Session.
//! - Initiating → Active transition on first join.
//! - `session_of()` reverse-index population/clearing via
//!   `InboundAction::Accept`.

use chrono::Utc;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, NegotiatedCodecs};
use rvoip_core::commands::InboundAction;
use rvoip_core::config::Config;
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::error::{Result as RvResult, RvoipError};
use rvoip_core::events::Event;
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, TenantId};
use rvoip_core::message::Message;
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::participant::{ParticipantKind, ParticipantRole};
use rvoip_core::session::{SessionMedium, SessionState};
use rvoip_core::stream::MediaStream;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast::Receiver, mpsc};

async fn next_event(rx: &mut Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("event channel timed out")
        .expect("event channel closed")
}

// ----------------------------------------------------------------------
// Minimal stub adapter for InboundAction::Accept coverage. Just enough
// to register, deliver an InboundConnection event, and accept it.
// ----------------------------------------------------------------------

struct StubAdapter {
    inbound: Mutex<Option<mpsc::Receiver<AdapterEvent>>>,
}

impl StubAdapter {
    fn new() -> (Arc<Self>, mpsc::Sender<AdapterEvent>) {
        let (tx, rx) = mpsc::channel(16);
        let adapter = Arc::new(Self {
            inbound: Mutex::new(Some(rx)),
        });
        (adapter, tx)
    }
}

#[async_trait::async_trait]
impl ConnectionAdapter for StubAdapter {
    fn transport(&self) -> Transport {
        Transport::Sip
    }
    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }
    async fn originate(&self, _request: OriginateRequest) -> RvResult<ConnectionHandle> {
        Err(RvoipError::NotImplemented("stub adapter: originate"))
    }
    async fn accept(&self, _conn: ConnectionId) -> RvResult<()> {
        Ok(())
    }
    async fn reject(&self, _conn: ConnectionId, _reason: RejectReason) -> RvResult<()> {
        Ok(())
    }
    async fn end(&self, _conn: ConnectionId, _reason: EndReason) -> RvResult<()> {
        Ok(())
    }
    async fn hold(&self, _conn: ConnectionId) -> RvResult<()> {
        Ok(())
    }
    async fn resume(&self, _conn: ConnectionId) -> RvResult<()> {
        Ok(())
    }
    async fn transfer(&self, _conn: ConnectionId, _target: TransferTarget) -> RvResult<()> {
        Ok(())
    }
    async fn streams(&self, _conn: ConnectionId) -> RvResult<Vec<Arc<dyn MediaStream>>> {
        Ok(vec![])
    }
    async fn send_message(&self, _conn: ConnectionId, _msg: Message) -> RvResult<()> {
        Ok(())
    }
    async fn send_dtmf(
        &self,
        _conn: ConnectionId,
        _digits: &str,
        _duration_ms: u32,
    ) -> RvResult<()> {
        Ok(())
    }
    async fn renegotiate_media(
        &self,
        _conn: ConnectionId,
        _caps: CapabilityDescriptor,
    ) -> RvResult<NegotiatedCodecs> {
        Ok(NegotiatedCodecs::default())
    }
    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        self.inbound
            .lock()
            .unwrap()
            .take()
            .expect("StubAdapter::subscribe_events called twice")
    }
    fn capabilities(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::default()
    }
    async fn verify_request_signature(
        &self,
        _conn: ConnectionId,
        _sig: SignatureHeaders,
    ) -> RvResult<IdentityAssurance> {
        Ok(IdentityAssurance::Anonymous)
    }
}

fn make_inbound_connection(id: &ConnectionId) -> Connection {
    Connection {
        id: id.clone(),
        session_id: SessionId::new(),
        participant_id: ParticipantId::new(),
        transport: Transport::Sip,
        direction: Direction::Inbound,
        state: ConnectionState::Connecting,
        capabilities: CapabilityDescriptor::default(),
        negotiated_codecs: NegotiatedCodecs::default(),
        streams: vec![],
        messaging_enabled: false,
        transport_handle: TransportHandle(Arc::new(())),
        opened_at: Utc::now(),
        closed_at: None,
    }
}

// ----------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------

#[tokio::test]
async fn start_session_emits_session_started_with_conversation_id() {
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
    // Drain ConversationOpened.
    let _ = next_event(&mut events).await;

    let sid = orch
        .start_session(cid.clone(), SessionMedium::Voice, vec![])
        .await
        .expect("start_session");

    match next_event(&mut events).await {
        Event::SessionStarted {
            session_id,
            conversation_id,
            ..
        } => {
            assert_eq!(session_id, sid);
            assert_eq!(conversation_id, cid);
        }
        other => panic!("expected SessionStarted, got {other:?}"),
    }

    let session = orch.session(&sid).expect("session tracked");
    let s = session.read().unwrap();
    assert_eq!(s.state, SessionState::Initiating);
    assert_eq!(s.medium, SessionMedium::Voice);
    assert!(s.participants.is_empty());
    assert!(s.connections.is_empty());
}

#[tokio::test]
async fn start_session_rejects_when_conversation_is_closed() {
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
        .expect("close");
    match orch.start_session(cid, SessionMedium::Voice, vec![]).await {
        Err(RvoipError::InvalidState(_)) => {}
        other => panic!("expected InvalidState, got {other:?}"),
    }
}

#[tokio::test]
async fn join_session_first_join_transitions_to_active() {
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
    let mut events = orch.subscribe_events();

    let pid = ParticipantId::new();
    orch.join_session(
        sid.clone(),
        pid.clone(),
        ParticipantKind::Human,
        ParticipantRole::Customer,
    )
    .await
    .expect("join_session");

    match next_event(&mut events).await {
        Event::ParticipantJoined {
            session_id,
            participant_id,
            ..
        } => {
            assert_eq!(session_id, sid);
            assert_eq!(participant_id, pid);
        }
        other => panic!("expected ParticipantJoined, got {other:?}"),
    }

    let session = orch.session(&sid).expect("session tracked");
    let s = session.read().unwrap();
    assert_eq!(s.state, SessionState::Active);
    assert!(s.participants.contains(&pid));

    // Conversation should have gained a matching Participant entry.
    let conv = orch.conversation(&cid).unwrap();
    let c = conv.read().unwrap();
    assert!(c.participants.iter().any(|p| p.id == pid));
}

#[tokio::test]
async fn leave_session_emits_participant_left_and_sets_left_at() {
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
        sid.clone(),
        pid.clone(),
        ParticipantKind::Human,
        ParticipantRole::Customer,
    )
    .await
    .expect("join_session");

    let mut events = orch.subscribe_events();
    orch.leave_session(sid.clone(), pid.clone())
        .await
        .expect("leave_session");

    match next_event(&mut events).await {
        Event::ParticipantLeft {
            session_id,
            participant_id,
            ..
        } => {
            assert_eq!(session_id, sid);
            assert_eq!(participant_id, pid);
        }
        other => panic!("expected ParticipantLeft, got {other:?}"),
    }

    let conv = orch.conversation(&cid).unwrap();
    let c = conv.read().unwrap();
    let p = c
        .participants
        .iter()
        .find(|p| p.id == pid)
        .expect("participant present");
    assert!(p.left_at.is_some(), "left_at populated after leave_session");
}

#[tokio::test]
async fn join_rejects_on_ended_session() {
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
    orch.end_session(sid.clone(), EndReason::Normal)
        .await
        .expect("end_session");
    match orch
        .join_session(
            sid,
            ParticipantId::new(),
            ParticipantKind::Human,
            ParticipantRole::Customer,
        )
        .await
    {
        Err(RvoipError::InvalidState(_)) => {}
        other => panic!("expected InvalidState, got {other:?}"),
    }
}

#[tokio::test]
async fn end_session_is_idempotent() {
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
    orch.end_session(sid.clone(), EndReason::Normal)
        .await
        .expect("first end");
    orch.end_session(sid, EndReason::Normal)
        .await
        .expect("second end is no-op");
}

#[tokio::test]
async fn end_session_emits_session_ended_and_clears_reverse_index() {
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

    let mut events = orch.subscribe_events();
    orch.end_session(sid.clone(), EndReason::Normal)
        .await
        .expect("end_session");

    match next_event(&mut events).await {
        Event::SessionEnded { session_id, .. } => {
            assert_eq!(session_id, sid);
        }
        other => panic!("expected SessionEnded, got {other:?}"),
    }

    let session = orch.session(&sid).expect("session row retained");
    let s = session.read().unwrap();
    assert_eq!(s.state, SessionState::Ended);
    assert!(s.ended_at.is_some());
    assert!(matches!(s.end_reason, Some(EndReason::Normal)));
}

#[tokio::test]
async fn inbound_accept_binds_connection_to_session_and_drives_active() {
    // P1.8 — InboundAction::Accept inserts a ConnectionRef into the
    // target Session's connections map and bumps Initiating → Active.
    let orch = Orchestrator::new(Config::default());
    let (adapter, inbound_tx) = StubAdapter::new();
    orch.register(adapter).expect("register");

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
    let pid = ParticipantId::new();
    let connid = ConnectionId::new();

    // Drive an inbound connection through the adapter's event stream so
    // the orchestrator tracks it.
    inbound_tx
        .send(AdapterEvent::InboundConnection {
            connection: make_inbound_connection(&connid),
        })
        .await
        .expect("emit InboundConnection");

    // Wait for the orchestrator to register the connection (the event
    // loop is async). Poll up to 500ms.
    for _ in 0..50 {
        if orch
            .adapter(Transport::Sip)
            .ok()
            .and_then(|_| Some(()))
            .is_some()
            && orch
                .conversation(&rvoip_core::ids::ConversationId::new())
                .is_none()
        {
            // sentinel, just buy time
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        if orch.session(&sid).is_some() {
            break;
        }
    }
    // Wait for InboundConnection event ingest.
    tokio::time::sleep(Duration::from_millis(50)).await;

    orch.route_inbound_connection(
        connid.clone(),
        InboundAction::Accept {
            session_id: sid.clone(),
            participant_id: pid.clone(),
        },
    )
    .await
    .expect("accept");

    // Reverse index populated.
    assert_eq!(orch.session_of(&connid), Some(sid.clone()));
    // Session has the connection + transitioned to Active.
    let session = orch.session(&sid).unwrap();
    let s = session.read().unwrap();
    assert_eq!(s.state, SessionState::Active);
    assert!(s.connections.contains_key(&connid));
    assert_eq!(s.connections[&connid].participant_id, pid);
}

#[tokio::test]
async fn adapter_ended_auto_ends_session_when_last_connection_leaves() {
    // P1.10 — when the last Connection bound to an Active Session
    // ends, the orchestrator auto-transitions the Session to Ended and
    // emits SessionEnded.
    let orch = Orchestrator::new(Config::default());
    let (adapter, inbound_tx) = StubAdapter::new();
    orch.register(adapter).expect("register");

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
    let connid = ConnectionId::new();
    inbound_tx
        .send(AdapterEvent::InboundConnection {
            connection: make_inbound_connection(&connid),
        })
        .await
        .expect("emit InboundConnection");
    tokio::time::sleep(Duration::from_millis(50)).await;

    orch.route_inbound_connection(
        connid.clone(),
        InboundAction::Accept {
            session_id: sid.clone(),
            participant_id: ParticipantId::new(),
        },
    )
    .await
    .expect("accept");

    let mut events = orch.subscribe_events();

    // Emit Ended for the only bound Connection.
    inbound_tx
        .send(AdapterEvent::Ended {
            connection_id: connid.clone(),
            reason: EndReason::Normal,
        })
        .await
        .expect("emit Ended");

    // We expect ConnectionEnded + SessionEnded, in either order
    // depending on which spawned task runs first.
    let mut saw_conn_ended = false;
    let mut saw_sess_ended = false;
    for _ in 0..2 {
        match next_event(&mut events).await {
            Event::ConnectionEnded { connection_id, .. } => {
                assert_eq!(connection_id, connid);
                saw_conn_ended = true;
            }
            Event::SessionEnded { session_id, .. } => {
                assert_eq!(session_id, sid);
                saw_sess_ended = true;
            }
            other => panic!("unexpected event during auto-end: {other:?}"),
        }
    }
    assert!(saw_conn_ended && saw_sess_ended);

    // Reverse index cleared.
    assert_eq!(orch.session_of(&connid), None);
}

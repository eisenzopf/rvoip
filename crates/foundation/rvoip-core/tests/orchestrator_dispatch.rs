//! Smoke test: Orchestrator dispatches commands through a registered adapter
//! and emits normalized events from adapter-event traffic.

use chrono::Utc;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, NegotiatedCodecs};
use rvoip_core::commands::InboundAction;
use rvoip_core::config::Config;
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::error::{Result, RvoipError};
use rvoip_core::events::Event;
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId};
use rvoip_core::message::Message;
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::stream::MediaStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Debug, Default)]
struct CallCounts {
    accept: AtomicUsize,
    reject: AtomicUsize,
    end: AtomicUsize,
    hold: AtomicUsize,
    resume: AtomicUsize,
    transfer: AtomicUsize,
    dtmf: AtomicUsize,
}

struct StubAdapter {
    counts: Arc<CallCounts>,
    inbound: Mutex<Option<mpsc::Receiver<AdapterEvent>>>,
}

impl StubAdapter {
    fn new() -> (Arc<Self>, mpsc::Sender<AdapterEvent>, Arc<CallCounts>) {
        let (tx, rx) = mpsc::channel(16);
        let counts = Arc::new(CallCounts::default());
        let adapter = Arc::new(Self {
            counts: counts.clone(),
            inbound: Mutex::new(Some(rx)),
        });
        (adapter, tx, counts)
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
    async fn originate(&self, request: OriginateRequest) -> Result<ConnectionHandle> {
        let conn = Connection {
            id: ConnectionId::new(),
            session_id: request.session_id,
            participant_id: request.participant_id,
            transport: Transport::Sip,
            direction: Direction::Outbound,
            state: ConnectionState::Connecting,
            capabilities: request.capabilities,
            negotiated_codecs: NegotiatedCodecs::default(),
            streams: vec![],
            messaging_enabled: false,
            transport_handle: TransportHandle(Arc::new(())),
            opened_at: Utc::now(),
            closed_at: None,
        };
        Ok(ConnectionHandle { connection: conn })
    }
    async fn accept(&self, _conn: ConnectionId) -> Result<()> {
        self.counts.accept.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn reject(&self, _conn: ConnectionId, _reason: RejectReason) -> Result<()> {
        self.counts.reject.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn end(&self, _conn: ConnectionId, _reason: EndReason) -> Result<()> {
        self.counts.end.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn hold(&self, _conn: ConnectionId) -> Result<()> {
        self.counts.hold.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn resume(&self, _conn: ConnectionId) -> Result<()> {
        self.counts.resume.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn transfer(&self, _conn: ConnectionId, _target: TransferTarget) -> Result<()> {
        self.counts.transfer.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn streams(&self, _conn: ConnectionId) -> Result<Vec<Arc<dyn MediaStream>>> {
        Ok(vec![])
    }
    async fn send_message(&self, _conn: ConnectionId, _message: Message) -> Result<()> {
        Ok(())
    }
    async fn send_dtmf(&self, _conn: ConnectionId, _digits: &str, _ms: u32) -> Result<()> {
        self.counts.dtmf.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn renegotiate_media(
        &self,
        _conn: ConnectionId,
        _capabilities: CapabilityDescriptor,
    ) -> Result<NegotiatedCodecs> {
        Ok(NegotiatedCodecs::default())
    }
    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        // Single-consumer: only register() should call this once.
        self.inbound
            .lock()
            .unwrap()
            .take()
            .expect("subscribe_events already consumed")
    }
    fn capabilities(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::default()
    }
    async fn verify_request_signature(
        &self,
        _conn: ConnectionId,
        _signature: SignatureHeaders,
    ) -> Result<IdentityAssurance> {
        Ok(IdentityAssurance::Anonymous)
    }
}

fn fake_inbound_connection() -> Connection {
    Connection {
        id: ConnectionId::new(),
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

#[tokio::test]
async fn register_then_dispatch_routes_through_adapter() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, adapter_tx, counts) = StubAdapter::new();
    orch.register(adapter).expect("first register succeeds");

    // Subscribe before pushing the inbound event.
    let mut events = orch.subscribe_events();

    // Adapter announces an inbound connection. Orchestrator should normalize
    // it into Event::ConnectionInbound and track the connection.
    let conn = fake_inbound_connection();
    let conn_id = conn.id.clone();
    adapter_tx
        .send(AdapterEvent::InboundConnection { connection: conn })
        .await
        .unwrap();

    // Wait for the normalized event.
    let normalized = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
        .await
        .expect("event arrives within 1s")
        .expect("broadcast not closed");
    match normalized {
        Event::ConnectionInbound { connection_id, .. } => assert_eq!(connection_id, conn_id),
        other => panic!("unexpected event: {other:?}"),
    }

    // Now route — accept dispatches to adapter.accept(). P1.8 made
    // InboundAction::Accept require a live SessionId, so open a
    // Conversation + start a Session first.
    let cid = orch
        .open_conversation(
            rvoip_core::ids::TenantId::new(),
            rvoip_core::conversation::ConversationPolicy::default(),
            std::collections::HashMap::new(),
        )
        .await
        .expect("open_conversation");
    let sid = orch
        .start_session(cid, rvoip_core::session::SessionMedium::Voice, vec![])
        .await
        .expect("start_session");
    orch.route_inbound_connection(
        conn_id.clone(),
        InboundAction::Accept {
            session_id: sid,
            participant_id: ParticipantId::new(),
        },
    )
    .await
    .unwrap();
    assert_eq!(counts.accept.load(Ordering::SeqCst), 1);

    // hold/resume/transfer/dtmf/end all dispatch.
    orch.hold(conn_id.clone()).await.unwrap();
    orch.resume(conn_id.clone()).await.unwrap();
    orch.transfer_connection(
        conn_id.clone(),
        TransferTarget::Uri("sip:bob@example.com".into()),
    )
    .await
    .unwrap();
    orch.send_dtmf(conn_id.clone(), "1234", 100).await.unwrap();
    orch.end_connection(conn_id, EndReason::Normal)
        .await
        .unwrap();

    assert_eq!(counts.hold.load(Ordering::SeqCst), 1);
    assert_eq!(counts.resume.load(Ordering::SeqCst), 1);
    assert_eq!(counts.transfer.load(Ordering::SeqCst), 1);
    assert_eq!(counts.dtmf.load(Ordering::SeqCst), 1);
    assert_eq!(counts.end.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn dispatch_without_adapter_returns_no_adapter_error() {
    let orch = Orchestrator::new(Config::default());
    let result = orch
        .end_connection(ConnectionId::new(), EndReason::Normal)
        .await;
    match result {
        Err(RvoipError::ConnectionNotFound(_)) => {}
        other => panic!("expected ConnectionNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn duplicate_register_rejects() {
    let orch = Orchestrator::new(Config::default());
    let (adapter1, _tx1, _) = StubAdapter::new();
    let (adapter2, _tx2, _) = StubAdapter::new();
    orch.register(adapter1).unwrap();
    let err = orch.register(adapter2).unwrap_err();
    matches!(err, RvoipError::AdapterAlreadyRegistered(Transport::Sip));
}

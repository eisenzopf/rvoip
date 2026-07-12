//! Smoke test: Orchestrator dispatches commands through a registered adapter
//! and emits normalized events from adapter-event traffic.

use chrono::Utc;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, AdapterLifecycleSink, AdapterLifecycleSinkSlot, ConnectionAdapter,
    ConnectionHandle, EndReason, InboundConnectionContext, InboundRoutingHint,
    InboundSignalingMetadata, OrchestratorAdapterEvent, OriginateRequest, RejectReason,
    SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, NegotiatedCodecs};
use rvoip_core::commands::InboundAction;
use rvoip_core::config::Config;
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::error::{Result, RvoipError};
use rvoip_core::events::Event;
use rvoip_core::identity::{
    AuthenticatedPrincipal, AuthenticationMethod, CredentialKind, IdentityAssurance,
};
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, StreamId};
use rvoip_core::message::Message;
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::stream::MediaStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
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
    inbound: Mutex<Option<mpsc::Receiver<OrchestratorAdapterEvent>>>,
    live: Mutex<HashSet<ConnectionId>>,
    inbound_contexts: Mutex<HashMap<ConnectionId, InboundConnectionContext>>,
    lifecycle: AdapterLifecycleSinkSlot,
}

#[derive(Clone)]
struct StubEventSender(mpsc::Sender<OrchestratorAdapterEvent>);

impl StubEventSender {
    async fn send(
        &self,
        event: AdapterEvent,
    ) -> std::result::Result<(), mpsc::error::SendError<OrchestratorAdapterEvent>> {
        self.0.send(event.into()).await
    }

    async fn send_atomic(
        &self,
        event: OrchestratorAdapterEvent,
    ) -> std::result::Result<(), mpsc::error::SendError<OrchestratorAdapterEvent>> {
        self.0.send(event).await
    }
}

impl StubAdapter {
    fn new() -> (Arc<Self>, StubEventSender, Arc<CallCounts>) {
        let (tx, rx) = mpsc::channel(16);
        let counts = Arc::new(CallCounts::default());
        let adapter = Arc::new(Self {
            counts: counts.clone(),
            inbound: Mutex::new(Some(rx)),
            live: Mutex::new(HashSet::new()),
            inbound_contexts: Mutex::new(HashMap::new()),
            lifecycle: AdapterLifecycleSinkSlot::default(),
        });
        (adapter, StubEventSender(tx), counts)
    }

    fn mark_live(&self, connection_id: ConnectionId) {
        self.live.lock().unwrap().insert(connection_id);
    }

    fn mark_ended(&self, connection_id: &ConnectionId) {
        self.live.lock().unwrap().remove(connection_id);
    }

    fn set_inbound_context(&self, context: InboundConnectionContext) {
        let connection_id = context.connection_id().clone();
        self.set_inbound_context_for(connection_id, context);
    }

    fn set_inbound_context_for(
        &self,
        connection_id: ConnectionId,
        context: InboundConnectionContext,
    ) {
        self.inbound_contexts
            .lock()
            .unwrap()
            .insert(connection_id, context);
    }

    async fn deliver_terminal(&self, event: AdapterEvent) {
        assert!(self.lifecycle.deliver_terminal(event).await);
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
    fn install_lifecycle_sink(&self, sink: Arc<dyn AdapterLifecycleSink>) -> Result<()> {
        self.lifecycle
            .install(sink)
            .map_err(|_| RvoipError::InvalidState("stub lifecycle sink already installed"))
    }
    fn is_connection_live(&self, conn: &ConnectionId) -> bool {
        self.live.lock().unwrap().contains(conn)
    }
    fn take_inbound_context(&self, conn: &ConnectionId) -> Option<InboundConnectionContext> {
        self.inbound_contexts.lock().unwrap().remove(conn)
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
        let (_sender, receiver) = mpsc::channel(1);
        receiver
    }
    fn subscribe_orchestrator_events(&self) -> mpsc::Receiver<OrchestratorAdapterEvent> {
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

fn authenticated_principal(tenant: &str) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        subject: "attachment-owner".into(),
        tenant: Some(tenant.into()),
        scopes: vec!["call:attach".into()],
        issuer: Some("https://issuer.invalid".into()),
        expires_at: None,
        method: AuthenticationMethod::Jwt,
        assurance: IdentityAssurance::Identified {
            credential_kind: CredentialKind::Oidc,
        },
    }
}

async fn wait_for_connection_principal(orchestrator: &Orchestrator, connection_id: &ConnectionId) {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if orchestrator.connection_principal(connection_id).is_ok() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("principal is retained within one second");
}

#[tokio::test]
async fn atomic_authenticated_inbound_handoff_preserves_legacy_normalized_order() {
    let orchestrator = Orchestrator::new(Config::default());
    let (adapter, adapter_tx, _) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();

    let connection = fake_inbound_connection();
    let connection_id = connection.id.clone();
    let owner = authenticated_principal("tenant-a");
    adapter.mark_live(connection_id.clone());
    adapter.set_inbound_context(
        InboundConnectionContext::new(
            connection_id.clone(),
            Transport::Sip,
            &owner,
            Some(InboundRoutingHint::new("atomic-attachment").unwrap()),
            InboundSignalingMetadata::default(),
        )
        .unwrap(),
    );
    adapter_tx
        .send_atomic(OrchestratorAdapterEvent::AuthenticatedInboundConnection {
            connection,
            participant_id: "participant-a".into(),
            principal: owner.clone(),
        })
        .await
        .unwrap();

    assert!(matches!(
        events.recv().await.unwrap(),
        Event::ConnectionInbound { connection_id: id, .. } if id == connection_id
    ));
    assert!(matches!(
        events.recv().await.unwrap(),
        Event::ConnectionAuthenticated { connection_id: id, .. } if id == connection_id
    ));
    assert!(matches!(
        events.recv().await.unwrap(),
        Event::ConnectionPrincipalAuthenticated { connection_id: id, .. } if id == connection_id
    ));
    let context = orchestrator
        .take_inbound_context(&connection_id, &owner)
        .unwrap()
        .expect("atomic handoff retained context");
    assert_eq!(
        context.routing_hint().unwrap().expose_secret(),
        "atomic-attachment"
    );
}

#[tokio::test]
async fn inbound_context_is_owner_bound_single_take_and_erased_on_terminal() {
    let orchestrator = Orchestrator::new(Config::default());
    let (adapter, adapter_tx, _) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();

    let connection = fake_inbound_connection();
    let connection_id = connection.id.clone();
    let owner = authenticated_principal("tenant-a");
    adapter.mark_live(connection_id.clone());
    adapter.set_inbound_context(
        InboundConnectionContext::new(
            connection_id.clone(),
            Transport::Sip,
            &owner,
            Some(InboundRoutingHint::new("single-use-attachment").unwrap()),
            InboundSignalingMetadata::new([("x-correlation-id", "opaque-value")]).unwrap(),
        )
        .unwrap(),
    );
    adapter_tx
        .send(AdapterEvent::InboundConnection { connection })
        .await
        .unwrap();
    adapter_tx
        .send(AdapterEvent::PrincipalAuthenticated {
            connection_id: connection_id.clone(),
            participant_id: "participant-a".into(),
            principal: owner.clone(),
        })
        .await
        .unwrap();
    wait_for_connection_principal(&orchestrator, &connection_id).await;

    let unrelated_tenant = authenticated_principal("tenant-b");
    assert!(matches!(
        orchestrator.take_inbound_context(&connection_id, &unrelated_tenant),
        Err(RvoipError::AdmissionRejected(
            "inbound context principal mismatch"
        ))
    ));
    assert!(matches!(
        orchestrator.take_inbound_context(&ConnectionId::new(), &owner),
        Err(RvoipError::ConnectionNotFound(_))
    ));

    let context = orchestrator
        .take_inbound_context(&connection_id, &owner)
        .unwrap()
        .expect("legitimate owner takes retained context");
    assert_eq!(
        context.routing_hint().unwrap().expose_secret(),
        "single-use-attachment"
    );
    assert_eq!(
        context
            .metadata()
            .values("x-correlation-id")
            .collect::<Vec<_>>(),
        vec!["opaque-value"]
    );
    assert!(orchestrator
        .take_inbound_context(&connection_id, &owner)
        .unwrap()
        .is_none());

    // A second connection retains untaken context only until terminal
    // teardown, after which even its route is no longer observable.
    let terminal_connection = fake_inbound_connection();
    let terminal_id = terminal_connection.id.clone();
    adapter.mark_live(terminal_id.clone());
    adapter.set_inbound_context(
        InboundConnectionContext::new(
            terminal_id.clone(),
            Transport::Sip,
            &owner,
            Some(InboundRoutingHint::new("must-be-erased").unwrap()),
            InboundSignalingMetadata::default(),
        )
        .unwrap(),
    );
    adapter_tx
        .send(AdapterEvent::InboundConnection {
            connection: terminal_connection,
        })
        .await
        .unwrap();
    adapter_tx
        .send(AdapterEvent::PrincipalAuthenticated {
            connection_id: terminal_id.clone(),
            participant_id: "participant-b".into(),
            principal: owner.clone(),
        })
        .await
        .unwrap();
    wait_for_connection_principal(&orchestrator, &terminal_id).await;
    adapter.mark_ended(&terminal_id);
    adapter
        .deliver_terminal(AdapterEvent::Ended {
            connection_id: terminal_id.clone(),
            reason: EndReason::Normal,
        })
        .await;
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if matches!(
                orchestrator.connection_transport(&terminal_id),
                Err(RvoipError::ConnectionNotFound(_))
            ) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        orchestrator.take_inbound_context(&terminal_id, &owner),
        Err(RvoipError::ConnectionNotFound(_))
    ));
}

#[tokio::test]
async fn inbound_context_rejects_adapter_binding_mismatches_and_defaults_to_none() {
    let orchestrator = Orchestrator::new(Config::default());
    let (adapter, adapter_tx, _) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let owner = authenticated_principal("tenant-a");

    let wrong_transport_connection = fake_inbound_connection();
    let wrong_transport_id = wrong_transport_connection.id.clone();
    adapter.mark_live(wrong_transport_id.clone());
    adapter.set_inbound_context(
        InboundConnectionContext::new(
            wrong_transport_id.clone(),
            Transport::WebRtc,
            &owner,
            Some(InboundRoutingHint::new("wrong-transport").unwrap()),
            InboundSignalingMetadata::default(),
        )
        .unwrap(),
    );
    adapter_tx
        .send(AdapterEvent::InboundConnection {
            connection: wrong_transport_connection,
        })
        .await
        .unwrap();
    adapter_tx
        .send(AdapterEvent::PrincipalAuthenticated {
            connection_id: wrong_transport_id.clone(),
            participant_id: "participant-a".into(),
            principal: owner.clone(),
        })
        .await
        .unwrap();
    wait_for_connection_principal(&orchestrator, &wrong_transport_id).await;
    assert!(orchestrator
        .take_inbound_context(&wrong_transport_id, &owner)
        .unwrap()
        .is_none());

    let wrong_connection = fake_inbound_connection();
    let wrong_connection_id = wrong_connection.id.clone();
    adapter.mark_live(wrong_connection_id.clone());
    adapter.set_inbound_context_for(
        wrong_connection_id.clone(),
        InboundConnectionContext::new(
            ConnectionId::new(),
            Transport::Sip,
            &owner,
            Some(InboundRoutingHint::new("wrong-connection").unwrap()),
            InboundSignalingMetadata::default(),
        )
        .unwrap(),
    );
    adapter_tx
        .send(AdapterEvent::InboundConnection {
            connection: wrong_connection,
        })
        .await
        .unwrap();
    adapter_tx
        .send(AdapterEvent::PrincipalAuthenticated {
            connection_id: wrong_connection_id.clone(),
            participant_id: "participant-wrong-connection".into(),
            principal: owner.clone(),
        })
        .await
        .unwrap();
    wait_for_connection_principal(&orchestrator, &wrong_connection_id).await;
    assert!(orchestrator
        .take_inbound_context(&wrong_connection_id, &owner)
        .unwrap()
        .is_none());

    let no_context_connection = fake_inbound_connection();
    let no_context_id = no_context_connection.id.clone();
    adapter.mark_live(no_context_id.clone());
    adapter_tx
        .send(AdapterEvent::InboundConnection {
            connection: no_context_connection,
        })
        .await
        .unwrap();
    adapter_tx
        .send(AdapterEvent::PrincipalAuthenticated {
            connection_id: no_context_id.clone(),
            participant_id: "participant-b".into(),
            principal: owner.clone(),
        })
        .await
        .unwrap();
    wait_for_connection_principal(&orchestrator, &no_context_id).await;
    assert!(orchestrator
        .take_inbound_context(&no_context_id, &owner)
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn register_then_dispatch_routes_through_adapter() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, adapter_tx, counts) = StubAdapter::new();
    orch.register(adapter.clone())
        .expect("first register succeeds");

    // Subscribe before pushing the inbound event.
    let mut events = orch.subscribe_events();

    // Adapter announces an inbound connection. Orchestrator should normalize
    // it into Event::ConnectionInbound and track the connection.
    let conn = fake_inbound_connection();
    let conn_id = conn.id.clone();
    adapter.mark_live(conn_id.clone());
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

#[tokio::test]
async fn same_adapter_cannot_replace_lifecycle_owner() {
    let first = Orchestrator::new(Config::default());
    let second = Orchestrator::new(Config::default());
    let (adapter, adapter_tx, _) = StubAdapter::new();
    first.register(adapter.clone()).expect("first registration");
    assert!(matches!(
        second.register(adapter.clone()),
        Err(RvoipError::InvalidState(_))
    ));

    let mut first_events = first.subscribe_events();
    let conn = fake_inbound_connection();
    let conn_id = conn.id.clone();
    adapter.mark_live(conn_id.clone());
    adapter_tx
        .send(AdapterEvent::InboundConnection { connection: conn })
        .await
        .expect("send inbound event");
    let _ = tokio::time::timeout(Duration::from_secs(1), first_events.recv())
        .await
        .expect("first owner receives inbound event")
        .expect("event stream open");
    adapter.mark_ended(&conn_id);
    adapter
        .deliver_terminal(AdapterEvent::Ended {
            connection_id: conn_id.clone(),
            reason: EndReason::Normal,
        })
        .await;
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), first_events.recv())
            .await
            .expect("first owner receives terminal event")
            .expect("event stream open"),
        Event::ConnectionEnded { connection_id, .. } if connection_id == conn_id
    ));
    assert!(matches!(
        first.connection_transport(&conn_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));
}

#[tokio::test]
async fn direct_terminal_fallback_cleans_routes_and_emits_once() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, adapter_tx, _) = StubAdapter::new();
    orch.register(adapter.clone()).expect("register adapter");
    let mut events = orch.subscribe_events();

    let conn = fake_inbound_connection();
    let conn_id = conn.id.clone();
    adapter.mark_live(conn_id.clone());
    adapter_tx
        .send(AdapterEvent::InboundConnection { connection: conn })
        .await
        .expect("send inbound event");
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("inbound event timeout")
            .expect("event stream open"),
        Event::ConnectionInbound { .. }
    ));

    let session_id = SessionId::new();
    let publisher = ConnectionId::new();
    let stream_id = StreamId::new();
    orch.add_subscription(
        session_id.clone(),
        conn_id.clone(),
        publisher.clone(),
        stream_id.clone(),
    );
    assert_eq!(
        orch.subscribers_for(&session_id, &publisher, &stream_id),
        vec![conn_id.clone()]
    );

    // Transport-owned state is removed before the direct lifecycle callback.
    adapter.mark_ended(&conn_id);
    adapter
        .deliver_terminal(AdapterEvent::Ended {
            connection_id: conn_id.clone(),
            reason: EndReason::Normal,
        })
        .await;

    assert!(matches!(
        orch.connection_transport(&conn_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));
    assert!(orch
        .subscribers_for(&session_id, &publisher, &stream_id)
        .is_empty());
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("terminal event timeout")
            .expect("event stream open"),
        Event::ConnectionEnded { connection_id, .. } if connection_id == conn_id
    ));

    // A duplicate transport terminal is idempotent and emits no second event.
    adapter
        .deliver_terminal(AdapterEvent::Ended {
            connection_id: conn_id,
            reason: EndReason::Normal,
        })
        .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), events.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn queued_nonterminal_events_cannot_resurrect_an_ended_route() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, adapter_tx, _) = StubAdapter::new();
    orch.register(adapter.clone()).expect("register adapter");
    let mut events = orch.subscribe_events();

    let conn = fake_inbound_connection();
    let conn_id = conn.id.clone();
    adapter.mark_live(conn_id.clone());
    adapter_tx
        .send(AdapterEvent::InboundConnection { connection: conn })
        .await
        .expect("send inbound event");
    let _ = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .expect("inbound event timeout")
        .expect("event stream open");

    adapter.mark_ended(&conn_id);
    adapter_tx
        .send(AdapterEvent::Connected {
            connection_id: conn_id.clone(),
        })
        .await
        .expect("queue stale connected event");
    adapter_tx
        .send(AdapterEvent::Dtmf {
            connection_id: conn_id.clone(),
            digits: "1".into(),
            duration_ms: 100,
        })
        .await
        .expect("queue stale DTMF event");
    adapter
        .deliver_terminal(AdapterEvent::Ended {
            connection_id: conn_id.clone(),
            reason: EndReason::Normal,
        })
        .await;

    assert!(matches!(
        orch.connection_transport(&conn_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));
    let mut terminal_count = 0;
    let mut stale_count = 0;
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(100), events.recv()).await
    {
        match event {
            Event::ConnectionEnded { connection_id, .. } if connection_id == conn_id => {
                terminal_count += 1;
            }
            Event::ConnectionConnected { connection_id, .. }
            | Event::DtmfReceived { connection_id, .. }
                if connection_id == conn_id =>
            {
                stale_count += 1;
            }
            _ => {}
        }
    }
    assert_eq!(terminal_count, 1);
    assert_eq!(stale_count, 0);
}

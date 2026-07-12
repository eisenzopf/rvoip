//! Smoke test: Orchestrator dispatches commands through a registered adapter
//! and emits normalized events from adapter-event traffic.

use chrono::{Duration as ChronoDuration, Utc};
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, AdapterLifecycleCapabilities, AdapterLifecycleSink,
    AdapterLifecycleSinkSlot, ConnectionAdapter, ConnectionHandle, EndReason,
    InboundConnectionContext, InboundRoutingHint, InboundSignalingMetadata,
    OrchestratorAdapterEvent, OriginateRequest, RejectReason, SignatureHeaders, TransferTarget,
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
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, StreamId, TenantId};
use rvoip_core::message::Message;
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::stream::{MediaStream, QualitySnapshot};
use rvoip_core::DataMessage;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tokio::sync::{mpsc, Barrier};

#[derive(Debug, Default)]
struct CallCounts {
    accept: AtomicUsize,
    reject: AtomicUsize,
    end: AtomicUsize,
    hold: AtomicUsize,
    resume: AtomicUsize,
    transfer: AtomicUsize,
    dtmf: AtomicUsize,
    reject_reasons: Mutex<Vec<RejectReason>>,
}

struct StubAdapter {
    transport: Transport,
    counts: Arc<CallCounts>,
    inbound: Mutex<Option<mpsc::Receiver<OrchestratorAdapterEvent>>>,
    live: Mutex<HashSet<ConnectionId>>,
    inbound_contexts: Mutex<HashMap<ConnectionId, InboundConnectionContext>>,
    lifecycle: AdapterLifecycleSinkSlot,
    lifecycle_capable: bool,
    reject_behavior: AtomicUsize,
    end_behavior: AtomicUsize,
    accept_behavior: AtomicUsize,
    originate_behavior: AtomicUsize,
    activate_behavior: AtomicUsize,
    next_outbound_id: Mutex<Option<ConnectionId>>,
}

const CLEANUP_SUCCEED: usize = 0;
const CLEANUP_FAIL: usize = 1;
const CLEANUP_HANG: usize = 2;
const COMMAND_SUCCEED: usize = 0;
const COMMAND_FAIL: usize = 1;
const COMMAND_TERMINAL: usize = 2;

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
        Self::new_for(Transport::Sip)
    }

    fn new_for(transport: Transport) -> (Arc<Self>, StubEventSender, Arc<CallCounts>) {
        Self::new_for_with_capability(transport, true)
    }

    fn new_for_with_capability(
        transport: Transport,
        lifecycle_capable: bool,
    ) -> (Arc<Self>, StubEventSender, Arc<CallCounts>) {
        let (tx, rx) = mpsc::channel(16);
        let counts = Arc::new(CallCounts::default());
        let adapter = Arc::new(Self {
            transport,
            counts: counts.clone(),
            inbound: Mutex::new(Some(rx)),
            live: Mutex::new(HashSet::new()),
            inbound_contexts: Mutex::new(HashMap::new()),
            lifecycle: AdapterLifecycleSinkSlot::default(),
            lifecycle_capable,
            reject_behavior: AtomicUsize::new(CLEANUP_SUCCEED),
            end_behavior: AtomicUsize::new(CLEANUP_SUCCEED),
            accept_behavior: AtomicUsize::new(COMMAND_SUCCEED),
            originate_behavior: AtomicUsize::new(COMMAND_SUCCEED),
            activate_behavior: AtomicUsize::new(COMMAND_SUCCEED),
            next_outbound_id: Mutex::new(None),
        });
        (adapter, StubEventSender(tx), counts)
    }

    fn set_cleanup_behavior(&self, reject: usize, end: usize) {
        self.reject_behavior.store(reject, Ordering::SeqCst);
        self.end_behavior.store(end, Ordering::SeqCst);
    }

    fn set_accept_behavior(&self, behavior: usize) {
        self.accept_behavior.store(behavior, Ordering::SeqCst);
    }

    fn set_originate_behavior(&self, behavior: usize) {
        self.originate_behavior.store(behavior, Ordering::SeqCst);
    }

    fn set_activate_behavior(&self, behavior: usize) {
        self.activate_behavior.store(behavior, Ordering::SeqCst);
    }

    fn set_next_outbound_id(&self, connection_id: ConnectionId) {
        *self.next_outbound_id.lock().unwrap() = Some(connection_id);
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
        self.transport
    }
    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }
    fn lifecycle_capabilities(&self) -> AdapterLifecycleCapabilities {
        if self.lifecycle_capable {
            AdapterLifecycleCapabilities {
                staged_outbound_activation: true,
                ..AdapterLifecycleCapabilities::FAIL_CLOSED_INBOUND
            }
        } else {
            AdapterLifecycleCapabilities::default()
        }
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
        if self.originate_behavior.load(Ordering::SeqCst) == COMMAND_FAIL {
            return Err(RvoipError::InvalidState("stub originate failure"));
        }
        let conn = Connection {
            id: self
                .next_outbound_id
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(ConnectionId::new),
            session_id: request.session_id,
            participant_id: request.participant_id,
            transport: self.transport,
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
        self.mark_live(conn.id.clone());
        if self.originate_behavior.load(Ordering::SeqCst) == COMMAND_TERMINAL {
            self.mark_ended(&conn.id);
            assert!(
                self.lifecycle
                    .deliver_terminal(AdapterEvent::Ended {
                        connection_id: conn.id.clone(),
                        reason: EndReason::Normal,
                    })
                    .await
            );
        }
        Ok(ConnectionHandle { connection: conn })
    }
    async fn activate_outbound(&self, conn: ConnectionId) -> Result<()> {
        match self.activate_behavior.load(Ordering::SeqCst) {
            COMMAND_SUCCEED => Ok(()),
            COMMAND_FAIL => Err(RvoipError::InvalidState("stub activation failure")),
            COMMAND_TERMINAL => {
                self.mark_ended(&conn);
                assert!(
                    self.lifecycle
                        .deliver_terminal(AdapterEvent::Ended {
                            connection_id: conn,
                            reason: EndReason::Normal,
                        })
                        .await
                );
                Ok(())
            }
            _ => unreachable!("unknown activation behavior"),
        }
    }
    async fn accept(&self, conn: ConnectionId) -> Result<()> {
        self.counts.accept.fetch_add(1, Ordering::SeqCst);
        match self.accept_behavior.load(Ordering::SeqCst) {
            COMMAND_SUCCEED => Ok(()),
            COMMAND_FAIL => Err(RvoipError::InvalidState("stub accept failure")),
            COMMAND_TERMINAL => {
                self.mark_ended(&conn);
                assert!(
                    self.lifecycle
                        .deliver_terminal(AdapterEvent::Ended {
                            connection_id: conn,
                            reason: EndReason::Normal,
                        })
                        .await
                );
                Ok(())
            }
            _ => unreachable!("unknown accept behavior"),
        }
    }
    async fn reject(&self, conn: ConnectionId, reason: RejectReason) -> Result<()> {
        self.counts.reject.fetch_add(1, Ordering::SeqCst);
        self.counts.reject_reasons.lock().unwrap().push(reason);
        match self.reject_behavior.load(Ordering::SeqCst) {
            CLEANUP_SUCCEED => {
                self.mark_ended(&conn);
                Ok(())
            }
            CLEANUP_FAIL => Err(RvoipError::InvalidState("stub reject failure")),
            CLEANUP_HANG => std::future::pending().await,
            _ => unreachable!("unknown stub reject behavior"),
        }
    }
    async fn end(&self, conn: ConnectionId, _reason: EndReason) -> Result<()> {
        self.counts.end.fetch_add(1, Ordering::SeqCst);
        match self.end_behavior.load(Ordering::SeqCst) {
            CLEANUP_SUCCEED => {
                self.mark_ended(&conn);
                Ok(())
            }
            CLEANUP_FAIL => Err(RvoipError::InvalidState("stub end failure")),
            CLEANUP_HANG => std::future::pending().await,
            _ => unreachable!("unknown stub end behavior"),
        }
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
    fake_inbound_connection_for(Transport::Sip)
}

fn fake_inbound_connection_for(transport: Transport) -> Connection {
    Connection {
        id: ConnectionId::new(),
        session_id: SessionId::new(),
        participant_id: ParticipantId::new(),
        transport,
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

    let mut duplicate_events = orchestrator.subscribe_events();
    let mut duplicate = fake_inbound_connection();
    duplicate.id = connection_id.clone();
    adapter.set_inbound_context(
        InboundConnectionContext::new(
            connection_id.clone(),
            Transport::Sip,
            &owner,
            Some(InboundRoutingHint::new("must-stay-retired").unwrap()),
            InboundSignalingMetadata::default(),
        )
        .unwrap(),
    );
    adapter_tx
        .send(AdapterEvent::InboundConnection {
            connection: duplicate,
        })
        .await
        .unwrap();
    adapter_tx
        .send(AdapterEvent::Connected {
            connection_id: connection_id.clone(),
        })
        .await
        .unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), duplicate_events.recv())
            .await
            .unwrap()
            .unwrap(),
        Event::ConnectionConnected { connection_id: id, .. } if id == connection_id
    ));
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
async fn principal_binding_mismatch_permanently_retires_context_for_generation() {
    let orchestrator = Orchestrator::new(Config::default());
    let (adapter, sender, _) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();

    let connection = fake_inbound_connection();
    let connection_id = connection.id.clone();
    let principal = authenticated_principal("tenant-owner");
    let wrong_owner = authenticated_principal("tenant-other");
    adapter.mark_live(connection_id.clone());
    adapter.set_inbound_context(
        InboundConnectionContext::new(
            connection_id.clone(),
            Transport::Sip,
            &wrong_owner,
            Some(InboundRoutingHint::new("poisoned-private-token").unwrap()),
            InboundSignalingMetadata::default(),
        )
        .unwrap(),
    );
    announce_atomic_inbound(&sender, connection, principal.clone()).await;
    for _ in 0..3 {
        let _ = events.recv().await.unwrap();
    }

    let mut duplicate = fake_inbound_connection();
    duplicate.id = connection_id.clone();
    adapter.set_inbound_context(
        InboundConnectionContext::new(
            connection_id.clone(),
            Transport::Sip,
            &principal,
            Some(InboundRoutingHint::new("must-not-repopulate").unwrap()),
            InboundSignalingMetadata::default(),
        )
        .unwrap(),
    );
    sender
        .send(AdapterEvent::InboundConnection {
            connection: duplicate,
        })
        .await
        .unwrap();
    sender
        .send(AdapterEvent::Connected {
            connection_id: connection_id.clone(),
        })
        .await
        .unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap(),
        Event::ConnectionConnected { connection_id: id, .. } if id == connection_id
    ));
    assert!(orchestrator
        .take_inbound_context(&connection_id, &principal)
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
async fn outbound_terminal_before_return_tombstones_id_without_publication() {
    let orchestrator = Orchestrator::new(Config::default());
    let (adapter, _, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let session_id = start_voice_session(&orchestrator).await;
    let connection_id = ConnectionId::new();
    adapter.set_next_outbound_id(connection_id.clone());
    adapter.set_originate_behavior(COMMAND_TERMINAL);
    let mut events = orchestrator.subscribe_events();

    assert!(matches!(
        orchestrator
            .originate_connection(outbound_request(session_id.clone()))
            .await,
        Err(RvoipError::ConnectionNotFound(id)) if id == connection_id
    ));
    assert!(matches!(
        orchestrator.connection_transport(&connection_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));
    assert_eq!(orchestrator.session_of(&connection_id), None);
    {
        let session = orchestrator.session(&session_id).unwrap();
        let session = session.read().unwrap();
        assert!(!session.connections.contains_key(&connection_id));
        assert_eq!(session.state, rvoip_core::session::SessionState::Initiating);
    }
    assert_eq!(counts.end.load(Ordering::SeqCst), 0);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn outbound_claim_cannot_reuse_pending_inbound_id() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(2))
        .unwrap();
    let (adapter, sender, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let session_id = start_voice_session(&orchestrator).await;
    let (connection, principal) =
        prepare_atomic_inbound(&adapter, "tenant-pending", "pending-secret");
    let connection_id = connection.id.clone();
    announce_atomic_inbound(&sender, connection, principal).await;
    let admission = admissions.recv().await.unwrap();
    adapter.set_next_outbound_id(connection_id.clone());

    assert!(matches!(
        orchestrator
            .originate_connection(outbound_request(session_id))
            .await,
        Err(RvoipError::AdmissionRejected(
            "outbound connection ID is not vacant"
        ))
    ));
    assert_eq!(counts.end.load(Ordering::SeqCst), 1);
    assert_eq!(
        orchestrator.connection_transport(&connection_id).unwrap(),
        Transport::Sip
    );
    assert!(admission.authenticated_principal().is_ok());
    assert!(admission.accept().await.is_err());
    assert!(matches!(
        orchestrator.connection_transport(&connection_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));
}

#[tokio::test]
async fn outbound_activation_failure_rolls_back_and_ends_adapter_route() {
    let orchestrator = Orchestrator::new(Config::default());
    let (adapter, _, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let session_id = start_voice_session(&orchestrator).await;
    let connection_id = ConnectionId::new();
    adapter.set_next_outbound_id(connection_id.clone());
    adapter.set_activate_behavior(COMMAND_FAIL);
    let mut events = orchestrator.subscribe_events();

    assert!(matches!(
        orchestrator
            .originate_connection(outbound_request(session_id.clone()))
            .await,
        Err(RvoipError::InvalidState("stub activation failure"))
    ));
    assert_eq!(counts.end.load(Ordering::SeqCst), 1);
    assert!(matches!(
        orchestrator.connection_transport(&connection_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));
    assert_eq!(orchestrator.session_of(&connection_id), None);
    {
        let session = orchestrator.session(&session_id).unwrap();
        let session = session.read().unwrap();
        assert!(!session.connections.contains_key(&connection_id));
        assert_eq!(session.state, rvoip_core::session::SessionState::Initiating);
    }
    let mut outbound = 0;
    let mut failed = 0;
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(50), events.recv()).await {
        match event {
            Event::ConnectionOutbound {
                connection_id: id, ..
            } if id == connection_id => {
                outbound += 1;
            }
            Event::ConnectionFailed {
                connection_id: id, ..
            } if id == connection_id => {
                failed += 1;
            }
            _ => {}
        }
    }
    assert_eq!((outbound, failed), (1, 1));
}

#[tokio::test]
async fn terminal_during_outbound_activation_does_not_emit_duplicate_failure() {
    let orchestrator = Orchestrator::new(Config::default());
    let (adapter, _, _) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let session_id = start_voice_session(&orchestrator).await;
    let connection_id = ConnectionId::new();
    adapter.set_next_outbound_id(connection_id.clone());
    adapter.set_activate_behavior(COMMAND_TERMINAL);
    let mut events = orchestrator.subscribe_events();

    assert!(matches!(
        orchestrator
            .originate_connection(outbound_request(session_id))
            .await,
        Err(RvoipError::ConnectionNotFound(id)) if id == connection_id
    ));
    let mut outbound = 0;
    let mut ended = 0;
    let mut failed = 0;
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(50), events.recv()).await {
        match event {
            Event::ConnectionOutbound {
                connection_id: id, ..
            } if id == connection_id => {
                outbound += 1;
            }
            Event::ConnectionEnded {
                connection_id: id, ..
            } if id == connection_id => {
                ended += 1;
            }
            Event::ConnectionFailed {
                connection_id: id, ..
            } if id == connection_id => {
                failed += 1;
            }
            _ => {}
        }
    }
    assert_eq!((outbound, ended, failed), (1, 1, 0));
}

#[tokio::test]
async fn inbound_accept_failure_conditionally_rolls_back_session_binding() {
    let orchestrator = Orchestrator::new(Config::default());
    let (adapter, sender, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let session_id = start_voice_session(&orchestrator).await;
    let (connection, principal) =
        prepare_atomic_inbound(&adapter, "tenant-accept", "accept-secret");
    let connection_id = connection.id.clone();
    announce_atomic_inbound(&sender, connection, principal).await;
    wait_for_connection_principal(&orchestrator, &connection_id).await;
    adapter.set_accept_behavior(COMMAND_FAIL);

    assert!(matches!(
        orchestrator
            .route_inbound_connection(
                connection_id.clone(),
                InboundAction::Accept {
                    session_id: session_id.clone(),
                    participant_id: ParticipantId::new(),
                },
            )
            .await,
        Err(RvoipError::InvalidState("stub accept failure"))
    ));
    assert_eq!(orchestrator.session_of(&connection_id), None);
    let session = orchestrator.session(&session_id).unwrap();
    let session = session.read().unwrap();
    assert!(!session.connections.contains_key(&connection_id));
    assert_eq!(session.state, rvoip_core::session::SessionState::Initiating);
    assert_eq!(counts.end.load(Ordering::SeqCst), 1);
    assert!(matches!(
        orchestrator.connection_transport(&connection_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));
}

#[tokio::test]
async fn terminal_during_inbound_accept_cannot_leave_stale_binding() {
    let orchestrator = Orchestrator::new(Config::default());
    let (adapter, sender, _) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let session_id = start_voice_session(&orchestrator).await;
    let (connection, principal) =
        prepare_atomic_inbound(&adapter, "tenant-terminal", "terminal-secret");
    let connection_id = connection.id.clone();
    announce_atomic_inbound(&sender, connection, principal).await;
    wait_for_connection_principal(&orchestrator, &connection_id).await;
    adapter.set_accept_behavior(COMMAND_TERMINAL);

    assert!(matches!(
        orchestrator
            .route_inbound_connection(
                connection_id.clone(),
                InboundAction::Accept {
                    session_id: session_id.clone(),
                    participant_id: ParticipantId::new(),
                },
            )
            .await,
        Err(RvoipError::ConnectionNotFound(id)) if id == connection_id
    ));
    assert_eq!(orchestrator.session_of(&connection_id), None);
    assert!(matches!(
        orchestrator.connection_transport(&connection_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));
    assert!(!orchestrator
        .session(&session_id)
        .unwrap()
        .read()
        .unwrap()
        .connections
        .contains_key(&connection_id));
}

#[tokio::test]
async fn bridge_to_outbound_failure_preserves_accepted_inbound_binding() {
    let orchestrator = Orchestrator::new(Config::default());
    let (adapter, sender, _) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let session_id = start_voice_session(&orchestrator).await;
    let (connection, principal) =
        prepare_atomic_inbound(&adapter, "tenant-bridge", "bridge-secret");
    let connection_id = connection.id.clone();
    announce_atomic_inbound(&sender, connection, principal).await;
    wait_for_connection_principal(&orchestrator, &connection_id).await;
    adapter.set_originate_behavior(COMMAND_FAIL);

    assert!(matches!(
        orchestrator
            .route_inbound_connection(
                connection_id.clone(),
                InboundAction::BridgeTo {
                    session_id: session_id.clone(),
                    outbound: outbound_request(SessionId::new()),
                },
            )
            .await,
        Err(RvoipError::InvalidState("stub originate failure"))
    ));
    assert_eq!(
        orchestrator.session_of(&connection_id),
        Some(session_id.clone())
    );
    assert!(orchestrator
        .session(&session_id)
        .unwrap()
        .read()
        .unwrap()
        .connections
        .contains_key(&connection_id));
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

fn prepare_atomic_inbound(
    adapter: &StubAdapter,
    tenant: &str,
    routing_hint: &str,
) -> (Connection, AuthenticatedPrincipal) {
    let connection = fake_inbound_connection();
    let principal = authenticated_principal(tenant);
    adapter.mark_live(connection.id.clone());
    adapter.set_inbound_context(
        InboundConnectionContext::new(
            connection.id.clone(),
            Transport::Sip,
            &principal,
            Some(InboundRoutingHint::new(routing_hint).unwrap()),
            InboundSignalingMetadata::default(),
        )
        .unwrap(),
    );
    (connection, principal)
}

async fn announce_atomic_inbound(
    sender: &StubEventSender,
    connection: Connection,
    principal: AuthenticatedPrincipal,
) {
    sender
        .send_atomic(OrchestratorAdapterEvent::AuthenticatedInboundConnection {
            connection,
            participant_id: "admission-participant".into(),
            principal,
        })
        .await
        .unwrap();
}

async fn wait_for_count(counter: &AtomicUsize, expected: usize) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while counter.load(Ordering::SeqCst) != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("counter reached expected value");
}

async fn start_voice_session(orchestrator: &Arc<Orchestrator>) -> SessionId {
    let conversation_id = orchestrator
        .open_conversation(
            TenantId::new(),
            rvoip_core::conversation::ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .unwrap();
    orchestrator
        .start_session(
            conversation_id,
            rvoip_core::session::SessionMedium::Voice,
            vec![],
        )
        .await
        .unwrap()
}

fn outbound_request(session_id: SessionId) -> OriginateRequest {
    OriginateRequest {
        session_id,
        participant_id: ParticipantId::new(),
        target: "sip:outbound@example.invalid".into(),
        direction: Direction::Outbound,
        capabilities: CapabilityDescriptor::default(),
        transport: Some(Transport::Sip),
    }
}

#[tokio::test]
async fn admission_gate_delays_publication_and_preserves_atomic_principal_context() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(2, Duration::from_secs(1))
        .unwrap();
    let (adapter, adapter_tx, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();

    let (connection, principal) =
        prepare_atomic_inbound(&adapter, "tenant-secret", "private-routing-token");
    let connection_id = connection.id.clone();
    announce_atomic_inbound(&adapter_tx, connection.clone(), principal.clone()).await;

    let mut admission = tokio::time::timeout(Duration::from_secs(1), admissions.recv())
        .await
        .unwrap()
        .expect("admission ticket");
    assert_eq!(admission.connection_id(), &connection_id);
    assert_eq!(admission.transport(), Transport::Sip);
    assert!(
        tokio::time::timeout(Duration::from_millis(30), events.recv())
            .await
            .is_err()
    );

    let rendered = format!("{admission:?}");
    assert!(!rendered.contains("private-routing-token"));
    assert!(!rendered.contains("attachment-owner"));
    assert!(!rendered.contains("tenant-secret"));
    assert!(admission
        .authenticated_principal()
        .unwrap()
        .has_same_owner(&principal));
    assert!(matches!(
        orchestrator.take_inbound_context(&connection_id, &principal),
        Err(RvoipError::AdmissionRejected(
            "inbound context is reserved by admission policy"
        ))
    ));
    assert!(matches!(
        orchestrator.hold(connection_id.clone()).await,
        Err(RvoipError::AdmissionRejected(
            "connection is not operational"
        ))
    ));
    let context = admission
        .take_inbound_context()
        .unwrap()
        .expect("context retained atomically");
    assert_eq!(
        context.routing_hint().unwrap().expose_secret(),
        "private-routing-token"
    );
    assert!(admission.take_inbound_context().unwrap().is_none());

    adapter.set_inbound_context(
        InboundConnectionContext::new(
            connection_id.clone(),
            Transport::Sip,
            &principal,
            Some(InboundRoutingHint::new("duplicate-must-not-repopulate").unwrap()),
            InboundSignalingMetadata::default(),
        )
        .unwrap(),
    );
    announce_atomic_inbound(&adapter_tx, connection, principal.clone()).await;
    tokio::time::sleep(Duration::from_millis(20)).await;

    admission.accept().await.unwrap();
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
    assert!(orchestrator
        .take_inbound_context(&connection_id, &principal)
        .unwrap()
        .is_none());
    assert_eq!(counts.reject.load(Ordering::SeqCst), 0);
    assert_eq!(counts.hold.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn explicit_rejection_erases_context_and_publishes_no_inbound_events() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(1))
        .unwrap();
    let (adapter, adapter_tx, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();
    let (connection, principal) =
        prepare_atomic_inbound(&adapter, "tenant-a", "reject-private-token");
    let connection_id = connection.id.clone();
    announce_atomic_inbound(&adapter_tx, connection, principal).await;

    let admission = admissions.recv().await.unwrap();
    admission.reject(RejectReason::Forbidden).await.unwrap();
    wait_for_count(&counts.reject, 1).await;
    assert!(matches!(
        counts.reject_reasons.lock().unwrap().as_slice(),
        [RejectReason::Forbidden]
    ));
    assert!(matches!(
        orchestrator.connection_transport(&connection_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));
    // Even if a broken adapter reports an old route as live after successful
    // shutdown, an untracked event cannot project onto the normalized bus.
    adapter.mark_live(connection_id.clone());
    adapter_tx
        .send(AdapterEvent::Connected {
            connection_id: connection_id.clone(),
        })
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn dropped_ticket_and_decision_timeout_fail_closed() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(2, Duration::from_millis(30))
        .unwrap();
    let (adapter, adapter_tx, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();

    let (dropped, principal) = prepare_atomic_inbound(&adapter, "tenant-a", "drop-private-token");
    let dropped_id = dropped.id.clone();
    announce_atomic_inbound(&adapter_tx, dropped, principal).await;
    drop(admissions.recv().await.unwrap());
    wait_for_count(&counts.reject, 1).await;
    assert!(matches!(
        orchestrator.connection_transport(&dropped_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));

    let (timed_out, principal) =
        prepare_atomic_inbound(&adapter, "tenant-a", "timeout-private-token");
    let timed_out_id = timed_out.id.clone();
    announce_atomic_inbound(&adapter_tx, timed_out, principal).await;
    let admission = admissions.recv().await.unwrap();
    wait_for_count(&counts.reject, 2).await;
    assert!(admission.accept().await.is_err());
    assert!(matches!(
        orchestrator.connection_transport(&timed_out_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn closed_receiver_and_capacity_exhaustion_reject_without_task_growth() {
    let closed = Orchestrator::new(Config::default());
    let closed_admissions = closed
        .install_inbound_admission_gate(1, Duration::from_secs(1))
        .unwrap();
    drop(closed_admissions);
    let (closed_adapter, closed_tx, closed_counts) = StubAdapter::new();
    closed.register(closed_adapter.clone()).unwrap();
    let (connection, principal) =
        prepare_atomic_inbound(&closed_adapter, "tenant-a", "closed-receiver-token");
    let connection_id = connection.id.clone();
    announce_atomic_inbound(&closed_tx, connection, principal).await;
    wait_for_count(&closed_counts.reject, 1).await;
    assert!(matches!(
        closed.connection_transport(&connection_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));

    let bounded = Orchestrator::new(Config::default());
    let mut admissions = bounded
        .install_inbound_admission_gate(1, Duration::from_secs(1))
        .unwrap();
    let (adapter, sender, counts) = StubAdapter::new();
    bounded.register(adapter.clone()).unwrap();
    let mut events = bounded.subscribe_events();
    let (first, first_principal) =
        prepare_atomic_inbound(&adapter, "tenant-a", "first-bounded-token");
    let first_id = first.id.clone();
    announce_atomic_inbound(&sender, first, first_principal).await;
    let first_admission = admissions.recv().await.unwrap();

    let (second, second_principal) =
        prepare_atomic_inbound(&adapter, "tenant-a", "second-bounded-token");
    let second_id = second.id.clone();
    announce_atomic_inbound(&sender, second, second_principal).await;
    wait_for_count(&counts.reject, 1).await;
    assert!(matches!(
        bounded.connection_transport(&second_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), events.recv())
            .await
            .is_err()
    );

    first_admission.accept().await.unwrap();
    assert!(matches!(
        events.recv().await.unwrap(),
        Event::ConnectionInbound { connection_id, .. } if connection_id == first_id
    ));
    let _ = events.recv().await.unwrap();
    let _ = events.recv().await.unwrap();

    // The waiter permit is released after resolution, so later work is
    // admitted without creating more than one concurrent waiter.
    let (third, third_principal) =
        prepare_atomic_inbound(&adapter, "tenant-a", "third-bounded-token");
    announce_atomic_inbound(&sender, third, third_principal).await;
    let third_admission = tokio::time::timeout(Duration::from_secs(1), admissions.recv())
        .await
        .unwrap()
        .unwrap();
    third_admission
        .reject(RejectReason::Forbidden)
        .await
        .unwrap();
    wait_for_count(&counts.reject, 2).await;
}

#[tokio::test]
async fn terminal_race_invalidates_ticket_and_late_accept_cannot_resurrect() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(1))
        .unwrap();
    let (adapter, sender, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();
    let (connection, principal) =
        prepare_atomic_inbound(&adapter, "tenant-a", "terminal-race-token");
    let connection_id = connection.id.clone();
    announce_atomic_inbound(&sender, connection, principal).await;
    let admission = admissions.recv().await.unwrap();

    adapter.mark_ended(&connection_id);
    adapter
        .deliver_terminal(AdapterEvent::Ended {
            connection_id: connection_id.clone(),
            reason: EndReason::Normal,
        })
        .await;
    assert!(admission.authenticated_principal().is_err());
    assert!(admission.accept().await.is_err());
    assert_eq!(counts.reject.load(Ordering::SeqCst), 0);
    assert!(matches!(
        orchestrator.connection_transport(&connection_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));
    let mut inbound_events = 0;
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(50), events.recv()).await {
        if matches!(
            event,
            Event::ConnectionInbound { .. }
                | Event::ConnectionAuthenticated { .. }
                | Event::ConnectionPrincipalAuthenticated { .. }
        ) {
            inbound_events += 1;
        }
    }
    assert_eq!(inbound_events, 0);
}

#[tokio::test]
async fn duplicate_inbound_handoffs_create_one_ticket_and_one_publication() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(2, Duration::from_secs(1))
        .unwrap();
    let (adapter, sender, _) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();
    let (connection, principal) = prepare_atomic_inbound(&adapter, "tenant-a", "duplicate-token");
    let connection_id = connection.id.clone();
    announce_atomic_inbound(&sender, connection.clone(), principal.clone()).await;
    let admission = admissions.recv().await.unwrap();
    let mut changed_authorization = principal.clone();
    changed_authorization.scopes.clear();
    changed_authorization.expires_at = Some(Utc::now() + ChronoDuration::minutes(5));
    announce_atomic_inbound(&sender, connection.clone(), changed_authorization).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), admissions.recv())
            .await
            .is_err()
    );
    let retained = admission.authenticated_principal().unwrap();
    assert_eq!(retained.scopes, principal.scopes);
    assert_eq!(retained.expires_at, principal.expires_at);

    admission.accept().await.unwrap();
    for _ in 0..3 {
        let _ = events.recv().await.unwrap();
    }
    let mut published_duplicate = principal.clone();
    published_duplicate.scopes = vec!["must:not:replace".into()];
    published_duplicate.expires_at = Some(Utc::now() + ChronoDuration::minutes(10));
    adapter.set_inbound_context(
        InboundConnectionContext::new(
            connection_id.clone(),
            Transport::Sip,
            &published_duplicate,
            Some(InboundRoutingHint::new("duplicate-private-token").unwrap()),
            InboundSignalingMetadata::default(),
        )
        .unwrap(),
    );
    announce_atomic_inbound(&sender, connection, published_duplicate).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), admissions.recv())
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err()
    );
    assert_eq!(
        orchestrator.connection_transport(&connection_id).unwrap(),
        Transport::Sip
    );
    let retained_after_duplicate = orchestrator.connection_principal(&connection_id).unwrap();
    assert_eq!(retained_after_duplicate.scopes, principal.scopes);
    assert_eq!(retained_after_duplicate.expires_at, principal.expires_at);
    assert_eq!(
        orchestrator
            .take_inbound_context(&connection_id, &principal)
            .unwrap()
            .unwrap()
            .routing_hint()
            .unwrap()
            .expose_secret(),
        "duplicate-token"
    );
    assert!(adapter.take_inbound_context(&connection_id).is_none());
}

#[tokio::test]
async fn gated_cross_transport_connection_id_collision_rejects_only_second_adapter() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(2, Duration::from_secs(1))
        .unwrap();
    let (sip, sip_sender, sip_counts) = StubAdapter::new_for(Transport::Sip);
    let (webrtc, webrtc_sender, webrtc_counts) = StubAdapter::new_for(Transport::WebRtc);
    orchestrator.register(sip.clone()).unwrap();
    orchestrator.register(webrtc.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();

    let (connection, principal) =
        prepare_atomic_inbound(&sip, "tenant-owner", "owner-private-token");
    let connection_id = connection.id.clone();
    announce_atomic_inbound(&sip_sender, connection, principal.clone()).await;
    let mut admission = admissions.recv().await.unwrap();

    let mut collision = fake_inbound_connection_for(Transport::WebRtc);
    collision.id = connection_id.clone();
    let collision_principal = authenticated_principal("tenant-attacker");
    webrtc.mark_live(connection_id.clone());
    webrtc.set_inbound_context(
        InboundConnectionContext::new(
            connection_id.clone(),
            Transport::WebRtc,
            &collision_principal,
            Some(InboundRoutingHint::new("attacker-private-token").unwrap()),
            InboundSignalingMetadata::default(),
        )
        .unwrap(),
    );
    announce_atomic_inbound(&webrtc_sender, collision, collision_principal).await;
    wait_for_count(&webrtc_counts.reject, 1).await;

    assert_eq!(sip_counts.reject.load(Ordering::SeqCst), 0);
    assert_eq!(
        orchestrator.connection_transport(&connection_id).unwrap(),
        Transport::Sip
    );
    assert!(sip.is_connection_live(&connection_id));
    assert!(!webrtc.is_connection_live(&connection_id));
    webrtc
        .deliver_terminal(AdapterEvent::Ended {
            connection_id: connection_id.clone(),
            reason: EndReason::Normal,
        })
        .await;
    assert_eq!(
        orchestrator.connection_transport(&connection_id).unwrap(),
        Transport::Sip
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(30), admissions.recv())
            .await
            .is_err()
    );
    assert!(admission
        .authenticated_principal()
        .unwrap()
        .has_same_owner(&principal));
    assert_eq!(
        admission
            .take_inbound_context()
            .unwrap()
            .unwrap()
            .routing_hint()
            .unwrap()
            .expose_secret(),
        "owner-private-token"
    );
    admission.accept().await.unwrap();
    for _ in 0..3 {
        let _ = events.recv().await.unwrap();
    }
}

#[tokio::test]
async fn compatibility_path_rejects_cross_transport_collision_without_republication() {
    let orchestrator = Orchestrator::new(Config::default());
    let (sip, sip_sender, sip_counts) = StubAdapter::new_for(Transport::Sip);
    let (webrtc, webrtc_sender, webrtc_counts) = StubAdapter::new_for(Transport::WebRtc);
    orchestrator.register(sip.clone()).unwrap();
    orchestrator.register(webrtc.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();

    let (connection, principal) =
        prepare_atomic_inbound(&sip, "tenant-owner", "owner-private-token");
    let connection_id = connection.id.clone();
    announce_atomic_inbound(&sip_sender, connection, principal).await;
    for _ in 0..3 {
        let _ = events.recv().await.unwrap();
    }

    let mut collision = fake_inbound_connection_for(Transport::WebRtc);
    collision.id = connection_id.clone();
    let collision_principal = authenticated_principal("tenant-attacker");
    webrtc.mark_live(connection_id.clone());
    announce_atomic_inbound(&webrtc_sender, collision, collision_principal).await;
    wait_for_count(&webrtc_counts.reject, 1).await;

    assert_eq!(sip_counts.reject.load(Ordering::SeqCst), 0);
    assert_eq!(
        orchestrator.connection_transport(&connection_id).unwrap(),
        Transport::Sip
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn published_principal_refresh_replaces_authorization_and_publishes() {
    let orchestrator = Orchestrator::new(Config::default());
    let (adapter, sender, _) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();
    let (connection, principal) =
        prepare_atomic_inbound(&adapter, "tenant-refresh", "refresh-token");
    let connection_id = connection.id.clone();
    announce_atomic_inbound(&sender, connection, principal.clone()).await;
    for _ in 0..3 {
        let _ = events.recv().await.unwrap();
    }

    let mut refreshed = principal.clone();
    refreshed.scopes = vec!["call:attach".into(), "call:transfer".into()];
    refreshed.expires_at = Some(Utc::now() + ChronoDuration::minutes(15));
    sender
        .send(AdapterEvent::PrincipalAuthenticated {
            connection_id: connection_id.clone(),
            participant_id: "refreshed-participant".into(),
            principal: refreshed.clone(),
        })
        .await
        .unwrap();

    assert!(matches!(
        events.recv().await.unwrap(),
        Event::ConnectionAuthenticated { connection_id: id, .. } if id == connection_id
    ));
    assert!(matches!(
        events.recv().await.unwrap(),
        Event::ConnectionPrincipalAuthenticated {
            connection_id: id,
            principal,
            ..
        } if id == connection_id && principal.scopes == refreshed.scopes
            && principal.expires_at == refreshed.expires_at
    ));
    let retained = orchestrator.connection_principal(&connection_id).unwrap();
    assert_eq!(retained.scopes, refreshed.scopes);
    assert_eq!(retained.expires_at, refreshed.expires_at);
}

#[tokio::test]
async fn published_invalid_principal_refreshes_fail_closed() {
    let orchestrator = Orchestrator::new(Config::default());
    let (adapter, sender, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();

    for (index, invalid) in [
        {
            let mut principal = authenticated_principal("tenant-owner-mismatch");
            principal.subject = "different-subject".into();
            principal
        },
        {
            let mut principal = authenticated_principal("tenant-placeholder");
            principal.tenant = None;
            principal
        },
        {
            let mut principal = authenticated_principal("tenant-expired");
            principal.expires_at = Some(Utc::now() - ChronoDuration::seconds(1));
            principal
        },
    ]
    .into_iter()
    .enumerate()
    {
        let tenant = match index {
            0 => "tenant-owner-mismatch",
            1 => "tenant-valid",
            _ => "tenant-expired",
        };
        let (connection, principal) = prepare_atomic_inbound(&adapter, tenant, "refresh-secret");
        let connection_id = connection.id.clone();
        announce_atomic_inbound(&sender, connection, principal).await;
        for _ in 0..3 {
            let _ = events.recv().await.unwrap();
        }
        sender
            .send(AdapterEvent::PrincipalAuthenticated {
                connection_id: connection_id.clone(),
                participant_id: "invalid-refresh".into(),
                principal: invalid,
            })
            .await
            .unwrap();
        wait_for_count(&counts.reject, index + 1).await;
        assert!(matches!(
            events.recv().await.unwrap(),
            Event::ConnectionFailed { connection_id: id, .. } if id == connection_id
        ));
        assert!(matches!(
            orchestrator.connection_principal(&connection_id),
            Err(RvoipError::ConnectionNotFound(_))
        ));
    }
}

#[tokio::test]
async fn published_atomic_cross_owner_duplicate_is_rejected_and_drained() {
    let orchestrator = Orchestrator::new(Config::default());
    let (adapter, sender, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();
    let (connection, principal) = prepare_atomic_inbound(&adapter, "tenant-owner", "owner-token");
    let connection_id = connection.id.clone();
    announce_atomic_inbound(&sender, connection.clone(), principal).await;
    for _ in 0..3 {
        let _ = events.recv().await.unwrap();
    }

    let attacker = authenticated_principal("tenant-attacker");
    adapter.set_inbound_context(
        InboundConnectionContext::new(
            connection_id.clone(),
            Transport::Sip,
            &attacker,
            Some(InboundRoutingHint::new("attacker-token").unwrap()),
            InboundSignalingMetadata::default(),
        )
        .unwrap(),
    );
    announce_atomic_inbound(&sender, connection, attacker).await;
    wait_for_count(&counts.reject, 1).await;
    assert!(matches!(
        events.recv().await.unwrap(),
        Event::ConnectionFailed { connection_id: id, .. } if id == connection_id
    ));
    assert!(matches!(
        orchestrator.connection_transport(&connection_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));
    assert!(adapter.take_inbound_context(&connection_id).is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accept_racing_atomic_cross_owner_duplicate_never_preserves_route() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(2, Duration::from_secs(2))
        .unwrap();
    let (adapter, sender, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();

    for expected_rejections in 1..=16 {
        let (connection, principal) =
            prepare_atomic_inbound(&adapter, "tenant-owner", "race-owner-token");
        let connection_id = connection.id.clone();
        announce_atomic_inbound(&sender, connection.clone(), principal).await;
        let admission = admissions.recv().await.unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let accept_barrier = Arc::clone(&barrier);
        let accept = tokio::spawn(async move {
            accept_barrier.wait().await;
            admission.accept().await
        });
        let duplicate_barrier = Arc::clone(&barrier);
        let duplicate_sender = sender.clone();
        let duplicate = tokio::spawn(async move {
            duplicate_barrier.wait().await;
            duplicate_sender
                .send_atomic(OrchestratorAdapterEvent::AuthenticatedInboundConnection {
                    connection,
                    participant_id: "attacker".into(),
                    principal: authenticated_principal("tenant-attacker"),
                })
                .await
        });
        barrier.wait().await;
        let _ = accept.await.unwrap();
        duplicate.await.unwrap().unwrap();
        wait_for_count(&counts.reject, expected_rejections).await;
        assert!(matches!(
            orchestrator.connection_transport(&connection_id),
            Err(RvoipError::ConnectionNotFound(_))
        ));
    }
}

#[tokio::test]
async fn anonymous_and_mismatched_contexts_remain_fail_closed_for_policy() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(2, Duration::from_secs(1))
        .unwrap();
    let (adapter, sender, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();

    let anonymous = fake_inbound_connection();
    adapter.mark_live(anonymous.id.clone());
    sender
        .send(AdapterEvent::InboundConnection {
            connection: anonymous,
        })
        .await
        .unwrap();
    let mut admission = admissions.recv().await.unwrap();
    assert!(matches!(
        admission.authenticated_principal(),
        Err(RvoipError::InvalidState(
            "connection has no authenticated principal"
        ))
    ));
    assert!(admission.take_inbound_context().is_err());
    admission.reject(RejectReason::Forbidden).await.unwrap();

    let connection = fake_inbound_connection();
    let principal = authenticated_principal("tenant-a");
    adapter.mark_live(connection.id.clone());
    adapter.set_inbound_context(
        InboundConnectionContext::new(
            connection.id.clone(),
            Transport::WebRtc,
            &principal,
            Some(InboundRoutingHint::new("wrong-transport-token").unwrap()),
            InboundSignalingMetadata::default(),
        )
        .unwrap(),
    );
    announce_atomic_inbound(&sender, connection, principal).await;
    let mut admission = admissions.recv().await.unwrap();
    assert!(admission.authenticated_principal().is_ok());
    assert!(admission.take_inbound_context().unwrap().is_none());
    admission.reject(RejectReason::Forbidden).await.unwrap();

    wait_for_count(&counts.reject, 2).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn tenantless_principals_and_inbound_direction_mismatches_fail_closed() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(1))
        .unwrap();
    let (adapter, sender, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();

    let tenantless_connection = fake_inbound_connection();
    let tenantless_id = tenantless_connection.id.clone();
    let mut tenantless = authenticated_principal("tenant-placeholder");
    tenantless.tenant = None;
    adapter.mark_live(tenantless_id.clone());
    announce_atomic_inbound(&sender, tenantless_connection, tenantless).await;
    wait_for_count(&counts.reject, 1).await;
    assert!(matches!(
        orchestrator.connection_transport(&tenantless_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));

    let mut wrong_direction = fake_inbound_connection();
    wrong_direction.direction = Direction::Outbound;
    let wrong_direction_id = wrong_direction.id.clone();
    adapter.mark_live(wrong_direction_id.clone());
    announce_atomic_inbound(
        &sender,
        wrong_direction,
        authenticated_principal("tenant-a"),
    )
    .await;
    wait_for_count(&counts.reject, 2).await;
    assert!(matches!(
        orchestrator.connection_transport(&wrong_direction_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));

    let split = fake_inbound_connection();
    let split_id = split.id.clone();
    adapter.mark_live(split_id.clone());
    sender
        .send(AdapterEvent::InboundConnection { connection: split })
        .await
        .unwrap();
    let admission = admissions.recv().await.unwrap();
    let mut tenantless = authenticated_principal("tenant-placeholder");
    tenantless.tenant = Some(String::new());
    sender
        .send(AdapterEvent::PrincipalAuthenticated {
            connection_id: split_id.clone(),
            participant_id: "tenantless-participant".into(),
            principal: tenantless,
        })
        .await
        .unwrap();
    wait_for_count(&counts.reject, 3).await;
    assert!(admission.accept().await.is_err());
    assert!(matches!(
        orchestrator.connection_transport(&split_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), admissions.recv())
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn split_authentication_is_deferred_until_admission_and_context_stays_reserved() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(1))
        .unwrap();
    let (adapter, sender, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();

    let connection = fake_inbound_connection();
    let connection_id = connection.id.clone();
    let principal = authenticated_principal("tenant-split-auth");
    adapter.mark_live(connection_id.clone());
    adapter.set_inbound_context(
        InboundConnectionContext::new(
            connection_id.clone(),
            Transport::Sip,
            &principal,
            Some(InboundRoutingHint::new("split-auth-private-token").unwrap()),
            InboundSignalingMetadata::default(),
        )
        .unwrap(),
    );
    sender
        .send(AdapterEvent::InboundConnection { connection })
        .await
        .unwrap();
    let mut admission = admissions.recv().await.unwrap();
    assert!(matches!(
        admission.authenticated_principal(),
        Err(RvoipError::InvalidState(
            "connection has no authenticated principal"
        ))
    ));

    sender
        .send(AdapterEvent::Authenticated {
            connection_id: connection_id.clone(),
            identity_id: "legacy-private-identity".into(),
            participant_id: "legacy-private-participant".into(),
            assurance: principal.assurance.clone(),
        })
        .await
        .unwrap();
    sender
        .send(AdapterEvent::PrincipalAuthenticated {
            connection_id: connection_id.clone(),
            participant_id: "split-auth-participant".into(),
            principal: principal.clone(),
        })
        .await
        .unwrap();
    wait_for_connection_principal(&orchestrator, &connection_id).await;

    assert!(
        tokio::time::timeout(Duration::from_millis(30), events.recv())
            .await
            .is_err()
    );
    assert!(admission
        .authenticated_principal()
        .unwrap()
        .has_same_owner(&principal));
    assert!(matches!(
        orchestrator.take_inbound_context(&connection_id, &principal),
        Err(RvoipError::AdmissionRejected(
            "inbound context is reserved by admission policy"
        ))
    ));
    let context = admission
        .take_inbound_context()
        .unwrap()
        .expect("the generation-bound ticket owns the context");
    assert_eq!(
        context.routing_hint().unwrap().expose_secret(),
        "split-auth-private-token"
    );

    admission.accept().await.unwrap();
    assert!(matches!(
        events.recv().await.unwrap(),
        Event::ConnectionInbound { connection_id: id, .. } if id == connection_id
    ));
    assert!(matches!(
        events.recv().await.unwrap(),
        Event::ConnectionAuthenticated {
            connection_id: id,
            identity_id,
            participant_id,
            ..
        } if id == connection_id
            && identity_id == principal.subject
            && participant_id == "split-auth-participant"
    ));
    assert!(matches!(
        events.recv().await.unwrap(),
        Event::ConnectionPrincipalAuthenticated { connection_id: id, .. }
            if id == connection_id
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), events.recv())
            .await
            .is_err()
    );
    assert_eq!(counts.reject.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn operational_events_before_admission_fail_closed_without_publication() {
    type EventFactory = fn(ConnectionId) -> AdapterEvent;
    let cases: [(&str, EventFactory); 6] = [
        ("connected", |connection_id| AdapterEvent::Connected {
            connection_id,
        }),
        ("dtmf", |connection_id| AdapterEvent::Dtmf {
            connection_id,
            digits: "1".into(),
            duration_ms: 100,
        }),
        ("quality", |connection_id| AdapterEvent::Quality {
            connection_id,
            snapshot: QualitySnapshot::default(),
        }),
        ("message", |connection_id| AdapterEvent::Message {
            connection_id,
            text: "must-not-escape".into(),
        }),
        ("data-message", |connection_id| AdapterEvent::DataMessage {
            connection_id,
            message: DataMessage::reliable(
                "bridgefu.context.v1",
                "application/json",
                br#"{"private":"must-not-escape"}"#.to_vec(),
            ),
        }),
        ("step-up-response", |connection_id| {
            AdapterEvent::StepUpResponse {
                connection_id,
                method: "passkey".into(),
                credential: "private-step-up-credential".into(),
            }
        }),
    ];

    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(1))
        .unwrap();
    let (adapter, sender, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();

    for (index, (case, event_factory)) in cases.into_iter().enumerate() {
        let (connection, principal) =
            prepare_atomic_inbound(&adapter, "tenant-a", "operational-private-token");
        let connection_id = connection.id.clone();
        announce_atomic_inbound(&sender, connection, principal).await;
        let admission = admissions.recv().await.unwrap();

        sender
            .send(event_factory(connection_id.clone()))
            .await
            .unwrap();
        wait_for_count(&counts.reject, index + 1).await;
        assert!(admission.accept().await.is_err(), "{case} ticket survived");
        assert!(
            matches!(
                orchestrator.connection_transport(&connection_id),
                Err(RvoipError::ConnectionNotFound(_))
            ),
            "{case} route survived"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(30), events.recv())
                .await
                .is_err(),
            "{case} escaped onto the normalized event bus"
        );
    }
}

#[tokio::test]
async fn cleanup_timeouts_erase_core_state_and_quarantine_only_adapter_routes() {
    for (case, reject_behavior, end_behavior, rejection_succeeds) in [
        ("reject-timeout", CLEANUP_HANG, CLEANUP_SUCCEED, true),
        ("end-timeout", CLEANUP_FAIL, CLEANUP_HANG, false),
    ] {
        let orchestrator = Orchestrator::new(Config::default());
        let mut admissions = orchestrator
            .install_inbound_admission_gate(1, Duration::from_secs(5))
            .unwrap();
        let (adapter, sender, counts) = StubAdapter::new();
        adapter.set_cleanup_behavior(reject_behavior, end_behavior);
        orchestrator.register(adapter.clone()).unwrap();
        let mut events = orchestrator.subscribe_events();
        let (connection, principal) =
            prepare_atomic_inbound(&adapter, "tenant-a", "quarantine-private-token");
        let connection_id = connection.id.clone();
        announce_atomic_inbound(&sender, connection, principal.clone()).await;
        let admission = admissions.recv().await.unwrap();

        let rejection =
            tokio::spawn(async move { admission.reject(RejectReason::Forbidden).await });
        wait_for_count(&counts.reject, 1).await;
        sender
            .send(AdapterEvent::Connected {
                connection_id: connection_id.clone(),
            })
            .await
            .unwrap();
        sender
            .send(AdapterEvent::DataMessage {
                connection_id: connection_id.clone(),
                message: DataMessage::reliable(
                    "bridgefu.context.v1",
                    "application/json",
                    br#"{"must":"not escape"}"#.to_vec(),
                ),
            })
            .await
            .unwrap();
        assert!(matches!(
            orchestrator.hold(connection_id.clone()).await,
            Err(RvoipError::ConnectionNotFound(_))
        ));
        assert!(matches!(
            orchestrator
                .media_graph_for_connection(connection_id.clone())
                .await,
            Err(RvoipError::ConnectionNotFound(_))
        ));
        assert!(matches!(
            orchestrator.connection_principal(&connection_id),
            Err(RvoipError::ConnectionNotFound(_))
        ));
        assert!(matches!(
            orchestrator.take_inbound_context(&connection_id, &principal),
            Err(RvoipError::ConnectionNotFound(_))
        ));

        let rejection_result = tokio::time::timeout(Duration::from_secs(3), rejection)
            .await
            .expect("bounded cleanup completion")
            .unwrap();
        assert_eq!(rejection_result.is_ok(), rejection_succeeds, "{case}");
        assert_eq!(counts.end.load(Ordering::SeqCst), 1, "{case}");
        assert!(matches!(
            orchestrator.connection_transport(&connection_id),
            Err(RvoipError::ConnectionNotFound(_))
        ));
        assert_eq!(
            orchestrator.adapter_cleanup_quarantine_count(),
            usize::from(!rejection_succeeds),
            "{case}"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), events.recv())
                .await
                .is_err(),
            "{case} leaked a concurrent event"
        );

        adapter.mark_ended(&connection_id);
        adapter
            .deliver_terminal(AdapterEvent::Ended {
                connection_id: connection_id.clone(),
                reason: EndReason::Normal,
            })
            .await;
        assert!(matches!(
            orchestrator.connection_transport(&connection_id),
            Err(RvoipError::ConnectionNotFound(_))
        ));
        assert_eq!(orchestrator.adapter_cleanup_quarantine_count(), 0);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), events.recv())
                .await
                .is_err(),
            "{case} emitted a terminal lifecycle for an unpublished route"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accept_and_operational_event_race_has_one_linearized_outcome() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(2, Duration::from_secs(2))
        .unwrap();
    let (adapter, sender, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();
    let mut rejected = 0;

    for iteration in 0..32 {
        let (connection, principal) =
            prepare_atomic_inbound(&adapter, "tenant-race", "race-private-token");
        let connection_id = connection.id.clone();
        announce_atomic_inbound(&sender, connection, principal).await;
        let admission = admissions.recv().await.unwrap();
        let barrier = Arc::new(Barrier::new(3));

        let accept_barrier = Arc::clone(&barrier);
        let accept_task = tokio::spawn(async move {
            accept_barrier.wait().await;
            if iteration % 2 == 0 {
                tokio::task::yield_now().await;
            }
            admission.accept().await
        });
        let event_barrier = Arc::clone(&barrier);
        let event_sender = sender.clone();
        let event_connection_id = connection_id.clone();
        let event_task = tokio::spawn(async move {
            event_barrier.wait().await;
            if iteration % 2 != 0 {
                tokio::task::yield_now().await;
            }
            event_sender
                .send(AdapterEvent::Connected {
                    connection_id: event_connection_id,
                })
                .await
        });
        barrier.wait().await;
        let accepted = accept_task.await.unwrap().is_ok();
        event_task.await.unwrap().unwrap();

        if accepted {
            assert_eq!(
                orchestrator.connection_transport(&connection_id).unwrap(),
                Transport::Sip,
                "accepted iteration {iteration} was silently removed"
            );
            let mut inbound = false;
            let mut authenticated = false;
            let mut principal_authenticated = false;
            let mut connected = false;
            tokio::time::timeout(Duration::from_secs(1), async {
                while !(inbound && authenticated && principal_authenticated && connected) {
                    match events.recv().await.unwrap() {
                        Event::ConnectionInbound {
                            connection_id: id, ..
                        } if id == connection_id => inbound = true,
                        Event::ConnectionAuthenticated {
                            connection_id: id, ..
                        } if id == connection_id => authenticated = true,
                        Event::ConnectionPrincipalAuthenticated {
                            connection_id: id, ..
                        } if id == connection_id => principal_authenticated = true,
                        Event::ConnectionConnected {
                            connection_id: id, ..
                        } if id == connection_id => connected = true,
                        _ => {}
                    }
                }
            })
            .await
            .expect("accepted route publishes its complete lifecycle");

            adapter.mark_ended(&connection_id);
            adapter
                .deliver_terminal(AdapterEvent::Ended {
                    connection_id: connection_id.clone(),
                    reason: EndReason::Normal,
                })
                .await;
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(1), events.recv())
                    .await
                    .unwrap()
                    .unwrap(),
                Event::ConnectionEnded { connection_id: id, .. } if id == connection_id
            ));
        } else {
            rejected += 1;
            wait_for_count(&counts.reject, rejected).await;
            assert!(matches!(
                orchestrator.connection_transport(&connection_id),
                Err(RvoipError::ConnectionNotFound(_))
            ));
            assert!(
                tokio::time::timeout(Duration::from_millis(10), events.recv())
                    .await
                    .is_err(),
                "rejected iteration {iteration} leaked a normalized event"
            );
        }
    }
}

#[tokio::test]
async fn retired_connection_id_cannot_be_reused_or_revived_by_stale_timeout() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(2, Duration::from_millis(500))
        .unwrap();
    let (adapter, sender, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();

    let (old_connection, old_principal) =
        prepare_atomic_inbound(&adapter, "tenant-old", "old-private-token");
    let connection_id = old_connection.id.clone();
    announce_atomic_inbound(&sender, old_connection, old_principal).await;
    let old_admission = admissions.recv().await.unwrap();
    tokio::time::sleep(Duration::from_millis(350)).await;

    adapter.mark_ended(&connection_id);
    adapter
        .deliver_terminal(AdapterEvent::Ended {
            connection_id: connection_id.clone(),
            reason: EndReason::Normal,
        })
        .await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while orchestrator.connection_transport(&connection_id).is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    let mut new_connection = fake_inbound_connection();
    new_connection.id = connection_id.clone();
    let new_principal = authenticated_principal("tenant-new");
    adapter.mark_live(connection_id.clone());
    adapter.set_inbound_context(
        InboundConnectionContext::new(
            connection_id.clone(),
            Transport::Sip,
            &new_principal,
            Some(InboundRoutingHint::new("new-private-token").unwrap()),
            InboundSignalingMetadata::default(),
        )
        .unwrap(),
    );
    announce_atomic_inbound(&sender, new_connection, new_principal).await;
    wait_for_count(&counts.reject, 1).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), admissions.recv())
            .await
            .is_err()
    );

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(old_admission.accept().await.is_err());
    assert_eq!(counts.reject.load(Ordering::SeqCst), 1);
    assert!(matches!(
        orchestrator.connection_transport(&connection_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn pending_principal_owner_change_fails_closed() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(1))
        .unwrap();
    let (adapter, sender, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();

    let (connection, principal) =
        prepare_atomic_inbound(&adapter, "tenant-a", "principal-change-private-token");
    let connection_id = connection.id.clone();
    announce_atomic_inbound(&sender, connection, principal).await;
    let admission = admissions.recv().await.unwrap();

    sender
        .send(AdapterEvent::PrincipalAuthenticated {
            connection_id: connection_id.clone(),
            participant_id: "replacement-participant".into(),
            principal: authenticated_principal("tenant-b"),
        })
        .await
        .unwrap();
    wait_for_count(&counts.reject, 1).await;
    assert!(admission.accept().await.is_err());
    assert!(matches!(
        orchestrator.connection_transport(&connection_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn principal_expiry_is_rechecked_when_admission_accepts() {
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(2))
        .unwrap();
    let (adapter, sender, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();

    let (connection, mut principal) =
        prepare_atomic_inbound(&adapter, "tenant-a", "expiry-private-token");
    let connection_id = connection.id.clone();
    principal.expires_at = Some(Utc::now() + ChronoDuration::milliseconds(250));
    adapter.set_inbound_context(
        InboundConnectionContext::new(
            connection_id.clone(),
            Transport::Sip,
            &principal,
            Some(InboundRoutingHint::new("expiry-private-token").unwrap()),
            InboundSignalingMetadata::default(),
        )
        .unwrap(),
    );
    assert!(!principal.is_expired());
    announce_atomic_inbound(&sender, connection, principal.clone()).await;
    let admission = admissions.recv().await.unwrap();
    assert!(admission
        .authenticated_principal()
        .unwrap()
        .has_same_owner(&principal));

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(principal.is_expired());
    assert!(admission.accept().await.is_err());
    wait_for_count(&counts.reject, 1).await;
    assert!(matches!(
        orchestrator.connection_transport(&connection_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn admission_gate_configuration_is_single_use_and_pre_registration() {
    let invalid = Orchestrator::new(Config::default());
    assert!(matches!(
        invalid.install_inbound_admission_gate(0, Duration::from_secs(1)),
        Err(RvoipError::InvalidState(_))
    ));
    assert!(matches!(
        invalid.install_inbound_admission_gate(1, Duration::ZERO),
        Err(RvoipError::InvalidState(_))
    ));
    let _receiver = invalid
        .install_inbound_admission_gate(1, Duration::from_secs(1))
        .unwrap();
    assert!(matches!(
        invalid.install_inbound_admission_gate(1, Duration::from_secs(1)),
        Err(RvoipError::InvalidState(
            "inbound admission gate already installed"
        ))
    ));

    let too_late = Orchestrator::new(Config::default());
    let (adapter, _, _) = StubAdapter::new();
    too_late.register(adapter).unwrap();
    assert!(matches!(
        too_late.install_inbound_admission_gate(1, Duration::from_secs(1)),
        Err(RvoipError::InvalidState(
            "inbound admission gate must be installed before adapters"
        ))
    ));
}

#[tokio::test]
async fn admission_gate_rejects_adapters_without_fail_closed_lifecycle_capability() {
    let gated = Orchestrator::new(Config::default());
    let _admissions = gated
        .install_inbound_admission_gate(1, Duration::from_secs(1))
        .unwrap();
    let (incompatible, _, _) = StubAdapter::new_for_with_capability(Transport::Sip, false);
    assert!(matches!(
        gated.register(incompatible),
        Err(RvoipError::InvalidState(
            "adapter does not support fail-closed inbound admission"
        ))
    ));

    let compatibility_mode = Orchestrator::new(Config::default());
    let (legacy, _, _) = StubAdapter::new_for_with_capability(Transport::Sip, false);
    compatibility_mode.register(legacy).unwrap();
}

#[tokio::test]
async fn retired_connection_id_budget_saturates_fail_closed() {
    let orchestrator = Orchestrator::new(Config::default());
    assert!(matches!(
        orchestrator.configure_connection_id_budget(0),
        Err(RvoipError::InvalidState(_))
    ));
    orchestrator.configure_connection_id_budget(2).unwrap();
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(1))
        .unwrap();
    let (adapter, sender, counts) = StubAdapter::new();
    orchestrator.register(adapter.clone()).unwrap();
    let mut events = orchestrator.subscribe_events();

    for expected_rejections in 1..=2 {
        let (connection, principal) =
            prepare_atomic_inbound(&adapter, "tenant-a", "budget-private-token");
        announce_atomic_inbound(&sender, connection, principal).await;
        admissions
            .recv()
            .await
            .unwrap()
            .reject(RejectReason::Forbidden)
            .await
            .unwrap();
        wait_for_count(&counts.reject, expected_rejections).await;
    }
    assert!(matches!(
        orchestrator.configure_connection_id_budget(3),
        Err(RvoipError::InvalidState(
            "connection ID budget must be configured before first use"
        ))
    ));

    let (overflow, principal) =
        prepare_atomic_inbound(&adapter, "tenant-a", "overflow-private-token");
    let overflow_id = overflow.id.clone();
    announce_atomic_inbound(&sender, overflow, principal).await;
    wait_for_count(&counts.reject, 3).await;
    assert!(matches!(
        orchestrator.connection_transport(&overflow_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), admissions.recv())
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err()
    );
}

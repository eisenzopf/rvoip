//! Exact-generation, fail-closed coverage for the private DataMessage channel
//! available only while an inbound route awaits final admission.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, AdapterLifecycleCapabilities, AdapterLifecycleSink,
    AdapterLifecycleSinkSlot, ConnectionAdapter, ConnectionHandle, EndReason,
    OrchestratorAdapterEvent, OriginateRequest, RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, NegotiatedCodecs};
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::events::Event;
use rvoip_core::identity::{AuthenticatedPrincipal, AuthenticationMethod, IdentityAssurance};
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId};
use rvoip_core::message::Message;
use rvoip_core::operational_events::{OperationalEvent, OperationalEventKind};
use rvoip_core::stream::MediaStream;
use rvoip_core::{Config, DataMessage, Orchestrator, Result, RvoipError, StagedInboundDataPolicy};
use tokio::sync::mpsc;

const SEND_LABEL: &str = "bridgefu.private-egress.command.v1";
const RECEIVE_LABEL: &str = "bridgefu.private-egress.response.v1";

struct TestAdapter {
    receiver: Mutex<Option<mpsc::Receiver<OrchestratorAdapterEvent>>>,
    live: Mutex<HashSet<ConnectionId>>,
    sent: Mutex<Vec<(ConnectionId, DataMessage)>>,
    rejects: AtomicUsize,
    lifecycle: AdapterLifecycleSinkSlot,
}

impl TestAdapter {
    fn new() -> (Arc<Self>, mpsc::Sender<OrchestratorAdapterEvent>) {
        let (sender, receiver) = mpsc::channel(32);
        (
            Arc::new(Self {
                receiver: Mutex::new(Some(receiver)),
                live: Mutex::new(HashSet::new()),
                sent: Mutex::new(Vec::new()),
                rejects: AtomicUsize::new(0),
                lifecycle: AdapterLifecycleSinkSlot::default(),
            }),
            sender,
        )
    }

    fn add(&self, connection_id: ConnectionId) {
        self.live.lock().unwrap().insert(connection_id);
    }

    fn retire(&self, connection_id: &ConnectionId) {
        self.live.lock().unwrap().remove(connection_id);
    }
}

#[async_trait]
impl ConnectionAdapter for TestAdapter {
    fn transport(&self) -> Transport {
        Transport::Sip
    }

    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }

    fn lifecycle_capabilities(&self) -> AdapterLifecycleCapabilities {
        AdapterLifecycleCapabilities::FAIL_CLOSED_INBOUND
    }

    fn install_lifecycle_sink(&self, sink: Arc<dyn AdapterLifecycleSink>) -> Result<()> {
        self.lifecycle
            .install(sink)
            .map_err(|_| RvoipError::InvalidState("lifecycle sink already installed"))
    }

    fn is_connection_live(&self, connection_id: &ConnectionId) -> bool {
        self.live.lock().unwrap().contains(connection_id)
    }

    async fn originate(&self, _request: OriginateRequest) -> Result<ConnectionHandle> {
        Err(RvoipError::NotImplemented(
            "staged data test adapter does not originate",
        ))
    }

    async fn accept(&self, _connection_id: ConnectionId) -> Result<()> {
        Ok(())
    }

    async fn reject(&self, connection_id: ConnectionId, _reason: RejectReason) -> Result<()> {
        self.rejects.fetch_add(1, Ordering::AcqRel);
        self.retire(&connection_id);
        Ok(())
    }

    async fn end(&self, connection_id: ConnectionId, _reason: EndReason) -> Result<()> {
        self.retire(&connection_id);
        Ok(())
    }

    async fn hold(&self, _connection_id: ConnectionId) -> Result<()> {
        Ok(())
    }

    async fn resume(&self, _connection_id: ConnectionId) -> Result<()> {
        Ok(())
    }

    async fn transfer(&self, _connection_id: ConnectionId, _target: TransferTarget) -> Result<()> {
        Ok(())
    }

    async fn streams(&self, _connection_id: ConnectionId) -> Result<Vec<Arc<dyn MediaStream>>> {
        Ok(Vec::new())
    }

    async fn send_message(&self, _connection_id: ConnectionId, _message: Message) -> Result<()> {
        Ok(())
    }

    async fn send_data_message(
        &self,
        connection_id: ConnectionId,
        message: DataMessage,
    ) -> Result<()> {
        self.sent.lock().unwrap().push((connection_id, message));
        Ok(())
    }

    async fn send_dtmf(
        &self,
        _connection_id: ConnectionId,
        _digits: &str,
        _duration_ms: u32,
    ) -> Result<()> {
        Ok(())
    }

    async fn renegotiate_media(
        &self,
        _connection_id: ConnectionId,
        _capabilities: CapabilityDescriptor,
    ) -> Result<NegotiatedCodecs> {
        Ok(NegotiatedCodecs::default())
    }

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        mpsc::channel(1).1
    }

    fn subscribe_orchestrator_events(&self) -> mpsc::Receiver<OrchestratorAdapterEvent> {
        self.receiver
            .lock()
            .unwrap()
            .take()
            .expect("adapter event stream is single-consumer")
    }

    fn capabilities(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::default()
    }

    async fn verify_request_signature(
        &self,
        _connection_id: ConnectionId,
        _signature: SignatureHeaders,
    ) -> Result<IdentityAssurance> {
        Ok(IdentityAssurance::Anonymous)
    }
}

struct Harness {
    orchestrator: Arc<Orchestrator>,
    admissions: mpsc::Receiver<rvoip_core::InboundAdmission>,
    operational: Option<mpsc::Receiver<OperationalEvent>>,
    adapter: Arc<TestAdapter>,
    events: mpsc::Sender<OrchestratorAdapterEvent>,
}

impl Harness {
    fn new(decision_timeout: Duration) -> Self {
        let orchestrator = Orchestrator::new(Config::default());
        let operational = orchestrator
            .install_operational_event_stream(16)
            .expect("install operational event stream");
        let admissions = orchestrator
            .install_inbound_admission_gate(4, decision_timeout)
            .expect("install inbound admission gate");
        let (adapter, events) = TestAdapter::new();
        orchestrator
            .register(Arc::clone(&adapter) as Arc<dyn ConnectionAdapter>)
            .expect("register test adapter");
        Self {
            orchestrator,
            admissions,
            operational: Some(operational),
            adapter,
            events,
        }
    }

    async fn pending(&mut self) -> (ConnectionId, rvoip_core::InboundAdmission) {
        let connection_id = ConnectionId::new();
        self.adapter.add(connection_id.clone());
        self.events
            .send(OrchestratorAdapterEvent::AuthenticatedInboundConnection {
                connection: connection(connection_id.clone()),
                participant_id: "private-gateway".into(),
                principal: principal(),
            })
            .await
            .expect("announce authenticated inbound connection");
        let admission = tokio::time::timeout(Duration::from_secs(1), self.admissions.recv())
            .await
            .expect("admission delivery deadline")
            .expect("admission gate remains live");
        (connection_id, admission)
    }
}

fn principal() -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        subject: "private-gateway".into(),
        tenant: Some("tenant-a".into()),
        scopes: vec!["call:attach".into()],
        issuer: Some("staged-data-test".into()),
        expires_at: None,
        method: AuthenticationMethod::MutualTls,
        assurance: IdentityAssurance::Anonymous,
    }
}

fn connection(connection_id: ConnectionId) -> Connection {
    Connection {
        id: connection_id,
        session_id: SessionId::new(),
        participant_id: ParticipantId::new(),
        transport: Transport::Sip,
        direction: Direction::Inbound,
        state: ConnectionState::Connecting,
        capabilities: CapabilityDescriptor::default(),
        negotiated_codecs: NegotiatedCodecs::default(),
        streams: Vec::new(),
        messaging_enabled: true,
        transport_handle: TransportHandle(Arc::new(())),
        opened_at: Utc::now(),
        closed_at: None,
    }
}

fn policy(capacity: usize) -> StagedInboundDataPolicy {
    StagedInboundDataPolicy::new([SEND_LABEL], [RECEIVE_LABEL], capacity)
}

fn send_message(sequence: u8) -> DataMessage {
    DataMessage::reliable(SEND_LABEL, "application/json", vec![sequence])
}

fn receive_message(sequence: u8) -> DataMessage {
    DataMessage::reliable(RECEIVE_LABEL, "application/json", vec![sequence])
}

async fn wait_for_reject(adapter: &TestAdapter) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while adapter.rejects.load(Ordering::Acquire) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("adapter rejection deadline");
}

async fn next_public_data(
    public: &mut tokio::sync::broadcast::Receiver<Event>,
    connection_id: &ConnectionId,
) -> DataMessage {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Ok(Event::DataMessageReceived {
                connection_id: candidate,
                message,
                ..
            }) = public.recv().await
            {
                if &candidate == connection_id {
                    return message;
                }
            }
        }
    })
    .await
    .expect("normalized data-message deadline")
}

async fn next_operational_data(
    operational: &mut mpsc::Receiver<OperationalEvent>,
    connection_id: &ConnectionId,
) -> DataMessage {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let event = operational.recv().await.expect("operational stream live");
            if &event.connection_id == connection_id {
                if let OperationalEventKind::DataMessage { message } = event.kind {
                    return message;
                }
            }
        }
    })
    .await
    .expect("operational data-message deadline")
}

#[tokio::test]
async fn allowed_staged_data_is_duplex_private_and_normal_data_resumes_after_accept() {
    let mut harness = Harness::new(Duration::from_secs(2));
    let mut operational = harness.operational.take().unwrap();
    let mut public = harness.orchestrator.subscribe_events();
    let (connection_id, mut admission) = harness.pending().await;
    let channel = admission
        .open_staged_data_channel(policy(4))
        .expect("open exact staged channel");
    let (sender, mut receiver) = channel.split();

    assert!(matches!(
        harness
            .orchestrator
            .send_data_message(connection_id.clone(), send_message(0))
            .await,
        Err(RvoipError::AdmissionRejected(_))
    ));
    sender
        .send(send_message(1))
        .await
        .expect("reserved outbound staged message");
    let sent = harness.adapter.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, connection_id);
    assert_eq!(sent[0].1.bytes.as_ref(), &[1]);

    harness
        .events
        .send(
            AdapterEvent::DataMessage {
                connection_id: connection_id.clone(),
                message: receive_message(2),
            }
            .into(),
        )
        .await
        .expect("publish reserved inbound staged message");
    let received = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("staged receive deadline")
        .expect("staged receiver remains active");
    assert_eq!(received.bytes.as_ref(), &[2]);
    assert!(
        tokio::time::timeout(Duration::from_millis(30), public.recv())
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(30), operational.recv())
            .await
            .is_err()
    );

    admission.accept().await.expect("accept pending route");
    assert!(receiver.recv().await.is_none(), "accept revokes receiver");
    assert!(sender.send(send_message(3)).await.is_err());

    let ordinary = receive_message(4);
    harness
        .events
        .send(
            AdapterEvent::DataMessage {
                connection_id: connection_id.clone(),
                message: ordinary.clone(),
            }
            .into(),
        )
        .await
        .expect("publish post-admission data message");
    assert_eq!(
        next_public_data(&mut public, &connection_id).await,
        ordinary
    );
    assert_eq!(
        next_operational_data(&mut operational, &connection_id).await,
        ordinary
    );
}

#[tokio::test]
async fn disallowed_pre_admission_data_rejects_without_event_visibility() {
    let mut harness = Harness::new(Duration::from_secs(2));
    let mut operational = harness.operational.take().unwrap();
    let mut public = harness.orchestrator.subscribe_events();
    let (connection_id, mut admission) = harness.pending().await;
    let (_sender, _receiver) = admission
        .open_staged_data_channel(policy(2))
        .expect("open staged channel")
        .split();

    harness
        .events
        .send(
            AdapterEvent::DataMessage {
                connection_id: connection_id.clone(),
                message: DataMessage::reliable(
                    "bridgefu.context.v1",
                    "application/json",
                    b"{}".to_vec(),
                ),
            }
            .into(),
        )
        .await
        .expect("publish disallowed message");
    wait_for_reject(&harness.adapter).await;
    assert!(admission.accept().await.is_err());
    assert!(matches!(
        harness.orchestrator.connection_transport(&connection_id),
        Err(RvoipError::ConnectionNotFound(_))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), public.recv())
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(30), operational.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn staged_receive_queue_overflow_is_fail_closed() {
    let mut harness = Harness::new(Duration::from_secs(2));
    let (connection_id, mut admission) = harness.pending().await;
    let (_sender, mut receiver) = admission
        .open_staged_data_channel(policy(1))
        .expect("open staged channel")
        .split();

    for sequence in [1, 2] {
        harness
            .events
            .send(
                AdapterEvent::DataMessage {
                    connection_id: connection_id.clone(),
                    message: receive_message(sequence),
                }
                .into(),
            )
            .await
            .expect("publish staged message");
    }
    wait_for_reject(&harness.adapter).await;
    assert!(admission.accept().await.is_err());
    assert_eq!(receiver.recv().await.unwrap().bytes.as_ref(), &[1]);
    assert!(receiver.recv().await.is_none());
}

#[tokio::test]
async fn accept_reject_drop_and_timeout_revoke_exact_staged_handles() {
    for disposition in ["accept", "reject", "drop", "timeout"] {
        let timeout = if disposition == "timeout" {
            Duration::from_millis(30)
        } else {
            Duration::from_secs(2)
        };
        let mut harness = Harness::new(timeout);
        let (_connection_id, mut admission) = harness.pending().await;
        let (sender, mut receiver) = admission
            .open_staged_data_channel(policy(2))
            .expect("open staged channel")
            .split();

        match disposition {
            "accept" => admission.accept().await.expect("accept route"),
            "reject" => admission
                .reject(RejectReason::Forbidden)
                .await
                .expect("reject route"),
            "drop" => {
                drop(admission);
                wait_for_reject(&harness.adapter).await;
            }
            "timeout" => {
                tokio::time::timeout(Duration::from_secs(1), async {
                    while sender.send(send_message(9)).await.is_ok() {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .expect("staged handle timeout revocation deadline");
                assert!(admission.accept().await.is_err());
            }
            _ => unreachable!(),
        }

        assert!(
            sender.send(send_message(10)).await.is_err(),
            "{disposition}"
        );
        assert!(receiver.recv().await.is_none(), "{disposition}");
    }
}

#[tokio::test]
async fn terminal_generation_retirement_invalidates_staged_handles() {
    let mut harness = Harness::new(Duration::from_secs(2));
    let (connection_id, mut admission) = harness.pending().await;
    let (sender, mut receiver) = admission
        .open_staged_data_channel(policy(2))
        .expect("open staged channel")
        .split();
    harness.adapter.retire(&connection_id);
    harness
        .events
        .send(
            AdapterEvent::Ended {
                connection_id: connection_id.clone(),
                reason: EndReason::Cancelled,
            }
            .into(),
        )
        .await
        .expect("publish terminal event");

    tokio::time::timeout(Duration::from_secs(1), async {
        while sender.send(send_message(11)).await.is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("stale sender deadline");
    assert!(receiver.recv().await.is_none());
    assert!(admission.accept().await.is_err());
}

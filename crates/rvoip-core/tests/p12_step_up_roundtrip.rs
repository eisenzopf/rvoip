//! P12.6 — step-up auth envelope round-trip.
//!
//! Exercises the orchestrator-side of the
//! `identity.step-up-request` / `identity.step-up-response` flow
//! (CONVERSATION_PROTOCOL.md §5.8) against a stub adapter. The full
//! adapter-side envelope serialization lives in rvoip-uctp; this test
//! confirms that:
//!
//! 1. `Orchestrator::request_step_up` dispatches into
//!    `ConnectionAdapter::send_step_up_request` with the right
//!    arguments.
//! 2. `Orchestrator::request_step_up` emits
//!    `Event::IdentityStepUpRequested` after the adapter accepts.
//! 3. A `StepUpResponse` adapter event surfaces as
//!    `Event::IdentityStepUpResponseReceived` with the same
//!    `(method, credential)` pair.
//! 4. Subsequent `Orchestrator::complete_step_up` with a stub
//!    `IdentityProvider` emits `Event::IdentityAssuranceChanged`.

use async_trait::async_trait;
use chrono::Utc;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    PlaybackHandle, RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{
    CapabilityDescriptor, IdentityAssuranceRequirement, NegotiatedCodecs,
};
use rvoip_core::commands::{AudioSource, MuteDirection};
use rvoip_core::config::Config;
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::error::{Result as RvResult, RvoipError};
use rvoip_core::events::Event;
use rvoip_core::identity::{
    Credential, Device, Identity, IdentityAssurance, IdentityProvider, ReachabilityChange,
    ReachabilityHint,
};
use rvoip_core::ids::{ConnectionId, IdentityId, ParticipantId, SessionId};
use rvoip_core::message::Message;
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::stream::MediaStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast::Receiver, mpsc};

#[derive(Default)]
struct StepUpCounts {
    requests: AtomicUsize,
    last_required: Mutex<Option<IdentityAssuranceRequirement>>,
}

struct StepUpAdapter {
    inbound: Mutex<Option<mpsc::Receiver<AdapterEvent>>>,
    counts: Arc<StepUpCounts>,
}

impl StepUpAdapter {
    fn new() -> (Arc<Self>, mpsc::Sender<AdapterEvent>, Arc<StepUpCounts>) {
        let (tx, rx) = mpsc::channel(16);
        let counts = Arc::new(StepUpCounts::default());
        (
            Arc::new(Self {
                inbound: Mutex::new(Some(rx)),
                counts: counts.clone(),
            }),
            tx,
            counts,
        )
    }
}

#[async_trait]
impl ConnectionAdapter for StepUpAdapter {
    fn transport(&self) -> Transport {
        // Quic is a UCTP substrate — the closest analogue to what
        // would carry a real step-up envelope on the wire.
        Transport::Quic
    }
    fn kind(&self) -> AdapterKind {
        AdapterKind::Substrate
    }
    async fn originate(&self, request: OriginateRequest) -> RvResult<ConnectionHandle> {
        Ok(ConnectionHandle {
            connection: Connection {
                id: ConnectionId::new(),
                session_id: request.session_id,
                participant_id: request.participant_id,
                transport: Transport::Quic,
                direction: Direction::Outbound,
                state: ConnectionState::Connecting,
                capabilities: request.capabilities,
                negotiated_codecs: NegotiatedCodecs::default(),
                streams: vec![],
                messaging_enabled: false,
                transport_handle: TransportHandle(Arc::new(())),
                opened_at: Utc::now(),
                closed_at: None,
            },
        })
    }
    async fn accept(&self, _c: ConnectionId) -> RvResult<()> {
        Ok(())
    }
    async fn reject(&self, _c: ConnectionId, _r: RejectReason) -> RvResult<()> {
        Ok(())
    }
    async fn end(&self, _c: ConnectionId, _r: EndReason) -> RvResult<()> {
        Ok(())
    }
    async fn hold(&self, _c: ConnectionId) -> RvResult<()> {
        Ok(())
    }
    async fn resume(&self, _c: ConnectionId) -> RvResult<()> {
        Ok(())
    }
    async fn transfer(&self, _c: ConnectionId, _t: TransferTarget) -> RvResult<()> {
        Ok(())
    }
    async fn streams(&self, _c: ConnectionId) -> RvResult<Vec<Arc<dyn MediaStream>>> {
        Ok(vec![])
    }
    async fn send_message(&self, _c: ConnectionId, _m: Message) -> RvResult<()> {
        Ok(())
    }
    async fn send_dtmf(&self, _c: ConnectionId, _d: &str, _ms: u32) -> RvResult<()> {
        Ok(())
    }
    async fn renegotiate_media(
        &self,
        _c: ConnectionId,
        _caps: CapabilityDescriptor,
    ) -> RvResult<NegotiatedCodecs> {
        Ok(NegotiatedCodecs::default())
    }
    async fn mute(&self, _c: ConnectionId, _d: MuteDirection) -> RvResult<()> {
        Ok(())
    }
    async fn unmute(&self, _c: ConnectionId, _d: MuteDirection) -> RvResult<()> {
        Ok(())
    }
    async fn play_audio(
        &self,
        _c: ConnectionId,
        _s: AudioSource,
    ) -> RvResult<PlaybackHandle> {
        Err(RvoipError::NotImplemented("play_audio"))
    }
    async fn send_step_up_request(
        &self,
        _c: ConnectionId,
        required: IdentityAssuranceRequirement,
        _allowed_methods: Vec<String>,
        _reason: Option<String>,
    ) -> RvResult<()> {
        self.counts.requests.fetch_add(1, Ordering::SeqCst);
        *self.counts.last_required.lock().unwrap() = Some(required);
        Ok(())
    }
    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        self.inbound.lock().unwrap().take().expect("once")
    }
    fn capabilities(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::default()
    }
    async fn verify_request_signature(
        &self,
        _c: ConnectionId,
        _s: SignatureHeaders,
    ) -> RvResult<IdentityAssurance> {
        Ok(IdentityAssurance::Anonymous)
    }
}

// Minimal IdentityProvider that accepts any credential and returns
// `Identified`. Used by the P12.6 round-trip's `complete_step_up`
// step. Most methods are NotImplemented — only `authenticate` is
// exercised by this test.
struct AcceptingProvider;

#[async_trait]
impl IdentityProvider for AcceptingProvider {
    async fn resolve(&self, _id_ref: &str) -> RvResult<Identity> {
        Err(RvoipError::NotImplemented("resolve"))
    }
    async fn devices(&self, _id: IdentityId) -> RvResult<Vec<Device>> {
        Ok(vec![])
    }
    async fn reachable_via(&self, _id: IdentityId) -> RvResult<Vec<ReachabilityHint>> {
        Ok(vec![])
    }
    async fn authenticate(
        &self,
        _credential: Credential,
    ) -> RvResult<(IdentityId, IdentityAssurance)> {
        Ok((
            IdentityId::from_string("id_p12_test"),
            IdentityAssurance::Identified {
                credential_kind: rvoip_core::identity::CredentialKind::OAuth2Dpop,
            },
        ))
    }
    async fn assurance_level(&self, _id: IdentityId) -> RvResult<IdentityAssurance> {
        Ok(IdentityAssurance::Anonymous)
    }
    fn subscribe_reachability(&self) -> mpsc::Receiver<ReachabilityChange> {
        let (_tx, rx) = mpsc::channel(1);
        rx
    }
}

async fn next_event(rx: &mut Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("event channel timeout")
        .expect("event channel closed")
}

async fn track_inbound(tx: &mpsc::Sender<AdapterEvent>, conn: &ConnectionId) {
    tx.send(AdapterEvent::InboundConnection {
        connection: Connection {
            id: conn.clone(),
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            transport: Transport::Quic,
            direction: Direction::Inbound,
            state: ConnectionState::Connecting,
            capabilities: CapabilityDescriptor::default(),
            negotiated_codecs: NegotiatedCodecs::default(),
            streams: vec![],
            messaging_enabled: false,
            transport_handle: TransportHandle(Arc::new(())),
            opened_at: Utc::now(),
            closed_at: None,
        },
    })
    .await
    .expect("inbound");
    // Yield to let the orchestrator's adapter-event pump observe.
    tokio::time::sleep(Duration::from_millis(30)).await;
}

#[tokio::test]
async fn request_step_up_dispatches_and_emits_event() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, tx, counts) = StepUpAdapter::new();
    orch.register(adapter).expect("register");

    let conn = ConnectionId::new();
    track_inbound(&tx, &conn).await;

    let mut events = orch.subscribe_events();

    orch.request_step_up(conn.clone(), IdentityAssuranceRequirement::UserAuthorized)
        .await
        .expect("request_step_up");

    // Adapter saw exactly one call with the right required level.
    assert_eq!(counts.requests.load(Ordering::SeqCst), 1);
    assert_eq!(
        *counts.last_required.lock().unwrap(),
        Some(IdentityAssuranceRequirement::UserAuthorized)
    );

    // Drain prefix events; the request-emit follows the inbound /
    // connected events from track_inbound.
    let mut saw_request = false;
    for _ in 0..6 {
        match next_event(&mut events).await {
            Event::IdentityStepUpRequested {
                connection_id,
                required,
                ..
            } => {
                assert_eq!(connection_id, conn);
                assert_eq!(required, IdentityAssuranceRequirement::UserAuthorized);
                saw_request = true;
                break;
            }
            _ => continue,
        }
    }
    assert!(saw_request, "IdentityStepUpRequested not emitted");
}

#[tokio::test]
async fn step_up_response_event_surfaces_to_consumer() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, tx, _counts) = StepUpAdapter::new();
    orch.register(adapter).expect("register");

    let conn = ConnectionId::new();
    track_inbound(&tx, &conn).await;

    let mut events = orch.subscribe_events();

    // Peer's response arrives as an AdapterEvent.
    tx.send(AdapterEvent::StepUpResponse {
        connection_id: conn.clone(),
        method: "passkey".into(),
        credential: "challenge-response-bytes-base64url".into(),
    })
    .await
    .expect("inject response");

    // Find the IdentityStepUpResponseReceived event.
    let mut saw_response = false;
    for _ in 0..6 {
        match next_event(&mut events).await {
            Event::IdentityStepUpResponseReceived {
                connection_id,
                method,
                credential,
                ..
            } => {
                assert_eq!(connection_id, conn);
                assert_eq!(method, "passkey");
                assert_eq!(credential, "challenge-response-bytes-base64url");
                saw_response = true;
                break;
            }
            _ => continue,
        }
    }
    assert!(saw_response, "IdentityStepUpResponseReceived not emitted");
}

#[tokio::test]
async fn complete_step_up_emits_assurance_changed() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, tx, _counts) = StepUpAdapter::new();
    orch.register(adapter).expect("register");

    let conn = ConnectionId::new();
    track_inbound(&tx, &conn).await;

    let mut events = orch.subscribe_events();

    let provider: Arc<dyn IdentityProvider> = Arc::new(AcceptingProvider);
    let assurance = orch
        .complete_step_up(
            conn.clone(),
            Credential::Bearer("test-token".into()),
            provider,
        )
        .await
        .expect("complete_step_up");

    assert!(matches!(
        assurance,
        IdentityAssurance::Identified { .. }
    ));

    // Find IdentityAssuranceChanged.
    let mut saw = false;
    for _ in 0..6 {
        match next_event(&mut events).await {
            Event::IdentityAssuranceChanged {
                connection_id,
                identity_id,
                ..
            } => {
                assert_eq!(connection_id, conn);
                assert_eq!(
                    identity_id.as_ref().map(|i| i.to_string()),
                    Some("id_p12_test".to_string())
                );
                saw = true;
                break;
            }
            _ => continue,
        }
    }
    assert!(saw, "IdentityAssuranceChanged not emitted");
}

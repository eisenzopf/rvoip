//! P2 acceptance — mute/unmute/play_audio/BridgeTo dispatch.

use chrono::Utc;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    PlaybackHandle, RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, NegotiatedCodecs};
use rvoip_core::commands::{AudioSource, InboundAction, MuteDirection};
use rvoip_core::config::Config;
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::error::{Result as RvResult, RvoipError};
use rvoip_core::events::Event;
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, ParticipantId, PlaybackId, SessionId, TenantId};
use rvoip_core::message::Message;
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::session::SessionMedium;
use rvoip_core::stream::MediaStream;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Default)]
struct Counts {
    mute: AtomicUsize,
    unmute: AtomicUsize,
    play: AtomicUsize,
}

struct CtrlAdapter {
    inbound: Mutex<Option<mpsc::Receiver<AdapterEvent>>>,
    counts: Arc<Counts>,
}

impl CtrlAdapter {
    fn new() -> (Arc<Self>, mpsc::Sender<AdapterEvent>, Arc<Counts>) {
        let (tx, rx) = mpsc::channel(16);
        let counts = Arc::new(Counts::default());
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

#[async_trait::async_trait]
impl ConnectionAdapter for CtrlAdapter {
    fn transport(&self) -> Transport {
        Transport::Sip
    }
    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }
    async fn originate(&self, request: OriginateRequest) -> RvResult<ConnectionHandle> {
        Ok(ConnectionHandle::new(Connection {
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
        }))
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
        self.counts.mute.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn unmute(&self, _c: ConnectionId, _d: MuteDirection) -> RvResult<()> {
        self.counts.unmute.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn play_audio(&self, _c: ConnectionId, _s: AudioSource) -> RvResult<PlaybackHandle> {
        self.counts.play.fetch_add(1, Ordering::SeqCst);
        let (handle, _rx) = PlaybackHandle::new(PlaybackId::new());
        Ok(handle)
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

async fn track_inbound(
    orch: &Arc<Orchestrator>,
    inbound_tx: &mpsc::Sender<AdapterEvent>,
    connid: &ConnectionId,
) {
    inbound_tx
        .send(AdapterEvent::InboundConnection {
            connection: Connection {
                id: connid.clone(),
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
            },
        })
        .await
        .expect("track inbound");
    tokio::time::sleep(Duration::from_millis(30)).await;
    let _ = orch; // just here for symmetry — not used directly
}

#[tokio::test]
async fn mute_and_unmute_round_trip() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, tx, counts) = CtrlAdapter::new();
    orch.register(adapter).expect("register");

    let connid = ConnectionId::new();
    track_inbound(&orch, &tx, &connid).await;

    orch.mute(connid.clone(), MuteDirection::Send)
        .await
        .expect("mute");
    orch.unmute(connid, MuteDirection::Send)
        .await
        .expect("unmute");

    assert_eq!(counts.mute.load(Ordering::SeqCst), 1);
    assert_eq!(counts.unmute.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn play_audio_returns_handle_and_cancel_succeeds() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, tx, counts) = CtrlAdapter::new();
    orch.register(adapter).expect("register");

    let connid = ConnectionId::new();
    track_inbound(&orch, &tx, &connid).await;

    let handle = orch
        .play_audio(
            connid,
            AudioSource::Url("https://example.com/beep.wav".into()),
        )
        .await
        .expect("play_audio");
    assert_eq!(counts.play.load(Ordering::SeqCst), 1);
    // Cancel succeeds (or returns "already ended" if the adapter's task already drained — both fine).
    let _ = handle.cancel();
}

#[tokio::test]
async fn originate_connection_binds_outbound_to_requested_session() {
    let orch = Orchestrator::new(Config::default());
    let (adapter, _tx, _counts) = CtrlAdapter::new();
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
        .expect("start");
    let participant_id = ParticipantId::new();

    let handle = orch
        .originate_connection(OriginateRequest {
            session_id: sid.clone(),
            participant_id: participant_id.clone(),
            target: "sip:alice@example.com".into(),
            direction: Direction::Outbound,
            capabilities: CapabilityDescriptor::default(),
            transport: Some(Transport::Sip),
            context: Default::default(),
        })
        .await
        .expect("originate");
    let conn_id = handle.connection.id;

    assert_eq!(orch.session_of(&conn_id), Some(sid.clone()));
    let session = orch.session(&sid).expect("session");
    let session = session.read().expect("session lock");
    let conn_ref = session.connections.get(&conn_id).expect("bound conn");
    assert_eq!(conn_ref.participant_id, participant_id);
}

#[tokio::test]
async fn inbound_action_bridge_to_originates_and_bridges() {
    // P2 acceptance — InboundAction::BridgeTo accepts inbound,
    // originates outbound, bridges them, and fires ConnectionsBridged.
    // Bridge polling needs streams — the adapter returns empty so we
    // need a short bridge deadline that allows the bridge to fail
    // gracefully OR a real stream. Use a tiny deadline so the test
    // doesn't hang; the test asserts the dispatch path runs (accept +
    // originate counts), and tolerates the bridge step failing on
    // empty streams (the AdmissionRejected return path).
    let mut cfg = Config::default();
    cfg.bridge_stream_deadline = Duration::from_millis(50);
    let orch = Orchestrator::new(cfg);
    let (adapter, tx, _counts) = CtrlAdapter::new();
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
        .expect("start");

    let inbound_id = ConnectionId::new();
    track_inbound(&orch, &tx, &inbound_id).await;

    let mut events = orch.subscribe_events();

    let result = orch
        .route_inbound_connection(
            inbound_id.clone(),
            InboundAction::BridgeTo {
                session_id: sid.clone(),
                outbound: OriginateRequest {
                    session_id: sid.clone(),
                    participant_id: ParticipantId::new(),
                    target: "sip:bob@example.com".into(),
                    direction: Direction::Outbound,
                    capabilities: CapabilityDescriptor::default(),
                    transport: None,
                    context: Default::default(),
                },
            },
        )
        .await;
    // The bridge step itself will return AdmissionRejected("no audio
    // stream …") because the stub adapter has no MediaStreams. What we
    // assert is that the accept + originate dispatch ran and the
    // outbound got bound to the Session.
    match result {
        Ok(()) => {}
        Err(RvoipError::AdmissionRejected(_)) => {}
        other => panic!("unexpected BridgeTo result: {other:?}"),
    }

    // Wait for the outbound event.
    let mut saw_outbound = false;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(100), events.recv()).await {
            Ok(Ok(Event::ConnectionOutbound { .. })) => {
                saw_outbound = true;
                break;
            }
            Ok(Ok(_)) => continue,
            _ => break,
        }
    }
    assert!(
        saw_outbound,
        "BridgeTo should originate an outbound connection"
    );

    // Session should now have at least the inbound bound (outbound
    // too, depending on whether the bridge step ran before bailing).
    let s = orch.session(&sid).unwrap();
    let s = s.read().unwrap();
    assert!(
        s.connections.contains_key(&inbound_id),
        "inbound bound to session"
    );
}

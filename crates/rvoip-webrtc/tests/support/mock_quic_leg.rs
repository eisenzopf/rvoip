//! Stand-in QUIC leg for cross-transport bridge demos/tests.
//!
//! Represents the "SIP/QUIC/other adapter" side without standing up a full
//! UCTP stack — frames injected on the mock's external side appear on the
//! bridged WebRTC leg and vice versa.

use std::sync::Mutex as StdMutex;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use dashmap::DashMap;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, NegotiatedCodecs};
use rvoip_core::connection::{
    Connection, ConnectionState, Direction, Transport, TransportHandle,
};
use rvoip_core::error::{Result, RvoipError};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, StreamId};
use rvoip_core::message::Message;
use rvoip_core::stream::{MediaFrame, MediaStream, QualitySnapshot, StreamKind};
use tokio::sync::mpsc;

pub struct MockMediaStream {
    id: StreamId,
    codec: rvoip_core::capability::CodecInfo,
    external_in_tx: mpsc::Sender<MediaFrame>,
    in_rx: StdMutex<Option<mpsc::Receiver<MediaFrame>>>,
    out_tx: mpsc::Sender<MediaFrame>,
    external_out_rx: StdMutex<Option<mpsc::Receiver<MediaFrame>>>,
}

impl MockMediaStream {
    pub fn new(codec_name: &str) -> std::sync::Arc<Self> {
        let (external_in_tx, in_rx) = mpsc::channel::<MediaFrame>(64);
        let (out_tx, external_out_rx) = mpsc::channel::<MediaFrame>(64);
        std::sync::Arc::new(Self {
            id: StreamId::new(),
            codec: rvoip_core::capability::CodecInfo {
                name: codec_name.into(),
                clock_rate_hz: 48_000,
                channels: 2,
                fmtp: None,
            },
            external_in_tx,
            in_rx: StdMutex::new(Some(in_rx)),
            out_tx,
            external_out_rx: StdMutex::new(Some(external_out_rx)),
        })
    }

    pub async fn inject(&self, frame: MediaFrame) {
        let _ = self.external_in_tx.send(frame).await;
    }

    pub fn take_external_out(&self) -> mpsc::Receiver<MediaFrame> {
        self.external_out_rx
            .lock()
            .unwrap()
            .take()
            .expect("first take_external_out")
    }

    pub fn id(&self) -> StreamId {
        self.id.clone()
    }
}

#[async_trait]
impl MediaStream for MockMediaStream {
    fn id(&self) -> StreamId {
        self.id.clone()
    }
    fn kind(&self) -> StreamKind {
        StreamKind::Audio
    }
    fn codec(&self) -> rvoip_core::capability::CodecInfo {
        self.codec.clone()
    }
    fn direction(&self) -> Direction {
        Direction::Inbound
    }
    fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
        self.in_rx
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| mpsc::channel(1).1)
    }
    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.out_tx.clone()
    }
    fn quality_snapshot(&self) -> QualitySnapshot {
        QualitySnapshot::default()
    }
    async fn close(self: std::sync::Arc<Self>) -> Result<()> {
        Ok(())
    }
}

pub struct MockQuicLeg {
    streams: DashMap<ConnectionId, std::sync::Arc<MockMediaStream>>,
    events_tx: mpsc::Sender<AdapterEvent>,
    events_rx: StdMutex<Option<mpsc::Receiver<AdapterEvent>>>,
}

impl MockQuicLeg {
    pub fn new() -> std::sync::Arc<Self> {
        let (events_tx, events_rx) = mpsc::channel(64);
        std::sync::Arc::new(Self {
            streams: DashMap::new(),
            events_tx,
            events_rx: StdMutex::new(Some(events_rx)),
        })
    }

    fn register_stream(&self, id: ConnectionId, stream: std::sync::Arc<MockMediaStream>) {
        self.streams.insert(id, stream);
    }

    /// Stand up a synthetic inbound QUIC connection with an Opus audio stream.
    pub async fn provision_inbound(
        self: &std::sync::Arc<Self>,
        session_id: SessionId,
        codec: &str,
    ) -> (ConnectionId, std::sync::Arc<MockMediaStream>) {
        let id = ConnectionId::new();
        let stream = MockMediaStream::new(codec);
        self.register_stream(id.clone(), stream.clone());

        let conn = Connection {
            id: id.clone(),
            session_id,
            participant_id: ParticipantId::new(),
            transport: Transport::Quic,
            direction: Direction::Inbound,
            state: ConnectionState::Connecting,
            capabilities: CapabilityDescriptor::default(),
            negotiated_codecs: NegotiatedCodecs::default(),
            streams: vec![],
            messaging_enabled: false,
            transport_handle: TransportHandle(std::sync::Arc::new(())),
            opened_at: Utc::now(),
            closed_at: None,
        };
        let _ = self
            .events_tx
            .send(AdapterEvent::InboundConnection { connection: conn })
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        (id, stream)
    }
}

#[async_trait]
impl ConnectionAdapter for MockQuicLeg {
    fn transport(&self) -> Transport {
        Transport::Quic
    }
    fn kind(&self) -> AdapterKind {
        AdapterKind::Substrate
    }

    async fn originate(&self, _request: OriginateRequest) -> Result<ConnectionHandle> {
        Err(RvoipError::NotImplemented("mock quic leg"))
    }
    async fn accept(&self, _conn: ConnectionId) -> Result<()> {
        Ok(())
    }
    async fn reject(&self, _conn: ConnectionId, _reason: RejectReason) -> Result<()> {
        Ok(())
    }
    async fn end(&self, _conn: ConnectionId, _reason: EndReason) -> Result<()> {
        Ok(())
    }
    async fn hold(&self, _conn: ConnectionId) -> Result<()> {
        Ok(())
    }
    async fn resume(&self, _conn: ConnectionId) -> Result<()> {
        Ok(())
    }
    async fn transfer(&self, _conn: ConnectionId, _target: TransferTarget) -> Result<()> {
        Ok(())
    }
    async fn streams(&self, conn: ConnectionId) -> Result<Vec<std::sync::Arc<dyn MediaStream>>> {
        Ok(self
            .streams
            .get(&conn)
            .map(|s| vec![s.clone() as std::sync::Arc<dyn MediaStream>])
            .unwrap_or_default())
    }
    async fn send_message(&self, _conn: ConnectionId, _message: Message) -> Result<()> {
        Ok(())
    }
    async fn send_dtmf(&self, _conn: ConnectionId, _digits: &str, _ms: u32) -> Result<()> {
        Ok(())
    }
    async fn renegotiate_media(
        &self,
        _conn: ConnectionId,
        _caps: CapabilityDescriptor,
    ) -> Result<NegotiatedCodecs> {
        Ok(NegotiatedCodecs::default())
    }
    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        self.events_rx
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| mpsc::channel(1).1)
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

pub fn mk_frame(stream_id: StreamId, byte: u8) -> MediaFrame {
    MediaFrame {
        stream_id,
        kind: StreamKind::Audio,
        payload: Bytes::from(vec![byte]),
        timestamp_rtp: byte as u32,
        captured_at: Utc::now(),
    }
}

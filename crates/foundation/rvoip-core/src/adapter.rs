use crate::capability::{CapabilityDescriptor, NegotiatedCodecs};
use crate::commands::{AudioSource, MuteDirection};
use crate::connection::Transport;
use crate::error::{Result, RvoipError};
use crate::identity::IdentityAssurance;
use crate::ids::ConnectionId;
use crate::message::Message;
use crate::stream::MediaStream;
use std::sync::Arc;
use tokio::sync::mpsc;

pub use rvoip_core_traits::adapter::{
    AdapterEvent, AdapterKind, ConnectionHandle, EndReason, OriginateRequest, PlaybackHandle,
    RejectReason, SignatureHeaders, TransferTarget,
};

/// The cross-transport adapter contract. Every transport-specific crate
/// (rvoip-sip, rvoip-webrtc, rvoip-quic, rvoip-webtransport, rvoip-websocket)
/// implements this so the [`crate::Orchestrator`] can dispatch generically.
#[async_trait::async_trait]
pub trait ConnectionAdapter: Send + Sync {
    fn transport(&self) -> Transport;
    fn kind(&self) -> AdapterKind;

    async fn originate(&self, request: OriginateRequest) -> Result<ConnectionHandle>;
    async fn accept(&self, conn: ConnectionId) -> Result<()>;
    async fn reject(&self, conn: ConnectionId, reason: RejectReason) -> Result<()>;
    async fn end(&self, conn: ConnectionId, reason: EndReason) -> Result<()>;
    async fn hold(&self, conn: ConnectionId) -> Result<()>;
    async fn resume(&self, conn: ConnectionId) -> Result<()>;
    async fn transfer(&self, conn: ConnectionId, target: TransferTarget) -> Result<()>;

    async fn streams(&self, conn: ConnectionId) -> Result<Vec<Arc<dyn MediaStream>>>;

    /// Allocate a fresh per-`(subscriber, publisher_strm)` MediaStream for
    /// the multi-party fanout path (plan §12 MP3c / G4). Required so a
    /// subscriber in an N-party room can demultiplex datagrams from
    /// multiple upstream publishers via distinct `stream_local_id`s on
    /// the wire — without this, all publishers land on the subscriber's
    /// default stream and the audio mixes at the jitter buffer.
    ///
    /// The default implementation returns
    /// [`RvoipError::NotImplemented`] so non-UCTP adapters (SIP,
    /// WebRTC) — which don't carry multi-party fanout responsibility —
    /// can stay unchanged. UCTP-family adapters override this to:
    /// 1. Allocate a fresh `stream_local_id` on the subscriber's
    ///    substrate connection.
    /// 2. Construct a directional `MediaStream` with that id.
    /// 3. Register it in the per-peer streams map so subsequent
    ///    [`Self::streams`] calls return it and inbound datagrams on
    ///    that id route correctly (subscribers may publish back).
    /// 4. Emit a `stream.opened` envelope to the peer announcing the
    ///    new id per CONVERSATION_PROTOCOL.md §10.1 multi-party note.
    ///
    /// `Orchestrator::fanout_frame` falls back to the legacy
    /// pick-by-kind behavior when this returns `NotImplemented`, so
    /// single-publisher rooms keep working everywhere.
    async fn allocate_subscriber_stream(
        &self,
        _subscriber: ConnectionId,
        _kind: crate::stream::StreamKind,
        _codec: crate::capability::CodecInfo,
    ) -> Result<Arc<dyn MediaStream>> {
        Err(RvoipError::NotImplemented(
            "ConnectionAdapter::allocate_subscriber_stream",
        ))
    }

    async fn send_message(&self, conn: ConnectionId, message: Message) -> Result<()>;
    async fn send_dtmf(&self, conn: ConnectionId, digits: &str, duration_ms: u32) -> Result<()>;
    async fn renegotiate_media(
        &self,
        conn: ConnectionId,
        capabilities: CapabilityDescriptor,
    ) -> Result<NegotiatedCodecs>;

    /// P2 — local mute/unmute on a per-direction basis. Default
    /// `NotImplemented` so adapters opt in; the Orchestrator surfaces
    /// the error verbatim when a caller invokes mute against a
    /// transport that hasn't wired it.
    async fn mute(&self, _conn: ConnectionId, _direction: MuteDirection) -> Result<()> {
        Err(RvoipError::NotImplemented("ConnectionAdapter::mute"))
    }
    async fn unmute(&self, _conn: ConnectionId, _direction: MuteDirection) -> Result<()> {
        Err(RvoipError::NotImplemented("ConnectionAdapter::unmute"))
    }

    /// P2 — play `source` toward the peer on `conn`. Adapters that
    /// implement this construct a [`PlaybackHandle`] via
    /// [`PlaybackHandle::new`], spawn the playback task watching the
    /// returned `cancel_rx`, and return the handle. Default
    /// `NotImplemented`.
    async fn play_audio(
        &self,
        _conn: ConnectionId,
        _source: AudioSource,
    ) -> Result<PlaybackHandle> {
        Err(RvoipError::NotImplemented("ConnectionAdapter::play_audio"))
    }

    /// P12.6 — send an `identity.step-up-request` envelope to the peer
    /// asking them to present higher-assurance credentials. The peer's
    /// `identity.step-up-response` arrives as
    /// [`AdapterEvent::StepUpResponse`] which the orchestrator
    /// re-emits as [`crate::events::Event::IdentityStepUpResponseReceived`].
    /// UCTP-family adapters override this; SIP / WebRTC default to
    /// `NotImplemented` since step-up is a UCTP-native flow per
    /// CONVERSATION_PROTOCOL.md §5.8.
    async fn send_step_up_request(
        &self,
        _conn: ConnectionId,
        _required: crate::capability::IdentityAssuranceRequirement,
        _allowed_methods: Vec<String>,
        _reason: Option<String>,
    ) -> Result<()> {
        Err(RvoipError::NotImplemented(
            "ConnectionAdapter::send_step_up_request",
        ))
    }

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent>;
    fn capabilities(&self) -> CapabilityDescriptor;

    async fn verify_request_signature(
        &self,
        conn: ConnectionId,
        signature: SignatureHeaders,
    ) -> Result<IdentityAssurance>;
}

use crate::capability::CodecInfo;
use crate::connection::Direction;
use crate::data::DataMessage;
use crate::error::Result;
use crate::ids::{ConnectionId, StreamId};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Provisional ownership of a stream's single-consumer inbound receiver.
///
/// Dropping an uncommitted reservation restores the receiver to its stream.
/// This lets an orchestrator reserve every source needed by a multi-leg media
/// operation before transferring any one source into a graph. Committing is
/// the explicit point at which rollback is disabled and the caller becomes
/// the receiver's permanent owner.
#[must_use = "dropping an uncommitted media receiver reservation restores it"]
pub struct MediaReceiverReservation {
    receiver: Option<mpsc::Receiver<MediaFrame>>,
    restore: Option<Box<dyn FnOnce(mpsc::Receiver<MediaFrame>) + Send + 'static>>,
    on_commit: Option<Box<dyn FnOnce() + Send + 'static>>,
}

impl MediaReceiverReservation {
    /// Create a rollback-capable receiver reservation.
    pub fn new(
        receiver: mpsc::Receiver<MediaFrame>,
        restore: impl FnOnce(mpsc::Receiver<MediaFrame>) + Send + 'static,
    ) -> Self {
        Self {
            receiver: Some(receiver),
            restore: Some(Box::new(restore)),
            on_commit: None,
        }
    }

    /// Install a hook that runs only when provisional ownership is committed.
    ///
    /// This is primarily useful for ownership accounting. A rolled-back
    /// reservation must not be counted as a destructive receiver acquisition.
    pub fn with_commit_hook(mut self, on_commit: impl FnOnce() + Send + 'static) -> Self {
        self.on_commit = Some(Box::new(on_commit));
        self
    }

    /// Transfer permanent ownership of the reserved receiver to the caller.
    pub fn commit(mut self) -> mpsc::Receiver<MediaFrame> {
        self.restore.take();
        if let Some(on_commit) = self.on_commit.take() {
            on_commit();
        }
        self.receiver
            .take()
            .expect("media receiver reservation commits exactly once")
    }
}

impl Drop for MediaReceiverReservation {
    fn drop(&mut self) {
        if let (Some(receiver), Some(restore)) = (self.receiver.take(), self.restore.take()) {
            restore(receiver);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum StreamKind {
    Audio,
    Video,
    Data,
}

#[derive(Clone)]
pub struct MediaFrame {
    pub stream_id: StreamId,
    pub kind: StreamKind,
    /// Encoded codec payload only.
    ///
    /// This field never contains a serialized RTP packet. Adapters remove the
    /// RTP header before constructing a `MediaFrame` and reconstruct it only
    /// at their wire boundary. RTP metadata that must survive a bridge lives
    /// in `timestamp_rtp` and `payload_type`; consumers must not inspect these
    /// bytes to guess whether an RTP header is present.
    pub payload: Bytes,
    pub timestamp_rtp: u32,
    pub captured_at: DateTime<Utc>,
    /// RTP payload type for this frame, when known. Set by adapter
    /// inbound pumps that read the wire RTP header (SIP, WebRTC, QUIC,
    /// WT). Used by the cross-transport `crate::bridge::frame_pump`
    /// to route RFC 4733 telephone-events (typically PT 101) to the
    /// DTMF event sink instead of through the codec transcoder. `None`
    /// for construction sites that don't have a wire RTP header
    /// (synthetic test frames, transcoder outputs).
    ///
    /// Gap plan §4.3 / CONVERSATION_PROTOCOL.md §7.5.
    pub payload_type: Option<u8>,
}

impl fmt::Debug for MediaFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MediaFrame")
            .field("stream_id", &self.stream_id)
            .field("kind", &self.kind)
            .field("payload_bytes", &self.payload.len())
            .field("timestamp_rtp", &self.timestamp_rtp)
            .field("captured_at", &self.captured_at)
            .field("payload_type", &self.payload_type)
            .finish()
    }
}

#[derive(Clone, Debug, Default)]
pub struct QualitySnapshot {
    pub jitter_ms: f32,
    pub packet_loss_pct: f32,
    pub mos: Option<f32>,
}

/// Synchronous decision made for one application data message crossing a
/// two-connection bridge.
///
/// The policy receives the exact source and target connection identities. A
/// forwarded message may be returned unchanged or replaced with a sanitized
/// transport-neutral message. Message bodies are deliberately absent from
/// this type's diagnostics.
pub enum BridgedDataMessageDecision {
    Forward(DataMessage),
    Drop,
}

impl fmt::Debug for BridgedDataMessageDecision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Forward(message) => formatter.debug_tuple("Forward").field(message).finish(),
            Self::Drop => formatter.write_str("Drop"),
        }
    }
}

/// Non-blocking application policy for bridged [`DataMessage`] traffic.
///
/// Implementations must return promptly and must not perform I/O. The
/// orchestrator evaluates policies on a bridge-owned bounded worker, outside
/// adapter-event ingest, and validates any transformed message before sending
/// it to the exact peer connection.
pub trait DataMessageBridgePolicy: Send + Sync {
    fn decide(
        &self,
        source: &ConnectionId,
        target: &ConnectionId,
        message: DataMessage,
    ) -> BridgedDataMessageDecision;
}

/// Compatibility policy used by rvoip-core bridge callers that do not install
/// an application policy.
#[derive(Clone, Copy, Debug, Default)]
pub struct PassThroughDataMessageBridgePolicy;

impl DataMessageBridgePolicy for PassThroughDataMessageBridgePolicy {
    fn decide(
        &self,
        _source: &ConnectionId,
        _target: &ConnectionId,
        message: DataMessage,
    ) -> BridgedDataMessageDecision {
        BridgedDataMessageDecision::Forward(message)
    }
}

/// Transport-agnostic media flow. Channel-based per INTERFACE_DESIGN §3.6 to
/// avoid per-frame async overhead at high frame rates.
#[async_trait::async_trait]
pub trait MediaStream: Send + Sync {
    fn id(&self) -> StreamId;
    fn kind(&self) -> StreamKind;
    fn codec(&self) -> CodecInfo;
    fn direction(&self) -> Direction;

    /// Report whether this stream's source codec and inbound-frame producer are
    /// ready to be attached to a long-lived consumer.
    ///
    /// Most transports publish only fully negotiated streams, so the
    /// compatibility default is `true`. Deferred transports may expose a stable
    /// stream identity before SDP/media negotiation finishes; those transports
    /// must return `false` until [`Self::codec`] is authoritative and a receiver
    /// acquired through [`Self::reserve_frames_in`] will be driven by the exact
    /// live route. This probe is deliberately non-destructive.
    fn source_ready(&self) -> bool {
        true
    }

    /// Acquire the stream's inbound receiver.
    ///
    /// This legacy API is intentionally retained for source compatibility.
    /// Built-in streams are single-consumer and return a closed receiver when
    /// it has already been acquired. New orchestration code should use
    /// [`Self::try_frames_in`] so duplicate acquisition is reported rather
    /// than silently behaving like end-of-stream.
    fn frames_in(&self) -> mpsc::Receiver<MediaFrame>;

    /// Fallibly acquire the stream's single-consumer inbound receiver.
    ///
    /// The default delegates to [`Self::frames_in`] so third-party stream
    /// implementations remain source compatible. Built-in transports
    /// override this method and return [`crate::error::RvoipError::InvalidState`]
    /// when ownership has already been transferred.
    fn try_frames_in(&self) -> Result<mpsc::Receiver<MediaFrame>> {
        Ok(self.frames_in())
    }

    /// Provisionally reserve the stream's single-consumer inbound receiver.
    ///
    /// The default preserves source compatibility for third-party stream
    /// implementations but fails before acquiring anything. Streams used by
    /// atomic multi-source orchestration should override this method and
    /// return a rollback-capable [`MediaReceiverReservation`]. Legacy callers
    /// may continue to use [`Self::try_frames_in`].
    fn reserve_frames_in(&self) -> Result<MediaReceiverReservation> {
        Err(crate::error::RvoipError::NotImplemented(
            "MediaStream::reserve_frames_in",
        ))
    }
    /// Obtain the legacy outbound-frame sender.
    ///
    /// This infallible API is retained for source compatibility. Streams whose
    /// lifecycle can reject writes before activation should return a closed
    /// sender in that state and override [`Self::try_frames_out`] so new callers
    /// receive a typed error instead of discovering the rejection on `send`.
    fn frames_out(&self) -> mpsc::Sender<MediaFrame>;

    /// Fallibly obtain the outbound-frame sender.
    ///
    /// The default delegates to [`Self::frames_out`] so third-party stream
    /// implementations remain source compatible. Deferred transports override
    /// this method and return [`crate::error::RvoipError::InvalidState`] until
    /// their media path has been activated.
    fn try_frames_out(&self) -> Result<mpsc::Sender<MediaFrame>> {
        Ok(self.frames_out())
    }

    fn quality_snapshot(&self) -> QualitySnapshot;

    async fn close(self: Arc<Self>) -> Result<()>;
}

/// Cheap, cloneable reference a `crate::Connection` holds to its media flows.
/// Wraps an `Arc<dyn MediaStream>` so the trait object can live in `Debug`
/// types like `Connection` without forcing every adapter to implement Debug.
#[derive(Clone)]
pub struct MediaStreamHandle(pub Arc<dyn MediaStream>);

impl MediaStreamHandle {
    pub fn new(stream: Arc<dyn MediaStream>) -> Self {
        Self(stream)
    }

    pub fn id(&self) -> StreamId {
        self.0.id()
    }

    pub fn kind(&self) -> StreamKind {
        self.0.kind()
    }

    pub fn stream(&self) -> &Arc<dyn MediaStream> {
        &self.0
    }
}

impl fmt::Debug for MediaStreamHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MediaStreamHandle")
            .field("id", &self.0.id())
            .field("kind", &self.0.kind())
            .finish()
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn media_frame_debug_reports_shape_without_packet_bytes() {
        const CANARY: &[u8] = b"media-frame-canary\r\nAuthorization: exposed";
        let frame = MediaFrame {
            stream_id: StreamId::from_string("stream-canary"),
            kind: StreamKind::Audio,
            payload: Bytes::from_static(CANARY),
            timestamp_rtp: 123,
            captured_at: Utc::now(),
            payload_type: Some(111),
        };
        let debug = format!("{frame:?}");
        assert!(!debug.contains("media-frame-canary"));
        assert!(!debug.contains("stream-canary"));
        assert!(debug.contains(&format!("payload_bytes: {}", CANARY.len())));
    }
}

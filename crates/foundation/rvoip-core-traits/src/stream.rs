use crate::capability::CodecInfo;
use crate::connection::Direction;
use crate::error::Result;
use crate::ids::StreamId;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;
use tokio::sync::mpsc;

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

/// Transport-agnostic media flow. Channel-based per INTERFACE_DESIGN §3.6 to
/// avoid per-frame async overhead at high frame rates.
#[async_trait::async_trait]
pub trait MediaStream: Send + Sync {
    fn id(&self) -> StreamId;
    fn kind(&self) -> StreamKind;
    fn codec(&self) -> CodecInfo;
    fn direction(&self) -> Direction;

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
    fn frames_out(&self) -> mpsc::Sender<MediaFrame>;

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

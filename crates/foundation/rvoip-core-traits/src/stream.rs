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

/// Non-destructive readiness level required by [`StreamSelector`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum MediaReadiness {
    /// The adapter has registered a matching stream identity.
    #[default]
    Registered,
    /// The matching stream's negotiated codec and inbound producer are ready.
    SourceReady,
    /// Source readiness is true and the stream accepts outbound frames.
    Bidirectional,
}

/// Transport-neutral criteria for awaiting one media stream.
#[derive(Clone, Eq, PartialEq)]
pub struct StreamSelector {
    pub kind: StreamKind,
    pub codec_name: Option<String>,
    pub direction: Option<Direction>,
    pub readiness: MediaReadiness,
}

impl StreamSelector {
    pub fn new(kind: StreamKind) -> Self {
        Self {
            kind,
            codec_name: None,
            direction: None,
            readiness: MediaReadiness::Registered,
        }
    }

    pub fn with_codec(mut self, codec_name: impl Into<String>) -> Self {
        self.codec_name = Some(codec_name.into());
        self
    }

    pub fn with_direction(mut self, direction: Direction) -> Self {
        self.direction = Some(direction);
        self
    }

    pub fn with_readiness(mut self, readiness: MediaReadiness) -> Self {
        self.readiness = readiness;
        self
    }

    pub fn matches(&self, stream: &dyn MediaStream) -> bool {
        if stream.kind() != self.kind
            || self
                .codec_name
                .as_ref()
                .is_some_and(|codec| !stream.codec().name.eq_ignore_ascii_case(codec))
            || self
                .direction
                .is_some_and(|direction| stream.direction() != direction)
        {
            return false;
        }
        match self.readiness {
            MediaReadiness::Registered => true,
            MediaReadiness::SourceReady => stream.source_ready(),
            MediaReadiness::Bidirectional => {
                stream.source_ready() && stream.try_frames_out().is_ok()
            }
        }
    }
}

impl fmt::Debug for StreamSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StreamSelector")
            .field("kind", &self.kind)
            .field("codec_present", &self.codec_name.is_some())
            .field("direction", &self.direction)
            .field("readiness", &self.readiness)
            .finish()
    }
}

/// Distinct outcomes from an awaitable stream-registration request.
#[derive(Clone, Copy, Debug, Eq, thiserror::Error, PartialEq)]
#[non_exhaustive]
pub enum StreamWaitError {
    #[error("connection was not registered when stream waiting began")]
    ConnectionNotFound,
    #[error("the captured connection lifecycle terminated before media was ready")]
    ConnectionTerminated,
    #[error("the connection lifecycle generation was replaced")]
    GenerationReplaced,
    #[error("the owning adapter route is unavailable")]
    AdapterUnavailable,
    #[error("stream waiting was cancelled")]
    Cancelled,
    #[error("stream readiness deadline elapsed")]
    DeadlineExceeded,
    #[error("the owning adapter failed while querying streams")]
    AdapterFailure,
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

/// One media-quality reading for a stream.
///
/// `jitter_ms`, `packet_loss_pct` and `mos` describe what this endpoint
/// *receives*. The `rtt_ms` and `remote_*` fields come from the peer's RTCP
/// reports about what this endpoint *sends*; they are `None` until such a
/// report arrives and stay `None` on transports or calls without RTCP.
#[derive(Clone, Debug, Default)]
pub struct QualitySnapshot {
    /// Interarrival jitter of the received stream, milliseconds.
    pub jitter_ms: f32,
    /// Loss on the received stream, percent (0–100).
    pub packet_loss_pct: f32,
    /// Estimated MOS (1.0–5.0) of the received stream, when available.
    pub mos: Option<f32>,
    /// Round-trip time measured from RTCP, milliseconds.
    pub rtt_ms: Option<f32>,
    /// Loss the peer reported on our sent stream, percent (0–100).
    pub remote_packet_loss_pct: Option<f32>,
    /// Interarrival jitter the peer reported on our sent stream, ms.
    pub remote_jitter_ms: Option<f32>,
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

/// Whether a [`GatedFrameInput`] currently lets frames cross.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GatedFrameAdmission {
    /// Frames may cross: reserve capacity on [`GatedFrameInput::channel`],
    /// then [`GatedFrameInput::commit`] the frame.
    Open,
    /// The stream's consumer is paused. The producer takes the frame and
    /// discards it, exactly as the stream's own input pump would. The frame
    /// counts as accepted.
    Discard,
    /// The consumer is gone. The producer stops, as it would on a closed
    /// channel.
    Closed,
}

/// A stream input whose consumer-side gate is applied by the producer.
///
/// A stream that buffers its input in front of a gate (for example a pump
/// task that discards frames while the consumer is paused) can offer this
/// through [`MediaStream::try_gated_frames_out`]. A media graph then applies
/// the gate itself and writes each frame straight into the consumer's
/// channel, so no intermediate task handles every frame.
///
/// The protocol for one frame:
///
/// 1. Check [`Self::admission`]. Only `Open` continues.
/// 2. Reserve capacity on [`Self::channel`]. A producer that waits for
///    capacity races [`Self::left_open`] and discards the frame (as accepted)
///    when the gate leaves `Open` first.
/// 3. [`Self::commit`] the frame through the permit. `commit` rechecks the
///    gate under its transition fence and never blocks, so a producer may
///    call it while holding its own lock.
pub trait GatedFrameInput: Send + Sync + 'static {
    /// The consumer's bounded channel.
    fn channel(&self) -> &mpsc::Sender<MediaFrame>;

    /// The gate state, checked before capacity is reserved.
    fn admission(&self) -> GatedFrameAdmission;

    /// Resolves once the gate is no longer `Open`, or the consumer is gone.
    /// Only a producer that is waiting for capacity awaits this.
    fn left_open(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>>;

    /// Send `frame` through `permit` if the gate is still open; otherwise
    /// discard it. Returns whether the frame crossed. Never blocks.
    fn commit(&self, permit: mpsc::Permit<'_, MediaFrame>, frame: MediaFrame) -> bool;
}

/// Where a stream's media lane is: the runtime that owns the stream's
/// socket, for per-frame work that reads from or writes to the stream.
///
/// Published by a stream through [`MediaStream::media_lane`]. Placement only:
/// work spawned on [`Self::handle`] behaves exactly as anywhere else; it just
/// runs next to the stream's own producer and writer instead of on the
/// caller's runtime. The two flags say which media graphs the stream wants
/// there.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct MediaLanePlacement {
    /// The runtime of the lane that owns the stream's socket.
    pub handle: tokio::runtime::Handle,
    /// A graph that reads from this stream (this stream is its source) may
    /// run on the lane.
    pub host_reader_graphs: bool,
    /// A graph that writes into this stream (this stream is its target) and
    /// whose source has no lane of its own may run on the lane.
    pub host_writer_graphs: bool,
}

impl MediaLanePlacement {
    /// A placement on `handle` that hosts no graph.
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        Self {
            handle,
            host_reader_graphs: false,
            host_writer_graphs: false,
        }
    }

    /// Set [`Self::host_reader_graphs`].
    pub fn with_reader_graphs(mut self, enabled: bool) -> Self {
        self.host_reader_graphs = enabled;
        self
    }

    /// Set [`Self::host_writer_graphs`].
    pub fn with_writer_graphs(mut self, enabled: bool) -> Self {
        self.host_writer_graphs = enabled;
        self
    }
}

/// A move-only writer for the sole producer of a stream's encoded G.711
/// output (for example a recording or prompt player that already holds
/// G.711 payloads).
///
/// Taken with [`MediaStream::try_take_encoded_g711_writer`]. While a writer
/// exists the stream accepts no other producer. Implementations keep the
/// stream's transport generation, encryption, timestamp normalisation and
/// pacing/backpressure semantics: a frame sent here leaves exactly as the
/// same payload written through [`MediaStream::try_frames_out`] would.
#[async_trait::async_trait]
pub trait EncodedG711Writer: Send {
    /// Send one encoded G.711 payload with its source RTP timestamp.
    async fn send_encoded(
        &mut self,
        payload: &Bytes,
        timestamp: u32,
        payload_type: u8,
    ) -> Result<()>;

    /// Handle `frames` (`(payload, timestamp)`, in order) now, without
    /// awaiting, exactly as the same sequence of [`Self::send_encoded`] calls
    /// would handle them, as far as that is possible synchronously. The
    /// report says how far it got; the caller sends every frame after that
    /// with `send_encoded`, in order.
    ///
    /// `batch` lets frames that leave together use one system call. The
    /// default handles nothing.
    fn try_send_encoded_now(
        &mut self,
        _frames: &[(&Bytes, u32)],
        _payload_type: u8,
        _batch: bool,
    ) -> EncodedSendNow {
        EncodedSendNow::default()
    }
}

/// What [`EncodedG711Writer::try_send_encoded_now`] did.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct EncodedSendNow {
    /// Leading frames for which `send_encoded` would have returned `Ok`
    /// (sent, or dropped where it drops them).
    pub sent: usize,
    /// `send_encoded`'s error for frame `sent`, which it consumed. Frames
    /// after it were not handled.
    pub failure: Option<crate::error::RvoipError>,
}

impl EncodedSendNow {
    /// Frames handled (sent, or failed).
    pub fn handled(&self) -> usize {
        self.sent + usize::from(self.failure.is_some())
    }
}

/// One inbound frame lent to an [`InboundFrameSink`]. Fields mean what the
/// same-named [`MediaFrame`] fields mean; the stream id and capture time are
/// not materialised, since a sink is bound to one stream when it is offered.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct InboundFrameView<'a> {
    /// The frame's kind.
    pub kind: StreamKind,
    /// Encoded codec payload only, as in [`MediaFrame::payload`]. Empty when
    /// the sink declared it does not read payloads
    /// ([`InboundFrameSink::wants_payload`]) and the stream skipped encoding.
    pub payload: &'a [u8],
    /// The RTP timestamp, as in [`MediaFrame::timestamp_rtp`].
    pub timestamp_rtp: u32,
    /// The payload type, as in [`MediaFrame::payload_type`].
    pub payload_type: Option<u8>,
}

impl<'a> InboundFrameView<'a> {
    /// A view of one frame.
    pub fn new(
        kind: StreamKind,
        payload: &'a [u8],
        timestamp_rtp: u32,
        payload_type: Option<u8>,
    ) -> Self {
        Self {
            kind,
            payload,
            timestamp_rtp,
            payload_type,
        }
    }
}

/// What an [`InboundFrameSink`] did with a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InboundFrameDelivery {
    /// Taken (as `frames_in` accepting the frame).
    Accepted,
    /// The consumer is gone (as `frames_in` being closed). The producer
    /// stops as it does when `frames_in` closes.
    Closed,
}

/// A synchronous consumer of one stream's inbound frames that runs on the
/// stream's media owner instead of reading `frames_in` from its own task.
///
/// Offered with [`MediaStream::offer_inbound_frame_sink`]. A stream that
/// adopts the sink hands it, in media order, every inbound frame it would
/// otherwise send on `frames_in` from the point of adoption; frames already
/// sent on `frames_in` stay there, so the consumer keeps its receiver and
/// keeps reading it. A stream that cannot adopt the sink (or stops using it,
/// for example when its media leaves the owner) keeps or resumes using
/// `frames_in`. Calls run on the media owner, so implementations must be
/// cheap and must not block, await or take contended locks.
pub trait InboundFrameSink: Send + 'static {
    /// One inbound frame, borrowed for the duration of the call.
    fn on_frame(&mut self, frame: InboundFrameView<'_>) -> InboundFrameDelivery;

    /// Whether this sink reads [`InboundFrameView::payload`]. A sink that
    /// only counts or validates frame metadata returns `false`; a stream may
    /// then skip producing the payload (the SIP stream skips its G.711
    /// re-encode) and lend an empty one. Every other field, and which frames
    /// are lent in which order, stay exactly as with a payload. Asked once,
    /// when the sink is adopted. The default (`true`) keeps payloads.
    fn wants_payload(&self) -> bool {
        true
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
    #[deprecated(
        since = "0.3.9",
        note = "use try_frames_in or reserve_frames_in so duplicate ownership is observable"
    )]
    fn frames_in(&self) -> mpsc::Receiver<MediaFrame>;

    /// Fallibly acquire the stream's single-consumer inbound receiver.
    ///
    /// The default delegates to [`Self::frames_in`] so third-party stream
    /// implementations remain source compatible. Built-in transports
    /// override this method and return [`crate::error::RvoipError::InvalidState`]
    /// when ownership has already been transferred.
    fn try_frames_in(&self) -> Result<mpsc::Receiver<MediaFrame>> {
        #[allow(deprecated)]
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

    /// Obtain a generation-aware queue. Implementations must check each entry
    /// after dequeueing and retain its delivery guard through the local send.
    /// Legacy queues cannot provide this contract implicitly.
    fn try_peer_frames_out(&self) -> Result<mpsc::Sender<crate::peer_media::PeerMediaFrame>> {
        Err(crate::error::RvoipError::NotImplemented(
            "MediaStream::try_peer_frames_out",
        ))
    }

    /// The stream's gated input (see [`GatedFrameInput`]), if it offers one.
    ///
    /// A media graph routing into this stream then writes into the consumer's
    /// channel itself instead of sending through [`Self::try_frames_out`].
    /// [`Self::try_peer_frames_out`] takes precedence when both are offered.
    /// The default, for every stream without such a gate, is `None`.
    fn try_gated_frames_out(&self) -> Option<Arc<dyn GatedFrameInput>> {
        None
    }

    /// The stream's media lane (see [`MediaLanePlacement`]), once the stream
    /// is bound, if it wants per-frame graph work to run there. The default,
    /// and every stream without a lane, is `None`.
    fn media_lane(&self) -> Option<MediaLanePlacement> {
        None
    }

    /// Offer a synchronous consumer for this stream's inbound frames (see
    /// [`InboundFrameSink`]). `Err` hands the sink back: the stream cannot
    /// host one (the default), or one was already offered. `Ok` means the
    /// stream may adopt it at any later frame; until then, and whenever it
    /// does not use it, frames keep arriving on `frames_in`.
    fn offer_inbound_frame_sink(
        &self,
        sink: Box<dyn InboundFrameSink>,
    ) -> std::result::Result<(), Box<dyn InboundFrameSink>> {
        Err(sink)
    }

    /// Take the stream's encoded G.711 writer (see [`EncodedG711Writer`]),
    /// if the stream offers one and has no other producer. `Ok(None)` (the
    /// default) means the caller writes through [`Self::try_frames_out`].
    fn try_take_encoded_g711_writer(&self) -> Result<Option<Box<dyn EncodedG711Writer>>> {
        Ok(None)
    }

    fn quality_snapshot(&self) -> QualitySnapshot;

    /// Whether [`Self::quality_snapshot`] reflects an actual measurement.
    ///
    /// `QualitySnapshot::default()` is all zeros, which reads as flawless
    /// rather than as unknown — the type has no way to say "no data". A
    /// transport that has not yet received a quality report should return
    /// `false` here so aggregators skip it instead of averaging in a
    /// perfect score for a call nobody has measured.
    ///
    /// Defaults to `true` for source compatibility: an implementation that
    /// always returns a real snapshot needs no change.
    fn has_quality_measurement(&self) -> bool {
        true
    }

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
    fn media_executor_seams_default_to_the_channels() {
        struct Plain;
        #[async_trait::async_trait]
        impl MediaStream for Plain {
            fn id(&self) -> StreamId {
                StreamId::new()
            }
            fn kind(&self) -> StreamKind {
                StreamKind::Audio
            }
            fn codec(&self) -> CodecInfo {
                CodecInfo::from_name_with_defaults("PCMU")
            }
            fn direction(&self) -> Direction {
                Direction::Inbound
            }
            fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
                mpsc::channel(1).1
            }
            fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
                mpsc::channel(1).0
            }
            fn quality_snapshot(&self) -> QualitySnapshot {
                QualitySnapshot::default()
            }
            async fn close(self: Arc<Self>) -> Result<()> {
                Ok(())
            }
        }
        struct Counting(usize);
        impl InboundFrameSink for Counting {
            fn on_frame(&mut self, frame: InboundFrameView<'_>) -> InboundFrameDelivery {
                assert_eq!(frame.kind, StreamKind::Audio);
                self.0 += 1;
                InboundFrameDelivery::Accepted
            }
        }
        let stream = Plain;
        assert!(stream.media_lane().is_none());
        assert!(stream.try_take_encoded_g711_writer().unwrap().is_none());
        let mut refused = stream
            .offer_inbound_frame_sink(Box::new(Counting(0)))
            .expect_err("the default refuses and hands the sink back");
        assert!(refused.wants_payload(), "sinks read payloads by default");
        let view = InboundFrameView::new(StreamKind::Audio, &[1, 2], 160, Some(0));
        assert_eq!(refused.on_frame(view), InboundFrameDelivery::Accepted);

        let mut report = EncodedSendNow::default();
        assert_eq!(report.handled(), 0);
        report.sent = 2;
        report.failure = Some(crate::error::RvoipError::InvalidState("closed"));
        assert_eq!(report.handled(), 3, "the failed frame was handled too");
    }

    #[tokio::test]
    async fn a_media_lane_placement_hosts_nothing_until_asked() {
        let placement = MediaLanePlacement::new(tokio::runtime::Handle::current());
        assert!(!placement.host_reader_graphs && !placement.host_writer_graphs);
        let placement = placement.with_reader_graphs(true).with_writer_graphs(true);
        assert!(placement.host_reader_graphs && placement.host_writer_graphs);
    }

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

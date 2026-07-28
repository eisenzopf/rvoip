//! `MediaStream` implementation over webrtc-rs tracks.

use std::collections::HashSet;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::{mpsc, Notify};
use tokio::task::JoinHandle;
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_remote::TrackRemote;

use rvoip_core::capability::CodecInfo;
use rvoip_core::connection::Direction;
use rvoip_core::error::{Result as RvoipResult, RvoipError};
use rvoip_core::ids::StreamId;
use rvoip_core::stream::{MediaReceiverReservation, MediaStream, QualitySnapshot, StreamKind};

use crate::media::dtmf::{DecodedDtmfEvent, TelephoneEventCodec};
use crate::media::outbound::OutboundAudioRtpWriter;
use crate::media::pump::{
    spawn_inbound_pump_tracked, spawn_outbound_pump_with_writer_tracked, InboundStats,
    WebRtcStatsSnapshot, DEFAULT_INBOUND_SEND_DEADLINE_MS, FRAME_CHANNEL_CAP,
};
use crate::media::stats::spawn_webrtc_stats_collector_tracked;

struct WebRtcMediaStreamInner {
    id: StreamId,
    codec: CodecInfo,
    direction: Direction,
    frames_in_tx: mpsc::Sender<rvoip_core::stream::MediaFrame>,
    frames_in_rx: Mutex<Option<mpsc::Receiver<rvoip_core::stream::MediaFrame>>>,
    frames_out_tx: mpsc::Sender<rvoip_core::stream::MediaFrame>,
    pumps: Mutex<Vec<JoinHandle<()>>>,
    /// Remote track identities already assigned an inbound pump. Audio and
    /// RFC 4733 telephone-event commonly arrive on distinct WebRTC tracks,
    /// so a boolean cannot represent the route correctly.
    remote_tracks: Mutex<HashSet<usize>>,
    inbound_stats: Arc<InboundStats>,
    dtmf_tx: Option<mpsc::Sender<DecodedDtmfEvent>>,
    dtmf_codecs: Vec<TelephoneEventCodec>,
    /// Local cancel used to stop owned pump tasks on `close()`. The adapter may
    /// also pass a route-level Notify into `enable_webrtc_stats` for global cancel.
    cancel: Arc<Notify>,
    send_deadline_ms: u64,
    task_counter: Option<Arc<AtomicUsize>>,
}

/// Concrete media stream — supports late remote-track attachment.
pub struct WebRtcMediaStream {
    inner: Arc<WebRtcMediaStreamInner>,
}

impl WebRtcMediaStream {
    /// Attach a remote track after the stream was created (late `on_track`).
    pub fn attach_remote(&self, track: Arc<dyn TrackRemote>) {
        let identity = track_identity(&track);
        if !self.inner.remote_tracks.lock().insert(identity) {
            return;
        }

        let handle = spawn_inbound_pump_tracked(
            track,
            self.inner.id.clone(),
            self.inner.frames_in_tx.clone(),
            Arc::clone(&self.inner.inbound_stats),
            self.inner.send_deadline_ms,
            Some(Arc::clone(&self.inner.cancel)),
            self.inner.dtmf_tx.clone(),
            self.inner.dtmf_codecs.clone(),
            self.inner.task_counter.clone(),
        );
        self.inner.pumps.lock().push(handle);
    }

    /// Poll webrtc-rs `get_stats` in the background to enrich [`QualitySnapshot`].
    /// `cancel` is honored so the loop exits cleanly when the route is closed.
    pub fn enable_webrtc_stats(
        &self,
        peer: Arc<dyn webrtc::peer_connection::PeerConnection>,
        cancel: Arc<Notify>,
    ) {
        let handle = spawn_webrtc_stats_collector_tracked(
            Arc::clone(&self.inner.inbound_stats),
            peer,
            cancel,
            self.inner.task_counter.clone(),
        );
        self.inner.pumps.lock().push(handle);
    }

    /// Typed inbound RTP stats snapshot (packets/bytes/loss/jitter/MOS).
    /// Richer than the core `QualitySnapshot` returned by `quality_snapshot()`.
    pub fn webrtc_stats_snapshot(&self) -> WebRtcStatsSnapshot {
        self.inner.inbound_stats.webrtc_snapshot()
    }

    /// Most recently observed non-DTMF RTP payload type on the remote wire.
    /// This is diagnostic evidence, not a substitute for negotiated codec
    /// policy.
    pub fn last_inbound_media_payload_type(&self) -> Option<u8> {
        self.inner.inbound_stats.last_media_payload_type()
    }

    /// Cancel, abort, and join every task owned by this stream.
    ///
    /// Route teardown uses this directly so retained stream Arcs cannot keep
    /// detached RTP or stats work alive after the peer route is gone.
    pub(crate) async fn shutdown_background_tasks(&self) {
        self.inner.cancel.notify_waiters();
        let tasks: Vec<_> = self.inner.pumps.lock().drain(..).collect();
        for mut task in tasks {
            if !task.is_finished() {
                task.abort();
            }
            let _ = (&mut task).await;
        }
    }

    /// Best-effort synchronous cancellation for adapter Drop paths where an
    /// async join is unavailable.
    pub(crate) fn abort_background_tasks(&self) {
        self.inner.cancel.notify_waiters();
        for task in self.inner.pumps.lock().drain(..) {
            task.abort();
        }
    }
}

/// Build a bidirectional audio stream from local + optional remote track.
///
/// D4 follow-up — `local_ssrc` and `payload_type` are passed to the
/// outbound pump so it can wrap codec-payload `MediaFrame`s (the
/// orchestrator-side `Transcoder` output) in fresh RTP headers.
pub fn from_tracks(
    id: StreamId,
    codec: CodecInfo,
    local: Arc<TrackLocalStaticRTP>,
    local_ssrc: u32,
    payload_type: u8,
    remote: Option<Arc<dyn TrackRemote>>,
) -> Arc<WebRtcMediaStream> {
    from_tracks_with_dtmf_events(id, codec, local, local_ssrc, payload_type, remote, None)
}

/// Build a media stream and route inbound RFC 4733 telephone-events to
/// `dtmf_tx` instead of forwarding them as ordinary audio frames.
pub fn from_tracks_with_dtmf_events(
    id: StreamId,
    codec: CodecInfo,
    local: Arc<TrackLocalStaticRTP>,
    local_ssrc: u32,
    payload_type: u8,
    remote: Option<Arc<dyn TrackRemote>>,
    dtmf_tx: Option<mpsc::Sender<DecodedDtmfEvent>>,
) -> Arc<WebRtcMediaStream> {
    from_tracks_with_dtmf_codecs(
        id,
        codec,
        local,
        local_ssrc,
        payload_type,
        remote,
        dtmf_tx,
        [TelephoneEventCodec::default()],
    )
}

/// Build a media stream and decode RFC 4733 using the exact negotiated
/// telephone-event payload mappings.
///
/// Dynamic WebRTC payload types and their clock rates come from the final SDP;
/// callers that have that negotiation result must use this constructor rather
/// than assuming the compatibility default of PT 101 at 8 kHz.
///
/// When `local` and `local_ssrc` come from an
/// [`RvoipPeerConnection`](crate::peer::RvoipPeerConnection), this constructor
/// reuses that peer's serialized outbound RTP writer. Audio sent through the
/// returned stream therefore shares sequence/timestamp ownership with
/// [`crate::media::dtmf::send_dtmf`] without requiring a new public handle.
pub fn from_tracks_with_dtmf_codecs(
    id: StreamId,
    codec: CodecInfo,
    local: Arc<TrackLocalStaticRTP>,
    local_ssrc: u32,
    payload_type: u8,
    remote: Option<Arc<dyn TrackRemote>>,
    dtmf_tx: Option<mpsc::Sender<DecodedDtmfEvent>>,
    dtmf_codecs: impl IntoIterator<Item = TelephoneEventCodec>,
) -> Arc<WebRtcMediaStream> {
    from_tracks_with_dtmf_events_and_task_counter(
        id,
        codec,
        local,
        local_ssrc,
        payload_type,
        remote,
        dtmf_tx,
        dtmf_codecs.into_iter().collect(),
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn from_tracks_with_dtmf_events_and_task_counter(
    id: StreamId,
    codec: CodecInfo,
    local: Arc<TrackLocalStaticRTP>,
    local_ssrc: u32,
    payload_type: u8,
    remote: Option<Arc<dyn TrackRemote>>,
    dtmf_tx: Option<mpsc::Sender<DecodedDtmfEvent>>,
    dtmf_codecs: Vec<TelephoneEventCodec>,
    task_counter: Option<Arc<AtomicUsize>>,
    outbound_writer: Option<Arc<OutboundAudioRtpWriter>>,
) -> Arc<WebRtcMediaStream> {
    let (frames_in_tx, frames_in_rx) = mpsc::channel(FRAME_CHANNEL_CAP);
    let (frames_out_tx, frames_out_rx) = mpsc::channel(FRAME_CHANNEL_CAP);
    let inbound_stats = Arc::new(InboundStats::default());
    let cancel = Arc::new(Notify::new());
    let send_deadline_ms = DEFAULT_INBOUND_SEND_DEADLINE_MS;

    let mut pumps = Vec::new();
    let outbound_writer = outbound_writer
        .unwrap_or_else(|| OutboundAudioRtpWriter::new(local, local_ssrc, codec.clock_rate_hz));
    pumps.push(spawn_outbound_pump_with_writer_tracked(
        outbound_writer,
        frames_out_rx,
        payload_type,
        task_counter.clone(),
    ));

    let remote_tracks = Mutex::new(HashSet::new());
    if let Some(remote_track) = remote {
        remote_tracks.lock().insert(track_identity(&remote_track));
        pumps.push(spawn_inbound_pump_tracked(
            remote_track,
            id.clone(),
            frames_in_tx.clone(),
            Arc::clone(&inbound_stats),
            send_deadline_ms,
            Some(Arc::clone(&cancel)),
            dtmf_tx.clone(),
            dtmf_codecs.clone(),
            task_counter.clone(),
        ));
    }

    Arc::new(WebRtcMediaStream {
        inner: Arc::new(WebRtcMediaStreamInner {
            id,
            codec,
            direction: Direction::Outbound,
            frames_in_tx,
            frames_in_rx: Mutex::new(Some(frames_in_rx)),
            frames_out_tx,
            pumps: Mutex::new(pumps),
            remote_tracks,
            inbound_stats,
            dtmf_tx,
            dtmf_codecs,
            cancel,
            send_deadline_ms,
            task_counter,
        }),
    })
}

fn track_identity(track: &Arc<dyn TrackRemote>) -> usize {
    Arc::as_ptr(track) as *const () as usize
}

#[async_trait]
impl MediaStream for WebRtcMediaStream {
    fn id(&self) -> StreamId {
        self.inner.id.clone()
    }

    fn kind(&self) -> StreamKind {
        StreamKind::Audio
    }

    fn codec(&self) -> CodecInfo {
        self.inner.codec.clone()
    }

    fn direction(&self) -> Direction {
        self.inner.direction
    }

    fn frames_in(&self) -> mpsc::Receiver<rvoip_core::stream::MediaFrame> {
        self.try_frames_in().unwrap_or_else(|_| mpsc::channel(1).1)
    }

    fn try_frames_in(&self) -> RvoipResult<mpsc::Receiver<rvoip_core::stream::MediaFrame>> {
        Ok(self.reserve_frames_in()?.commit())
    }

    fn reserve_frames_in(&self) -> RvoipResult<MediaReceiverReservation> {
        let receiver = self
            .inner
            .frames_in_rx
            .lock()
            .take()
            .ok_or(RvoipError::InvalidState(
                "WebRTC media receiver has already been acquired",
            ))?;
        let inner = Arc::clone(&self.inner);
        Ok(MediaReceiverReservation::new(receiver, move |receiver| {
            let mut slot = inner.frames_in_rx.lock();
            debug_assert!(slot.is_none(), "reserved WebRTC receiver slot was replaced");
            if slot.is_none() {
                *slot = Some(receiver);
            }
        }))
    }

    fn frames_out(&self) -> mpsc::Sender<rvoip_core::stream::MediaFrame> {
        self.inner.frames_out_tx.clone()
    }

    fn quality_snapshot(&self) -> QualitySnapshot {
        self.inner.inbound_stats.snapshot()
    }

    async fn close(self: Arc<Self>) -> RvoipResult<()> {
        self.shutdown_background_tasks().await;
        Ok(())
    }
}

#[cfg(test)]
mod receiver_ownership_tests {
    use super::*;

    #[test]
    fn second_receiver_acquisition_is_a_typed_error() {
        let (frames_in_tx, frames_in_rx) = mpsc::channel(1);
        let (frames_out_tx, _frames_out_rx) = mpsc::channel(1);
        let stream = WebRtcMediaStream {
            inner: Arc::new(WebRtcMediaStreamInner {
                id: StreamId::new(),
                codec: CodecInfo {
                    name: "opus".into(),
                    clock_rate_hz: 48_000,
                    channels: 1,
                    fmtp: None,
                },
                direction: Direction::Inbound,
                frames_in_tx,
                frames_in_rx: Mutex::new(Some(frames_in_rx)),
                frames_out_tx,
                pumps: Mutex::new(Vec::new()),
                remote_tracks: Mutex::new(HashSet::new()),
                inbound_stats: Arc::new(InboundStats::default()),
                dtmf_tx: None,
                dtmf_codecs: Vec::new(),
                cancel: Arc::new(Notify::new()),
                send_deadline_ms: DEFAULT_INBOUND_SEND_DEADLINE_MS,
                task_counter: None,
            }),
        };

        let reservation = stream.reserve_frames_in().expect("reserve receiver");
        assert!(matches!(
            stream.try_frames_in(),
            Err(RvoipError::InvalidState(_))
        ));
        drop(reservation);
        assert!(stream.try_frames_in().is_ok());
        assert!(matches!(
            stream.try_frames_in(),
            Err(RvoipError::InvalidState(_))
        ));
    }
}

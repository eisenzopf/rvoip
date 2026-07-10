//! `MediaStream` implementation over webrtc-rs tracks.

use std::sync::atomic::{AtomicBool, Ordering};
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
use rvoip_core::stream::{MediaStream, QualitySnapshot, StreamKind};

use crate::media::dtmf::DecodedDtmfEvent;
use crate::media::pump::{
    spawn_inbound_pump, spawn_outbound_pump, InboundStats, WebRtcStatsSnapshot,
    DEFAULT_INBOUND_SEND_DEADLINE_MS, FRAME_CHANNEL_CAP,
};
use crate::media::stats::spawn_webrtc_stats_collector;

struct WebRtcMediaStreamInner {
    id: StreamId,
    codec: CodecInfo,
    direction: Direction,
    frames_in_tx: mpsc::Sender<rvoip_core::stream::MediaFrame>,
    frames_in_rx: Mutex<Option<mpsc::Receiver<rvoip_core::stream::MediaFrame>>>,
    frames_out_tx: mpsc::Sender<rvoip_core::stream::MediaFrame>,
    pumps: Mutex<Vec<JoinHandle<()>>>,
    remote_attached: AtomicBool,
    inbound_stats: Arc<InboundStats>,
    dtmf_tx: Option<mpsc::Sender<DecodedDtmfEvent>>,
    /// Local cancel used to stop owned pump tasks on `close()`. The adapter may
    /// also pass a route-level Notify into `enable_webrtc_stats` for global cancel.
    cancel: Arc<Notify>,
    send_deadline_ms: u64,
}

/// Concrete media stream — supports late remote-track attachment.
pub struct WebRtcMediaStream {
    inner: Arc<WebRtcMediaStreamInner>,
}

impl WebRtcMediaStream {
    /// Attach a remote track after the stream was created (late `on_track`).
    pub fn attach_remote(&self, track: Arc<dyn TrackRemote>) {
        if self
            .inner
            .remote_attached
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let handle = spawn_inbound_pump(
            track,
            self.inner.id.clone(),
            self.inner.frames_in_tx.clone(),
            Arc::clone(&self.inner.inbound_stats),
            self.inner.send_deadline_ms,
            Some(Arc::clone(&self.inner.cancel)),
            self.inner.dtmf_tx.clone(),
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
        let handle =
            spawn_webrtc_stats_collector(Arc::clone(&self.inner.inbound_stats), peer, cancel);
        self.inner.pumps.lock().push(handle);
    }

    /// Typed inbound RTP stats snapshot (packets/bytes/loss/jitter/MOS).
    /// Richer than the core `QualitySnapshot` returned by `quality_snapshot()`.
    pub fn webrtc_stats_snapshot(&self) -> WebRtcStatsSnapshot {
        self.inner.inbound_stats.webrtc_snapshot()
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
    let (frames_in_tx, frames_in_rx) = mpsc::channel(FRAME_CHANNEL_CAP);
    let (frames_out_tx, frames_out_rx) = mpsc::channel(FRAME_CHANNEL_CAP);
    let inbound_stats = Arc::new(InboundStats::default());
    let cancel = Arc::new(Notify::new());
    let send_deadline_ms = DEFAULT_INBOUND_SEND_DEADLINE_MS;

    let mut pumps = Vec::new();
    pumps.push(spawn_outbound_pump(
        local,
        frames_out_rx,
        local_ssrc,
        payload_type,
    ));

    let remote_attached = AtomicBool::new(false);
    if let Some(remote_track) = remote {
        pumps.push(spawn_inbound_pump(
            remote_track,
            id.clone(),
            frames_in_tx.clone(),
            Arc::clone(&inbound_stats),
            send_deadline_ms,
            Some(Arc::clone(&cancel)),
            dtmf_tx.clone(),
        ));
        remote_attached.store(true, Ordering::SeqCst);
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
            remote_attached,
            inbound_stats,
            dtmf_tx,
            cancel,
            send_deadline_ms,
        }),
    })
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
        self.inner
            .frames_in_rx
            .lock()
            .take()
            .ok_or(RvoipError::InvalidState(
                "WebRTC media receiver has already been acquired",
            ))
    }

    fn frames_out(&self) -> mpsc::Sender<rvoip_core::stream::MediaFrame> {
        self.inner.frames_out_tx.clone()
    }

    fn quality_snapshot(&self) -> QualitySnapshot {
        self.inner.inbound_stats.snapshot()
    }

    async fn close(self: Arc<Self>) -> RvoipResult<()> {
        // Signal background tasks (stats poller, inbound pump) to exit cleanly,
        // then abort anything that hasn't finished yet.
        self.inner.cancel.notify_waiters();
        for handle in self.inner.pumps.lock().drain(..) {
            handle.abort();
        }
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
                remote_attached: AtomicBool::new(false),
                inbound_stats: Arc::new(InboundStats::default()),
                dtmf_tx: None,
                cancel: Arc::new(Notify::new()),
                send_deadline_ms: DEFAULT_INBOUND_SEND_DEADLINE_MS,
            }),
        };

        assert!(stream.try_frames_in().is_ok());
        assert!(matches!(
            stream.try_frames_in(),
            Err(RvoipError::InvalidState(_))
        ));
    }
}

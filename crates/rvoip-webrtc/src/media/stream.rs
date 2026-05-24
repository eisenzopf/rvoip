//! `MediaStream` implementation over webrtc-rs tracks.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_remote::TrackRemote;

use rvoip_core::capability::CodecInfo;
use rvoip_core::connection::Direction;
use rvoip_core::error::Result as RvoipResult;
use rvoip_core::ids::StreamId;
use rvoip_core::stream::{MediaStream, QualitySnapshot, StreamKind};

use crate::media::pump::{spawn_inbound_pump, spawn_outbound_pump, InboundStats, FRAME_CHANNEL_CAP};
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
        );
        self.inner.pumps.lock().push(handle);
    }

    /// Poll webrtc-rs `get_stats` in the background to enrich [`QualitySnapshot`].
    pub fn enable_webrtc_stats(&self, peer: Arc<dyn webrtc::peer_connection::PeerConnection>) {
        let handle = spawn_webrtc_stats_collector(
            Arc::clone(&self.inner.inbound_stats),
            peer,
        );
        self.inner.pumps.lock().push(handle);
    }
}

/// Build a bidirectional audio stream from local + optional remote track.
pub fn from_tracks(
    id: StreamId,
    codec: CodecInfo,
    local: Arc<TrackLocalStaticRTP>,
    remote: Option<Arc<dyn TrackRemote>>,
) -> Arc<WebRtcMediaStream> {
    let (frames_in_tx, frames_in_rx) = mpsc::channel(FRAME_CHANNEL_CAP);
    let (frames_out_tx, frames_out_rx) = mpsc::channel(FRAME_CHANNEL_CAP);
    let inbound_stats = Arc::new(InboundStats::default());

    let mut pumps = Vec::new();
    pumps.push(spawn_outbound_pump(local, frames_out_rx));

    let remote_attached = AtomicBool::new(false);
    if let Some(remote_track) = remote {
        pumps.push(spawn_inbound_pump(
            remote_track,
            id.clone(),
            frames_in_tx.clone(),
            Arc::clone(&inbound_stats),
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
        self.inner
            .frames_in_rx
            .lock()
            .take()
            .unwrap_or_else(|| mpsc::channel(1).1)
    }

    fn frames_out(&self) -> mpsc::Sender<rvoip_core::stream::MediaFrame> {
        self.inner.frames_out_tx.clone()
    }

    fn quality_snapshot(&self) -> QualitySnapshot {
        self.inner.inbound_stats.snapshot()
    }

    async fn close(self: Arc<Self>) -> RvoipResult<()> {
        for handle in self.inner.pumps.lock().drain(..) {
            handle.abort();
        }
        Ok(())
    }
}

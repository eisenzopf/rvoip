//! Event fan-in from webrtc-rs `PeerConnectionEventHandler` to internal channels.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use crate::peer::ice::IceCandidateLog;

use tokio::sync::mpsc;
use tracing::warn;
use webrtc::data_channel::DataChannel;
use webrtc::media_stream::track_remote::TrackRemote;
use webrtc::peer_connection::{
    PeerConnectionEventHandler, RTCIceCandidate, RTCIceGatheringState,
    RTCPeerConnectionIceErrorEvent, RTCPeerConnectionIceEvent, RTCPeerConnectionState,
    RTCIceConnectionState, RTCSignalingState,
};

/// Per-channel drop counters surfaced via [`HandlerChannels::drops`].
#[derive(Default, Debug)]
pub struct HandlerDropCounters {
    pub remote_track: AtomicU64,
    pub data_channel: AtomicU64,
    pub state: AtomicU64,
}

impl HandlerDropCounters {
    pub fn snapshot(&self) -> (u64, u64, u64) {
        (
            self.remote_track.load(Ordering::Relaxed),
            self.data_channel.load(Ordering::Relaxed),
            self.state.load(Ordering::Relaxed),
        )
    }
}

/// Channels populated by [`ConnectionHandler`] and consumed by
/// [`super::session::RvoipPeerConnection`] / the adapter event pump.
#[derive(Clone)]
pub struct HandlerChannels {
    pub gather_complete: mpsc::Sender<()>,
    pub connected: mpsc::Sender<()>,
    pub connected_flag: Arc<AtomicBool>,
    pub failed: mpsc::Sender<()>,
    pub failed_flag: Arc<AtomicBool>,
    pub ice_candidates: IceCandidateLog,
    pub remote_track: mpsc::Sender<Arc<dyn TrackRemote>>,
    pub data_channel: mpsc::Sender<Arc<dyn DataChannel>>,
    /// Outbound (locally-gathered) ICE candidates. Drains in order via the
    /// per-peer `recv_local_ice_candidate()` API so trickle-capable signalers
    /// can forward them to the remote peer.
    pub local_ice: mpsc::Sender<RTCIceCandidate>,
    pub drops: Arc<HandlerDropCounters>,
}

impl HandlerChannels {
    pub fn pair(
        cap: usize,
    ) -> (
        Self,
        HandlerReceivers,
        Arc<AtomicBool>,
        Arc<AtomicBool>,
        IceCandidateLog,
        Arc<HandlerDropCounters>,
    ) {
        // One-shot flags use cap=1 (only first signal matters); track/dc/ICE honor `cap`.
        let (gather_complete_tx, gather_complete_rx) = mpsc::channel(1);
        let (connected_tx, connected_rx) = mpsc::channel(1);
        let connected_flag = Arc::new(AtomicBool::new(false));
        let (failed_tx, failed_rx) = mpsc::channel(1);
        let failed_flag = Arc::new(AtomicBool::new(false));
        let ice_candidates = IceCandidateLog::new();
        let (remote_track_tx, remote_track_rx) = mpsc::channel(cap.max(2));
        let (data_channel_tx, data_channel_rx) = mpsc::channel(cap.max(2));
        let (local_ice_tx, local_ice_rx) = mpsc::channel(cap.max(2));
        let drops = Arc::new(HandlerDropCounters::default());
        (
            Self {
                gather_complete: gather_complete_tx,
                connected: connected_tx,
                connected_flag: Arc::clone(&connected_flag),
                failed: failed_tx,
                failed_flag: Arc::clone(&failed_flag),
                ice_candidates: ice_candidates.clone(),
                remote_track: remote_track_tx,
                data_channel: data_channel_tx,
                local_ice: local_ice_tx,
                drops: Arc::clone(&drops),
            },
            HandlerReceivers {
                gather_complete: gather_complete_rx,
                connected: connected_rx,
                failed: failed_rx,
                remote_track: remote_track_rx,
                data_channel: data_channel_rx,
                local_ice: local_ice_rx,
            },
            connected_flag,
            failed_flag,
            ice_candidates,
            drops,
        )
    }
}

pub struct HandlerReceivers {
    pub gather_complete: mpsc::Receiver<()>,
    pub connected: mpsc::Receiver<()>,
    pub failed: mpsc::Receiver<()>,
    pub remote_track: mpsc::Receiver<Arc<dyn TrackRemote>>,
    pub data_channel: mpsc::Receiver<Arc<dyn DataChannel>>,
    pub local_ice: mpsc::Receiver<RTCIceCandidate>,
}

#[derive(Clone)]
pub struct ConnectionHandler {
    channels: HandlerChannels,
}

impl ConnectionHandler {
    pub fn new(channels: HandlerChannels) -> Arc<Self> {
        Arc::new(Self { channels })
    }
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for ConnectionHandler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            // try_send is fine here: gather_complete is one-shot and the consumer
            // is awaiting recv() on a cap-1 channel.
            let _ = self.channels.gather_complete.try_send(());
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        match state {
            RTCPeerConnectionState::Connected => {
                self.channels
                    .connected_flag
                    .store(true, Ordering::Release);
                let _ = self.channels.connected.try_send(());
            }
            RTCPeerConnectionState::Failed => {
                self.channels.failed_flag.store(true, Ordering::Release);
                if self.channels.failed.try_send(()).is_err() {
                    // Flag-based detection still works for waiters via wait_failed().
                    self.channels.drops.state.fetch_add(1, Ordering::Relaxed);
                }
            }
            RTCPeerConnectionState::Closed => {}
            _ => {}
        }
    }

    async fn on_ice_candidate(&self, event: RTCPeerConnectionIceEvent) {
        self.channels.ice_candidates.record(format!(
            "{:?} {}:{}",
            event.candidate.typ, event.candidate.address, event.candidate.port
        ));
        // Best-effort forward for trickle signalers. Drop on backpressure rather
        // than block the webrtc-rs internal event task.
        if self
            .channels
            .local_ice
            .try_send(event.candidate)
            .is_err()
        {
            let dropped = self
                .channels
                .drops
                .state
                .fetch_add(1, Ordering::Relaxed)
                + 1;
            warn!(
                dropped,
                "local ICE candidate channel full; dropping (trickle signaler too slow?)"
            );
        }
    }

    async fn on_ice_candidate_error(&self, _event: RTCPeerConnectionIceErrorEvent) {}

    async fn on_signaling_state_change(&self, _state: RTCSignalingState) {}

    async fn on_ice_connection_state_change(&self, _state: RTCIceConnectionState) {}

    async fn on_data_channel(&self, dc: Arc<dyn DataChannel>) {
        if let Err(e) = self.channels.data_channel.try_send(dc) {
            let dropped = self
                .channels
                .drops
                .data_channel
                .fetch_add(1, Ordering::Relaxed)
                + 1;
            warn!(
                dropped,
                ?e,
                "WebRTC data-channel event channel full; dropping inbound DC"
            );
        }
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        if let Err(e) = self.channels.remote_track.try_send(track) {
            let dropped = self
                .channels
                .drops
                .remote_track
                .fetch_add(1, Ordering::Relaxed)
                + 1;
            warn!(
                dropped,
                ?e,
                "WebRTC remote-track event channel full; dropping inbound track"
            );
        }
    }
}

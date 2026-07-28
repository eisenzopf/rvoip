//! Event fan-in from webrtc-rs `PeerConnectionEventHandler` to internal channels.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use crate::peer::ice::IceCandidateLog;

use parking_lot::Mutex as SyncMutex;
use tokio::sync::mpsc;
use tracing::warn;
use webrtc::data_channel::DataChannel;
use webrtc::media_stream::track_remote::TrackRemote;
use webrtc::peer_connection::{
    PeerConnectionEventHandler, RTCIceCandidate, RTCIceConnectionState, RTCIceGatheringState,
    RTCPeerConnectionIceErrorEvent, RTCPeerConnectionIceEvent, RTCPeerConnectionState,
    RTCSignalingState,
};

pub(crate) const MAX_SEEN_DATA_CHANNELS: usize = 64;

/// Route-owned local ICE signaling event. Completion is explicit and
/// overflow is terminal: signalers must never mistake either condition for a
/// candidate or silently continue with a partial candidate set.
#[derive(Clone, Debug)]
pub enum LocalIceEvent {
    Candidate(RTCIceCandidate),
    Complete,
    Overflow,
}

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
    pub data_channels_seen: Arc<SyncMutex<Vec<Arc<dyn DataChannel>>>>,
    /// Outbound (locally-gathered) ICE candidates. Drains in order via the
    /// per-peer `recv_local_ice_candidate()` API so trickle-capable signalers
    /// can forward them to the remote peer.
    pub local_ice: mpsc::Sender<LocalIceEvent>,
    pub local_ice_complete_pending: Arc<AtomicBool>,
    pub local_ice_overflowed: Arc<AtomicBool>,
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
        let data_channels_seen = Arc::new(SyncMutex::new(Vec::new()));
        let (local_ice_tx, local_ice_rx) = mpsc::channel(cap.max(2));
        let local_ice_complete_pending = Arc::new(AtomicBool::new(false));
        let local_ice_overflowed = Arc::new(AtomicBool::new(false));
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
                data_channels_seen: Arc::clone(&data_channels_seen),
                local_ice: local_ice_tx,
                local_ice_complete_pending: Arc::clone(&local_ice_complete_pending),
                local_ice_overflowed: Arc::clone(&local_ice_overflowed),
                drops: Arc::clone(&drops),
            },
            HandlerReceivers {
                gather_complete: gather_complete_rx,
                connected: connected_rx,
                failed: failed_rx,
                remote_track: remote_track_rx,
                data_channel: data_channel_rx,
                data_channels_seen,
                local_ice: local_ice_rx,
                local_ice_complete_pending,
                local_ice_overflowed,
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
    pub data_channels_seen: Arc<SyncMutex<Vec<Arc<dyn DataChannel>>>>,
    pub local_ice: mpsc::Receiver<LocalIceEvent>,
    pub local_ice_complete_pending: Arc<AtomicBool>,
    pub local_ice_overflowed: Arc<AtomicBool>,
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
            if self
                .channels
                .local_ice
                .try_send(LocalIceEvent::Complete)
                .is_err()
            {
                // Completion is lossless even when the bounded candidate
                // queue is full; the consumer synthesizes it after draining.
                self.channels
                    .local_ice_complete_pending
                    .store(true, Ordering::Release);
            }
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        match state {
            RTCPeerConnectionState::Connected => {
                self.channels.connected_flag.store(true, Ordering::Release);
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
        if self.channels.local_ice_overflowed.load(Ordering::Acquire)
            || self
                .channels
                .local_ice
                .try_send(LocalIceEvent::Candidate(event.candidate))
                .is_err()
        {
            self.channels
                .local_ice_overflowed
                .store(true, Ordering::Release);
            let dropped = self.channels.drops.state.fetch_add(1, Ordering::Relaxed) + 1;
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
        let accepted = {
            let mut seen = self.channels.data_channels_seen.lock();
            if seen.len() >= MAX_SEEN_DATA_CHANNELS {
                false
            } else {
                seen.push(Arc::clone(&dc));
                true
            }
        };
        if !accepted {
            let dropped = self
                .channels
                .drops
                .data_channel
                .fetch_add(1, Ordering::Relaxed)
                + 1;
            warn!(
                dropped,
                maximum = MAX_SEEN_DATA_CHANNELS,
                "too many remotely-created WebRTC data channels; closing excess channel"
            );
            let _ = dc.close().await;
            return;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_ice_overflow_is_sticky_and_completion_remains_pending() {
        let (channels, mut receivers, ..) = HandlerChannels::pair(1);
        let handler = ConnectionHandler::new(channels);

        handler
            .on_ice_candidate(RTCPeerConnectionIceEvent::default())
            .await;
        handler
            .on_ice_candidate(RTCPeerConnectionIceEvent::default())
            .await;
        handler
            .on_ice_candidate(RTCPeerConnectionIceEvent::default())
            .await;
        assert!(receivers.local_ice_overflowed.load(Ordering::Acquire));

        handler
            .on_ice_gathering_state_change(RTCIceGatheringState::Complete)
            .await;
        assert!(
            receivers.local_ice_complete_pending.load(Ordering::Acquire),
            "completion must survive a full bounded candidate queue"
        );
        assert!(matches!(
            receivers.local_ice.recv().await,
            Some(LocalIceEvent::Candidate(_))
        ));
        assert!(matches!(
            receivers.local_ice.recv().await,
            Some(LocalIceEvent::Candidate(_))
        ));
    }
}

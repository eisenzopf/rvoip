//! Event fan-in from webrtc-rs `PeerConnectionEventHandler` to internal channels.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::peer::ice::IceCandidateLog;

use tokio::sync::mpsc;
use webrtc::data_channel::DataChannel;
use webrtc::media_stream::track_remote::TrackRemote;
use webrtc::peer_connection::{
    PeerConnectionEventHandler, RTCIceGatheringState, RTCPeerConnectionIceErrorEvent,
    RTCPeerConnectionIceEvent, RTCPeerConnectionState, RTCIceConnectionState, RTCSignalingState,
};

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
}

impl HandlerChannels {
    pub fn pair(cap: usize) -> (Self, HandlerReceivers, Arc<AtomicBool>, Arc<AtomicBool>, IceCandidateLog) {
        let (gather_complete_tx, gather_complete_rx) = mpsc::channel(cap);
        let (connected_tx, connected_rx) = mpsc::channel(cap);
        let connected_flag = Arc::new(AtomicBool::new(false));
        let (failed_tx, failed_rx) = mpsc::channel(cap);
        let failed_flag = Arc::new(AtomicBool::new(false));
        let ice_candidates = IceCandidateLog::new();
        let (remote_track_tx, remote_track_rx) = mpsc::channel(cap);
        let (data_channel_tx, data_channel_rx) = mpsc::channel(cap);
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
            },
            HandlerReceivers {
                gather_complete: gather_complete_rx,
                connected: connected_rx,
                failed: failed_rx,
                remote_track: remote_track_rx,
                data_channel: data_channel_rx,
            },
            connected_flag,
            failed_flag,
            ice_candidates,
        )
    }
}

pub struct HandlerReceivers {
    pub gather_complete: mpsc::Receiver<()>,
    pub connected: mpsc::Receiver<()>,
    pub failed: mpsc::Receiver<()>,
    pub remote_track: mpsc::Receiver<Arc<dyn TrackRemote>>,
    pub data_channel: mpsc::Receiver<Arc<dyn DataChannel>>,
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
                let _ = self.channels.failed.try_send(());
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
    }

    async fn on_ice_candidate_error(&self, _event: RTCPeerConnectionIceErrorEvent) {}

    async fn on_signaling_state_change(&self, _state: RTCSignalingState) {}

    async fn on_ice_connection_state_change(&self, _state: RTCIceConnectionState) {}

    async fn on_data_channel(&self, dc: Arc<dyn DataChannel>) {
        let _ = self.channels.data_channel.try_send(dc);
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        let _ = self.channels.remote_track.try_send(track);
    }
}

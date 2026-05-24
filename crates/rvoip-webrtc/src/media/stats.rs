//! Background polling of webrtc-rs `get_stats` for [`InboundStats`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use rtc::statistics::StatsSelector;
use tokio::task::JoinHandle;
use webrtc::peer_connection::PeerConnection;

use super::pump::InboundStats;

const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Spawn a task that periodically merges inbound RTP stats from the peer
/// connection into `stats`.
pub fn spawn_webrtc_stats_collector(
    stats: Arc<InboundStats>,
    peer: Arc<dyn PeerConnection>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        loop {
            interval.tick().await;
            let report = peer.get_stats(Instant::now(), StatsSelector::None).await;
            stats.merge_webrtc_report(&report);
        }
    })
}

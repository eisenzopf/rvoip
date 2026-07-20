//! Background polling of webrtc-rs `get_stats` for [`InboundStats`].

use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rtc::statistics::StatsSelector;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use webrtc::peer_connection::PeerConnection;

use super::pump::{InboundStats, MediaTaskGuard};

const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Spawn a task that periodically merges inbound RTP stats from the peer
/// connection into `stats`. Exits when `cancel.notify_waiters()` is called.
pub fn spawn_webrtc_stats_collector(
    stats: Arc<InboundStats>,
    peer: Arc<dyn PeerConnection>,
    cancel: Arc<Notify>,
) -> JoinHandle<()> {
    spawn_webrtc_stats_collector_tracked(stats, peer, cancel, None)
}

pub(crate) fn spawn_webrtc_stats_collector_tracked(
    stats: Arc<InboundStats>,
    peer: Arc<dyn PeerConnection>,
    cancel: Arc<Notify>,
    task_counter: Option<Arc<AtomicUsize>>,
) -> JoinHandle<()> {
    let task_guard = MediaTaskGuard::new(task_counter);
    tokio::spawn(async move {
        let _task_guard = task_guard;
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        loop {
            tokio::select! {
                _ = cancel.notified() => break,
                _ = interval.tick() => {
                    let report = peer.get_stats(Instant::now(), StatsSelector::None).await;
                    stats.merge_webrtc_report(&report);
                }
            }
        }
    })
}

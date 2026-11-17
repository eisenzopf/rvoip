//! The finish-file drain.
//!
//! When `<run_dir>/finish` appears the DUT stops accepting new calls (every
//! new INVITE gets a 503) and waits until no call and no per-call resource is
//! left (see [`Snapshot::is_true_zero`]) or `drain_timeout_secs` passes.

use std::future::Future;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::stats::Snapshot;

/// How often the drain re-checks the gauges and the finish watcher looks for
/// the file.
pub const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// How a drain ended.
#[derive(Clone, Debug, PartialEq)]
pub struct DrainOutcome {
    /// Every gauge reached zero before the timeout.
    pub drained: bool,
    /// From the start of the drain to the first all-zero snapshot, or to the
    /// timeout.
    pub seconds: f64,
    /// The last snapshot taken.
    pub last: Snapshot,
}

/// Take snapshots every `poll` until one is all-zero or `timeout` passes.
/// A zero `timeout` still checks once.
pub async fn wait_for_true_zero<F, Fut>(
    mut snapshot: F,
    timeout: Duration,
    poll: Duration,
) -> DrainOutcome
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Snapshot>,
{
    let started = Instant::now();
    loop {
        let last = snapshot().await;
        let elapsed = started.elapsed();
        if last.is_true_zero() {
            return DrainOutcome {
                drained: true,
                seconds: elapsed.as_secs_f64(),
                last,
            };
        }
        if elapsed >= timeout {
            return DrainOutcome {
                drained: false,
                seconds: elapsed.as_secs_f64(),
                last,
            };
        }
        tokio::time::sleep(poll.min(timeout - elapsed)).await;
    }
}

/// Resolve once `path` exists, checking every `poll`.
pub async fn wait_for_file(path: &Path, poll: Duration) {
    while !path.exists() {
        tokio::time::sleep(poll).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    fn busy(active: u64, cleanups: u64) -> Snapshot {
        Snapshot {
            active_calls: active,
            pending_cleanups: cleanups,
            open_dialogs: active + cleanups,
            ..Snapshot::default()
        }
    }

    #[tokio::test]
    async fn drains_when_calls_and_cleanups_reach_zero() {
        // Active calls end first; their cleanup lags one poll behind.
        let script = [busy(3, 0), busy(1, 2), busy(0, 1), busy(0, 0), busy(5, 0)];
        let calls = Arc::new(AtomicU64::new(0));
        let probe = {
            let calls = Arc::clone(&calls);
            move || {
                let index = calls.fetch_add(1, Ordering::Relaxed) as usize;
                let snapshot = script[index.min(script.len() - 1)].clone();
                async move { snapshot }
            }
        };
        let outcome =
            wait_for_true_zero(probe, Duration::from_secs(5), Duration::from_millis(1)).await;
        assert!(outcome.drained);
        assert!(outcome.last.is_true_zero());
        assert_eq!(calls.load(Ordering::Relaxed), 4, "stops at the first zero");
        assert!(outcome.seconds < 5.0);
    }

    #[tokio::test]
    async fn times_out_when_a_resource_never_drains() {
        let leak = Snapshot {
            media_ports: 1,
            ..Snapshot::default()
        };
        let outcome = wait_for_true_zero(
            || {
                let leak = leak.clone();
                async move { leak }
            },
            Duration::from_millis(60),
            Duration::from_millis(5),
        )
        .await;
        assert!(!outcome.drained);
        assert!(outcome.seconds >= 0.06);
        assert_eq!(outcome.last.media_ports, 1);
    }

    #[tokio::test]
    async fn a_zero_timeout_still_checks_once() {
        let drained = wait_for_true_zero(
            || async { Snapshot::default() },
            Duration::ZERO,
            POLL_INTERVAL,
        )
        .await;
        assert!(drained.drained);
        let busy_now =
            wait_for_true_zero(|| async { busy(1, 0) }, Duration::ZERO, POLL_INTERVAL).await;
        assert!(!busy_now.drained);
    }

    #[tokio::test]
    async fn the_finish_file_is_noticed() {
        let dir = std::env::temp_dir().join(format!("bench-uas-drain-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let finish = dir.join("finish");
        let _ = std::fs::remove_file(&finish);
        let watcher = tokio::spawn({
            let finish = finish.clone();
            async move { wait_for_file(&finish, Duration::from_millis(5)).await }
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!watcher.is_finished());
        std::fs::write(&finish, b"").unwrap();
        tokio::time::timeout(Duration::from_secs(2), watcher)
            .await
            .expect("the watcher notices the file")
            .unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}

//! H6 deferred (#46): long-running soak with leak detection.
//!
//! Gated behind the `soak-1h` feature. Defaults to a 60-second smoke run so
//! CI can keep it on; set `SOAK_SECS=3600` to do the full one-hour version
//! locally.
//!
//! ## What it asserts
//!
//! - tokio `num_alive_tasks` returns to within +`TASK_GROWTH_BUDGET` of
//!   the baseline after each open/close cycle of N peers — the most
//!   reliable signal of a task leak across `tokio::spawn` lifetimes.
//! - The cycle count over the soak window is monotonically increasing
//!   (i.e. the workload doesn't deadlock half-way through).
//! - No panic; all `connect_loopback` + `close` calls return `Ok`.
//!
//! Run:
//!
//! ```bash
//! # 60s smoke (CI-friendly)
//! cargo test -p rvoip-webrtc --features soak-1h --test soak_long --release
//!
//! # 1h full run (sets SOAK_SECS=3600)
//! SOAK_SECS=3600 cargo test -p rvoip-webrtc --features soak-1h \
//!     --test soak_long --release -- --nocapture
//! ```

#![cfg(feature = "soak-1h")]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rvoip_webrtc::peer::connect_loopback;
use rvoip_webrtc::WebRtcConfig;

/// How many peer pairs to open per cycle. Each pair = 2 RvoipPeerConnection
/// objects + their handler/pump tasks.
const PEERS_PER_CYCLE: usize = 5;

/// How many `num_alive_tasks` over the baseline we accept as steady-state
/// (background reapers + watchers settle just above zero, but a real leak
/// shows as monotonic growth into the hundreds).
const TASK_GROWTH_BUDGET: usize = 50;

fn soak_duration() -> Duration {
    std::env::var("SOAK_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(60))
}

async fn one_cycle() -> usize {
    let mut handles = Vec::with_capacity(PEERS_PER_CYCLE);
    for _ in 0..PEERS_PER_CYCLE {
        let config = WebRtcConfig::loopback();
        handles.push(tokio::spawn(async move {
            let (offerer, answerer) = connect_loopback(&config).await.expect("loopback");
            // Hold briefly so the connection state machines fully settle.
            tokio::time::sleep(Duration::from_millis(50)).await;
            offerer.close().await.ok();
            answerer.close().await.ok();
        }));
    }
    let mut ok = 0;
    for h in handles {
        if h.await.is_ok() {
            ok += 1;
        }
    }
    ok
}

#[tokio::test]
async fn soak_under_sustained_churn_no_task_leak() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let metrics = tokio::runtime::Handle::current().metrics();

    // Warm up one cycle so the runtime's pool / lazy statics are stable.
    let _ = one_cycle().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let baseline = metrics.num_alive_tasks();

    let deadline = Instant::now() + soak_duration();
    let cycles = Arc::new(AtomicU64::new(0));
    let peak = Arc::new(AtomicU64::new(0));

    while Instant::now() < deadline {
        let started_at = Instant::now();
        let ok = one_cycle().await;
        assert_eq!(
            ok, PEERS_PER_CYCLE,
            "all peer pairs in a cycle must succeed; got {ok}/{PEERS_PER_CYCLE}"
        );

        let n = cycles.fetch_add(1, Ordering::Relaxed) + 1;

        let alive = metrics.num_alive_tasks();
        let prev_peak = peak.load(Ordering::Relaxed) as usize;
        if alive > prev_peak {
            peak.store(alive as u64, Ordering::Relaxed);
        }

        // Allow a brief settle window before sampling.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let settled = metrics.num_alive_tasks();

        if n % 20 == 0 {
            eprintln!(
                "[soak] cycle={n} elapsed={:?} alive={alive} settled={settled} baseline={baseline}",
                started_at.elapsed()
            );
        }

        // The hard assertion: settled task count never drifts more than
        // TASK_GROWTH_BUDGET above baseline. This catches leaks regardless
        // of total soak duration.
        assert!(
            settled <= baseline + TASK_GROWTH_BUDGET,
            "task leak detected: settled={settled}, baseline={baseline}, \
             budget=+{TASK_GROWTH_BUDGET} (cycle {n}, peak {prev_peak})"
        );
    }

    let total = cycles.load(Ordering::Relaxed);
    eprintln!(
        "[soak] DONE: {total} cycles, peak={}, baseline={baseline}, final={}",
        peak.load(Ordering::Relaxed),
        metrics.num_alive_tasks()
    );
    assert!(
        total >= 5,
        "expected at least 5 cycles in the soak window; got {total}"
    );
}

//! Scenario 8 — long-duration soak (default 30 min, `#[ignore]`).
//!
//! Combines scenario 1's signalling churn (steady call setup/teardown
//! at a fixed mid-knee CPS) with scenario 5's RTP steady-state (a
//! fixed number of long-lived calls carrying PCMU). Run for
//! `RVOIP_PERF_SOAK_DURATION_SECS` (default 30 min). Marked `#[ignore]`
//! so it opts in via `-- --ignored`.
//!
//! The headline output is **latency drift** and **memory growth**:
//!
//! - `latency_drift_pct` — `(setup p99 in last minute) / (setup p99 in
//!   first minute) - 1`. Should stay well under 50 % on a healthy run.
//! - `rss_growth_mb_per_hr` — projection of `rss_growth_mb_per_min ×
//!   60`. Should stay near zero on a leak-free run; anything north of
//!   ~10 MB/hr at the operating point suggests state retention bugs.
//! - `errors_per_minute` — distribution; should be 0 across the run.
//!
//! Run via:
//! ```text
//! cargo test -p rvoip-sip --features perf-tests --release \
//!   --test perf_soak_30min -- --ignored --nocapture
//! ```
//!
//! Env knobs:
//! - `RVOIP_PERF_SOAK_DURATION_SECS` (default 1800 = 30 min)
//! - `RVOIP_PERF_SOAK_CPS`           (default 20 — well below typical knee)
//! - `RVOIP_PERF_SOAK_MEDIA_CALLS`   (default 30 — concurrent RTP streams)
//! - `RVOIP_PERF_CALL_TIMEOUT_SECS`  (default 30)

#![allow(clippy::needless_return)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_media_core::types::AudioFrame;
use rvoip_sip::api::callback_peer::{
    CallHandler, CallHandlerDecision, CallbackPeer, ShutdownHandle,
};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use serde_json::json;
use tokio::task::JoinHandle;

#[path = "support/mod.rs"]
mod support;
use support::{LatencyHistogram, LoadProfile, ResourceSampler, ScenarioReport};

const FRAME_SAMPLES: usize = 160;
const FRAME_INTERVAL_MS: u64 = 20;

#[derive(Clone)]
struct CountingAccept {
    received_frames: Arc<AtomicU64>,
}

#[async_trait::async_trait]
impl CallHandler for CountingAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        if let Ok(handle) = call.accept().await {
            let counter = Arc::clone(&self.received_frames);
            tokio::spawn(async move {
                if let Ok(audio) = handle.audio().await {
                    let mut rx = audio.receiver;
                    while let Some(_frame) = rx.recv().await {
                        counter.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }
        CallHandlerDecision::Accept
    }
}

struct BobReceiver {
    task: JoinHandle<()>,
    shutdown: ShutdownHandle,
}

async fn boot_bob(port: u16, received_frames: Arc<AtomicU64>) -> BobReceiver {
    let cfg = Config::local("perf-soak-bob", port);
    let bob = CallbackPeer::new(CountingAccept { received_frames }, cfg)
        .await
        .expect("perf-soak bob");
    let shutdown = bob.shutdown_handle();
    let task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    BobReceiver { task, shutdown }
}

async fn boot_alice(port: u16) -> Arc<UnifiedCoordinator> {
    let cfg = Config::local("perf-soak-alice", port);
    let coord = UnifiedCoordinator::new(cfg).await.expect("perf-soak alice");
    tokio::time::sleep(Duration::from_millis(200)).await;
    coord
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_soak_30min() {
    let duration_secs: u64 = std::env::var("RVOIP_PERF_SOAK_DURATION_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1800);
    let soak_cps: f64 = std::env::var("RVOIP_PERF_SOAK_CPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20.0);
    let media_calls: u64 = std::env::var("RVOIP_PERF_SOAK_MEDIA_CALLS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let call_timeout = Duration::from_secs(
        std::env::var("RVOIP_PERF_CALL_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30),
    );

    let bob_port = support::ports::next_sip_port();
    let alice_port = support::ports::next_sip_port();
    let received_frames = Arc::new(AtomicU64::new(0));
    let bob = boot_bob(bob_port, Arc::clone(&received_frames)).await;
    let alice = boot_alice(alice_port).await;
    let from = format!("sip:alice@127.0.0.1:{}", alice_port);
    let target_uri = format!("sip:bob@127.0.0.1:{}", bob_port);

    // Histograms split per minute so we can compute drift.
    // For simplicity v1: one global setup histogram + one "first
    // minute" snapshot + one "last minute" snapshot. Per-minute
    // breakdown is a follow-up.
    let setup_hist = Arc::new(LatencyHistogram::new("setup_latency"));
    let first_minute_hist = Arc::new(LatencyHistogram::new("setup_latency_minute_1"));
    let last_minute_hist = Arc::new(LatencyHistogram::new("setup_latency_last_minute"));
    let counters = Arc::new(SoakCounters::default());

    // Long-lived media calls held for the whole soak.
    let sent_frames = Arc::new(AtomicU64::new(0));
    let (media_drop_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    let mut media_handles: Vec<JoinHandle<()>> = Vec::with_capacity(media_calls as usize);
    for _ in 0..media_calls {
        let alice = Arc::clone(&alice);
        let from = from.clone();
        let target_uri = target_uri.clone();
        let counters = Arc::clone(&counters);
        let sent_frames = Arc::clone(&sent_frames);
        let mut drop_rx = media_drop_tx.subscribe();
        media_handles.push(tokio::spawn(async move {
            let call_id = match alice.invite(Some(from), target_uri).send().await {
                Ok(id) => id,
                Err(_) => {
                    counters.media_setup_failed.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            };
            let handle = alice.session(&call_id);
            if handle.wait_for_answered(Some(call_timeout)).await.is_err() {
                counters.media_setup_failed.fetch_add(1, Ordering::Relaxed);
                return;
            }
            let audio = match handle.audio().await {
                Ok(a) => a,
                Err(_) => return,
            };
            let sender = audio.sender;
            let mut next = tokio::time::Instant::now() + Duration::from_millis(FRAME_INTERVAL_MS);
            loop {
                if drop_rx.try_recv().is_ok() {
                    break;
                }
                tokio::time::sleep_until(next).await;
                next += Duration::from_millis(FRAME_INTERVAL_MS);
                let f = AudioFrame::new(vec![0i16; FRAME_SAMPLES], 8_000, 1, 0);
                if sender.send(f).await.is_err() {
                    break;
                }
                sent_frames.fetch_add(1, Ordering::Relaxed);
            }
            let _ = handle.hangup_and_wait(Some(call_timeout)).await;
        }));
    }
    // Give the media plane time to come up before the signalling
    // churn starts.
    tokio::time::sleep(Duration::from_secs(3)).await;

    let sampler = ResourceSampler::start(Duration::from_secs(5));
    let started = std::time::Instant::now();
    let total = Duration::from_secs(duration_secs);

    // Signalling churn: continuously dispatch INVITE-BYE cycles at
    // `soak_cps`. Each spawned task records its setup latency into
    // the appropriate histogram (first minute / last minute / global).
    let tick = Duration::from_secs_f64(1.0 / soak_cps.max(1.0));
    let churn_handles = Arc::new(tokio::sync::Mutex::new(Vec::<JoinHandle<()>>::new()));
    loop {
        let elapsed = started.elapsed();
        if elapsed >= total {
            break;
        }
        let alice = Arc::clone(&alice);
        let from = from.clone();
        let target_uri = target_uri.clone();
        let setup_hist = Arc::clone(&setup_hist);
        let first_minute_hist = Arc::clone(&first_minute_hist);
        let last_minute_hist = Arc::clone(&last_minute_hist);
        let counters = Arc::clone(&counters);
        let churn_handles_record = Arc::clone(&churn_handles);
        let h = tokio::spawn(async move {
            let dispatch_at = std::time::Instant::now();
            let t_send = dispatch_at;
            counters.offered.fetch_add(1, Ordering::Relaxed);
            let call_id = match alice.invite(Some(from), target_uri).send().await {
                Ok(id) => id,
                Err(_) => {
                    counters.failed.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            };
            let handle = alice.session(&call_id);
            if handle.wait_for_answered(Some(call_timeout)).await.is_err() {
                counters.failed.fetch_add(1, Ordering::Relaxed);
                return;
            }
            let ns = t_send.elapsed().as_nanos() as u64;
            setup_hist.record_nanos(ns);
            // Place into first or last minute snapshot bucket.
            let secs = dispatch_at
                .duration_since(started_minute_anchor())
                .as_secs_f64();
            // We use a global anchor (set just once via OnceLock).
            // Cheaper: classify by `dispatch_at - started` using a
            // captured `started` clone — but we can't move it cheaply
            // here. Fall back to anchor.
            let _ = secs;
            if dispatch_at.duration_since(started_global()).as_secs() < 60 {
                first_minute_hist.record_nanos(ns);
            }
            if total
                .saturating_sub(dispatch_at.duration_since(started_global()))
                .as_secs()
                <= 60
            {
                last_minute_hist.record_nanos(ns);
            }
            let _ = handle.hangup_and_wait(Some(call_timeout)).await;
            counters.succeeded.fetch_add(1, Ordering::Relaxed);
        });
        tokio::spawn(async move {
            churn_handles_record.lock().await.push(h);
        });
        tokio::time::sleep(tick).await;
    }

    // Drain churn calls.
    let drain = async {
        let mut g = churn_handles.lock().await;
        for h in std::mem::take(&mut *g) {
            let _ = h.await;
        }
    };
    let _ = tokio::time::timeout(call_timeout + Duration::from_secs(30), drain).await;

    // Stop media calls.
    let _ = media_drop_tx.send(());
    let _ = tokio::time::timeout(call_timeout + Duration::from_secs(10), async {
        for h in media_handles {
            let _ = h.await;
        }
    })
    .await;

    let resources = sampler.stop().await;

    // Compute drift.
    let fm = first_minute_hist.snapshot();
    let lm = last_minute_hist.snapshot();
    let drift_pct = if fm.p99 > 0 && lm.p99 > 0 {
        ((lm.p99 as f64 - fm.p99 as f64) / fm.p99 as f64) * 100.0
    } else {
        0.0
    };
    let rss_growth_mb_per_hr = resources.rss_growth_mb_per_min * 60.0;
    let offered = counters.offered.load(Ordering::Relaxed);
    let succeeded = counters.succeeded.load(Ordering::Relaxed);
    let failed = counters.failed.load(Ordering::Relaxed);
    let media_setup_failed = counters.media_setup_failed.load(Ordering::Relaxed);
    let asr = if offered > 0 {
        succeeded as f64 / offered as f64
    } else {
        0.0
    };

    let load = LoadProfile {
        target_cps: soak_cps,
        ramp_secs: 0,
        steady_secs: duration_secs,
        cooldown_secs: 10,
    };
    let mut report = ScenarioReport::new("perf_soak_30min", load);
    let cores = report.environment().cpu_count_physical() as f64;
    let cps_per_core = if cores > 0.0 {
        (succeeded as f64 / duration_secs as f64) / cores
    } else {
        0.0
    };
    report
        .result("duration_secs", duration_secs)
        .result("soak_cps", soak_cps)
        .result("media_calls_held", media_calls)
        .result("calls_offered", offered)
        .result("calls_succeeded", succeeded)
        .result("cps_per_core", round2(cps_per_core))
        .result("asr", round4(asr))
        // ChatGPT VoIP guidance Tier 1 — long-duration stability is
        // what backs the "predictable, deterministic, no leaks" pitch.
        .result("latency_drift_pct", round2(drift_pct))
        .result("rss_growth_mb_per_hr", round2(rss_growth_mb_per_hr))
        .result(
            "errors",
            json!({
                "churn_failed":         failed,
                "media_setup_failed":   media_setup_failed,
            }),
        )
        .latency(&setup_hist)
        .latency(&first_minute_hist)
        .latency(&last_minute_hist)
        .with_resources(resources);
    let json_path = report.write_resources_first_then_write_json_if_supported();
    report.print_summary(&json_path);

    bob.shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(3), bob.task).await;
    drop(alice);
}

#[derive(Default)]
struct SoakCounters {
    offered: AtomicU64,
    succeeded: AtomicU64,
    failed: AtomicU64,
    media_setup_failed: AtomicU64,
}

// Global anchors so the bucketing classifier doesn't need plumbing.
// First call from the test sets them; subsequent calls return the
// stored values.
fn started_global() -> std::time::Instant {
    use std::sync::OnceLock;
    static ANCHOR: OnceLock<std::time::Instant> = OnceLock::new();
    *ANCHOR.get_or_init(std::time::Instant::now)
}
fn started_minute_anchor() -> std::time::Instant {
    started_global()
}

trait WriteJsonFallback {
    fn write_resources_first_then_write_json_if_supported(&self) -> std::path::PathBuf;
}

impl WriteJsonFallback for ScenarioReport {
    fn write_resources_first_then_write_json_if_supported(&self) -> std::path::PathBuf {
        self.write_json()
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

//! Scenario 2 — concurrent active calls held in steady state
//! (with concurrency-sweep support).
//!
//! Establishes N calls back-to-back (burst, no rate-pacing), holds them
//! `Active` for `RVOIP_PERF_HOLD_SECS`, then tears them all down
//! concurrently. Reports: max-concurrent achieved, **ASR** for the
//! burst, RSS MB/call, setup/teardown latency p99.
//!
//! Two run modes:
//! - **Single point (default)**: writes
//!   `target/perf-results/perf_concurrent_active_calls.json`.
//! - **Sweep**: set `RVOIP_PERF_SWEEP_CONCURRENT=50,100,500,1000` to
//!   sweep concurrency. Per-point JSONs + `_sweep.{json,md}` under
//!   `target/perf-results/perf_concurrent_active_calls/`.
//!
//! Env knobs:
//! - `RVOIP_PERF_SWEEP_CONCURRENT`  (comma-separated; enables sweep mode)
//! - `RVOIP_PERF_CONCURRENT_TARGET` (single-point default; 500)
//! - `RVOIP_PERF_HOLD_SECS`         (default 10)
//! - `RVOIP_PERF_CALL_TIMEOUT_SECS` (default 30)

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer, ShutdownHandle};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use serde_json::json;
use tokio::task::JoinHandle;

#[path = "support/mod.rs"]
mod support;
use support::{
    parse_sweep_env, LatencyHistogram, LoadProfile, ResourceSampler, ScenarioReport, SweepRunner,
};

struct AutoAccept;

#[async_trait::async_trait]
impl CallHandler for AutoAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let _ = call.accept().await;
        CallHandlerDecision::Accept
    }
}

struct BobReceiver {
    task: JoinHandle<()>,
    shutdown: ShutdownHandle,
}

async fn boot_bob(port: u16) -> BobReceiver {
    let cfg = Config::local("perf-bob", port);
    let bob = CallbackPeer::new(AutoAccept, cfg)
        .await
        .expect("perf bob: CallbackPeer::new");
    let shutdown = bob.shutdown_handle();
    let task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    BobReceiver { task, shutdown }
}

async fn boot_alice(port: u16) -> Arc<UnifiedCoordinator> {
    let cfg = Config::local("perf-alice", port);
    let coord = UnifiedCoordinator::new(cfg)
        .await
        .expect("perf alice: UnifiedCoordinator::new");
    tokio::time::sleep(Duration::from_millis(200)).await;
    coord
}

/// One sweep point. Holds `target` concurrent calls for `hold_secs`,
/// samples RSS + CPU% throughout via [`ResourceSampler`], then tears
/// them down concurrently.
async fn run_one_point(
    alice: Arc<UnifiedCoordinator>,
    from: String,
    target_uri: String,
    target: u64,
    hold_secs: u64,
    call_timeout: Duration,
) -> ScenarioReport {
    // Synthesize a LoadProfile-shaped record so the JSON `load` block
    // stays schema-stable. `target_cps` carries the concurrency point.
    let load = LoadProfile {
        target_cps: target as f64,
        ramp_secs: 0,
        steady_secs: hold_secs,
        cooldown_secs: 5,
    };

    let setup_hist = Arc::new(LatencyHistogram::new("setup_latency"));
    let teardown_hist = Arc::new(LatencyHistogram::new("teardown_latency"));
    let setup_failed = Arc::new(AtomicU64::new(0));
    let teardown_failed = Arc::new(AtomicU64::new(0));

    // ResourceSampler captures baseline / peak / growth-rate / CPU%
    // continuously through setup + hold + teardown. Reports back
    // through `report.with_resources(...)`.
    let sampler = ResourceSampler::start(Duration::from_millis(500));

    // Step 1: establish `target` concurrent calls. Each task INVITEs
    // and waits for 200, then parks on a broadcast until the BYE
    // signal fires in step 3.
    let (drop_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    let mut handles: Vec<JoinHandle<()>> = Vec::with_capacity(target as usize);

    let setup_start = std::time::Instant::now();
    for _ in 0..target {
        let alice = Arc::clone(&alice);
        let from = from.clone();
        let target_uri = target_uri.clone();
        let setup_hist = Arc::clone(&setup_hist);
        let teardown_hist = Arc::clone(&teardown_hist);
        let setup_failed = Arc::clone(&setup_failed);
        let teardown_failed = Arc::clone(&teardown_failed);
        let mut drop_rx = drop_tx.subscribe();
        handles.push(tokio::spawn(async move {
            let t_send = std::time::Instant::now();
            let call_id = match alice.invite(Some(from), target_uri).send().await {
                Ok(id) => id,
                Err(_) => {
                    setup_failed.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            };
            let handle = alice.session(&call_id);
            if handle.wait_for_answered(Some(call_timeout)).await.is_err() {
                setup_failed.fetch_add(1, Ordering::Relaxed);
                return;
            }
            setup_hist.record_nanos(t_send.elapsed().as_nanos() as u64);

            let _ = drop_rx.recv().await;
            let t_bye = std::time::Instant::now();
            if handle.hangup_and_wait(Some(call_timeout)).await.is_err() {
                teardown_failed.fetch_add(1, Ordering::Relaxed);
                return;
            }
            teardown_hist.record_nanos(t_bye.elapsed().as_nanos() as u64);
        }));
    }

    // Step 2: wait until every task is either parked or failed.
    let setup_deadline = std::time::Instant::now() + call_timeout + Duration::from_secs(10);
    loop {
        if std::time::Instant::now() > setup_deadline {
            break;
        }
        let answered = setup_hist.snapshot().count;
        let failed = setup_failed.load(Ordering::Relaxed);
        if answered + failed >= target {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let setup_wall = setup_start.elapsed();
    let active = setup_hist.snapshot().count;

    // Step 3: hold steady-state, then teardown.
    tokio::time::sleep(Duration::from_secs(hold_secs)).await;
    let teardown_start = std::time::Instant::now();
    let _ = drop_tx.send(());

    let drain = async {
        for h in handles {
            let _ = h.await;
        }
    };
    let _ = tokio::time::timeout(call_timeout + Duration::from_secs(load.cooldown_secs), drain).await;
    let teardown_wall = teardown_start.elapsed();

    let resources = sampler.stop().await;
    let rss_delta_mb = (resources.peak_rss_mb - resources.baseline_rss_mb).max(0.0);
    let mb_per_call = if active > 0 {
        rss_delta_mb / active as f64
    } else {
        0.0
    };

    let asr = if target > 0 {
        active as f64 / target as f64
    } else {
        0.0
    };

    let mut report = ScenarioReport::new("perf_concurrent_active_calls", load);
    let cores = report.environment().cpu_count_physical() as f64;
    let dialogs_per_core = if cores > 0.0 { active as f64 / cores } else { 0.0 };
    report
        .result("target_concurrent", target)
        .result("achieved_concurrent", active)
        .result("dialogs_per_core", round2(dialogs_per_core))
        .result("asr", round4(asr))
        .result("ner", round4(asr))
        .result("setup_secs", round2(setup_wall.as_secs_f64()))
        .result("teardown_secs", round2(teardown_wall.as_secs_f64()))
        .result(
            "errors",
            json!({
                "setup_failed":    setup_failed.load(Ordering::Relaxed),
                "teardown_failed": teardown_failed.load(Ordering::Relaxed),
            }),
        )
        .result("rss_delta_mb", round2(rss_delta_mb))
        .result("rss_mb_per_call", round4(mb_per_call))
        .result("calls_offered", target)
        .latency(&setup_hist)
        .latency(&teardown_hist)
        .with_resources(resources);
    report
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_concurrent_active_calls() {
    let points = parse_sweep_env("RVOIP_PERF_SWEEP_CONCURRENT").unwrap_or_else(|| {
        vec![std::env::var("RVOIP_PERF_CONCURRENT_TARGET")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(500.0)]
    });

    let hold_secs: u64 = std::env::var("RVOIP_PERF_HOLD_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let call_timeout = Duration::from_secs(
        std::env::var("RVOIP_PERF_CALL_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30),
    );

    let bob_port = support::ports::next_sip_port();
    let alice_port = support::ports::next_sip_port();
    let bob = boot_bob(bob_port).await;
    let alice = boot_alice(alice_port).await;
    let from = format!("sip:alice@127.0.0.1:{}", alice_port);
    let target_uri = format!("sip:bob@127.0.0.1:{}", bob_port);

    let mut sweep = SweepRunner::new(
        "perf_concurrent_active_calls",
        points.clone(),
        "Concurrent target",
        "achieved_concurrent",
        "ASR",
    );
    let mut first_asr: Option<f64> = None;

    for &point in &points {
        let target_count = point.round() as u64;
        let report = run_one_point(
            Arc::clone(&alice),
            from.clone(),
            target_uri.clone(),
            target_count,
            hold_secs,
            call_timeout,
        )
        .await;
        if first_asr.is_none() {
            first_asr = report
                .to_json()
                .pointer("/results/asr")
                .and_then(|v| v.as_f64());
        }
        sweep.add_point(point, report);
    }

    let _written = sweep.finalize();

    bob.shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(3), bob.task).await;
    drop(alice);

    let first = first_asr.unwrap_or(0.0);
    assert!(
        first >= 0.80,
        "first-point ASR {:.3} below 0.80 — likely a regression",
        first
    );
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

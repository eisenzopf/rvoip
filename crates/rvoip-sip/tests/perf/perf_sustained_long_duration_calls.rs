//! Scenario 3.10 — sustained calls with realistic long duration.
//!
//! OpenSIPS' standard perf-test methodology uses a 30-second mean
//! call duration (calls held open, not back-to-back INVITE-BYE). This
//! exercises a different code-path mix than scenario 1: the dialog
//! table actually grows and stays large, and per-dialog timer
//! state accumulates.
//!
//! Reports the same headline as scenario 1 (CPS, ASR, latency
//! percentiles) plus `dialog_table_size_at_steady` (a sample taken
//! mid-window via `SessionStore::has_session` heuristic — see notes).
//!
//! Run via:
//! ```text
//! cargo test -p rvoip-sip --features perf-tests --release \
//!   --test perf_sustained_long_duration_calls -- --nocapture
//! ```
//!
//! Env knobs:
//! - `RVOIP_PERF_SWEEP_CPS`              (enables sweep mode)
//! - `RVOIP_PERF_TARGET_CPS`             (single-point default; 30)
//! - `RVOIP_PERF_LONG_CALL_DURATION_SECS` (default 30 — mean per call)
//! - `RVOIP_PERF_STEADY_SECS`            (default 60)
//! - `RVOIP_PERF_CALL_TIMEOUT_SECS`      (default 60)

#![allow(clippy::needless_return)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::callback_peer::{
    CallHandler, CallHandlerDecision, CallbackPeer, ShutdownHandle,
};
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

#[derive(Default)]
struct Counters {
    offered: AtomicU64,
    succeeded: AtomicU64,
    setup_failed: AtomicU64,
    bye_failed: AtomicU64,
    timeout: AtomicU64,
}

struct BobReceiver {
    task: JoinHandle<()>,
    shutdown: ShutdownHandle,
}

async fn boot_bob(port: u16) -> BobReceiver {
    let bob = CallbackPeer::new(AutoAccept, Config::local("perf-bob", port))
        .await
        .expect("perf bob");
    let shutdown = bob.shutdown_handle();
    let task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    BobReceiver { task, shutdown }
}

async fn boot_alice(port: u16) -> Arc<UnifiedCoordinator> {
    let coord = UnifiedCoordinator::new(Config::local("perf-alice", port))
        .await
        .expect("perf alice");
    tokio::time::sleep(Duration::from_millis(200)).await;
    coord
}

/// Held-call task: INVITE → 200 → ACK → sleep `call_duration ± jitter`
/// → BYE. Industry-standard load shape (matches OpenSIPS' 30-s test).
async fn run_held_call(
    alice: Arc<UnifiedCoordinator>,
    from: String,
    target: String,
    setup_hist: Arc<LatencyHistogram>,
    counters: Arc<Counters>,
    call_duration: Duration,
    call_timeout: Duration,
    jitter_seed: u64,
    active_dialogs: Arc<AtomicU64>,
) {
    counters.offered.fetch_add(1, Ordering::Relaxed);
    let t_send = std::time::Instant::now();
    let call_id = match alice.invite(Some(from), target).send().await {
        Ok(id) => id,
        Err(_) => {
            counters.setup_failed.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    let handle = alice.session(&call_id);
    if handle.wait_for_answered(Some(call_timeout)).await.is_err() {
        counters.setup_failed.fetch_add(1, Ordering::Relaxed);
        return;
    }
    setup_hist.record_nanos(t_send.elapsed().as_nanos() as u64);
    active_dialogs.fetch_add(1, Ordering::Relaxed);

    // Hold the call for duration ± ~10% jitter (deterministic via the
    // seed so two runs of the same scenario settings are reproducible).
    let jitter_pct = (jitter_seed % 21) as i64 - 10; // -10 .. +10
    let dur_ms = call_duration.as_millis() as i64;
    let adjusted = (dur_ms + (dur_ms * jitter_pct / 100)).max(100) as u64;
    tokio::time::sleep(Duration::from_millis(adjusted)).await;

    match handle.hangup_and_wait(Some(call_timeout)).await {
        Ok(_) => {
            counters.succeeded.fetch_add(1, Ordering::Relaxed);
        }
        Err(e) => {
            if matches!(e, rvoip_sip::SessionError::Timeout(_)) {
                counters.timeout.fetch_add(1, Ordering::Relaxed);
            } else {
                counters.bye_failed.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    active_dialogs.fetch_sub(1, Ordering::Relaxed);
}

async fn run_one_point(
    alice: Arc<UnifiedCoordinator>,
    from: String,
    target: String,
    load: LoadProfile,
    call_duration: Duration,
    call_timeout: Duration,
) -> ScenarioReport {
    let setup_hist = Arc::new(LatencyHistogram::new("setup_latency"));
    let counters = Arc::new(Counters::default());
    let handles = Arc::new(tokio::sync::Mutex::new(Vec::<JoinHandle<()>>::new()));
    let active_dialogs = Arc::new(AtomicU64::new(0));
    let dialog_samples = Arc::new(tokio::sync::Mutex::new(Vec::<u64>::new()));
    let sampler = ResourceSampler::start(Duration::from_millis(500));

    // Background sampler: take dialog count every 2 s. Mid-window
    // average is `dialog_table_size_at_steady`.
    let active_for_sampler = Arc::clone(&active_dialogs);
    let samples_for_task = Arc::clone(&dialog_samples);
    let sampler_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_for_task = Arc::clone(&sampler_stop);
    let dialog_sampler_task = tokio::spawn(async move {
        loop {
            if stop_for_task.load(Ordering::Relaxed) {
                break;
            }
            let n = active_for_sampler.load(Ordering::Relaxed);
            samples_for_task.lock().await.push(n);
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    let active_wall = {
        let alice = Arc::clone(&alice);
        let setup_hist = Arc::clone(&setup_hist);
        let counters = Arc::clone(&counters);
        let handles = Arc::clone(&handles);
        let active_dialogs = Arc::clone(&active_dialogs);
        load.run(move |seq| {
            let alice = Arc::clone(&alice);
            let setup_hist = Arc::clone(&setup_hist);
            let counters = Arc::clone(&counters);
            let handles = Arc::clone(&handles);
            let active_dialogs = Arc::clone(&active_dialogs);
            let from = from.clone();
            let target = target.clone();
            let h = tokio::spawn(async move {
                run_held_call(
                    alice,
                    from,
                    target,
                    setup_hist,
                    counters,
                    call_duration,
                    call_timeout,
                    seq,
                    active_dialogs,
                )
                .await;
            });
            let handles_for_record = Arc::clone(&handles);
            tokio::spawn(async move {
                handles_for_record.lock().await.push(h);
            });
        })
        .await
    };

    // Cooldown: wait long enough for the longest-lived call to finish
    // (call_duration + 10% jitter + a margin).
    let cooldown_budget = Duration::from_secs(load.cooldown_secs) + call_duration + call_timeout;
    let collected = {
        let mut g = handles.lock().await;
        std::mem::take(&mut *g)
    };
    let _ = tokio::time::timeout(cooldown_budget, async {
        for h in collected {
            let _ = h.await;
        }
    })
    .await;

    sampler_stop.store(true, Ordering::Relaxed);
    let _ = dialog_sampler_task.await;
    let resources = sampler.stop().await;

    let offered = counters.offered.load(Ordering::Relaxed);
    let succeeded = counters.succeeded.load(Ordering::Relaxed);
    let asr = if offered > 0 {
        succeeded as f64 / offered as f64
    } else {
        0.0
    };
    let achieved_cps = if active_wall.as_secs_f64() > 0.0 {
        succeeded as f64 / active_wall.as_secs_f64()
    } else {
        0.0
    };

    // dialog_table_size_at_steady = mean of samples in the middle 60%
    // of the run (excluding ramp-up and drain tails). For a short run
    // this just means the mean of all samples beyond the first one.
    let samples = dialog_samples.lock().await.clone();
    let mid_start = samples.len() / 5;
    let mid_end = samples.len().saturating_sub(samples.len() / 5);
    let dialog_size_steady: f64 = if mid_end > mid_start {
        let mid: f64 = samples[mid_start..mid_end].iter().map(|&v| v as f64).sum();
        mid / (mid_end - mid_start) as f64
    } else {
        0.0
    };
    let dialog_size_peak = samples.iter().copied().max().unwrap_or(0);

    let mut report = ScenarioReport::new("perf_sustained_long_duration_calls", load);
    let cores = report.environment().cpu_count_physical() as f64;
    let cps_per_core = if cores > 0.0 {
        achieved_cps / cores
    } else {
        0.0
    };
    report
        .result("achieved_cps", round2(achieved_cps))
        .result("cps_per_core", round2(cps_per_core))
        .result("asr", round4(asr))
        .result("ner", round4(asr))
        .result("calls_offered", offered)
        .result("calls_succeeded", succeeded)
        .result("call_duration_secs", call_duration.as_secs())
        .result("dialog_table_size_at_steady", round2(dialog_size_steady))
        .result("dialog_table_size_peak", dialog_size_peak)
        .result(
            "errors",
            json!({
                "setup_failed": counters.setup_failed.load(Ordering::Relaxed),
                "bye_failed":   counters.bye_failed.load(Ordering::Relaxed),
                "timeout":      counters.timeout.load(Ordering::Relaxed),
            }),
        )
        .latency(&setup_hist)
        .with_resources(resources);
    report
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_sustained_long_duration_calls() {
    let points = parse_sweep_env("RVOIP_PERF_SWEEP_CPS").unwrap_or_else(|| {
        vec![std::env::var("RVOIP_PERF_TARGET_CPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30.0)]
    });
    let call_duration = Duration::from_secs(
        std::env::var("RVOIP_PERF_LONG_CALL_DURATION_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30),
    );
    let per_call_timeout = Duration::from_secs(
        std::env::var("RVOIP_PERF_CALL_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60),
    );
    let default_steady = std::env::var("RVOIP_PERF_STEADY_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);

    let bob_port = support::ports::next_sip_port();
    let alice_port = support::ports::next_sip_port();
    let bob = boot_bob(bob_port).await;
    let alice = boot_alice(alice_port).await;
    let from = format!("sip:alice@127.0.0.1:{}", alice_port);
    let target = format!("sip:bob@127.0.0.1:{}", bob_port);

    let mut sweep = SweepRunner::new(
        "perf_sustained_long_duration_calls",
        points.clone(),
        "CPS target",
        "achieved_cps",
        "ASR",
    );

    for &point in &points {
        let load = LoadProfile::for_point(point, default_steady);
        let report = run_one_point(
            Arc::clone(&alice),
            from.clone(),
            target.clone(),
            load,
            call_duration,
            per_call_timeout,
        )
        .await;
        sweep.add_point(point, report);
    }

    let _written = sweep.finalize();

    bob.shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(3), bob.task).await;
    drop(alice);
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

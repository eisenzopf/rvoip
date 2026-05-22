//! Scenario 1 — sustained call-setup CPS (with concurrency-sweep support).
//!
//! Drives INVITE → 200 OK → ACK → BYE → 200 OK on loopback at the
//! offered CPS dictated by [`LoadProfile`]. Reports the **headline**
//! VoIP signalling number: **ASR** (Answer-Seizure Ratio, ITU E.411) at
//! the operating point, plus setup / full-cycle latency
//! p50/p95/p99/p99.9.
//!
//! Two run modes:
//!
//! - **Single point (default)**: `cargo test -p rvoip-sip --features
//!   perf-tests --test perf_call_setup_cps --release -- --nocapture`
//!   writes `target/perf-results/perf_call_setup_cps.json`.
//! - **Sweep** (industry pattern from OpenSIPS / Kamailio / SBC perf
//!   reports): set `RVOIP_PERF_SWEEP_CPS=10,50,100,500,1000` and the
//!   test loops once per point, sharing the booted peers. Writes
//!   per-point JSONs plus aggregated `_sweep.json` and a
//!   publication-ready `_sweep.md` table under
//!   `target/perf-results/perf_call_setup_cps/`.
//!
//! Env knobs:
//! - `RVOIP_PERF_SWEEP_CPS`     (comma-separated points; enables sweep mode)
//! - `RVOIP_PERF_TARGET_CPS`    (single-point default; 100)
//! - `RVOIP_PERF_RAMP_SECS`     (default 5)
//! - `RVOIP_PERF_STEADY_SECS`   (default 30)
//! - `RVOIP_PERF_COOLDOWN_SECS` (default 5)
//! - `RVOIP_PERF_CALL_TIMEOUT_SECS` (default 15) — per-call timeout
//!
//! See `docs/BENCHMARKING.md` for full interpretation.

#![allow(clippy::needless_return)]

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

/// Auto-accept handler — every inbound INVITE answered immediately
/// (no provisional 180; real PDD measurement is Phase 3's job).
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
    invite_send_failed: AtomicU64,
    answer_failed: AtomicU64,
    bye_failed: AtomicU64,
    timeout: AtomicU64,
}

struct BobReceiver {
    _coord: Arc<UnifiedCoordinator>,
    task: JoinHandle<()>,
    shutdown: ShutdownHandle,
}

async fn boot_bob(port: u16) -> BobReceiver {
    let cfg = Config::local("perf-bob", port);
    let bob = CallbackPeer::new(AutoAccept, cfg)
        .await
        .expect("perf bob: CallbackPeer::new");
    let coord = bob.coordinator().clone();
    let shutdown = bob.shutdown_handle();
    let task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    BobReceiver {
        _coord: coord,
        task,
        shutdown,
    }
}

async fn boot_alice(port: u16) -> Arc<UnifiedCoordinator> {
    let cfg = Config::local("perf-alice", port);
    let coord = UnifiedCoordinator::new(cfg)
        .await
        .expect("perf alice: UnifiedCoordinator::new");
    tokio::time::sleep(Duration::from_millis(200)).await;
    coord
}

async fn run_one_call(
    alice: Arc<UnifiedCoordinator>,
    from: String,
    target: String,
    setup_hist: Arc<LatencyHistogram>,
    full_hist: Arc<LatencyHistogram>,
    counters: Arc<Counters>,
    per_call_timeout: Duration,
) {
    counters.offered.fetch_add(1, Ordering::Relaxed);
    let t_send = std::time::Instant::now();

    let call_id = match alice.invite(Some(from), target).send().await {
        Ok(id) => id,
        Err(_) => {
            counters.invite_send_failed.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    let handle = alice.session(&call_id);

    match handle.wait_for_answered(Some(per_call_timeout)).await {
        Ok(_) => {
            setup_hist.record_nanos(t_send.elapsed().as_nanos() as u64);
        }
        Err(e) => {
            if matches!(e, rvoip_sip::SessionError::Timeout(_)) {
                counters.timeout.fetch_add(1, Ordering::Relaxed);
            } else {
                counters.answer_failed.fetch_add(1, Ordering::Relaxed);
            }
            return;
        }
    }

    match handle.hangup_and_wait(Some(per_call_timeout)).await {
        Ok(_) => {
            full_hist.record_nanos(t_send.elapsed().as_nanos() as u64);
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
}

/// One sweep point: fresh histograms + counters, run the load profile
/// once, return a populated `ScenarioReport`. Peers stay shared across
/// sweep points so the bind cost is paid once per test.
async fn run_one_point(
    alice: Arc<UnifiedCoordinator>,
    from: String,
    target: String,
    load: LoadProfile,
    per_call_timeout: Duration,
) -> ScenarioReport {
    let setup_hist = Arc::new(LatencyHistogram::new("setup_latency"));
    let full_hist = Arc::new(LatencyHistogram::new("full_cycle"));
    let counters = Arc::new(Counters::default());
    let handles = Arc::new(tokio::sync::Mutex::new(Vec::<JoinHandle<()>>::new()));

    // ChatGPT guidance §1.5.B + §1.5.C: sample CPU% + RSS every 500 ms
    // during the active phase so the report carries the leak indicator
    // (rss_growth_mb_per_min) and a populated avg_cpu_pct field.
    let sampler = ResourceSampler::start(Duration::from_millis(500));

    let active_wall = {
        let alice = Arc::clone(&alice);
        let setup_hist = Arc::clone(&setup_hist);
        let full_hist = Arc::clone(&full_hist);
        let counters = Arc::clone(&counters);
        let handles = Arc::clone(&handles);
        load.run(move |_seq| {
            let alice = Arc::clone(&alice);
            let setup_hist = Arc::clone(&setup_hist);
            let full_hist = Arc::clone(&full_hist);
            let counters = Arc::clone(&counters);
            let handles = Arc::clone(&handles);
            let from = from.clone();
            let target = target.clone();
            let h = tokio::spawn(async move {
                run_one_call(
                    alice,
                    from,
                    target,
                    setup_hist,
                    full_hist,
                    counters,
                    per_call_timeout,
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

    // Cooldown drain — outstanding calls must finish (or time out)
    // before we snapshot histograms for this point.
    let cooldown_budget = Duration::from_secs(load.cooldown_secs) + per_call_timeout;
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

    let mut report = ScenarioReport::new("perf_call_setup_cps", load);
    let cores = report.environment().cpu_count_physical() as f64;
    let cps_per_core = if cores > 0.0 { achieved_cps / cores } else { 0.0 };
    report
        .result("achieved_cps", round2(achieved_cps))
        .result("cps_per_core", round2(cps_per_core))
        .result("asr", round4(asr))
        // `ner` (Network Efficiency Ratio) excludes user-driven
        // rejections (busy / no-answer). With AutoAccept on the bob
        // side there are no user rejections in the denominator, so
        // NER == ASR here. The placeholder slot makes the JSON shape
        // forward-compatible with the user-rejection scenarios in
        // Phase 3 of the perf plan.
        .result("ner", round4(asr))
        .result("calls_offered", offered)
        .result("calls_succeeded", succeeded)
        .result(
            "errors",
            json!({
                "invite_send_failed": counters.invite_send_failed.load(Ordering::Relaxed),
                "answer_failed":      counters.answer_failed.load(Ordering::Relaxed),
                "bye_failed":         counters.bye_failed.load(Ordering::Relaxed),
                "timeout":            counters.timeout.load(Ordering::Relaxed),
            }),
        )
        .latency(&setup_hist)
        .latency(&full_hist)
        .with_resources(resources);
    report
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_call_setup_cps() {
    // Sweep points: env-driven list, or fall back to a single-point
    // run pinned at RVOIP_PERF_TARGET_CPS (default 100).
    let points = parse_sweep_env("RVOIP_PERF_SWEEP_CPS").unwrap_or_else(|| {
        vec![std::env::var("RVOIP_PERF_TARGET_CPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(100.0)]
    });

    let per_call_timeout = Duration::from_secs(
        std::env::var("RVOIP_PERF_CALL_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(15),
    );
    let default_steady = std::env::var("RVOIP_PERF_STEADY_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    let bob_port = support::ports::next_sip_port();
    let alice_port = support::ports::next_sip_port();
    let bob = boot_bob(bob_port).await;
    let alice = boot_alice(alice_port).await;
    let from = format!("sip:alice@127.0.0.1:{}", alice_port);
    let target = format!("sip:bob@127.0.0.1:{}", bob_port);

    let mut sweep = SweepRunner::new(
        "perf_call_setup_cps",
        points.clone(),
        "CPS target",
        "achieved_cps",
        "ASR",
    );
    let mut first_asr: Option<f64> = None;

    for &point in &points {
        let load = LoadProfile::for_point(point, default_steady);
        let report = run_one_point(
            Arc::clone(&alice),
            from.clone(),
            target.clone(),
            load,
            per_call_timeout,
        )
        .await;
        // Capture first-point ASR for the smoke acceptance assert below.
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

    // Smoke acceptance — only on the first point, the lightest load.
    // Sweep mode deliberately keeps ramping past the knee, so failing
    // the test at high points would mask the very degradation we want
    // to observe in the markdown table.
    let first = first_asr.unwrap_or(0.0);
    assert!(
        first >= 0.95,
        "first-point ASR {:.3} below 0.95 — likely a perf regression or env issue",
        first
    );
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

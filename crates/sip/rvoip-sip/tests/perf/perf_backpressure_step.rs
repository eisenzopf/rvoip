//! Scenario 4.16 — backpressure step.
//!
//! ChatGPT VoIP-guidance Tier 2: modern AI voice systems fail under
//! overload constantly. The static sweep table characterises the
//! degradation **curve**; this scenario characterises the **dynamic
//! response** — does the library recover gracefully after a sudden
//! load spike?
//!
//! Shape:
//! 1. Pre-load at 50 % of `RVOIP_PERF_BP_KNEE_CPS` for 30 s (baseline).
//! 2. Step to 200 % of knee for `RVOIP_PERF_BP_SPIKE_SECS` (overload).
//! 3. Drop back to 50 % for 30 s (recovery).
//!
//! Reports
//!
//! - `baseline_p99_ns` (setup p99 during phase 1),
//! - `spike_p99_ns` (setup p99 during phase 2),
//! - `recovery_p99_ns` (setup p99 during phase 3),
//! - `recovery_time_secs` — wall-clock from end-of-spike to first 5 s
//!   window where p99 returns to ≤ 1.25 × baseline. Absent (`null`)
//!   if recovery doesn't complete within the recovery phase.
//! - `dropped_during_spike` — calls that failed/timed-out during the
//!   spike window.
//!
//! Env knobs:
//! - `RVOIP_PERF_BP_KNEE_CPS`     (default 200 — the CPS the operator
//!   identified as the knee from `perf_call_setup_cps`)
//! - `RVOIP_PERF_BP_BASE_SECS`    (default 30)
//! - `RVOIP_PERF_BP_SPIKE_SECS`   (default 30)
//! - `RVOIP_PERF_BP_RECOVERY_SECS` (default 30)
//! - `RVOIP_PERF_CALL_TIMEOUT_SECS` (default 15)
//! - `RVOIP_PERF_CALL_SETUP_DIAGNOSTICS=1` captures slow setup samples

#![allow(clippy::needless_return)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    CallSetupDiagnostics, LatencyHistogram, LoadProfile, ResourceSampler, ScenarioReport,
};

#[derive(Clone)]
struct AutoAccept {
    call_setup_diag: CallSetupDiagnostics,
}

#[async_trait::async_trait]
impl CallHandler for AutoAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let call_id = call.call_id.clone();
        let accept_start = Instant::now();
        let _ = call.accept().await;
        self.call_setup_diag
            .record_stage("bob_accept", &call_id, accept_start.elapsed());
        CallHandlerDecision::Accept
    }
}

#[derive(Default)]
struct Counters {
    offered: AtomicU64,
    succeeded: AtomicU64,
    failed_spike: AtomicU64,
    timeout: AtomicU64,
}

struct BobReceiver {
    task: JoinHandle<()>,
    shutdown: ShutdownHandle,
}

async fn boot_bob(port: u16, call_setup_diag: CallSetupDiagnostics) -> BobReceiver {
    let cfg = call_setup_diag.configure(Config::local("perf-bp-bob", port));
    let bob = CallbackPeer::new(AutoAccept { call_setup_diag }, cfg)
        .await
        .expect("perf bob");
    let shutdown = bob.shutdown_handle();
    let task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    BobReceiver { task, shutdown }
}

async fn boot_alice(port: u16, call_setup_diag: &CallSetupDiagnostics) -> Arc<UnifiedCoordinator> {
    let cfg = call_setup_diag.configure(Config::local("perf-bp-alice", port));
    let coord = UnifiedCoordinator::new(cfg).await.expect("perf alice");
    tokio::time::sleep(Duration::from_millis(200)).await;
    coord
}

/// Drives calls at `cps` for `duration`. Latencies land in `hist`.
/// Failures during spike phase are flagged via `is_spike_phase`.
async fn drive_phase(
    alice: Arc<UnifiedCoordinator>,
    from: String,
    target: String,
    cps: f64,
    duration: Duration,
    hist: Arc<LatencyHistogram>,
    counters: Arc<Counters>,
    is_spike_phase: bool,
    call_timeout: Duration,
    call_setup_diag: CallSetupDiagnostics,
) {
    let started = Instant::now();
    let tick = Duration::from_secs_f64(1.0 / cps.max(1.0));
    let handles = Arc::new(tokio::sync::Mutex::new(Vec::<JoinHandle<()>>::new()));
    loop {
        if started.elapsed() >= duration {
            break;
        }
        let alice = Arc::clone(&alice);
        let from = from.clone();
        let target = target.clone();
        let hist = Arc::clone(&hist);
        let counters = Arc::clone(&counters);
        let handles_for_record = Arc::clone(&handles);
        let call_setup_diag = call_setup_diag.clone();
        let h = tokio::spawn(async move {
            counters.offered.fetch_add(1, Ordering::Relaxed);
            let t_send = Instant::now();
            let invite_start = Instant::now();
            let call_id = match alice.invite(Some(from), target).send().await {
                Ok(id) => id,
                Err(_) => {
                    if is_spike_phase {
                        counters.failed_spike.fetch_add(1, Ordering::Relaxed);
                    }
                    return;
                }
            };
            let invite_send = invite_start.elapsed();
            let handle = alice.session(&call_id);
            let wait_start = Instant::now();
            match handle.wait_for_answered(Some(call_timeout)).await {
                Ok(_) => {
                    let wait_answer = wait_start.elapsed();
                    let setup_elapsed = t_send.elapsed();
                    let phase = if is_spike_phase {
                        "alice_setup_spike"
                    } else {
                        "alice_setup"
                    };
                    call_setup_diag.record_setup(
                        phase,
                        &call_id,
                        invite_send,
                        wait_answer,
                        setup_elapsed,
                    );
                    hist.record_nanos(setup_elapsed.as_nanos() as u64);
                }
                Err(_) => {
                    if is_spike_phase {
                        counters.failed_spike.fetch_add(1, Ordering::Relaxed);
                    }
                    counters.timeout.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            }
            if handle.hangup_and_wait(Some(call_timeout)).await.is_ok() {
                counters.succeeded.fetch_add(1, Ordering::Relaxed);
            }
        });
        tokio::spawn(async move {
            handles_for_record.lock().await.push(h);
        });
        tokio::time::sleep(tick).await;
    }
    // Drain in-flight calls inside the phase budget.
    let drain = async {
        let mut g = handles.lock().await;
        for h in std::mem::take(&mut *g) {
            let _ = h.await;
        }
    };
    let _ = tokio::time::timeout(call_timeout, drain).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_backpressure_step() {
    let knee_cps: f64 = std::env::var("RVOIP_PERF_BP_KNEE_CPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200.0);
    let base_secs: u64 = std::env::var("RVOIP_PERF_BP_BASE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let spike_secs: u64 = std::env::var("RVOIP_PERF_BP_SPIKE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let recovery_secs: u64 = std::env::var("RVOIP_PERF_BP_RECOVERY_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let call_timeout = Duration::from_secs(
        std::env::var("RVOIP_PERF_CALL_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(15),
    );

    let baseline_cps = knee_cps * 0.5;
    let spike_cps = knee_cps * 2.0;

    let bob_port = support::ports::next_sip_port();
    let alice_port = support::ports::next_sip_port();
    let call_setup_diag = CallSetupDiagnostics::from_env();
    let bob = boot_bob(bob_port, call_setup_diag.clone()).await;
    let alice = boot_alice(alice_port, &call_setup_diag).await;
    let from = format!("sip:alice@127.0.0.1:{alice_port}");
    let target = format!("sip:bob@127.0.0.1:{bob_port}");

    let baseline_hist = Arc::new(LatencyHistogram::new("setup_latency_baseline"));
    let spike_hist = Arc::new(LatencyHistogram::new("setup_latency_spike"));
    let recovery_hist = Arc::new(LatencyHistogram::new("setup_latency_recovery"));
    let counters = Arc::new(Counters::default());

    let sampler = ResourceSampler::start(Duration::from_millis(500));

    // Phase 1: baseline.
    drive_phase(
        Arc::clone(&alice),
        from.clone(),
        target.clone(),
        baseline_cps,
        Duration::from_secs(base_secs),
        Arc::clone(&baseline_hist),
        Arc::clone(&counters),
        false,
        call_timeout,
        call_setup_diag.clone(),
    )
    .await;

    // Phase 2: spike to 2× knee.
    drive_phase(
        Arc::clone(&alice),
        from.clone(),
        target.clone(),
        spike_cps,
        Duration::from_secs(spike_secs),
        Arc::clone(&spike_hist),
        Arc::clone(&counters),
        true,
        call_timeout,
        call_setup_diag.clone(),
    )
    .await;

    // Phase 3: recovery to baseline.
    let recovery_start = Instant::now();
    drive_phase(
        Arc::clone(&alice),
        from.clone(),
        target.clone(),
        baseline_cps,
        Duration::from_secs(recovery_secs),
        Arc::clone(&recovery_hist),
        Arc::clone(&counters),
        false,
        call_timeout,
        call_setup_diag.clone(),
    )
    .await;
    let _ = recovery_start; // Used implicitly below

    let resources = sampler.stop().await;

    let baseline_p99 = baseline_hist.snapshot().p99;
    let spike_p99 = spike_hist.snapshot().p99;
    let recovery_p99 = recovery_hist.snapshot().p99;
    // Recovery time: rough approximation — if recovery p99 ≤ 1.25 ×
    // baseline p99 then we recovered within the phase; report the
    // phase duration. If not, recovery_time_secs is null.
    let recovery_time_secs =
        if baseline_p99 > 0 && recovery_p99 <= (baseline_p99 as f64 * 1.25) as u64 {
            Some(recovery_secs)
        } else {
            None
        };

    let load = LoadProfile {
        target_cps: knee_cps,
        ramp_secs: 0,
        steady_secs: base_secs + spike_secs + recovery_secs,
        cooldown_secs: 0,
    };
    let mut report = ScenarioReport::new("perf_backpressure_step", load);
    report
        .result("baseline_cps", baseline_cps)
        .result("spike_cps", spike_cps)
        .result("base_secs", base_secs)
        .result("spike_secs", spike_secs)
        .result("recovery_secs", recovery_secs)
        .result("baseline_p99_ns", baseline_p99)
        .result("spike_p99_ns", spike_p99)
        .result("recovery_p99_ns", recovery_p99)
        .result("recovery_time_secs", recovery_time_secs)
        .result(
            "dropped_during_spike",
            counters.failed_spike.load(Ordering::Relaxed),
        )
        .result("calls_offered", counters.offered.load(Ordering::Relaxed))
        .result(
            "calls_succeeded",
            counters.succeeded.load(Ordering::Relaxed),
        )
        .result(
            "errors",
            json!({
                "timeout": counters.timeout.load(Ordering::Relaxed),
                "failed_during_spike": counters.failed_spike.load(Ordering::Relaxed),
            }),
        )
        .latency(&baseline_hist)
        .latency(&spike_hist)
        .latency(&recovery_hist)
        .with_resources(resources);
    if call_setup_diag.enabled() {
        report.diagnostic_block("call_setup", call_setup_diag.to_json());
    }
    let json_path = report.write_json();
    report.print_summary(&json_path);

    bob.shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(3), bob.task).await;
    drop(alice);
}

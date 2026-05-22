//! Scenario 4.15 — contact-center transfers throughput.
//!
//! N established calls, a driver fires REFER (blind transfer)
//! operations at random calls at a configurable rate. Reports
//! transfer-completion latency p99 + per-op success rate.
//!
//! Simplified model: REFER is sent, we measure the round-trip to
//! receive the dialog-core acknowledgement (the 202 Accepted that
//! makes the REFER request "succeed"). Full transfer semantics
//! (REFER → NOTIFY chain → new dialog established at transferee)
//! involves multiple peers and a far more complex flow that belongs
//! in a dedicated functional test; the throughput metric this
//! scenario reports is the REFER dispatch + accept rate.
//!
//! Env knobs:
//! - `RVOIP_PERF_SWEEP_TRANSFER_CALLS` (enables sweep)
//! - `RVOIP_PERF_TRANSFER_CALLS`       (single-point default; 20)
//! - `RVOIP_PERF_TRANSFERS_PER_CALL`   (default 3)
//! - `RVOIP_PERF_TRANSFER_DURATION_SECS` (default 8)
//! - `RVOIP_PERF_CALL_TIMEOUT_SECS`    (default 15)

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
    let bob = CallbackPeer::new(AutoAccept, Config::local("perf-xfer-bob", port))
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
    let coord = UnifiedCoordinator::new(Config::local("perf-xfer-alice", port))
        .await
        .expect("perf alice");
    tokio::time::sleep(Duration::from_millis(200)).await;
    coord
}

async fn run_one_point(
    alice: Arc<UnifiedCoordinator>,
    from: String,
    target: String,
    transfer_target: String,
    n_agents: u64,
    transfers_per_call: u64,
    duration: Duration,
    call_timeout: Duration,
) -> ScenarioReport {
    let load = LoadProfile {
        target_cps: n_agents as f64,
        ramp_secs: 0,
        steady_secs: duration.as_secs(),
        cooldown_secs: 5,
    };
    let setup_hist = Arc::new(LatencyHistogram::new("setup_latency"));
    let transfer_hist = Arc::new(LatencyHistogram::new("transfer_completion"));
    let setup_failed = Arc::new(AtomicU64::new(0));
    let transfers_ok = Arc::new(AtomicU64::new(0));
    let transfers_fail = Arc::new(AtomicU64::new(0));

    let sampler = ResourceSampler::start(Duration::from_millis(500));

    let (drop_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    let mut handles: Vec<JoinHandle<()>> = Vec::with_capacity(n_agents as usize);

    for _ in 0..n_agents {
        let alice = Arc::clone(&alice);
        let from = from.clone();
        let target = target.clone();
        let transfer_target = transfer_target.clone();
        let setup_hist = Arc::clone(&setup_hist);
        let transfer_hist = Arc::clone(&transfer_hist);
        let setup_failed = Arc::clone(&setup_failed);
        let transfers_ok = Arc::clone(&transfers_ok);
        let transfers_fail = Arc::clone(&transfers_fail);
        let mut drop_rx = drop_tx.subscribe();
        let total_window = duration;
        handles.push(tokio::spawn(async move {
            let t_send = std::time::Instant::now();
            let call_id = match alice.invite(Some(from), target).send().await {
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

            // Fire `transfers_per_call` REFERs across the window.
            let stagger = if transfers_per_call > 0 {
                total_window / (transfers_per_call as u32 + 1)
            } else {
                total_window
            };
            for _ in 0..transfers_per_call {
                tokio::time::sleep(stagger).await;
                let t0 = std::time::Instant::now();
                match handle.refer(transfer_target.clone()).send().await {
                    Ok(_) => {
                        transfer_hist.record_nanos(t0.elapsed().as_nanos() as u64);
                        transfers_ok.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        transfers_fail.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            let _ = drop_rx.recv().await;
            let _ = handle.hangup_and_wait(Some(call_timeout)).await;
        }));
    }

    // Wait for setup convergence.
    let setup_deadline = std::time::Instant::now() + call_timeout + Duration::from_secs(10);
    loop {
        if std::time::Instant::now() > setup_deadline {
            break;
        }
        if setup_hist.snapshot().count + setup_failed.load(Ordering::Relaxed) >= n_agents {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let active = setup_hist.snapshot().count;

    tokio::time::sleep(duration).await;
    let _ = drop_tx.send(());

    let drain = async {
        for h in handles {
            let _ = h.await;
        }
    };
    let _ = tokio::time::timeout(call_timeout + Duration::from_secs(load.cooldown_secs), drain).await;

    let resources = sampler.stop().await;
    let ok = transfers_ok.load(Ordering::Relaxed);
    let fail = transfers_fail.load(Ordering::Relaxed);
    let total = ok + fail;
    let asr = if n_agents > 0 {
        active as f64 / n_agents as f64
    } else {
        0.0
    };
    let transfer_success_rate = if total > 0 {
        ok as f64 / total as f64
    } else {
        0.0
    };

    let mut report = ScenarioReport::new("perf_contact_center_transfers", load);
    let cores = report.environment().cpu_count_physical() as f64;
    let dialogs_per_core = if cores > 0.0 { active as f64 / cores } else { 0.0 };
    report
        .result("target_agents", n_agents)
        .result("achieved_agents", active)
        .result("dialogs_per_core", round2(dialogs_per_core))
        .result("asr", round4(asr))
        .result("ner", round4(asr))
        .result("transfers_offered", total)
        .result("transfers_succeeded", ok)
        .result("transfer_success_rate", round4(transfer_success_rate))
        .result(
            "errors",
            json!({
                "setup_failed":    setup_failed.load(Ordering::Relaxed),
                "transfer_failed": fail,
            }),
        )
        .result("calls_offered", n_agents)
        .latency(&setup_hist)
        .latency(&transfer_hist)
        .with_resources(resources);
    report
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_contact_center_transfers() {
    let points = parse_sweep_env("RVOIP_PERF_SWEEP_TRANSFER_CALLS").unwrap_or_else(|| {
        vec![std::env::var("RVOIP_PERF_TRANSFER_CALLS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(20.0)]
    });
    let transfers_per_call: u64 = std::env::var("RVOIP_PERF_TRANSFERS_PER_CALL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let duration_secs: u64 = std::env::var("RVOIP_PERF_TRANSFER_DURATION_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let call_timeout = Duration::from_secs(
        std::env::var("RVOIP_PERF_CALL_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(15),
    );

    let bob_port = support::ports::next_sip_port();
    let alice_port = support::ports::next_sip_port();
    let bob = boot_bob(bob_port).await;
    let alice = boot_alice(alice_port).await;
    let from = format!("sip:alice@127.0.0.1:{alice_port}");
    let target = format!("sip:bob@127.0.0.1:{bob_port}");
    let transfer_target = format!("sip:transfer-target@127.0.0.1:{bob_port}");

    let mut sweep = SweepRunner::new(
        "perf_contact_center_transfers",
        points.clone(),
        "Agents target",
        "achieved_agents",
        "ASR",
    );

    for &point in &points {
        let report = run_one_point(
            Arc::clone(&alice),
            from.clone(),
            target.clone(),
            transfer_target.clone(),
            point.round() as u64,
            transfers_per_call,
            Duration::from_secs(duration_secs),
            call_timeout,
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

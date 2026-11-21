//! Scenario 3.13 — B2BUA forwarding throughput.
//!
//! 3-peer setup: alice → b2bua → carol. Alice drives INVITEs at the
//! offered CPS; the B2BUA peer accepts each inbound call and forks an
//! outbound INVITE to carol. Reports alice-side setup CPS (the metric
//! Kamailio publishes as "B2BUA CPS" at ~2.5–3 k per instance for
//! OpenSIPS).
//!
//! The B2BUA handler used here is intentionally minimal: it accepts
//! the inbound leg, dispatches an independent outbound to carol via
//! its own coordinator, and lets both legs run to completion. A
//! real B2BUA bridges media and synchronises BYE — out of scope for
//! a throughput micro-benchmark.
//!
//! Env knobs:
//! - `RVOIP_PERF_SWEEP_B2BUA_CPS`  (enables sweep mode)
//! - `RVOIP_PERF_TARGET_CPS`       (single-point default; 30)
//! - `RVOIP_PERF_STEADY_SECS`      (default 30)
//! - `RVOIP_PERF_CALL_TIMEOUT_SECS` (default 15)

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

/// Carol-side handler: plain auto-accept.
struct AutoAccept;

#[async_trait::async_trait]
impl CallHandler for AutoAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let _ = call.accept().await;
        CallHandlerDecision::Accept
    }
}

/// B2BUA-side handler: on each inbound INVITE, accept it AND fire a
/// fresh INVITE on the outbound coordinator pointed at carol.
#[derive(Clone)]
struct ForwardingB2bua {
    outbound: Arc<UnifiedCoordinator>,
    outbound_target: String,
    outbound_from: String,
}

#[async_trait::async_trait]
impl CallHandler for ForwardingB2bua {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        // Accept the inbound leg.
        let _ = call.accept().await;
        // Fire-and-forget outbound leg to carol.
        let outbound = Arc::clone(&self.outbound);
        let from = self.outbound_from.clone();
        let target = self.outbound_target.clone();
        tokio::spawn(async move {
            let _ = outbound.invite(Some(from), target).send().await;
        });
        CallHandlerDecision::Accept
    }
}

#[derive(Default)]
struct Counters {
    offered: AtomicU64,
    succeeded: AtomicU64,
    failed: AtomicU64,
    timeout: AtomicU64,
}

struct PeerHandle {
    task: JoinHandle<()>,
    shutdown: ShutdownHandle,
}

async fn boot_carol(port: u16) -> PeerHandle {
    let p = CallbackPeer::new(AutoAccept, Config::local("perf-carol", port))
        .await
        .expect("carol");
    let shutdown = p.shutdown_handle();
    let task = tokio::spawn(async move {
        let _ = p.run().await;
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    PeerHandle { task, shutdown }
}

async fn boot_b2bua(b2bua_port: u16, outbound_target_port: u16) -> (PeerHandle, u16) {
    // The B2BUA peer's outbound is a separate UnifiedCoordinator on
    // its own port so its outbound INVITEs don't loop back into its
    // own inbound listener.
    let outbound_port = support::ports::next_sip_port();
    let outbound = UnifiedCoordinator::new(Config::local("perf-b2bua-out", outbound_port))
        .await
        .expect("b2bua outbound coord");
    let handler = ForwardingB2bua {
        outbound: Arc::clone(&outbound),
        outbound_from: format!("sip:b2bua@127.0.0.1:{outbound_port}"),
        outbound_target: format!("sip:carol@127.0.0.1:{outbound_target_port}"),
    };
    let p = CallbackPeer::new(handler, Config::local("perf-b2bua", b2bua_port))
        .await
        .expect("b2bua");
    let shutdown = p.shutdown_handle();
    let task = tokio::spawn(async move {
        let _ = p.run().await;
        drop(outbound);
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    (PeerHandle { task, shutdown }, outbound_port)
}

async fn boot_alice(port: u16) -> Arc<UnifiedCoordinator> {
    let coord = UnifiedCoordinator::new(Config::local("perf-alice", port))
        .await
        .expect("alice");
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
    call_timeout: Duration,
) {
    counters.offered.fetch_add(1, Ordering::Relaxed);
    let t_send = std::time::Instant::now();
    let call_id = match alice.invite(Some(from), target).send().await {
        Ok(id) => id,
        Err(_) => {
            counters.failed.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    let handle = alice.session(&call_id);
    if handle.wait_for_answered(Some(call_timeout)).await.is_err() {
        counters.timeout.fetch_add(1, Ordering::Relaxed);
        return;
    }
    setup_hist.record_nanos(t_send.elapsed().as_nanos() as u64);
    if handle.hangup_and_wait(Some(call_timeout)).await.is_ok() {
        full_hist.record_nanos(t_send.elapsed().as_nanos() as u64);
        counters.succeeded.fetch_add(1, Ordering::Relaxed);
    } else {
        counters.failed.fetch_add(1, Ordering::Relaxed);
    }
}

async fn run_one_point(
    alice: Arc<UnifiedCoordinator>,
    from: String,
    target: String,
    load: LoadProfile,
    call_timeout: Duration,
) -> ScenarioReport {
    let setup_hist = Arc::new(LatencyHistogram::new("setup_latency"));
    let full_hist = Arc::new(LatencyHistogram::new("full_cycle"));
    let counters = Arc::new(Counters::default());
    let handles = Arc::new(tokio::sync::Mutex::new(Vec::<JoinHandle<()>>::new()));
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
                    call_timeout,
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

    let cooldown_budget = Duration::from_secs(load.cooldown_secs) + call_timeout;
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

    let mut report = ScenarioReport::new("perf_b2bua_forwarding", load);
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
        .result(
            "errors",
            json!({
                "failed":  counters.failed.load(Ordering::Relaxed),
                "timeout": counters.timeout.load(Ordering::Relaxed),
            }),
        )
        .latency(&setup_hist)
        .latency(&full_hist)
        .with_resources(resources);
    report
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_b2bua_forwarding() {
    let points = parse_sweep_env("RVOIP_PERF_SWEEP_B2BUA_CPS").unwrap_or_else(|| {
        vec![std::env::var("RVOIP_PERF_TARGET_CPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30.0)]
    });
    let call_timeout = Duration::from_secs(
        std::env::var("RVOIP_PERF_CALL_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(15),
    );
    let default_steady = std::env::var("RVOIP_PERF_STEADY_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    let carol_port = support::ports::next_sip_port();
    let b2bua_port = support::ports::next_sip_port();
    let alice_port = support::ports::next_sip_port();

    let carol = boot_carol(carol_port).await;
    let (b2bua, _b2bua_out_port) = boot_b2bua(b2bua_port, carol_port).await;
    let alice = boot_alice(alice_port).await;
    let from = format!("sip:alice@127.0.0.1:{alice_port}");
    let target = format!("sip:b2bua@127.0.0.1:{b2bua_port}");

    let mut sweep = SweepRunner::new(
        "perf_b2bua_forwarding",
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
            call_timeout,
        )
        .await;
        sweep.add_point(point, report);
    }

    let _written = sweep.finalize();

    b2bua.shutdown.shutdown();
    carol.shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(3), b2bua.task).await;
    let _ = tokio::time::timeout(Duration::from_secs(3), carol.task).await;
    drop(alice);
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

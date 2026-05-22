//! Scenario 3.12 — mixed workload (calls + REGISTER refreshes).
//!
//! Carrier RFPs ask for performance under realistic cross-load: a real
//! PBX simultaneously handles call setups + REGISTER refreshes. This
//! scenario runs both at proportional rates and reports per-flow ASR
//! / RSR plus combined throughput.
//!
//! Single-axis scenarios miss the cross-load contention path
//! (e.g. shared transaction tables, allocator pressure across
//! signalling flows). The mixed-workload number is what
//! enterprise/carrier decision-makers actually require.
//!
//! Flow proportions (default 80/20 call/REG; the 10% mid-call slice
//! mentioned in the plan is deferred to a follow-up):
//! - 80% of offered ops are INVITE→BYE cycles
//! - 20% of offered ops are REGISTER refreshes
//!
//! Env knobs:
//! - `RVOIP_PERF_SWEEP_MIXED_OPS`  (enables sweep mode)
//! - `RVOIP_PERF_MIXED_OPS`        (single-point default; 50 ops/sec total)
//! - `RVOIP_PERF_CALL_RATIO_PCT`   (default 80 — share of ops that are calls)
//! - `RVOIP_PERF_STEADY_SECS`      (default 20)
//! - `RVOIP_PERF_CALL_TIMEOUT_SECS` (default 15)

#![allow(clippy::needless_return)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer, ShutdownHandle};
use rvoip_sip::api::events::Event;
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use serde_json::json;
use tokio::net::UdpSocket;
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
struct FlowCounters {
    offered: AtomicU64,
    succeeded: AtomicU64,
    failed: AtomicU64,
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

async fn boot_mock_registrar(port: u16) -> JoinHandle<()> {
    use rvoip_sip_core::parser::parse_message;
    use rvoip_sip_core::prelude::*;
    use rvoip_sip_core::types::header::HeaderName;
    use rvoip_sip_core::types::headers::HeaderValue;
    use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

    let sock = Arc::new(
        UdpSocket::bind(format!("127.0.0.1:{port}"))
            .await
            .expect("mock registrar bind"),
    );
    tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from) = match sock.recv_from(&mut buf).await {
                Ok(p) => p,
                Err(_) => return,
            };
            let parsed = match parse_message(&buf[..n]) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let req = match parsed {
                Message::Request(r) if r.method() == Method::Register => r,
                _ => continue,
            };
            let mut resp = create_response(&req, StatusCode::Ok);
            if let Some(c) = req.header(&HeaderName::Contact) {
                resp.headers.push(c.clone());
            }
            resp.headers.push(TypedHeader::Other(
                HeaderName::Expires,
                HeaderValue::Raw(b"3600".to_vec()),
            ));
            let _ = sock.send_to(&Message::Response(resp).to_bytes(), from).await;
        }
    })
}

async fn run_one_call(
    alice: Arc<UnifiedCoordinator>,
    from: String,
    target: String,
    setup_hist: Arc<LatencyHistogram>,
    counters: Arc<FlowCounters>,
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
        counters.succeeded.fetch_add(1, Ordering::Relaxed);
    } else {
        counters.failed.fetch_add(1, Ordering::Relaxed);
    }
}

async fn run_one_register(
    alice: Arc<UnifiedCoordinator>,
    registrar_uri: String,
    from_uri: String,
    contact_uri: String,
    latency: Arc<LatencyHistogram>,
    counters: Arc<FlowCounters>,
    reg_timeout: Duration,
) {
    counters.offered.fetch_add(1, Ordering::Relaxed);
    let mut events: EventReceiver = match alice.events().await {
        Ok(e) => e,
        Err(_) => {
            counters.failed.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    let t_send = std::time::Instant::now();
    let handle = alice
        .register(registrar_uri, "alice", "secret")
        .with_from_uri(from_uri)
        .with_contact_uri(contact_uri)
        .with_expires(3600)
        .send()
        .await;
    if handle.is_err() {
        counters.failed.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let waited = tokio::time::timeout(reg_timeout, async {
        loop {
            match events.next().await {
                Some(Event::RegistrationSuccess { .. }) => return true,
                Some(Event::RegistrationFailed { .. }) => return false,
                Some(_) => continue,
                None => return false,
            }
        }
    })
    .await;
    match waited {
        Ok(true) => {
            latency.record_nanos(t_send.elapsed().as_nanos() as u64);
            counters.succeeded.fetch_add(1, Ordering::Relaxed);
        }
        Ok(false) => {
            counters.failed.fetch_add(1, Ordering::Relaxed);
        }
        Err(_) => {
            counters.timeout.fetch_add(1, Ordering::Relaxed);
        }
    }
}

async fn run_one_point(
    alice: Arc<UnifiedCoordinator>,
    call_from: String,
    call_target: String,
    registrar_uri: String,
    alice_port: u16,
    total_ops_per_sec: f64,
    call_ratio_pct: u8,
    steady_secs: u64,
    call_timeout: Duration,
) -> ScenarioReport {
    let load = LoadProfile {
        target_cps: total_ops_per_sec,
        ramp_secs: 0,
        steady_secs,
        cooldown_secs: 5,
    };

    let call_hist = Arc::new(LatencyHistogram::new("setup_latency"));
    let reg_hist = Arc::new(LatencyHistogram::new("register_latency"));
    let call_counters = Arc::new(FlowCounters::default());
    let reg_counters = Arc::new(FlowCounters::default());
    let handles = Arc::new(tokio::sync::Mutex::new(Vec::<JoinHandle<()>>::new()));
    let sampler = ResourceSampler::start(Duration::from_millis(500));

    let active_wall = {
        let alice = Arc::clone(&alice);
        let call_hist = Arc::clone(&call_hist);
        let reg_hist = Arc::clone(&reg_hist);
        let call_counters = Arc::clone(&call_counters);
        let reg_counters = Arc::clone(&reg_counters);
        let handles = Arc::clone(&handles);
        let registrar_uri = registrar_uri.clone();
        load.run(move |seq| {
            // Deterministic mix: cycle 1..=100 each second; if cycle ≤
            // call_ratio_pct it's a call, otherwise REG.
            let bucket = (seq % 100) as u8;
            let is_call = bucket < call_ratio_pct;
            let alice = Arc::clone(&alice);
            let call_hist = Arc::clone(&call_hist);
            let reg_hist = Arc::clone(&reg_hist);
            let call_counters = Arc::clone(&call_counters);
            let reg_counters = Arc::clone(&reg_counters);
            let from = call_from.clone();
            let target = call_target.clone();
            let registrar_uri = registrar_uri.clone();
            let aor = format!("sip:user-{seq:08}@127.0.0.1:{alice_port}");
            let aor_contact = aor.clone();
            let handles_for_record = Arc::clone(&handles);
            let h = if is_call {
                tokio::spawn(async move {
                    run_one_call(alice, from, target, call_hist, call_counters, call_timeout)
                        .await;
                })
            } else {
                tokio::spawn(async move {
                    run_one_register(
                        alice,
                        registrar_uri,
                        aor,
                        aor_contact,
                        reg_hist,
                        reg_counters,
                        call_timeout,
                    )
                    .await;
                })
            };
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

    let call_offered = call_counters.offered.load(Ordering::Relaxed);
    let call_succeeded = call_counters.succeeded.load(Ordering::Relaxed);
    let asr = if call_offered > 0 {
        call_succeeded as f64 / call_offered as f64
    } else {
        0.0
    };
    let reg_offered = reg_counters.offered.load(Ordering::Relaxed);
    let reg_succeeded = reg_counters.succeeded.load(Ordering::Relaxed);
    let rsr = if reg_offered > 0 {
        reg_succeeded as f64 / reg_offered as f64
    } else {
        0.0
    };
    let achieved_cps = if active_wall.as_secs_f64() > 0.0 {
        (call_succeeded + reg_succeeded) as f64 / active_wall.as_secs_f64()
    } else {
        0.0
    };

    let mut report = ScenarioReport::new("perf_mixed_workload", load);
    let cores = report.environment().cpu_count_physical() as f64;
    let cps_per_core = if cores > 0.0 { achieved_cps / cores } else { 0.0 };
    report
        .result("call_ratio_pct", call_ratio_pct)
        .result("achieved_cps", round2(achieved_cps))
        .result("cps_per_core", round2(cps_per_core))
        .result("asr", round4(asr))
        .result("rsr", round4(rsr))
        .result("call_offered", call_offered)
        .result("call_succeeded", call_succeeded)
        .result("reg_offered", reg_offered)
        .result("reg_succeeded", reg_succeeded)
        .result(
            "errors",
            json!({
                "call_failed":   call_counters.failed.load(Ordering::Relaxed),
                "call_timeout":  call_counters.timeout.load(Ordering::Relaxed),
                "reg_failed":    reg_counters.failed.load(Ordering::Relaxed),
                "reg_timeout":   reg_counters.timeout.load(Ordering::Relaxed),
            }),
        )
        .latency(&call_hist)
        .latency(&reg_hist)
        .with_resources(resources);
    report
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_mixed_workload() {
    let points = parse_sweep_env("RVOIP_PERF_SWEEP_MIXED_OPS").unwrap_or_else(|| {
        vec![std::env::var("RVOIP_PERF_MIXED_OPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(50.0)]
    });
    let call_ratio_pct: u8 = std::env::var("RVOIP_PERF_CALL_RATIO_PCT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(80);
    let steady_secs: u64 = std::env::var("RVOIP_PERF_STEADY_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let call_timeout = Duration::from_secs(
        std::env::var("RVOIP_PERF_CALL_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(15),
    );

    let bob_port = support::ports::next_sip_port();
    let registrar_port = support::ports::next_sip_port();
    let alice_port = support::ports::next_sip_port();

    let bob = boot_bob(bob_port).await;
    let registrar_task = boot_mock_registrar(registrar_port).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let alice = boot_alice(alice_port).await;
    let call_from = format!("sip:alice@127.0.0.1:{}", alice_port);
    let call_target = format!("sip:bob@127.0.0.1:{}", bob_port);
    let registrar_uri = format!("sip:127.0.0.1:{registrar_port}");

    let mut sweep = SweepRunner::new(
        "perf_mixed_workload",
        points.clone(),
        "Total ops/sec",
        "achieved_cps",
        "ASR",
    );

    for &point in &points {
        let report = run_one_point(
            Arc::clone(&alice),
            call_from.clone(),
            call_target.clone(),
            registrar_uri.clone(),
            alice_port,
            point,
            call_ratio_pct,
            steady_secs,
            call_timeout,
        )
        .await;
        sweep.add_point(point, report);
    }

    let _written = sweep.finalize();

    registrar_task.abort();
    let _ = registrar_task.await;
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

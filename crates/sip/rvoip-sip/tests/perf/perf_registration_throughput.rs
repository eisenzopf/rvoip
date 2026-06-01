//! Scenario 3 — REGISTER throughput (with concurrency-sweep support).
//!
//! Drives REGISTER from `rvoip-sip` against a mock UDP registrar that
//! replies 200 OK + Contact + Expires (no auth challenge — see plan
//! Phase 3 for the 401-digest variant). Reports REGs/sec, **RSR**
//! (Register-Success Ratio — Kamailio's REG-side analogue of ASR) and
//! REGISTER latency p50/p95/p99/p99.9 measured client-side.
//!
//! Two run modes:
//! - **Single point (default)**: writes
//!   `target/perf-results/perf_registration_throughput.json`.
//! - **Sweep**: set `RVOIP_PERF_SWEEP_REG_RPS=10,50,100,500` to sweep
//!   offered REGs/sec; per-point JSONs + `_sweep.{json,md}` under
//!   `target/perf-results/perf_registration_throughput/`.
//!
//! Env knobs:
//! - `RVOIP_PERF_SWEEP_REG_RPS` (comma-separated; enables sweep mode)
//! - `RVOIP_PERF_TARGET_CPS`    (single-point default, reused; 100)
//! - `RVOIP_PERF_RAMP_SECS`     (default 3)
//! - `RVOIP_PERF_STEADY_SECS`   (default 20)
//! - `RVOIP_PERF_COOLDOWN_SECS` (default 5)
//! - `RVOIP_PERF_REG_TIMEOUT_SECS` (default 10) — per-REGISTER timeout

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::events::Event;
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

#[derive(Default)]
struct Counters {
    offered: AtomicU64,
    succeeded: AtomicU64,
    send_failed: AtomicU64,
    timeout: AtomicU64,
}

async fn boot_mock_registrar(port: u16, count: Arc<AtomicU64>) -> JoinHandle<()> {
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
            let request = match parsed {
                Message::Request(r) if r.method() == Method::Register => r,
                _ => continue,
            };
            count.fetch_add(1, Ordering::Relaxed);

            let mut resp = create_response(&request, StatusCode::Ok);
            if let Some(contact) = request.header(&HeaderName::Contact) {
                resp.headers.push(contact.clone());
            }
            resp.headers.push(TypedHeader::Other(
                HeaderName::Expires,
                HeaderValue::Raw(b"3600".to_vec()),
            ));
            let bytes_out = Message::Response(resp).to_bytes();
            let _ = sock.send_to(&bytes_out, from).await;
        }
    })
}

async fn run_one_register(
    alice: Arc<UnifiedCoordinator>,
    registrar_uri: String,
    from_uri: String,
    contact_uri: String,
    latency: Arc<LatencyHistogram>,
    counters: Arc<Counters>,
    timeout: Duration,
) {
    counters.offered.fetch_add(1, Ordering::Relaxed);
    let mut events: EventReceiver = match alice.events().await {
        Ok(e) => e,
        Err(_) => {
            counters.send_failed.fetch_add(1, Ordering::Relaxed);
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
    let _handle = match handle {
        Ok(h) => h,
        Err(_) => {
            counters.send_failed.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };

    let waited = tokio::time::timeout(timeout, async {
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
            counters.send_failed.fetch_add(1, Ordering::Relaxed);
        }
        Err(_) => {
            counters.timeout.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// One sweep point: fresh histograms + counters, run the offered REG
/// rate for the configured steady window. Returns the per-point report.
async fn run_one_point(
    alice: Arc<UnifiedCoordinator>,
    registrar_uri: String,
    alice_port: u16,
    server_count_baseline: u64,
    server_count: Arc<AtomicU64>,
    point_rps: f64,
    default_steady: u64,
    reg_timeout: Duration,
) -> ScenarioReport {
    let load = LoadProfile::for_point(point_rps, default_steady);
    let latency = Arc::new(LatencyHistogram::new("register_latency"));
    let counters = Arc::new(Counters::default());
    let handles = Arc::new(tokio::sync::Mutex::new(Vec::<JoinHandle<()>>::new()));
    let sampler = ResourceSampler::start(Duration::from_millis(500));

    let from_template = format!("sip:alice-%d@127.0.0.1:{alice_port}");
    let contact_template = format!("sip:alice-%d@127.0.0.1:{alice_port}");

    let active_wall = {
        let alice = Arc::clone(&alice);
        let latency = Arc::clone(&latency);
        let counters = Arc::clone(&counters);
        let handles = Arc::clone(&handles);
        let registrar_uri = registrar_uri.clone();
        load.run(move |seq| {
            let alice = Arc::clone(&alice);
            let latency = Arc::clone(&latency);
            let counters = Arc::clone(&counters);
            let handles_for_record = Arc::clone(&handles);
            let registrar_uri = registrar_uri.clone();
            let from = from_template.replace("%d", &seq.to_string());
            let contact = contact_template.replace("%d", &seq.to_string());
            let h = tokio::spawn(async move {
                run_one_register(
                    alice,
                    registrar_uri,
                    from,
                    contact,
                    latency,
                    counters,
                    reg_timeout,
                )
                .await;
            });
            tokio::spawn(async move {
                handles_for_record.lock().await.push(h);
            });
        })
        .await
    };

    let cooldown_budget = Duration::from_secs(load.cooldown_secs) + reg_timeout;
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
    let send_failed = counters.send_failed.load(Ordering::Relaxed);
    let timeout_count = counters.timeout.load(Ordering::Relaxed);
    let rsr = if offered > 0 {
        succeeded as f64 / offered as f64
    } else {
        0.0
    };
    let achieved_rps = if active_wall.as_secs_f64() > 0.0 {
        succeeded as f64 / active_wall.as_secs_f64()
    } else {
        0.0
    };
    let server_observed_delta = server_count
        .load(Ordering::Relaxed)
        .saturating_sub(server_count_baseline);

    let mut report = ScenarioReport::new("perf_registration_throughput", load);
    let cores = report.environment().cpu_count_physical() as f64;
    let regs_per_core_per_sec = if cores > 0.0 {
        achieved_rps / cores
    } else {
        0.0
    };
    report
        .result("achieved_rps", round2(achieved_rps))
        .result("regs_per_core_per_sec", round2(regs_per_core_per_sec))
        // RSR = Register-Success Ratio. Direct analogue of ASR for the
        // REG-side flow; Kamailio's perf docs report it as "complete
        // registrations / attempted registrations".
        .result("rsr", round4(rsr))
        .result("registers_offered", offered)
        .result("registers_succeeded", succeeded)
        .result("registers_observed_by_registrar", server_observed_delta)
        .result(
            "errors",
            json!({
                "send_failed":   send_failed,
                "timeout":       timeout_count,
            }),
        )
        .latency(&latency)
        .with_resources(resources);
    report
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_registration_throughput() {
    let points = parse_sweep_env("RVOIP_PERF_SWEEP_REG_RPS").unwrap_or_else(|| {
        vec![std::env::var("RVOIP_PERF_TARGET_CPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(100.0)]
    });

    let reg_timeout = Duration::from_secs(
        std::env::var("RVOIP_PERF_REG_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10),
    );
    let default_steady = std::env::var("RVOIP_PERF_STEADY_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);

    let registrar_port = support::ports::next_sip_port();
    let alice_port = support::ports::next_sip_port();
    let server_count = Arc::new(AtomicU64::new(0));
    let registrar_task = boot_mock_registrar(registrar_port, Arc::clone(&server_count)).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let cfg = Config::local("perf-reg-alice", alice_port);
    let alice = UnifiedCoordinator::new(cfg)
        .await
        .expect("perf alice: UnifiedCoordinator::new");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let registrar_uri = format!("sip:127.0.0.1:{registrar_port}");

    let mut sweep = SweepRunner::new(
        "perf_registration_throughput",
        points.clone(),
        "REG/s target",
        "achieved_rps",
        "RSR",
    );
    let mut first_rsr: Option<f64> = None;

    for &point in &points {
        let baseline = server_count.load(Ordering::Relaxed);
        let report = run_one_point(
            Arc::clone(&alice),
            registrar_uri.clone(),
            alice_port,
            baseline,
            Arc::clone(&server_count),
            point,
            default_steady,
            reg_timeout,
        )
        .await;
        if first_rsr.is_none() {
            first_rsr = report
                .to_json()
                .pointer("/results/rsr")
                .and_then(|v| v.as_f64());
        }
        sweep.add_point(point, report);
    }

    let _written = sweep.finalize();

    registrar_task.abort();
    let _ = registrar_task.await;
    drop(alice);

    let first = first_rsr.unwrap_or(0.0);
    assert!(
        first >= 0.95,
        "first-point RSR {:.3} below 0.95 — likely a perf regression or env issue",
        first
    );
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

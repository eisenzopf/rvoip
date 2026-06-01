//! Scenario 3.11 — registrar binding scale (REG refresh against
//! populated binding table).
//!
//! Kamailio publishes this as a distinct KPI from REGs/sec: how fast
//! can the registrar handle **refreshes** when the location table
//! already holds many bindings. Memory profile and lookup performance
//! dominate above ~50k bindings.
//!
//! This scenario:
//!
//! 1. Pre-populates the mock registrar with `RVOIP_PERF_BINDINGS`
//!    distinct AORs (one REGISTER per AOR).
//! 2. Drives **refresh** REGISTERs (same AORs again) at the configured
//!    rate.
//! 3. Reports refresh-RPS, refresh latency p99, and the binding table
//!    size at steady-state.
//!
//! Env knobs:
//! - `RVOIP_PERF_SWEEP_BINDINGS`   (sweeps binding count; e.g. 1000,10000)
//! - `RVOIP_PERF_BINDINGS`         (single-point default; 200)
//! - `RVOIP_PERF_REFRESH_RPS`      (default 100 — sustained refresh rate)
//! - `RVOIP_PERF_STEADY_SECS`      (default 20)

#![allow(clippy::needless_return)]

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
    failed: AtomicU64,
    timeout: AtomicU64,
}

/// Mock registrar that holds an in-memory binding table — needed so
/// the "table size" metric has meaning.
struct BindingTable {
    bindings: tokio::sync::Mutex<std::collections::HashMap<String, String>>,
}

async fn boot_mock_registrar(
    port: u16,
    count_in: Arc<AtomicU64>,
    table: Arc<BindingTable>,
) -> JoinHandle<()> {
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
            count_in.fetch_add(1, Ordering::Relaxed);

            // Track the binding (AOR → placeholder). The From URI
            // stands in for the AOR; the Contact value isn't surfaced
            // through a stable API here so we store a constant marker.
            if let Some(from_uri) = req.from().map(|f| f.uri.to_string()) {
                table
                    .bindings
                    .lock()
                    .await
                    .insert(from_uri, "<bound>".to_string());
            }

            let mut resp = create_response(&req, StatusCode::Ok);
            if let Some(c) = req.header(&HeaderName::Contact) {
                resp.headers.push(c.clone());
            }
            resp.headers.push(TypedHeader::Other(
                HeaderName::Expires,
                HeaderValue::Raw(b"3600".to_vec()),
            ));
            let _ = sock
                .send_to(&Message::Response(resp).to_bytes(), from)
                .await;
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
            counters.failed.fetch_add(1, Ordering::Relaxed);
        }
        Err(_) => {
            counters.timeout.fetch_add(1, Ordering::Relaxed);
        }
    }
}

async fn run_one_point(
    alice: Arc<UnifiedCoordinator>,
    registrar_uri: String,
    alice_port: u16,
    binding_count: u64,
    refresh_rps: f64,
    steady_secs: u64,
    reg_timeout: Duration,
    table: Arc<BindingTable>,
) -> ScenarioReport {
    let prepop_latency = Arc::new(LatencyHistogram::new("prepop_register_latency"));
    let refresh_latency = Arc::new(LatencyHistogram::new("register_latency"));
    let prepop_counters = Arc::new(Counters::default());
    let refresh_counters = Arc::new(Counters::default());

    let sampler = ResourceSampler::start(Duration::from_millis(500));

    // ----- Phase A: pre-populate the binding table. Sequential to
    // keep the load shape simple; we just need the table populated.
    for i in 0..binding_count {
        let from_uri = format!("sip:user-{i:08}@127.0.0.1:{alice_port}");
        let contact_uri = from_uri.clone();
        run_one_register(
            Arc::clone(&alice),
            registrar_uri.clone(),
            from_uri,
            contact_uri,
            Arc::clone(&prepop_latency),
            Arc::clone(&prepop_counters),
            reg_timeout,
        )
        .await;
    }
    let bindings_at_steady = table.bindings.lock().await.len() as u64;

    // ----- Phase B: sustained refresh load against the populated table.
    let load = LoadProfile {
        target_cps: refresh_rps,
        ramp_secs: 0,
        steady_secs,
        cooldown_secs: 5,
    };
    let handles = Arc::new(tokio::sync::Mutex::new(Vec::<JoinHandle<()>>::new()));
    let active_wall = {
        let alice = Arc::clone(&alice);
        let refresh_latency = Arc::clone(&refresh_latency);
        let refresh_counters = Arc::clone(&refresh_counters);
        let handles = Arc::clone(&handles);
        let registrar_uri = registrar_uri.clone();
        load.run(move |seq| {
            // Round-robin over the populated AORs.
            let aor_idx = seq % binding_count.max(1);
            let from_uri = format!("sip:user-{aor_idx:08}@127.0.0.1:{alice_port}");
            let contact_uri = from_uri.clone();
            let alice = Arc::clone(&alice);
            let refresh_latency = Arc::clone(&refresh_latency);
            let refresh_counters = Arc::clone(&refresh_counters);
            let registrar_uri = registrar_uri.clone();
            let handles_for_record = Arc::clone(&handles);
            let h = tokio::spawn(async move {
                run_one_register(
                    alice,
                    registrar_uri,
                    from_uri,
                    contact_uri,
                    refresh_latency,
                    refresh_counters,
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
    let refresh_offered = refresh_counters.offered.load(Ordering::Relaxed);
    let refresh_succeeded = refresh_counters.succeeded.load(Ordering::Relaxed);
    let rsr = if refresh_offered > 0 {
        refresh_succeeded as f64 / refresh_offered as f64
    } else {
        0.0
    };
    let achieved_rps = if active_wall.as_secs_f64() > 0.0 {
        refresh_succeeded as f64 / active_wall.as_secs_f64()
    } else {
        0.0
    };

    let mut report = ScenarioReport::new("perf_registrar_binding_scale", load);
    let cores = report.environment().cpu_count_physical() as f64;
    let regs_per_core_per_sec = if cores > 0.0 {
        achieved_rps / cores
    } else {
        0.0
    };
    report
        .result("binding_count_target", binding_count)
        .result("bindings_at_steady", bindings_at_steady)
        .result("achieved_rps", round2(achieved_rps))
        .result("regs_per_core_per_sec", round2(regs_per_core_per_sec))
        .result("rsr", round4(rsr))
        .result("refresh_offered", refresh_offered)
        .result("refresh_succeeded", refresh_succeeded)
        .result(
            "errors",
            json!({
                "prepop_failed": prepop_counters.failed.load(Ordering::Relaxed),
                "refresh_failed": refresh_counters.failed.load(Ordering::Relaxed),
                "refresh_timeout": refresh_counters.timeout.load(Ordering::Relaxed),
            }),
        )
        .latency(&prepop_latency)
        .latency(&refresh_latency)
        .with_resources(resources);
    report
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_registrar_binding_scale() {
    let points = parse_sweep_env("RVOIP_PERF_SWEEP_BINDINGS").unwrap_or_else(|| {
        vec![std::env::var("RVOIP_PERF_BINDINGS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(200.0)]
    });
    let refresh_rps: f64 = std::env::var("RVOIP_PERF_REFRESH_RPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100.0);
    let steady_secs: u64 = std::env::var("RVOIP_PERF_STEADY_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let reg_timeout = Duration::from_secs(
        std::env::var("RVOIP_PERF_REG_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10),
    );

    let registrar_port = support::ports::next_sip_port();
    let alice_port = support::ports::next_sip_port();
    let table = Arc::new(BindingTable {
        bindings: tokio::sync::Mutex::new(std::collections::HashMap::new()),
    });
    let count_in = Arc::new(AtomicU64::new(0));
    let registrar_task =
        boot_mock_registrar(registrar_port, Arc::clone(&count_in), Arc::clone(&table)).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let alice = UnifiedCoordinator::new(Config::local("perf-alice", alice_port))
        .await
        .expect("perf alice");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let registrar_uri = format!("sip:127.0.0.1:{registrar_port}");

    let mut sweep = SweepRunner::new(
        "perf_registrar_binding_scale",
        points.clone(),
        "Bindings target",
        "achieved_rps",
        "RSR",
    );

    for &point in &points {
        // Clear table between sweep points to keep per-point bindings
        // count accurate.
        table.bindings.lock().await.clear();
        let report = run_one_point(
            Arc::clone(&alice),
            registrar_uri.clone(),
            alice_port,
            point.round() as u64,
            refresh_rps,
            steady_secs,
            reg_timeout,
            Arc::clone(&table),
        )
        .await;
        sweep.add_point(point, report);
    }

    let _written = sweep.finalize();

    registrar_task.abort();
    let _ = registrar_task.await;
    drop(alice);
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

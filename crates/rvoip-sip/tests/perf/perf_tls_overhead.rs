//! Scenario 6 — TLS overhead.
//!
//! Scenario 1's INVITE→200→ACK→BYE→200 lifecycle, but over SIPS
//! (TLS) instead of plain UDP. Reports the same metrics plus a
//! `delta_vs_udp_baseline` block computed by reading the matching
//! scenario 1 JSON file from a previous run (if present).
//!
//! Requires the `dev-insecure-tls` feature (already wired in
//! Cargo.toml's `[[test]]` entry). Self-signed cert is generated via
//! rcgen at runtime and discarded when the test exits.
//!
//! Two run modes (single-point + sweep) per the standard harness.
//!
//! Env knobs:
//! - `RVOIP_PERF_SWEEP_TLS_CPS`    (enables sweep mode)
//! - `RVOIP_PERF_TARGET_CPS`       (reused as the single-point default)
//! - `RVOIP_PERF_STEADY_SECS`      (default 30 — same as scenario 1)
//! - `RVOIP_PERF_CALL_TIMEOUT_SECS` (default 15)
//!
//! Caveats: `dev-insecure-tls` is in use — the client side does
//! `tls_insecure_skip_verify=true`, so the handshake cost includes
//! everything *except* cert-chain validation. A real deployment with a
//! CA trust store will be marginally slower than these numbers show.

#![allow(clippy::needless_return)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer, ShutdownHandle};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::{Config, SipTlsMode, UnifiedCoordinator};
use serde_json::{json, Value};
use tokio::task::JoinHandle;

#[path = "support/mod.rs"]
mod support;
use support::{
    parse_sweep_env, LatencyHistogram, LoadProfile, ResourceSampler, ScenarioReport, SweepRunner,
};

/// Generate a self-signed cert valid for `localhost` + `127.0.0.1`.
/// Returned `TempDir` must outlive any peers using these paths.
fn write_self_signed_cert() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let cert_path = dir.path().join("server.crt");
    let key_path = dir.path().join("server.key");
    let cert = rcgen::generate_simple_self_signed(vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
    ])
    .expect("rcgen self-signed");
    std::fs::File::create(&cert_path)
        .and_then(|mut f| f.write_all(cert.serialize_pem().expect("cert PEM").as_bytes()))
        .expect("write cert");
    std::fs::File::create(&key_path)
        .and_then(|mut f| f.write_all(cert.serialize_private_key_pem().as_bytes()))
        .expect("write key");
    (dir, cert_path, key_path)
}

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
    task: JoinHandle<()>,
    shutdown: ShutdownHandle,
}

fn make_tls_config(
    name: &str,
    sip_port: u16,
    tls_port: u16,
    cert_path: &Path,
    key_path: &Path,
) -> Config {
    let mut cfg = Config::local(name, sip_port);
    cfg.sip_tls_mode = SipTlsMode::ClientAndServer;
    cfg.tls_bind_addr = Some(format!("127.0.0.1:{tls_port}").parse().expect("tls addr"));
    cfg.tls_cert_path = Some(cert_path.to_path_buf());
    cfg.tls_key_path = Some(key_path.to_path_buf());
    cfg.contact_uri = Some(format!("sips:{name}@127.0.0.1:{tls_port};transport=tls"));
    // dev-insecure-tls: skip server-cert validation. Cost reported here
    // therefore excludes cert-chain processing.
    cfg.tls_insecure_skip_verify = true;
    cfg
}

async fn boot_bob(sip_port: u16, tls_port: u16, cert_path: &Path, key_path: &Path) -> BobReceiver {
    let cfg = make_tls_config("perf-bob", sip_port, tls_port, cert_path, key_path);
    let bob = CallbackPeer::new(AutoAccept, cfg)
        .await
        .expect("perf bob: CallbackPeer::new (TLS)");
    let shutdown = bob.shutdown_handle();
    let task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    BobReceiver { task, shutdown }
}

async fn boot_alice(
    sip_port: u16,
    tls_port: u16,
    cert_path: &Path,
    key_path: &Path,
) -> Arc<UnifiedCoordinator> {
    let cfg = make_tls_config("perf-alice", sip_port, tls_port, cert_path, key_path);
    let coord = UnifiedCoordinator::new(cfg)
        .await
        .expect("perf alice: UnifiedCoordinator::new (TLS)");
    tokio::time::sleep(Duration::from_millis(250)).await;
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

    let mut report = ScenarioReport::new("perf_tls_overhead", load.clone());
    let cores = report.environment().cpu_count_physical() as f64;
    let cps_per_core = if cores > 0.0 { achieved_cps / cores } else { 0.0 };

    // Delta vs the matching UDP baseline (scenario 1). Reads
    // target/perf-results/perf_call_setup_cps[/<point>].json if it
    // exists; emits nulls otherwise so the JSON shape stays stable.
    let delta = read_udp_baseline_delta(load.target_cps, achieved_cps, &setup_hist);

    report
        .result("achieved_cps", round2(achieved_cps))
        .result("cps_per_core", round2(cps_per_core))
        .result("asr", round4(asr))
        .result("ner", round4(asr))
        .result("calls_offered", offered)
        .result("calls_succeeded", succeeded)
        .result("delta_vs_udp_baseline", delta)
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

/// Try to load the matching scenario 1 point's JSON. Returns a
/// `delta_vs_udp_baseline` block (or all nulls if the baseline isn't
/// on disk yet — running scenario 1 first populates it).
fn read_udp_baseline_delta(
    target_cps: f64,
    achieved_tls_cps: f64,
    setup_hist: &LatencyHistogram,
) -> Value {
    let target_int = target_cps as u64;
    let candidates = [
        // Sweep mode: scenario-dir/<point>.json
        format!("perf_call_setup_cps/{target_int}.json"),
        // Single-point mode: flat file
        "perf_call_setup_cps.json".to_string(),
    ];
    let target_dir = perf_target_dir();
    for c in &candidates {
        let p = target_dir.join(c);
        if let Ok(bytes) = std::fs::read(&p) {
            if let Ok(udp) = serde_json::from_slice::<Value>(&bytes) {
                let udp_cps = udp
                    .pointer("/results/achieved_cps")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let udp_setup_p99 = udp
                    .pointer("/latency_ns/setup_latency/p99")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let udp_peak_rss = udp
                    .pointer("/resources/peak_rss_mb")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let tls_setup_p99 = setup_hist.snapshot().p99;
                let cps_delta_pct = if udp_cps > 0.0 {
                    ((achieved_tls_cps - udp_cps) / udp_cps) * 100.0
                } else {
                    0.0
                };
                let p99_delta_pct = if udp_setup_p99 > 0 {
                    ((tls_setup_p99 as f64 - udp_setup_p99 as f64)
                        / udp_setup_p99 as f64)
                        * 100.0
                } else {
                    0.0
                };
                return json!({
                    "baseline_source": p.to_string_lossy(),
                    "udp_achieved_cps": udp_cps,
                    "udp_setup_p99_ns": udp_setup_p99,
                    "udp_peak_rss_mb": udp_peak_rss,
                    "tls_achieved_cps": achieved_tls_cps,
                    "tls_setup_p99_ns": tls_setup_p99,
                    "cps_delta_pct":   round2(cps_delta_pct),
                    "setup_p99_delta_pct": round2(p99_delta_pct),
                });
            }
        }
    }
    json!({
        "baseline_source": null,
        "note": "run perf_call_setup_cps at the same target_cps first to populate this block",
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_tls_overhead() {
    let points = parse_sweep_env("RVOIP_PERF_SWEEP_TLS_CPS").unwrap_or_else(|| {
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

    let (cert_dir, cert_path, key_path) = write_self_signed_cert();

    let bob_sip = support::ports::next_sip_port();
    let bob_tls = support::ports::next_sip_port();
    let alice_sip = support::ports::next_sip_port();
    let alice_tls = support::ports::next_sip_port();

    let bob = boot_bob(bob_sip, bob_tls, &cert_path, &key_path).await;
    let alice = boot_alice(alice_sip, alice_tls, &cert_path, &key_path).await;
    let from = format!("sips:alice@127.0.0.1:{alice_tls};transport=tls");
    let target = format!("sips:bob@127.0.0.1:{bob_tls};transport=tls");

    let mut sweep = SweepRunner::new(
        "perf_tls_overhead",
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
            per_call_timeout,
        )
        .await;
        sweep.add_point(point, report);
    }

    let _written = sweep.finalize();

    bob.shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(3), bob.task).await;
    drop(alice);
    drop(cert_dir); // Clean up the temp cert dir last.
}

fn perf_target_dir() -> PathBuf {
    let manifest = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"),
    );
    manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("target").join("perf-results"))
        .unwrap_or_else(|| PathBuf::from("target/perf-results"))
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

//! Scenario 7 — SRTP overhead.
//!
//! Scenario 5's RTP steady-state pattern with SDES SRTP enabled
//! (`Config::offer_srtp = true`, `srtp_required = true`). Reports the
//! same metrics plus a `delta_vs_plain_rtp_baseline` block read from a
//! previous scenario 5 run.
//!
//! Env knobs:
//! - `RVOIP_PERF_SWEEP_SRTP_CALLS`     (enables sweep mode)
//! - `RVOIP_PERF_RTP_CALLS`            (reused single-point default; 50)
//! - `RVOIP_PERF_RTP_DURATION_SECS`    (default 10)
//! - `RVOIP_PERF_CALL_TIMEOUT_SECS`    (default 30)

#![allow(clippy::needless_return)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_media_core::types::AudioFrame;
use rvoip_sip::api::callback_peer::{
    CallHandler, CallHandlerDecision, CallbackPeer, ShutdownHandle,
};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use serde_json::{json, Value};
use tokio::task::JoinHandle;

#[path = "support/mod.rs"]
mod support;
use support::{
    parse_sweep_env, LatencyHistogram, LoadProfile, ResourceSampler, ScenarioReport, SweepRunner,
};

const FRAME_SAMPLES: usize = 160;
const FRAME_INTERVAL_MS: u64 = 20;
const PPS_PER_STREAM: u64 = 50;

#[derive(Clone)]
struct CountingAccept {
    received_frames: Arc<AtomicU64>,
}

#[async_trait::async_trait]
impl CallHandler for CountingAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        if let Ok(handle) = call.accept().await {
            let counter = Arc::clone(&self.received_frames);
            tokio::spawn(async move {
                let audio = match handle.audio().await {
                    Ok(a) => a,
                    Err(_) => return,
                };
                let mut rx = audio.receiver;
                while let Some(_frame) = rx.recv().await {
                    counter.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
        CallHandlerDecision::Accept
    }
}

struct BobReceiver {
    task: JoinHandle<()>,
    shutdown: ShutdownHandle,
}

fn make_srtp_config(name: &str, port: u16) -> Config {
    let mut cfg = Config::local(name, port);
    cfg.offer_srtp = true;
    cfg.srtp_required = true;
    cfg
}

async fn boot_bob(port: u16, received_frames: Arc<AtomicU64>) -> BobReceiver {
    let bob = CallbackPeer::new(
        CountingAccept { received_frames },
        make_srtp_config("perf-bob", port),
    )
    .await
    .expect("perf bob: CallbackPeer::new (SRTP)");
    let shutdown = bob.shutdown_handle();
    let task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    BobReceiver { task, shutdown }
}

async fn boot_alice(port: u16) -> Arc<UnifiedCoordinator> {
    let coord = UnifiedCoordinator::new(make_srtp_config("perf-alice", port))
        .await
        .expect("perf alice: UnifiedCoordinator::new (SRTP)");
    tokio::time::sleep(Duration::from_millis(200)).await;
    coord
}

async fn run_one_point(
    alice: Arc<UnifiedCoordinator>,
    from: String,
    target_uri: String,
    target: u64,
    duration: Duration,
    call_timeout: Duration,
    sent_frames: Arc<AtomicU64>,
    received_frames: Arc<AtomicU64>,
) -> ScenarioReport {
    let load = LoadProfile {
        target_cps: target as f64,
        ramp_secs: 0,
        steady_secs: duration.as_secs(),
        cooldown_secs: 5,
    };

    let setup_hist = Arc::new(LatencyHistogram::new("setup_latency"));
    let send_hist = Arc::new(LatencyHistogram::new("encode_to_wire"));
    let setup_failed = Arc::new(AtomicU64::new(0));
    sent_frames.store(0, Ordering::Relaxed);
    received_frames.store(0, Ordering::Relaxed);

    let sampler = ResourceSampler::start(Duration::from_millis(500));

    let (drop_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    let mut handles: Vec<JoinHandle<()>> = Vec::with_capacity(target as usize);

    for _ in 0..target {
        let alice = Arc::clone(&alice);
        let from = from.clone();
        let target_uri = target_uri.clone();
        let setup_hist = Arc::clone(&setup_hist);
        let send_hist = Arc::clone(&send_hist);
        let setup_failed = Arc::clone(&setup_failed);
        let sent_frames = Arc::clone(&sent_frames);
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

            let audio = match handle.audio().await {
                Ok(a) => a,
                Err(_) => return,
            };
            let sender = audio.sender;
            let mut next_tick =
                tokio::time::Instant::now() + Duration::from_millis(FRAME_INTERVAL_MS);
            loop {
                if drop_rx.try_recv().is_ok() {
                    break;
                }
                tokio::time::sleep_until(next_tick).await;
                next_tick += Duration::from_millis(FRAME_INTERVAL_MS);
                let frame = AudioFrame::new(vec![0i16; FRAME_SAMPLES], 8_000, 1, 0);
                let t0 = std::time::Instant::now();
                if sender.send(frame).await.is_err() {
                    break;
                }
                send_hist.record_nanos(t0.elapsed().as_nanos() as u64);
                sent_frames.fetch_add(1, Ordering::Relaxed);
            }
            let _ = handle.hangup_and_wait(Some(call_timeout)).await;
        }));
    }

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
    let active = setup_hist.snapshot().count;

    tokio::time::sleep(duration).await;
    let _ = drop_tx.send(());

    let drain = async {
        for h in handles {
            let _ = h.await;
        }
    };
    let _ = tokio::time::timeout(
        call_timeout + Duration::from_secs(load.cooldown_secs),
        drain,
    )
    .await;

    let resources = sampler.stop().await;
    let sent = sent_frames.load(Ordering::Relaxed);
    let received = received_frames.load(Ordering::Relaxed);
    let frame_loss_pct = if sent > 0 {
        ((sent.saturating_sub(received)) as f64 / sent as f64) * 100.0
    } else {
        0.0
    };
    let asr = if target > 0 {
        active as f64 / target as f64
    } else {
        0.0
    };

    let mut report = ScenarioReport::new("perf_srtp_overhead", load);
    let cores = report.environment().cpu_count_physical() as f64;
    let streams_per_core = if cores > 0.0 {
        active as f64 / cores
    } else {
        0.0
    };
    let pps = active * PPS_PER_STREAM;
    let packets_per_core_per_sec = if cores > 0.0 { pps as f64 / cores } else { 0.0 };
    let delta = read_plain_rtp_baseline_delta(target as f64, active, frame_loss_pct, &send_hist);
    report
        .result("target_concurrent_streams", target)
        .result("achieved_concurrent_streams", active)
        .result("streams_per_core", round2(streams_per_core))
        .result("packets_per_core_per_sec", round2(packets_per_core_per_sec))
        .result("asr", round4(asr))
        .result("ner", round4(asr))
        .result("frames_sent", sent)
        .result("frames_received", received)
        .result("frame_loss_pct", round4(frame_loss_pct))
        .result("delta_vs_plain_rtp_baseline", delta)
        .result(
            "errors",
            json!({
                "setup_failed": setup_failed.load(Ordering::Relaxed),
            }),
        )
        .result("calls_offered", target)
        .latency(&setup_hist)
        .latency(&send_hist)
        .with_resources(resources);
    report
}

fn read_plain_rtp_baseline_delta(
    target: f64,
    active: u64,
    srtp_loss_pct: f64,
    send_hist: &LatencyHistogram,
) -> Value {
    let target_int = target as u64;
    let candidates = [
        format!("perf_rtp_steady_state/{target_int}.json"),
        "perf_rtp_steady_state.json".to_string(),
    ];
    let target_dir = perf_target_dir();
    for c in &candidates {
        let p = target_dir.join(c);
        if let Ok(bytes) = std::fs::read(&p) {
            if let Ok(rtp) = serde_json::from_slice::<Value>(&bytes) {
                let rtp_active = rtp
                    .pointer("/results/achieved_concurrent_streams")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let rtp_loss_pct = rtp
                    .pointer("/results/frame_loss_pct")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let rtp_send_p99 = rtp
                    .pointer("/latency_ns/encode_to_wire/p99")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let srtp_send_p99 = send_hist.snapshot().p99;
                let active_delta_pct = if rtp_active > 0 {
                    ((active as f64 - rtp_active as f64) / rtp_active as f64) * 100.0
                } else {
                    0.0
                };
                let send_p99_delta_pct = if rtp_send_p99 > 0 {
                    ((srtp_send_p99 as f64 - rtp_send_p99 as f64) / rtp_send_p99 as f64) * 100.0
                } else {
                    0.0
                };
                return json!({
                    "baseline_source": p.to_string_lossy(),
                    "rtp_active_streams": rtp_active,
                    "rtp_frame_loss_pct": rtp_loss_pct,
                    "rtp_encode_to_wire_p99_ns": rtp_send_p99,
                    "srtp_active_streams": active,
                    "srtp_frame_loss_pct": srtp_loss_pct,
                    "srtp_encode_to_wire_p99_ns": srtp_send_p99,
                    "active_delta_pct": round2(active_delta_pct),
                    "send_p99_delta_pct": round2(send_p99_delta_pct),
                });
            }
        }
    }
    json!({
        "baseline_source": null,
        "note": "run perf_rtp_steady_state at the same stream count first to populate this block",
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_srtp_overhead() {
    let points = parse_sweep_env("RVOIP_PERF_SWEEP_SRTP_CALLS").unwrap_or_else(|| {
        vec![std::env::var("RVOIP_PERF_RTP_CALLS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(50.0)]
    });
    let duration_secs: u64 = std::env::var("RVOIP_PERF_RTP_DURATION_SECS")
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
    let sent_frames = Arc::new(AtomicU64::new(0));
    let received_frames = Arc::new(AtomicU64::new(0));
    let bob = boot_bob(bob_port, Arc::clone(&received_frames)).await;
    let alice = boot_alice(alice_port).await;
    let from = format!("sip:alice@127.0.0.1:{}", alice_port);
    let target_uri = format!("sip:bob@127.0.0.1:{}", bob_port);

    let mut sweep = SweepRunner::new(
        "perf_srtp_overhead",
        points.clone(),
        "Streams target",
        "achieved_concurrent_streams",
        "ASR",
    );

    for &point in &points {
        let target_count = point.round() as u64;
        let report = run_one_point(
            Arc::clone(&alice),
            from.clone(),
            target_uri.clone(),
            target_count,
            Duration::from_secs(duration_secs),
            call_timeout,
            Arc::clone(&sent_frames),
            Arc::clone(&received_frames),
        )
        .await;
        sweep.add_point(point, report);
    }

    let _written = sweep.finalize();

    bob.shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(3), bob.task).await;
    drop(alice);
}

fn perf_target_dir() -> PathBuf {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
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

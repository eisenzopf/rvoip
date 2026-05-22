//! Scenario 5 — RTP steady-state.
//!
//! N concurrent calls each carrying live G.711 PCMU at 20 ms (50 pps
//! per direction). Measures the media-plane equivalent of scenario 2:
//! how many simultaneous calls can carry clean audio.
//!
//! Reports
//!
//! - `frame_loss_pct` — `(frames_sent - frames_received) / frames_sent`
//!   across all calls. rtpengine's quality threshold is <0.1%.
//! - `encode_to_wire_p99` — per-frame `AudioSender::send().await`
//!   latency. Captures the cost of the in-process encode-and-enqueue
//!   path (does **not** include the UDP socket TX).
//! - `streams_per_core` — concurrent streams normalised by physical
//!   core count.
//!
//! Two run modes:
//! - **Single point (default)**: writes
//!   `target/perf-results/perf_rtp_steady_state.json`.
//! - **Sweep**: set `RVOIP_PERF_SWEEP_RTP_CALLS=50,100,250,500,1000`
//!   to sweep the concurrent-stream count; per-point JSONs +
//!   `_sweep.{json,md}` under `target/perf-results/perf_rtp_steady_state/`.
//!
//! Env knobs:
//! - `RVOIP_PERF_SWEEP_RTP_CALLS`  (enables sweep mode)
//! - `RVOIP_PERF_RTP_CALLS`        (single-point default; 50)
//! - `RVOIP_PERF_RTP_DURATION_SECS` (default 10 — steady-state media window)
//! - `RVOIP_PERF_CALL_TIMEOUT_SECS` (default 30)

#![allow(clippy::needless_return)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_media_core::types::AudioFrame;
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

/// PCMU framing: 8 kHz, 20 ms, mono → 160 samples per frame, 50 fps.
const FRAME_SAMPLES: usize = 160;
const FRAME_INTERVAL_MS: u64 = 20;
const PPS_PER_STREAM: u64 = 50;

/// Bob-side handler that, on every incoming call, accepts it, grabs
/// the SessionHandle's audio stream, and spawns a counter task that
/// pulls frames off the receiver. Each pulled frame increments the
/// shared `received_frames` counter — the denominator side of
/// `frame_loss_pct`.
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

async fn boot_bob(port: u16, received_frames: Arc<AtomicU64>) -> BobReceiver {
    let cfg = Config::local("perf-bob", port);
    let handler = CountingAccept { received_frames };
    let bob = CallbackPeer::new(handler, cfg)
        .await
        .expect("perf bob: CallbackPeer::new");
    let shutdown = bob.shutdown_handle();
    let task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    BobReceiver { task, shutdown }
}

async fn boot_alice(port: u16) -> Arc<UnifiedCoordinator> {
    let cfg = Config::local("perf-alice", port);
    let coord = UnifiedCoordinator::new(cfg)
        .await
        .expect("perf alice: UnifiedCoordinator::new");
    tokio::time::sleep(Duration::from_millis(200)).await;
    coord
}

/// One sweep point: establish `target` concurrent calls, push audio
/// for `duration`, then teardown. All calls share one alice + one bob.
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
    // Schema-stable LoadProfile shape: `target_cps` carries the
    // stream-count point.
    let load = LoadProfile {
        target_cps: target as f64,
        ramp_secs: 0,
        steady_secs: duration.as_secs(),
        cooldown_secs: 5,
    };

    let setup_hist = Arc::new(LatencyHistogram::new("setup_latency"));
    let send_hist = Arc::new(LatencyHistogram::new("encode_to_wire"));
    let setup_failed = Arc::new(AtomicU64::new(0));
    let teardown_failed = Arc::new(AtomicU64::new(0));

    // Reset counters for this point (sampler stays the same).
    sent_frames.store(0, Ordering::Relaxed);
    received_frames.store(0, Ordering::Relaxed);

    let sampler = ResourceSampler::start(Duration::from_millis(500));
    let setup_start = std::time::Instant::now();

    let (drop_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    let mut handles: Vec<JoinHandle<()>> = Vec::with_capacity(target as usize);

    for _ in 0..target {
        let alice = Arc::clone(&alice);
        let from = from.clone();
        let target_uri = target_uri.clone();
        let setup_hist = Arc::clone(&setup_hist);
        let send_hist = Arc::clone(&send_hist);
        let setup_failed = Arc::clone(&setup_failed);
        let teardown_failed = Arc::clone(&teardown_failed);
        let sent_frames = Arc::clone(&sent_frames);
        let mut drop_rx = drop_tx.subscribe();
        handles.push(tokio::spawn(async move {
            // Step 1: establish the call.
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

            // Step 2: grab the audio sender for this call.
            let audio = match handle.audio().await {
                Ok(a) => a,
                Err(_) => {
                    return;
                }
            };
            let sender = audio.sender;

            // Step 3: send PCMU @ 50 pps until the drop signal fires.
            //
            // We pre-build the silence frame once and clone it each
            // tick — AudioFrame's samples are Vec<i16>, so the clone
            // is one heap copy per send. That's the realistic cost a
            // real codec/AudioSource would pay. Timing the send
            // itself captures encode-and-enqueue latency.
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

            // Step 4: teardown.
            if handle.hangup_and_wait(Some(call_timeout)).await.is_err() {
                teardown_failed.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }

    // Wait until setup phase converges (every task has either started
    // sending or failed).
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
    let setup_wall = setup_start.elapsed();
    let active = setup_hist.snapshot().count;

    // Hold steady-state media for the duration.
    tokio::time::sleep(duration).await;
    let _ = drop_tx.send(());

    // Drain.
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

    let mut report = ScenarioReport::new("perf_rtp_steady_state", load);
    let cores = report.environment().cpu_count_physical() as f64;
    let streams_per_core = if cores > 0.0 {
        active as f64 / cores
    } else {
        0.0
    };
    let pps = active * PPS_PER_STREAM;
    let packets_per_core_per_sec = if cores > 0.0 { pps as f64 / cores } else { 0.0 };
    report
        .result("target_concurrent_streams", target)
        .result("achieved_concurrent_streams", active)
        .result("streams_per_core", round2(streams_per_core))
        .result("packets_per_core_per_sec", round2(packets_per_core_per_sec))
        .result("asr", round4(asr))
        .result("ner", round4(asr))
        .result("frames_sent", sent)
        .result("frames_received", received)
        // rtpengine threshold: <0.1% loss = MOS-acceptable. Surfaced as
        // a percentage so it's directly comparable to vendor reports.
        .result("frame_loss_pct", round4(frame_loss_pct))
        .result(
            "errors",
            json!({
                "setup_failed":    setup_failed.load(Ordering::Relaxed),
                "teardown_failed": teardown_failed.load(Ordering::Relaxed),
            }),
        )
        .result("setup_secs", round2(setup_wall.as_secs_f64()))
        .result("calls_offered", target)
        .latency(&setup_hist)
        .latency(&send_hist)
        .with_resources(resources);
    report
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_rtp_steady_state() {
    let points = parse_sweep_env("RVOIP_PERF_SWEEP_RTP_CALLS").unwrap_or_else(|| {
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

    // Shared counters survive across sweep points; we reset them
    // before each point inside `run_one_point`.
    let sent_frames = Arc::new(AtomicU64::new(0));
    let received_frames = Arc::new(AtomicU64::new(0));

    let bob = boot_bob(bob_port, Arc::clone(&received_frames)).await;
    let alice = boot_alice(alice_port).await;
    let from = format!("sip:alice@127.0.0.1:{}", alice_port);
    let target_uri = format!("sip:bob@127.0.0.1:{}", bob_port);

    let mut sweep = SweepRunner::new(
        "perf_rtp_steady_state",
        points.clone(),
        "Streams target",
        "achieved_concurrent_streams",
        "ASR",
    );
    let mut first_loss: Option<f64> = None;

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
        if first_loss.is_none() {
            first_loss = report
                .to_json()
                .pointer("/results/frame_loss_pct")
                .and_then(|v| v.as_f64());
        }
        sweep.add_point(point, report);
    }

    let _written = sweep.finalize();

    bob.shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(3), bob.task).await;
    drop(alice);

    // First-point smoke acceptance: frame loss at the smallest sweep
    // point should be well under 5% (loose threshold since loopback
    // RTP delivery to a per-call mpsc::channel(512) can drop under
    // burst conditions in this synthetic harness). Tighter thresholds
    // are deferred to the methodology doc.
    let first = first_loss.unwrap_or(100.0);
    assert!(
        first <= 5.0,
        "first-point frame_loss_pct {:.3} above 5% — likely a regression",
        first
    );
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

//! Scenario 4.14 — AI agent load (ingress callback latency).
//!
//! Models the contact-center / voice-AI workload the rvoip project
//! targets: N concurrent calls, each forking received audio into an
//! "ASR endpoint" that acknowledges every frame. The metric that
//! matters in this workload is **ingress → application-callback
//! latency** — every millisecond is perceived latency in a turn-based
//! voice agent.
//!
//! The mock ASR is in-process and trivial: it stamps a timestamp on
//! receipt and immediately bumps a counter. The "callback latency"
//! here is the wall clock from the bob-side `audio().recv()` returning
//! a frame to the moment the application code observes it. (For an
//! out-of-process ASR, real-world numbers would include the network
//! hop on top of these.)
//!
//! Reports
//!
//! - `ingress_callback_p99_ns` — per-frame application-callback
//!   latency p99,
//! - `frame_loss_pct` — sender vs receiver,
//! - `streams_per_core` — concurrent calls / physical cores.
//!
//! Env knobs:
//! - `RVOIP_PERF_SWEEP_AI_AGENTS`   (enables sweep mode)
//! - `RVOIP_PERF_AI_AGENTS`         (single-point default; 30)
//! - `RVOIP_PERF_RTP_DURATION_SECS` (default 10)
//! - `RVOIP_PERF_CALL_TIMEOUT_SECS` (default 30)

#![allow(clippy::needless_return)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rvoip_media_core::types::AudioFrame;
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

const FRAME_SAMPLES: usize = 160;
const FRAME_INTERVAL_MS: u64 = 20;

/// AI-agent receiver: on each frame's arrival in the bob-side audio
/// stream, records the per-frame **ingress→callback** latency.
///
/// The trick: alice stamps `t0 = Instant::now()` as the i16 sample
/// values *before* sending. Bob reads those bytes back, reconstructs
/// the timestamp, and computes elapsed. This avoids per-call shared
/// state but assumes loopback delivery preserves the samples.
///
/// Limitation: codec passes through unmodified on loopback (no
/// G.711 encode/decode). On a transcoding path the latency-stamping
/// trick would break.
#[derive(Clone)]
struct AsrAccept {
    received_frames: Arc<AtomicU64>,
    callback_hist: Arc<LatencyHistogram>,
    started_at: Instant,
}

#[async_trait::async_trait]
impl CallHandler for AsrAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        if let Ok(handle) = call.accept().await {
            let counter = Arc::clone(&self.received_frames);
            let hist = Arc::clone(&self.callback_hist);
            let started_at = self.started_at;
            tokio::spawn(async move {
                if let Ok(audio) = handle.audio().await {
                    let mut rx = audio.receiver;
                    while let Some(frame) = rx.recv().await {
                        counter.fetch_add(1, Ordering::Relaxed);
                        // Reconstruct the send-timestamp packed into
                        // the first 4 samples (as a u64 micros-since-
                        // started). Skip frames where the marker
                        // doesn't decode (start-up jitter / loopback
                        // edge cases).
                        if frame.samples.len() >= 4 {
                            let micros = ((frame.samples[0] as u16 as u64) << 48)
                                | ((frame.samples[1] as u16 as u64) << 32)
                                | ((frame.samples[2] as u16 as u64) << 16)
                                | (frame.samples[3] as u16 as u64);
                            if micros > 0 {
                                let now_micros =
                                    started_at.elapsed().as_micros() as u64;
                                if now_micros > micros {
                                    let elapsed_ns =
                                        (now_micros - micros).saturating_mul(1000);
                                    hist.record_nanos(elapsed_ns);
                                }
                            }
                        }
                    }
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

async fn boot_bob(
    port: u16,
    received_frames: Arc<AtomicU64>,
    callback_hist: Arc<LatencyHistogram>,
    started_at: Instant,
) -> BobReceiver {
    let handler = AsrAccept {
        received_frames,
        callback_hist,
        started_at,
    };
    let bob = CallbackPeer::new(handler, Config::local("perf-ai-bob", port))
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
    let coord = UnifiedCoordinator::new(Config::local("perf-ai-alice", port))
        .await
        .expect("perf alice");
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
    callback_hist: Arc<LatencyHistogram>,
    started_at: Instant,
) -> ScenarioReport {
    let load = LoadProfile {
        target_cps: target as f64,
        ramp_secs: 0,
        steady_secs: duration.as_secs(),
        cooldown_secs: 5,
    };
    let setup_hist = Arc::new(LatencyHistogram::new("setup_latency"));
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
        let setup_failed = Arc::clone(&setup_failed);
        let sent_frames = Arc::clone(&sent_frames);
        let mut drop_rx = drop_tx.subscribe();
        handles.push(tokio::spawn(async move {
            let t_send = Instant::now();
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
            let mut next = tokio::time::Instant::now() + Duration::from_millis(FRAME_INTERVAL_MS);
            loop {
                if drop_rx.try_recv().is_ok() {
                    break;
                }
                tokio::time::sleep_until(next).await;
                next += Duration::from_millis(FRAME_INTERVAL_MS);
                // Pack the send-time (micros since `started_at`) into
                // the first 4 i16 samples so bob can recover it.
                let micros = started_at.elapsed().as_micros() as u64;
                let mut samples = vec![0i16; FRAME_SAMPLES];
                samples[0] = ((micros >> 48) & 0xFFFF) as i16;
                samples[1] = ((micros >> 32) & 0xFFFF) as i16;
                samples[2] = ((micros >> 16) & 0xFFFF) as i16;
                samples[3] = (micros & 0xFFFF) as i16;
                let frame = AudioFrame::new(samples, 8_000, 1, 0);
                if sender.send(frame).await.is_err() {
                    break;
                }
                sent_frames.fetch_add(1, Ordering::Relaxed);
            }
            let _ = handle.hangup_and_wait(Some(call_timeout)).await;
        }));
    }

    let setup_deadline = Instant::now() + call_timeout + Duration::from_secs(10);
    loop {
        if Instant::now() > setup_deadline {
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
    let _ = tokio::time::timeout(call_timeout + Duration::from_secs(load.cooldown_secs), drain).await;

    let resources = sampler.stop().await;
    let sent = sent_frames.load(Ordering::Relaxed);
    let received = received_frames.load(Ordering::Relaxed);
    let loss_pct = if sent > 0 {
        ((sent.saturating_sub(received)) as f64 / sent as f64) * 100.0
    } else {
        0.0
    };
    let asr = if target > 0 { active as f64 / target as f64 } else { 0.0 };

    let mut report = ScenarioReport::new("perf_ai_agent_load", load);
    let cores = report.environment().cpu_count_physical() as f64;
    let streams_per_core = if cores > 0.0 { active as f64 / cores } else { 0.0 };
    report
        .result("target_agents", target)
        .result("achieved_agents", active)
        .result("streams_per_core", round2(streams_per_core))
        .result("asr", round4(asr))
        .result("ner", round4(asr))
        .result("frames_sent", sent)
        .result("frames_received", received)
        .result("frame_loss_pct", round4(loss_pct))
        .result(
            "errors",
            json!({
                "setup_failed": setup_failed.load(Ordering::Relaxed),
            }),
        )
        .latency(&setup_hist)
        .latency(&callback_hist)
        .with_resources(resources);
    report
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_ai_agent_load() {
    let points = parse_sweep_env("RVOIP_PERF_SWEEP_AI_AGENTS").unwrap_or_else(|| {
        vec![std::env::var("RVOIP_PERF_AI_AGENTS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30.0)]
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
    let started_at = Instant::now();
    let sent_frames = Arc::new(AtomicU64::new(0));
    let received_frames = Arc::new(AtomicU64::new(0));
    let callback_hist = Arc::new(LatencyHistogram::new("ingress_callback"));
    let bob = boot_bob(
        bob_port,
        Arc::clone(&received_frames),
        Arc::clone(&callback_hist),
        started_at,
    )
    .await;
    let alice = boot_alice(alice_port).await;
    let from = format!("sip:alice@127.0.0.1:{alice_port}");
    let target_uri = format!("sip:bob@127.0.0.1:{bob_port}");

    let mut sweep = SweepRunner::new(
        "perf_ai_agent_load",
        points.clone(),
        "Agents target",
        "achieved_agents",
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
            Arc::clone(&callback_hist),
            started_at,
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

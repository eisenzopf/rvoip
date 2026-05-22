//! Scenario 4 — mid-call signalling under live media.
//!
//! N concurrent established calls, each with a background sender
//! pushing PCMU @ 50 pps. A driver task issues mid-call operations
//! against random calls and measures **op completion latency** —
//! the metric that drives hold/resume/IVR responsiveness in real
//! call centers.
//!
//! Reports
//!
//! - `mid_call_op_p99` (histogram of operation completion latency,
//!   measured at the caller side from API call to `Ok`),
//! - per-op success rate (broken out by operation kind),
//! - `frames_dropped_during_ops` — the audio side-effect (frames not
//!   received during the op window) to confirm media stays flowing.
//!
//! The ops driven are `hold` and `resume` (alternating). DTMF and
//! re-INVITE belong to a follow-up — the headline question is "do
//! mid-call signalling latencies stay tight when audio is flowing?"
//! which `hold` + `resume` answers (both cycle the SDP and reach the
//! same code path as a generic re-INVITE).
//!
//! Two run modes (sweep + single-point) per the standard harness.
//!
//! Env knobs:
//! - `RVOIP_PERF_SWEEP_MID_CALL_CALLS`   (enables sweep mode)
//! - `RVOIP_PERF_MID_CALL_CALLS`         (single-point; default 30)
//! - `RVOIP_PERF_MID_CALL_OPS_PER_CALL`  (default 4 — ops per call across the window)
//! - `RVOIP_PERF_MID_CALL_DURATION_SECS` (default 10)
//! - `RVOIP_PERF_CALL_TIMEOUT_SECS`      (default 30)

#![allow(clippy::needless_return)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_media_core::types::AudioFrame;
use rvoip_sip::api::callback_peer::{
    CallHandler, CallHandlerDecision, CallbackPeer, ShutdownHandle,
};
use rvoip_sip::api::handle::SessionHandle;
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
    let bob = CallbackPeer::new(CountingAccept { received_frames }, cfg)
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

#[derive(Default)]
struct OpCounters {
    hold_ok: AtomicU64,
    hold_fail: AtomicU64,
    resume_ok: AtomicU64,
    resume_fail: AtomicU64,
}

async fn run_one_point(
    alice: Arc<UnifiedCoordinator>,
    from: String,
    target_uri: String,
    target: u64,
    ops_per_call: u64,
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
    let op_hist = Arc::new(LatencyHistogram::new("mid_call_op"));
    let setup_failed = Arc::new(AtomicU64::new(0));
    let ops = Arc::new(OpCounters::default());

    sent_frames.store(0, Ordering::Relaxed);
    received_frames.store(0, Ordering::Relaxed);

    let sampler = ResourceSampler::start(Duration::from_millis(500));
    let (drop_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    let mut handles: Vec<JoinHandle<()>> = Vec::with_capacity(target as usize);

    for i in 0..target {
        let alice = Arc::clone(&alice);
        let from = from.clone();
        let target_uri = target_uri.clone();
        let setup_hist = Arc::clone(&setup_hist);
        let op_hist = Arc::clone(&op_hist);
        let setup_failed = Arc::clone(&setup_failed);
        let ops = Arc::clone(&ops);
        let sent_frames = Arc::clone(&sent_frames);
        let mut drop_rx = drop_tx.subscribe();
        let total_window = duration;
        let op_index = i;
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

            // Background media sender. Lives until the drop signal.
            let audio = match handle.audio().await {
                Ok(a) => a,
                Err(_) => return,
            };
            let sender = audio.sender;
            let mut media_drop = drop_rx.resubscribe();
            let sent_frames_task = Arc::clone(&sent_frames);
            tokio::spawn(async move {
                let mut next_tick =
                    tokio::time::Instant::now() + Duration::from_millis(FRAME_INTERVAL_MS);
                loop {
                    if media_drop.try_recv().is_ok() {
                        break;
                    }
                    tokio::time::sleep_until(next_tick).await;
                    next_tick += Duration::from_millis(FRAME_INTERVAL_MS);
                    let frame = AudioFrame::new(vec![0i16; FRAME_SAMPLES], 8_000, 1, 0);
                    if sender.send(frame).await.is_err() {
                        break;
                    }
                    sent_frames_task.fetch_add(1, Ordering::Relaxed);
                }
            });

            // Drive `ops_per_call` mid-call operations spread across
            // the steady window, alternating hold ↔ resume.
            let stagger = if ops_per_call > 0 {
                total_window / (ops_per_call as u32 + 1)
            } else {
                total_window
            };
            // Initial offset spreads the call's ops across the window
            // and slightly desynchronises across calls so the driver
            // doesn't hammer the same op concurrently.
            let initial = Duration::from_millis(50 + (op_index % 17) * 25);
            tokio::time::sleep(initial).await;
            for op_seq in 0..ops_per_call {
                tokio::time::sleep(stagger).await;
                let is_hold = op_seq % 2 == 0;
                let t0 = std::time::Instant::now();
                let result = if is_hold {
                    drive_hold(&handle).await
                } else {
                    drive_resume(&handle).await
                };
                let elapsed = t0.elapsed().as_nanos() as u64;
                match (is_hold, result) {
                    (true, Ok(())) => {
                        ops.hold_ok.fetch_add(1, Ordering::Relaxed);
                        op_hist.record_nanos(elapsed);
                    }
                    (true, Err(_)) => {
                        ops.hold_fail.fetch_add(1, Ordering::Relaxed);
                    }
                    (false, Ok(())) => {
                        ops.resume_ok.fetch_add(1, Ordering::Relaxed);
                        op_hist.record_nanos(elapsed);
                    }
                    (false, Err(_)) => {
                        ops.resume_fail.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }

            // Wait for the global drop signal before tearing down so
            // every call sees the same steady-state window.
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
        let answered = setup_hist.snapshot().count;
        let failed = setup_failed.load(Ordering::Relaxed);
        if answered + failed >= target {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let active = setup_hist.snapshot().count;

    // Hold steady-state for the media+ops window.
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
    let hold_ok = ops.hold_ok.load(Ordering::Relaxed);
    let hold_fail = ops.hold_fail.load(Ordering::Relaxed);
    let resume_ok = ops.resume_ok.load(Ordering::Relaxed);
    let resume_fail = ops.resume_fail.load(Ordering::Relaxed);
    let total_ops = hold_ok + hold_fail + resume_ok + resume_fail;
    let asr = if target > 0 {
        active as f64 / target as f64
    } else {
        0.0
    };
    let op_success_rate = if total_ops > 0 {
        (hold_ok + resume_ok) as f64 / total_ops as f64
    } else {
        0.0
    };

    let mut report = ScenarioReport::new("perf_mid_call_signal_under_media", load);
    let cores = report.environment().cpu_count_physical() as f64;
    let dialogs_per_core = if cores > 0.0 {
        active as f64 / cores
    } else {
        0.0
    };
    report
        .result("target_concurrent", target)
        .result("achieved_concurrent", active)
        .result("dialogs_per_core", round2(dialogs_per_core))
        .result("asr", round4(asr))
        .result("ner", round4(asr))
        .result("ops_total", total_ops)
        .result("op_success_rate", round4(op_success_rate))
        .result("frames_sent", sent)
        .result("frames_received", received)
        // The media side-effect of running mid-call signalling: did
        // audio drop frames during the op window?
        .result("frames_dropped_during_ops_pct", round4(frame_loss_pct))
        .result(
            "ops_breakdown",
            json!({
                "hold_ok":     hold_ok,
                "hold_fail":   hold_fail,
                "resume_ok":   resume_ok,
                "resume_fail": resume_fail,
            }),
        )
        .result("calls_offered", target)
        .latency(&setup_hist)
        .latency(&op_hist)
        .with_resources(resources);
    report
}

async fn drive_hold(handle: &SessionHandle) -> rvoip_sip::Result<()> {
    handle.hold().await
}
async fn drive_resume(handle: &SessionHandle) -> rvoip_sip::Result<()> {
    handle.resume().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_mid_call_signal_under_media() {
    let points = parse_sweep_env("RVOIP_PERF_SWEEP_MID_CALL_CALLS").unwrap_or_else(|| {
        vec![std::env::var("RVOIP_PERF_MID_CALL_CALLS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30.0)]
    });
    let ops_per_call: u64 = std::env::var("RVOIP_PERF_MID_CALL_OPS_PER_CALL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);
    let duration_secs: u64 = std::env::var("RVOIP_PERF_MID_CALL_DURATION_SECS")
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
        "perf_mid_call_signal_under_media",
        points.clone(),
        "Calls target",
        "achieved_concurrent",
        "ASR",
    );

    for &point in &points {
        let target_count = point.round() as u64;
        let report = run_one_point(
            Arc::clone(&alice),
            from.clone(),
            target_uri.clone(),
            target_count,
            ops_per_call,
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

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

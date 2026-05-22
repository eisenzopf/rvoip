//! Scenario 3.9 — real Post-Dial Delay (PDD) measurement.
//!
//! Scenarios 1/2 measure `setup_latency` = INVITE → 200 OK because
//! the `AutoAccept` handler answers immediately. Real-world VoIP PDD
//! is INVITE → first 18x (180 Ringing or 183 Session Progress) — the
//! moment the dialer hears the ring. ITU-T E.411 / per-trunk SLAs cite
//! this number directly; carrier docs typically use "PDD < 2 s
//! excellent, < 4–5 s acceptable."
//!
//! This scenario uses a `RingingThenAccept` handler that sends 180
//! immediately, sleeps `RVOIP_PERF_RING_DELAY_MS` (default 800 ms —
//! typical PBX), then sends 200 OK. Alice subscribes to events and
//! records the time INVITE → first `CallProgress` as `pdd`, then
//! INVITE → `CallAnswered` as `setup_latency`.
//!
//! Run via:
//! ```text
//! cargo test -p rvoip-sip --features perf-tests --release \
//!   --test perf_pdd_with_180_first -- --nocapture
//! ```
//!
//! Env knobs:
//! - `RVOIP_PERF_SWEEP_CPS`        (enables sweep mode)
//! - `RVOIP_PERF_TARGET_CPS`       (single-point default; 50)
//! - `RVOIP_PERF_RING_DELAY_MS`    (default 800 — sleep between 180 and 200)
//! - `RVOIP_PERF_STEADY_SECS`      (default 30)
//! - `RVOIP_PERF_CALL_TIMEOUT_SECS` (default 15)

#![allow(clippy::needless_return)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::callback_peer::{
    CallHandler, CallHandlerDecision, CallbackPeer, ShutdownHandle,
};
use rvoip_sip::api::events::Event;
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use serde_json::json;
use tokio::task::JoinHandle;

#[path = "support/mod.rs"]
mod support;
use support::{
    parse_sweep_env, LatencyHistogram, LoadProfile, ResourceSampler, ScenarioReport, SweepRunner,
};

/// Sends 180 Ringing immediately, sleeps `ring_delay`, then sends 200
/// OK. Models a typical PBX call flow.
#[derive(Clone)]
struct RingingThenAccept {
    ring_delay: Duration,
}

#[async_trait::async_trait]
impl CallHandler for RingingThenAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        // Fire 180 Ringing immediately.
        let _ = call.send_provisional_builder(180).send().await;
        // Sleep the configured ring time.
        let ring_delay = self.ring_delay;
        tokio::spawn(async move {
            tokio::time::sleep(ring_delay).await;
            let _ = call.accept().await;
        });
        CallHandlerDecision::Accept
    }
}

#[derive(Default)]
struct Counters {
    offered: AtomicU64,
    succeeded: AtomicU64,
    no_provisional: AtomicU64,
    answer_failed: AtomicU64,
    bye_failed: AtomicU64,
    timeout: AtomicU64,
}

struct BobReceiver {
    task: JoinHandle<()>,
    shutdown: ShutdownHandle,
}

async fn boot_bob(port: u16, ring_delay: Duration) -> BobReceiver {
    let cfg = Config::local("perf-pdd-bob", port);
    let handler = RingingThenAccept { ring_delay };
    let bob = CallbackPeer::new(handler, cfg)
        .await
        .expect("perf bob: CallbackPeer::new (PDD)");
    let shutdown = bob.shutdown_handle();
    let task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    BobReceiver { task, shutdown }
}

async fn boot_alice(port: u16) -> Arc<UnifiedCoordinator> {
    let cfg = Config::local("perf-pdd-alice", port);
    let coord = UnifiedCoordinator::new(cfg)
        .await
        .expect("perf alice: UnifiedCoordinator::new (PDD)");
    tokio::time::sleep(Duration::from_millis(200)).await;
    coord
}

/// One call: INVITE → wait for first CallProgress (180) → record PDD →
/// wait for CallAnswered → record setup_latency → BYE → record full_cycle.
async fn run_one_call(
    alice: Arc<UnifiedCoordinator>,
    from: String,
    target: String,
    pdd_hist: Arc<LatencyHistogram>,
    setup_hist: Arc<LatencyHistogram>,
    full_hist: Arc<LatencyHistogram>,
    counters: Arc<Counters>,
    per_call_timeout: Duration,
) {
    counters.offered.fetch_add(1, Ordering::Relaxed);
    // Subscribe BEFORE INVITE so the first 180 is not missed.
    let mut events = match alice.events().await {
        Ok(e) => e,
        Err(_) => {
            counters.answer_failed.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    let t_send = std::time::Instant::now();
    let call_id = match alice.invite(Some(from), target).send().await {
        Ok(id) => id,
        Err(_) => {
            counters.answer_failed.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };

    // Wait for the first CallProgress event with our call_id, or for
    // a terminal failure. Anything else (other call's progress, etc.)
    // is filtered out.
    let result = tokio::time::timeout(per_call_timeout, async {
        loop {
            match events.next().await {
                Some(Event::CallProgress {
                    call_id: cid,
                    status_code,
                    ..
                }) if cid == call_id && status_code >= 100 && status_code < 200 => {
                    return Some("provisional");
                }
                Some(Event::CallAnswered { call_id: cid, .. }) if cid == call_id => {
                    // We somehow missed the provisional and went straight
                    // to answered (no 180 emitted). Mark and fall through.
                    return Some("answered");
                }
                Some(Event::CallFailed { call_id: cid, .. }) if cid == call_id => {
                    return Some("failed");
                }
                Some(_) => continue,
                None => return None,
            }
        }
    })
    .await;

    match result {
        Ok(Some("provisional")) => {
            pdd_hist.record_nanos(t_send.elapsed().as_nanos() as u64);
        }
        Ok(Some("answered")) => {
            // No 180 seen — count as such but continue waiting for
            // teardown.
            counters.no_provisional.fetch_add(1, Ordering::Relaxed);
        }
        Ok(Some("failed")) | Ok(None) => {
            counters.answer_failed.fetch_add(1, Ordering::Relaxed);
            return;
        }
        Err(_) => {
            counters.timeout.fetch_add(1, Ordering::Relaxed);
            return;
        }
        _ => {}
    }

    // Now wait for the 200 OK.
    let handle = alice.session(&call_id);
    match handle.wait_for_answered(Some(per_call_timeout)).await {
        Ok(_) => setup_hist.record_nanos(t_send.elapsed().as_nanos() as u64),
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
    let pdd_hist = Arc::new(LatencyHistogram::new("pdd"));
    let setup_hist = Arc::new(LatencyHistogram::new("setup_latency"));
    let full_hist = Arc::new(LatencyHistogram::new("full_cycle"));
    let counters = Arc::new(Counters::default());
    let handles = Arc::new(tokio::sync::Mutex::new(Vec::<JoinHandle<()>>::new()));
    let sampler = ResourceSampler::start(Duration::from_millis(500));

    let active_wall = {
        let alice = Arc::clone(&alice);
        let pdd_hist = Arc::clone(&pdd_hist);
        let setup_hist = Arc::clone(&setup_hist);
        let full_hist = Arc::clone(&full_hist);
        let counters = Arc::clone(&counters);
        let handles = Arc::clone(&handles);
        load.run(move |_seq| {
            let alice = Arc::clone(&alice);
            let pdd_hist = Arc::clone(&pdd_hist);
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
                    pdd_hist,
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

    let mut report = ScenarioReport::new("perf_pdd_with_180_first", load);
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
                "no_provisional":     counters.no_provisional.load(Ordering::Relaxed),
                "answer_failed":      counters.answer_failed.load(Ordering::Relaxed),
                "bye_failed":         counters.bye_failed.load(Ordering::Relaxed),
                "timeout":            counters.timeout.load(Ordering::Relaxed),
            }),
        )
        .latency(&pdd_hist)
        .latency(&setup_hist)
        .latency(&full_hist)
        .with_resources(resources);
    report
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_pdd_with_180_first() {
    let points = parse_sweep_env("RVOIP_PERF_SWEEP_CPS").unwrap_or_else(|| {
        vec![std::env::var("RVOIP_PERF_TARGET_CPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(50.0)]
    });
    let ring_delay = Duration::from_millis(
        std::env::var("RVOIP_PERF_RING_DELAY_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(800),
    );
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

    let bob_port = support::ports::next_sip_port();
    let alice_port = support::ports::next_sip_port();
    let bob = boot_bob(bob_port, ring_delay).await;
    let alice = boot_alice(alice_port).await;
    let from = format!("sip:alice@127.0.0.1:{}", alice_port);
    let target = format!("sip:bob@127.0.0.1:{}", bob_port);

    let mut sweep = SweepRunner::new(
        "perf_pdd_with_180_first",
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
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

//! Scenario 4.17 — transport recovery.
//!
//! ChatGPT Tier 2 reliability metric: how does the SIP stack handle
//! a peer disappearing and coming back? Real-world this is a NIC
//! flap, a SIP-trunk migration, or a peer process restart.
//!
//! This scenario simulates the failure mode in-process by shutting
//! the bob peer down mid-stream, observing alice's behaviour while
//! bob is gone (timeouts, retransmits), then booting bob fresh on
//! the same port and verifying signalling resumes.
//!
//! Reports
//!
//! - `pre_failure_p99_ns` — setup p99 before bob disappears,
//! - `gone_window_attempts` — INVITEs offered while bob was down,
//! - `gone_window_timeouts` — how many timed out (expected ≈ 100 %),
//! - `recovery_first_success_after_secs` — wall-clock from bob coming
//!   back online to alice's first successful call,
//! - `post_recovery_p99_ns` — setup p99 after recovery.
//!
//! Env knobs:
//! - `RVOIP_PERF_REC_CPS`         (default 5 — gentle rate)
//! - `RVOIP_PERF_REC_PRE_SECS`    (default 8)
//! - `RVOIP_PERF_REC_GONE_SECS`   (default 8)
//! - `RVOIP_PERF_REC_POST_SECS`   (default 12)
//! - `RVOIP_PERF_CALL_TIMEOUT_SECS` (default 5 — short so we don't
//!   wait long during the gone-window)

#![allow(clippy::needless_return)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rvoip_sip::api::callback_peer::{
    CallHandler, CallHandlerDecision, CallbackPeer, ShutdownHandle,
};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use serde_json::json;
use tokio::task::JoinHandle;

#[path = "support/mod.rs"]
mod support;
use support::{LatencyHistogram, LoadProfile, ResourceSampler, ScenarioReport};

struct AutoAccept;

#[async_trait::async_trait]
impl CallHandler for AutoAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let _ = call.accept().await;
        CallHandlerDecision::Accept
    }
}

struct BobReceiver {
    task: JoinHandle<()>,
    shutdown: ShutdownHandle,
}

async fn boot_bob(port: u16) -> BobReceiver {
    let bob = CallbackPeer::new(AutoAccept, Config::local("perf-rec-bob", port))
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
    let coord = UnifiedCoordinator::new(Config::local("perf-rec-alice", port))
        .await
        .expect("perf alice");
    tokio::time::sleep(Duration::from_millis(200)).await;
    coord
}

async fn drive_calls(
    alice: Arc<UnifiedCoordinator>,
    from: String,
    target: String,
    cps: f64,
    duration: Duration,
    hist: Arc<LatencyHistogram>,
    offered: Arc<AtomicU64>,
    succeeded: Arc<AtomicU64>,
    timed_out: Arc<AtomicU64>,
    first_success_at: Arc<tokio::sync::Mutex<Option<Instant>>>,
    call_timeout: Duration,
) {
    let started = Instant::now();
    let tick = Duration::from_secs_f64(1.0 / cps.max(1.0));
    let handles = Arc::new(tokio::sync::Mutex::new(Vec::<JoinHandle<()>>::new()));
    loop {
        if started.elapsed() >= duration {
            break;
        }
        let alice = Arc::clone(&alice);
        let from = from.clone();
        let target = target.clone();
        let hist = Arc::clone(&hist);
        let offered = Arc::clone(&offered);
        let succeeded = Arc::clone(&succeeded);
        let timed_out = Arc::clone(&timed_out);
        let first_success_at = Arc::clone(&first_success_at);
        let handles_for_record = Arc::clone(&handles);
        let h = tokio::spawn(async move {
            offered.fetch_add(1, Ordering::Relaxed);
            let t_send = Instant::now();
            let call_id = match alice.invite(Some(from), target).send().await {
                Ok(id) => id,
                Err(_) => {
                    timed_out.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            };
            let handle = alice.session(&call_id);
            match handle.wait_for_answered(Some(call_timeout)).await {
                Ok(_) => {
                    hist.record_nanos(t_send.elapsed().as_nanos() as u64);
                    succeeded.fetch_add(1, Ordering::Relaxed);
                    let mut g = first_success_at.lock().await;
                    if g.is_none() {
                        *g = Some(Instant::now());
                    }
                }
                Err(_) => {
                    timed_out.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            }
            let _ = handle.hangup_and_wait(Some(call_timeout)).await;
        });
        tokio::spawn(async move {
            handles_for_record.lock().await.push(h);
        });
        tokio::time::sleep(tick).await;
    }
    let _ = tokio::time::timeout(call_timeout + Duration::from_secs(5), async {
        let mut g = handles.lock().await;
        for h in std::mem::take(&mut *g) {
            let _ = h.await;
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_transport_recovery() {
    let cps: f64 = std::env::var("RVOIP_PERF_REC_CPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5.0);
    let pre_secs: u64 = std::env::var("RVOIP_PERF_REC_PRE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let gone_secs: u64 = std::env::var("RVOIP_PERF_REC_GONE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let post_secs: u64 = std::env::var("RVOIP_PERF_REC_POST_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(12);
    let call_timeout = Duration::from_secs(
        std::env::var("RVOIP_PERF_CALL_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(5),
    );

    let bob_port = support::ports::next_sip_port();
    let alice_port = support::ports::next_sip_port();
    let bob = boot_bob(bob_port).await;
    let alice = boot_alice(alice_port).await;
    let from = format!("sip:alice@127.0.0.1:{alice_port}");
    let target = format!("sip:bob@127.0.0.1:{bob_port}");

    let pre_hist = Arc::new(LatencyHistogram::new("setup_latency_pre"));
    let gone_hist = Arc::new(LatencyHistogram::new("setup_latency_gone"));
    let post_hist = Arc::new(LatencyHistogram::new("setup_latency_post"));
    let pre_offered = Arc::new(AtomicU64::new(0));
    let pre_succ = Arc::new(AtomicU64::new(0));
    let pre_to = Arc::new(AtomicU64::new(0));
    let gone_offered = Arc::new(AtomicU64::new(0));
    let gone_succ = Arc::new(AtomicU64::new(0));
    let gone_to = Arc::new(AtomicU64::new(0));
    let post_offered = Arc::new(AtomicU64::new(0));
    let post_succ = Arc::new(AtomicU64::new(0));
    let post_to = Arc::new(AtomicU64::new(0));
    let unused_first = Arc::new(tokio::sync::Mutex::new(None));
    let post_first = Arc::new(tokio::sync::Mutex::new(None));

    let sampler = ResourceSampler::start(Duration::from_millis(500));

    // Phase 1: pre-failure baseline.
    drive_calls(
        Arc::clone(&alice),
        from.clone(),
        target.clone(),
        cps,
        Duration::from_secs(pre_secs),
        Arc::clone(&pre_hist),
        Arc::clone(&pre_offered),
        Arc::clone(&pre_succ),
        Arc::clone(&pre_to),
        Arc::clone(&unused_first),
        call_timeout,
    )
    .await;

    // Phase 2: shut bob down (simulates transport failure).
    bob.shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(3), bob.task).await;
    let bob_down_at = Instant::now();

    drive_calls(
        Arc::clone(&alice),
        from.clone(),
        target.clone(),
        cps,
        Duration::from_secs(gone_secs),
        Arc::clone(&gone_hist),
        Arc::clone(&gone_offered),
        Arc::clone(&gone_succ),
        Arc::clone(&gone_to),
        Arc::clone(&unused_first),
        call_timeout,
    )
    .await;

    // Phase 3: bring bob back on the same port. Tear-down above
    // releases the socket; the new peer can re-bind.
    let bob = boot_bob(bob_port).await;
    let bob_back_at = Instant::now();

    drive_calls(
        Arc::clone(&alice),
        from.clone(),
        target.clone(),
        cps,
        Duration::from_secs(post_secs),
        Arc::clone(&post_hist),
        Arc::clone(&post_offered),
        Arc::clone(&post_succ),
        Arc::clone(&post_to),
        Arc::clone(&post_first),
        call_timeout,
    )
    .await;

    let resources = sampler.stop().await;
    let recovery_first_success_after_secs = post_first
        .lock()
        .await
        .map(|t| t.duration_since(bob_back_at).as_secs_f64());

    let load = LoadProfile {
        target_cps: cps,
        ramp_secs: 0,
        steady_secs: pre_secs + gone_secs + post_secs,
        cooldown_secs: 0,
    };
    let mut report = ScenarioReport::new("perf_transport_recovery", load);
    let bob_down_for_secs = bob_back_at.duration_since(bob_down_at).as_secs_f64();
    report
        .result("cps", cps)
        .result("pre_secs", pre_secs)
        .result("gone_secs", gone_secs)
        .result("post_secs", post_secs)
        .result("bob_down_for_secs", round2(bob_down_for_secs))
        .result("pre_failure_p99_ns", pre_hist.snapshot().p99)
        .result("gone_window_attempts", gone_offered.load(Ordering::Relaxed))
        .result("gone_window_timeouts", gone_to.load(Ordering::Relaxed))
        .result("gone_window_successes", gone_succ.load(Ordering::Relaxed))
        .result(
            "recovery_first_success_after_secs",
            recovery_first_success_after_secs.map(round2),
        )
        .result("post_recovery_p99_ns", post_hist.snapshot().p99)
        .result(
            "errors",
            json!({
                "pre_timeouts":  pre_to.load(Ordering::Relaxed),
                "gone_timeouts": gone_to.load(Ordering::Relaxed),
                "post_timeouts": post_to.load(Ordering::Relaxed),
            }),
        )
        .latency(&pre_hist)
        .latency(&gone_hist)
        .latency(&post_hist)
        .with_resources(resources);
    let json_path = report.write_json();
    report.print_summary(&json_path);

    bob.shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(3), bob.task).await;
    drop(alice);
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

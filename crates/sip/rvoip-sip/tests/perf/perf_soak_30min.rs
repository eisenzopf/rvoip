//! Scenario 8 — long-duration soak (default 30 min, `#[ignore]`).
//!
//! Combines scenario 1's signalling churn (steady call setup/teardown
//! at a fixed mid-knee CPS) with scenario 5's RTP steady-state (a
//! fixed number of long-lived calls carrying PCMU). Run for
//! `RVOIP_PERF_SOAK_DURATION_SECS` (default 30 min). Marked `#[ignore]`
//! so it opts in via `-- --ignored`.
//!
//! The headline output is **latency drift** and **memory growth**:
//!
//! - `latency_drift_pct` — `(setup p99 in last minute) / (setup p99 in
//!   first minute) - 1`. Should stay well under 50 % on a healthy run.
//! - `rss_gate_growth_mb_per_hr` — projection of the post-drain RSS slope
//!   when a drain window is available, otherwise the final tail-window RSS
//!   slope. This is the release gate because one-time allocator warmup and
//!   bounded event-ring reservation should not masquerade as an unbounded
//!   leak.
//! - `errors_per_minute` — distribution; should be 0 across the run.
//!
//! Run via:
//! ```text
//! cargo test -p rvoip-sip --features perf-tests --release \
//!   --test perf_soak_30min -- --ignored --nocapture
//! ```
//!
//! Env knobs:
//! - `RVOIP_PERF_SOAK_DURATION_SECS` (default 1800 = 30 min)
//! - `RVOIP_PERF_SOAK_CPS`           (default 20 — well below typical knee)
//! - `RVOIP_PERF_SOAK_MEDIA_CALLS`   (default 30 — concurrent RTP streams)
//! - `RVOIP_PERF_CALL_TIMEOUT_SECS`  (default 30)
//! - `RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR` (default from `Config`)
//! - `RVOIP_PERF_APP_EVENT_CHANNEL_CAPACITY` (default 256)
//! - `RVOIP_PERF_SIP_TRANSACTION_COMMAND_CHANNEL_CAPACITY` (default from `Config`)
//! - `RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS` (default 40; covers UDP Timer J)
//! - `RVOIP_PERF_RSS_TAIL_WINDOW_SECS` (default 60)

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
use tokio::task::{JoinHandle, JoinSet};

#[path = "support/mod.rs"]
mod support;
use support::{LatencyHistogram, LoadProfile, ResourceSample, ResourceSampler, ScenarioReport};

const FRAME_SAMPLES: usize = 160;
const FRAME_INTERVAL_MS: u64 = 20;
const DEFAULT_PERF_APP_EVENT_CHANNEL_CAPACITY: usize = Config::DEFAULT_APP_EVENT_CHANNEL_CAPACITY;
const DEFAULT_RETENTION_DRAIN_WAIT_SECS: usize = 40;

#[derive(Clone)]
struct CountingAccept {
    received_frames: Arc<AtomicU64>,
    active_audio_receivers: Arc<AtomicU64>,
    completed_audio_receivers: Arc<AtomicU64>,
}

#[async_trait::async_trait]
impl CallHandler for CountingAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        if let Ok(handle) = call.accept().await {
            let counter = Arc::clone(&self.received_frames);
            let active_receivers = Arc::clone(&self.active_audio_receivers);
            let completed_receivers = Arc::clone(&self.completed_audio_receivers);
            tokio::spawn(async move {
                active_receivers.fetch_add(1, Ordering::Relaxed);
                if let Ok(audio) = handle.audio().await {
                    let mut rx = audio.receiver;
                    while let Some(_frame) = rx.recv().await {
                        counter.fetch_add(1, Ordering::Relaxed);
                    }
                }
                active_receivers.fetch_sub(1, Ordering::Relaxed);
                completed_receivers.fetch_add(1, Ordering::Relaxed);
            });
        }
        CallHandlerDecision::Accept
    }
}

#[derive(Clone, Default)]
struct BobHandlerDiagnostics {
    received_frames: Arc<AtomicU64>,
    active_audio_receivers: Arc<AtomicU64>,
    completed_audio_receivers: Arc<AtomicU64>,
}

struct BobReceiver {
    task: JoinHandle<()>,
    shutdown: ShutdownHandle,
    coordinator: Arc<UnifiedCoordinator>,
}

async fn boot_bob(cfg: Config, diagnostics: BobHandlerDiagnostics) -> BobReceiver {
    let bob = CallbackPeer::new(
        CountingAccept {
            received_frames: diagnostics.received_frames,
            active_audio_receivers: diagnostics.active_audio_receivers,
            completed_audio_receivers: diagnostics.completed_audio_receivers,
        },
        cfg,
    )
    .await
    .expect("perf-soak bob");
    let shutdown = bob.shutdown_handle();
    let coordinator = bob.coordinator().clone();
    let task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    BobReceiver {
        task,
        shutdown,
        coordinator,
    }
}

async fn boot_alice(cfg: Config) -> Arc<UnifiedCoordinator> {
    let coord = UnifiedCoordinator::new(cfg).await.expect("perf-soak alice");
    tokio::time::sleep(Duration::from_millis(200)).await;
    coord
}

fn perf_config(name: &str, port: u16) -> Config {
    let app_event_capacity = read_positive_usize_env("RVOIP_PERF_APP_EVENT_CHANNEL_CAPACITY")
        .or_else(|| read_positive_usize_env("RVOIP_PERF_GLOBAL_EVENT_CHANNEL_CAPACITY"))
        .unwrap_or(DEFAULT_PERF_APP_EVENT_CHANNEL_CAPACITY);
    let mut config = Config::local(name, port).with_app_event_channel_capacity(app_event_capacity);
    if let Some(capacity) =
        read_positive_usize_env("RVOIP_PERF_SIP_TRANSACTION_COMMAND_CHANNEL_CAPACITY")
    {
        config = config.with_sip_transaction_command_channel_capacity(capacity);
    }
    config
}

fn retention_drain_wait() -> Duration {
    Duration::from_secs(
        read_positive_usize_env("RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS")
            .unwrap_or(DEFAULT_RETENTION_DRAIN_WAIT_SECS)
            .try_into()
            .unwrap_or(u64::MAX),
    )
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_soak_30min() {
    let duration_secs: u64 = std::env::var("RVOIP_PERF_SOAK_DURATION_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1800);
    let soak_cps: f64 = std::env::var("RVOIP_PERF_SOAK_CPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20.0);
    let media_calls: u64 = std::env::var("RVOIP_PERF_SOAK_MEDIA_CALLS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let call_timeout = Duration::from_secs(
        std::env::var("RVOIP_PERF_CALL_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30),
    );

    let bob_port = support::ports::next_sip_port();
    let alice_port = support::ports::next_sip_port();
    let bob_cfg = perf_config("perf-soak-bob", bob_port);
    let alice_cfg = perf_config("perf-soak-alice", alice_port);
    let rss_gate = RssGrowthGate::resolve(&alice_cfg, &bob_cfg);
    let app_event_capacity = bob_cfg.global_event_channel_capacity;
    let session_event_dispatcher_capacity = bob_cfg.session_event_dispatcher_channel_capacity;
    let sip_transaction_command_channel_capacity = bob_cfg
        .sip_transaction_command_channel_capacity
        .unwrap_or(Config::DEFAULT_SIP_TRANSACTION_COMMAND_CHANNEL_CAPACITY);
    let retention_drain_wait = retention_drain_wait();
    let bob_diagnostics = BobHandlerDiagnostics::default();
    let bob = boot_bob(bob_cfg, bob_diagnostics.clone()).await;
    let alice = boot_alice(alice_cfg).await;
    let from = format!("sip:alice@127.0.0.1:{}", alice_port);
    let target_uri = format!("sip:bob@127.0.0.1:{}", bob_port);

    // Histograms split per minute so we can compute drift.
    // For simplicity v1: one global setup histogram + one "first
    // minute" snapshot + one "last minute" snapshot. Per-minute
    // breakdown is a follow-up.
    let setup_hist = Arc::new(LatencyHistogram::new("setup_latency"));
    let first_minute_hist = Arc::new(LatencyHistogram::new("setup_latency_minute_1"));
    let last_minute_hist = Arc::new(LatencyHistogram::new("setup_latency_last_minute"));
    let counters = Arc::new(SoakCounters::default());

    // Long-lived media calls held for the whole soak.
    let sent_frames = Arc::new(AtomicU64::new(0));
    let (media_drop_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    let mut media_handles: Vec<JoinHandle<()>> = Vec::with_capacity(media_calls as usize);
    for _ in 0..media_calls {
        let alice = Arc::clone(&alice);
        let from = from.clone();
        let target_uri = target_uri.clone();
        let counters = Arc::clone(&counters);
        let sent_frames = Arc::clone(&sent_frames);
        let mut drop_rx = media_drop_tx.subscribe();
        media_handles.push(tokio::spawn(async move {
            let call_id = match alice.invite(Some(from), target_uri).send().await {
                Ok(id) => id,
                Err(_) => {
                    counters.media_setup_failed.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            };
            let handle = alice.session(&call_id);
            if handle.wait_for_answered(Some(call_timeout)).await.is_err() {
                counters.media_setup_failed.fetch_add(1, Ordering::Relaxed);
                return;
            }
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
                let f = AudioFrame::new(vec![0i16; FRAME_SAMPLES], 8_000, 1, 0);
                if sender.send(f).await.is_err() {
                    break;
                }
                sent_frames.fetch_add(1, Ordering::Relaxed);
            }
            let _ = handle.hangup_and_wait(Some(call_timeout)).await;
        }));
    }
    // Give the media plane time to come up before the signalling
    // churn starts.
    tokio::time::sleep(Duration::from_secs(3)).await;

    let sampler = ResourceSampler::start(Duration::from_secs(5));
    let retention_sampler = RetentionSampler::start(
        Arc::clone(&alice),
        Arc::clone(&bob.coordinator),
        Duration::from_secs(5),
    );
    let started = std::time::Instant::now();
    let total = Duration::from_secs(duration_secs);

    // Signalling churn: continuously dispatch INVITE-BYE cycles at
    // `soak_cps`. Each spawned task records its setup latency into
    // the appropriate histogram (first minute / last minute / global).
    let tick = Duration::from_secs_f64(1.0 / soak_cps.max(1.0));
    let mut churn_tasks = JoinSet::<()>::new();
    loop {
        while let Some(result) = churn_tasks.try_join_next() {
            let _ = result;
        }

        let elapsed = started.elapsed();
        if elapsed >= total {
            break;
        }
        let alice = Arc::clone(&alice);
        let from = from.clone();
        let target_uri = target_uri.clone();
        let setup_hist = Arc::clone(&setup_hist);
        let first_minute_hist = Arc::clone(&first_minute_hist);
        let last_minute_hist = Arc::clone(&last_minute_hist);
        let counters = Arc::clone(&counters);
        churn_tasks.spawn(async move {
            let dispatch_at = std::time::Instant::now();
            let t_send = dispatch_at;
            counters.offered.fetch_add(1, Ordering::Relaxed);
            let call_id = match alice.invite(Some(from), target_uri).send().await {
                Ok(id) => id,
                Err(_) => {
                    counters.failed.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            };
            let handle = alice.session(&call_id);
            if handle.wait_for_answered(Some(call_timeout)).await.is_err() {
                counters.failed.fetch_add(1, Ordering::Relaxed);
                return;
            }
            let ns = t_send.elapsed().as_nanos() as u64;
            setup_hist.record_nanos(ns);
            // Place into first or last minute snapshot bucket.
            let secs = dispatch_at
                .duration_since(started_minute_anchor())
                .as_secs_f64();
            // We use a global anchor (set just once via OnceLock).
            // Cheaper: classify by `dispatch_at - started` using a
            // captured `started` clone — but we can't move it cheaply
            // here. Fall back to anchor.
            let _ = secs;
            if dispatch_at.duration_since(started_global()).as_secs() < 60 {
                first_minute_hist.record_nanos(ns);
            }
            if total
                .saturating_sub(dispatch_at.duration_since(started_global()))
                .as_secs()
                <= 60
            {
                last_minute_hist.record_nanos(ns);
            }
            let _ = handle.hangup_and_wait(Some(call_timeout)).await;
            counters.succeeded.fetch_add(1, Ordering::Relaxed);
        });
        tokio::time::sleep(tick).await;
    }

    // Drain churn calls.
    let drain_result = tokio::time::timeout(call_timeout + Duration::from_secs(30), async {
        while let Some(result) = churn_tasks.join_next().await {
            let _ = result;
        }
    })
    .await;
    if drain_result.is_err() {
        churn_tasks.abort_all();
        while let Some(result) = churn_tasks.join_next().await {
            let _ = result;
        }
    }

    // Stop media calls.
    let _ = media_drop_tx.send(());
    let _ = tokio::time::timeout(call_timeout + Duration::from_secs(10), async {
        for h in media_handles {
            let _ = h.await;
        }
    })
    .await;

    tokio::time::sleep(retention_drain_wait).await;
    let retention_samples = retention_sampler.stop().await;
    let final_retention = retention_samples
        .last()
        .cloned()
        .unwrap_or_else(|| json!({}));
    let retained_after_drain = retained_total(&final_retention);
    let resources = sampler.stop().await;

    // Compute drift.
    let fm = first_minute_hist.snapshot();
    let lm = last_minute_hist.snapshot();
    let drift_pct = if fm.p99 > 0 && lm.p99 > 0 {
        ((lm.p99 as f64 - fm.p99 as f64) / fm.p99 as f64) * 100.0
    } else {
        0.0
    };
    let rss_growth_mb_per_hr = resources.rss_growth_mb_per_min * 60.0;
    let rss_sustained_growth_mb_per_hr = resources.rss_tail_growth_mb_per_min * 60.0;
    let rss_post_drain_samples: Vec<ResourceSample> = resources
        .samples
        .iter()
        .filter(|sample| sample.t_secs >= duration_secs as f64)
        .cloned()
        .collect();
    let rss_post_drain_growth_mb_per_hr = rss_growth_mb_per_min(&rss_post_drain_samples) * 60.0;
    let (rss_gate_growth_mb_per_hr, rss_gate_window) = if rss_post_drain_samples.len() >= 2 {
        (rss_post_drain_growth_mb_per_hr, "post_drain")
    } else {
        (rss_sustained_growth_mb_per_hr, "tail")
    };
    let rss_windows = rss_window_summaries(
        &resources.samples,
        duration_secs as f64,
        retention_drain_wait.as_secs_f64(),
    );
    let offered = counters.offered.load(Ordering::Relaxed);
    let succeeded = counters.succeeded.load(Ordering::Relaxed);
    let failed = counters.failed.load(Ordering::Relaxed);
    let media_setup_failed = counters.media_setup_failed.load(Ordering::Relaxed);
    let active_audio_receivers = bob_diagnostics
        .active_audio_receivers
        .load(Ordering::Relaxed);
    let completed_audio_receivers = bob_diagnostics
        .completed_audio_receivers
        .load(Ordering::Relaxed);
    let received_frames = bob_diagnostics.received_frames.load(Ordering::Relaxed);
    let asr = if offered > 0 {
        succeeded as f64 / offered as f64
    } else {
        0.0
    };

    let load = LoadProfile {
        target_cps: soak_cps,
        ramp_secs: 0,
        steady_secs: duration_secs,
        cooldown_secs: retention_drain_wait.as_secs(),
    };
    let mut report = ScenarioReport::new("perf_soak_30min", load);
    let cores = report.environment().cpu_count_physical() as f64;
    let cps_per_core = if cores > 0.0 {
        (succeeded as f64 / duration_secs as f64) / cores
    } else {
        0.0
    };
    report
        .result("duration_secs", duration_secs)
        .result("soak_cps", soak_cps)
        .result("media_calls_held", media_calls)
        .result("global_event_channel_capacity", app_event_capacity)
        .result(
            "session_event_dispatcher_channel_capacity",
            session_event_dispatcher_capacity,
        )
        .result(
            "sip_transaction_command_channel_capacity",
            sip_transaction_command_channel_capacity,
        )
        .result("retention_drain_wait_secs", retention_drain_wait.as_secs())
        .result(
            "retention_drain_reason",
            "default covers UDP non-INVITE Timer J plus transaction runner grace",
        )
        .result("calls_offered", offered)
        .result("calls_succeeded", succeeded)
        .result("cps_per_core", round2(cps_per_core))
        .result("asr", round4(asr))
        .result("bob_received_frames", received_frames)
        .result("bob_active_audio_receivers", active_audio_receivers)
        .result("bob_completed_audio_receivers", completed_audio_receivers)
        // ChatGPT VoIP guidance Tier 1 — long-duration stability is
        // what backs the "predictable, deterministic, no leaks" pitch.
        .result("latency_drift_pct", round2(drift_pct))
        .result("rss_growth_mb_per_hr", round2(rss_growth_mb_per_hr))
        .result(
            "rss_sustained_growth_mb_per_hr",
            round2(rss_sustained_growth_mb_per_hr),
        )
        .result(
            "rss_post_drain_growth_mb_per_hr",
            round2(rss_post_drain_growth_mb_per_hr),
        )
        .result(
            "rss_post_drain_sample_count",
            rss_post_drain_samples.len() as u64,
        )
        .result(
            "rss_gate_growth_mb_per_hr",
            round2(rss_gate_growth_mb_per_hr),
        )
        .result("rss_gate_window", rss_gate_window)
        .result_block("rss_gate", rss_gate.to_json())
        .result("retained_objects_after_drain", retained_after_drain)
        .result(
            "transaction_manager_active_after_drain",
            transaction_manager_active_total(&final_retention),
        )
        .result(
            "transaction_runner_active_after_drain",
            global_retention_metric(
                &final_retention,
                "/sip_dialog_diagnostics/transaction_runner/active",
            ),
        )
        .result(
            "lifecycle_expired_terminal_entries_after_drain",
            lifecycle_expired_terminal_entries_total(&final_retention),
        )
        .result(
            "lifecycle_terminal_entries_after_drain",
            lifecycle_terminal_entries_total(&final_retention),
        )
        .result_block(
            "retention",
            retention_summary(&retention_samples, retained_after_drain),
        )
        .diagnostic_block(
            "retention_samples",
            json!({
                "sample_count": retention_samples.len(),
                "samples": retention_samples,
                "final_retained_objects": retained_after_drain,
            }),
        )
        .diagnostic_block(
            "rss_windows",
            json!({
                "windows": rss_windows,
                "gate_window": rss_gate_window,
                "gate_growth_mb_per_hr": round2(rss_gate_growth_mb_per_hr),
            }),
        )
        .result(
            "errors",
            json!({
                "churn_failed":         failed,
                "media_setup_failed":   media_setup_failed,
            }),
        )
        .latency(&setup_hist)
        .latency(&first_minute_hist)
        .latency(&last_minute_hist)
        .with_resources(resources);
    let json_path = report.write_resources_first_then_write_json_if_supported();
    report.print_summary(&json_path);

    bob.shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(3), bob.task).await;
    drop(alice);

    let mut gate_failures = Vec::new();
    if rss_gate_growth_mb_per_hr > rss_gate.effective_mb_per_hr {
        gate_failures.push(format!(
            "RSS gate growth {:.2} MB/hr over {} window exceeded effective threshold {:.2} MB/hr ({})",
            rss_gate_growth_mb_per_hr, rss_gate_window, rss_gate.effective_mb_per_hr, rss_gate.source
        ));
    }
    if asr < 0.999 {
        gate_failures.push(format!("ASR {:.4} below 0.999", asr));
    }
    if failed != 0 {
        gate_failures.push(format!("churn_failed={failed}"));
    }
    if media_setup_failed != 0 {
        gate_failures.push(format!("media_setup_failed={media_setup_failed}"));
    }
    if retained_after_drain != 0 {
        gate_failures.push(format!(
            "retained_objects_after_drain={retained_after_drain}"
        ));
    }
    if active_audio_receivers != 0 {
        gate_failures.push(format!(
            "bob_active_audio_receivers={active_audio_receivers}"
        ));
    }
    assert!(
        gate_failures.is_empty(),
        "perf_soak_30min gate failed:\n{}",
        gate_failures.join("\n")
    );
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_session_churn_leak() {
    let churn_calls: u64 = std::env::var("RVOIP_PERF_LEAK_CHURN_CALLS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(250);
    let call_timeout = Duration::from_secs(
        std::env::var("RVOIP_PERF_CALL_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30),
    );

    let bob_port = support::ports::next_sip_port();
    let alice_port = support::ports::next_sip_port();
    let bob_cfg = perf_config("perf-leak-bob", bob_port);
    let alice_cfg = perf_config("perf-leak-alice", alice_port);
    let app_event_capacity = bob_cfg.global_event_channel_capacity;
    let session_event_dispatcher_capacity = bob_cfg.session_event_dispatcher_channel_capacity;
    let sip_transaction_command_channel_capacity = bob_cfg
        .sip_transaction_command_channel_capacity
        .unwrap_or(Config::DEFAULT_SIP_TRANSACTION_COMMAND_CHANNEL_CAPACITY);
    let retention_drain_wait = retention_drain_wait();
    let bob_diagnostics = BobHandlerDiagnostics::default();
    let bob = boot_bob(bob_cfg, bob_diagnostics.clone()).await;
    let alice = boot_alice(alice_cfg).await;
    let from = format!("sip:alice@127.0.0.1:{}", alice_port);
    let target_uri = format!("sip:bob@127.0.0.1:{}", bob_port);
    let setup_hist = LatencyHistogram::new("setup_latency");
    let sampler = ResourceSampler::start(Duration::from_secs(1));
    let started = std::time::Instant::now();

    let mut succeeded = 0_u64;
    let mut failed = 0_u64;
    for _ in 0..churn_calls {
        let sent = std::time::Instant::now();
        match alice
            .invite(Some(from.clone()), target_uri.clone())
            .send()
            .await
        {
            Ok(call_id) => {
                let handle = alice.session(&call_id);
                if handle.wait_for_answered(Some(call_timeout)).await.is_ok() {
                    setup_hist.record_nanos(sent.elapsed().as_nanos() as u64);
                    let _ = handle.hangup_and_wait(Some(call_timeout)).await;
                    succeeded += 1;
                } else {
                    failed += 1;
                }
            }
            Err(_) => {
                failed += 1;
            }
        }
    }

    tokio::time::sleep(retention_drain_wait).await;
    let final_retention =
        capture_retention_sample("after_churn", started, &alice, &bob.coordinator).await;
    let retained_after_churn = retained_total(&final_retention);
    let active_audio_receivers = bob_diagnostics
        .active_audio_receivers
        .load(Ordering::Relaxed);
    let completed_audio_receivers = bob_diagnostics
        .completed_audio_receivers
        .load(Ordering::Relaxed);
    let resources = sampler.stop().await;

    let load = LoadProfile {
        target_cps: 0.0,
        ramp_secs: 0,
        steady_secs: started.elapsed().as_secs(),
        cooldown_secs: retention_drain_wait.as_secs(),
    };
    let mut report = ScenarioReport::new("perf_session_churn_leak", load);
    report
        .result("calls_offered", churn_calls)
        .result("global_event_channel_capacity", app_event_capacity)
        .result(
            "session_event_dispatcher_channel_capacity",
            session_event_dispatcher_capacity,
        )
        .result(
            "sip_transaction_command_channel_capacity",
            sip_transaction_command_channel_capacity,
        )
        .result("retention_drain_wait_secs", retention_drain_wait.as_secs())
        .result(
            "retention_drain_reason",
            "default covers UDP non-INVITE Timer J plus transaction runner grace",
        )
        .result("calls_succeeded", succeeded)
        .result("calls_failed", failed)
        .result("retained_objects_after_churn", retained_after_churn)
        .result(
            "transaction_manager_active_after_churn",
            transaction_manager_active_total(&final_retention),
        )
        .result(
            "transaction_runner_active_after_churn",
            global_retention_metric(
                &final_retention,
                "/sip_dialog_diagnostics/transaction_runner/active",
            ),
        )
        .result(
            "lifecycle_expired_terminal_entries_after_churn",
            lifecycle_expired_terminal_entries_total(&final_retention),
        )
        .result(
            "lifecycle_terminal_entries_after_churn",
            lifecycle_terminal_entries_total(&final_retention),
        )
        .result("bob_active_audio_receivers", active_audio_receivers)
        .result("bob_completed_audio_receivers", completed_audio_receivers)
        .result_block(
            "retention_final",
            retention_sample_summary(&final_retention),
        )
        .diagnostic_block("retention_final_raw", final_retention)
        .latency(&setup_hist)
        .with_resources(resources);
    let json_path = report.write_resources_first_then_write_json_if_supported();
    report.print_summary(&json_path);

    bob.shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(3), bob.task).await;
    drop(alice);

    assert_eq!(failed, 0, "all churn calls should complete");
    assert_eq!(
        retained_after_churn, 0,
        "completed call churn retained per-call objects"
    );
    assert_eq!(
        active_audio_receivers, 0,
        "completed call churn left audio receiver tasks active"
    );
}

struct RssGrowthGate {
    effective_mb_per_hr: f64,
    source: &'static str,
    env_override_mb_per_hr: Option<f64>,
    alice_config_mb_per_hr: Option<f64>,
    bob_config_mb_per_hr: Option<f64>,
}

impl RssGrowthGate {
    fn resolve(alice: &Config, bob: &Config) -> Self {
        let env_override = read_positive_f64_env("RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR");
        let alice_config = alice.perf_max_rss_growth_mb_per_hr;
        let bob_config = bob.perf_max_rss_growth_mb_per_hr;

        let (effective, source) = if let Some(env) = env_override {
            (env, "env:RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR")
        } else {
            match (alice_config, bob_config) {
                (Some(a), Some(b)) => (a.min(b), "config:strictest_endpoint"),
                (Some(a), None) => (a, "config:alice"),
                (None, Some(b)) => (b, "config:bob"),
                (None, None) => (
                    Config::DEFAULT_PERF_MAX_RSS_GROWTH_MB_PER_HR,
                    "config:default",
                ),
            }
        };

        Self {
            effective_mb_per_hr: effective,
            source,
            env_override_mb_per_hr: env_override,
            alice_config_mb_per_hr: alice_config,
            bob_config_mb_per_hr: bob_config,
        }
    }

    fn to_json(&self) -> serde_json::Value {
        json!({
            "effective_mb_per_hr": self.effective_mb_per_hr,
            "source": self.source,
            "env_override_mb_per_hr": self.env_override_mb_per_hr,
            "alice_config_mb_per_hr": self.alice_config_mb_per_hr,
            "bob_config_mb_per_hr": self.bob_config_mb_per_hr,
            "default_mb_per_hr": Config::DEFAULT_PERF_MAX_RSS_GROWTH_MB_PER_HR,
        })
    }
}

fn read_positive_f64_env(name: &str) -> Option<f64> {
    let raw = match std::env::var(name) {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return None,
        Err(err) => panic!("{name} could not be read: {err}"),
    };
    let value: f64 = raw
        .parse()
        .unwrap_or_else(|_| panic!("{name} must be a finite number greater than 0, got {raw:?}"));
    assert!(
        value.is_finite() && value > 0.0,
        "{name} must be a finite number greater than 0, got {raw:?}"
    );
    Some(value)
}

fn read_positive_usize_env(name: &str) -> Option<usize> {
    let raw = match std::env::var(name) {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return None,
        Err(err) => panic!("{name} could not be read: {err}"),
    };
    let value: usize = raw
        .parse()
        .unwrap_or_else(|_| panic!("{name} must be a positive integer, got {raw:?}"));
    assert!(value > 0, "{name} must be a positive integer, got {raw:?}");
    Some(value)
}

struct RetentionSampler {
    stop_tx: tokio::sync::watch::Sender<bool>,
    task: JoinHandle<Vec<serde_json::Value>>,
}

impl RetentionSampler {
    fn start(
        alice: Arc<UnifiedCoordinator>,
        bob: Arc<UnifiedCoordinator>,
        interval: Duration,
    ) -> Self {
        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(async move {
            let started = std::time::Instant::now();
            let mut samples = Vec::new();
            loop {
                samples.push(capture_retention_sample("periodic", started, &alice, &bob).await);
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {}
                    _ = stop_rx.changed() => break,
                }
            }
            samples.push(capture_retention_sample("after_drain", started, &alice, &bob).await);
            samples
        });
        Self { stop_tx, task }
    }

    async fn stop(self) -> Vec<serde_json::Value> {
        let _ = self.stop_tx.send(true);
        self.task.await.unwrap_or_default()
    }
}

async fn capture_retention_sample(
    label: &'static str,
    started: std::time::Instant,
    alice: &Arc<UnifiedCoordinator>,
    bob: &Arc<UnifiedCoordinator>,
) -> serde_json::Value {
    let alice_snapshot = alice.perf_diagnostic_snapshot().await;
    let bob_snapshot = bob.perf_diagnostic_snapshot().await;
    let sample = json!({
        "label": label,
        "t_secs": round2(started.elapsed().as_secs_f64()),
        "alice": alice_snapshot,
        "bob": bob_snapshot,
    });
    let retained = retained_total(&sample);
    json!({
        "label": label,
        "t_secs": round2(started.elapsed().as_secs_f64()),
        "retained_total": retained,
        "alice": sample["alice"].clone(),
        "bob": sample["bob"].clone(),
    })
}

fn retained_total(sample: &serde_json::Value) -> u64 {
    endpoint_retained_total(&sample["alice"])
        + endpoint_retained_total(&sample["bob"])
        + global_retained_total(sample)
}

fn global_retained_total(sample: &serde_json::Value) -> u64 {
    const POINTERS: &[&str] = &[
        "/sip_dialog_diagnostics/transaction_runner/active",
        "/sip_dialog_diagnostics/transaction_cleanup/in_flight",
    ];

    POINTERS
        .iter()
        .map(|pointer| global_retention_metric(sample, pointer))
        .sum()
}

fn global_retention_metric(sample: &serde_json::Value, pointer: &str) -> u64 {
    // rvoip-sip-dialog diagnostics are process-global counters. They are
    // included in both endpoint snapshots for context, but retained-object
    // totals must count them once.
    sample["alice"]
        .pointer(pointer)
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
}

fn transaction_manager_active_total(sample: &serde_json::Value) -> u64 {
    endpoint_metric(&sample["alice"], "/transaction_manager/total")
        + endpoint_metric(&sample["bob"], "/transaction_manager/total")
}

fn lifecycle_expired_terminal_entries_total(sample: &serde_json::Value) -> u64 {
    endpoint_metric(&sample["alice"], "/lifecycle/expired_terminal_entries")
        + endpoint_metric(&sample["bob"], "/lifecycle/expired_terminal_entries")
}

fn lifecycle_terminal_entries_total(sample: &serde_json::Value) -> u64 {
    endpoint_metric(&sample["alice"], "/lifecycle/terminal_entries")
        + endpoint_metric(&sample["bob"], "/lifecycle/terminal_entries")
}

fn endpoint_metric(snapshot: &serde_json::Value, pointer: &str) -> u64 {
    snapshot
        .pointer(pointer)
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
}

fn retention_summary(samples: &[serde_json::Value], final_retained: u64) -> serde_json::Value {
    let max_retained_objects = samples
        .iter()
        .filter_map(|sample| sample["retained_total"].as_u64())
        .max()
        .unwrap_or(0);
    json!({
        "sample_count": samples.len(),
        "max_retained_objects": max_retained_objects,
        "final_retained_objects": final_retained,
        "first": samples.first().map(retention_sample_summary),
        "last": samples.last().map(retention_sample_summary),
    })
}

fn retention_sample_summary(sample: &serde_json::Value) -> serde_json::Value {
    json!({
        "label": sample["label"].clone(),
        "t_secs": sample["t_secs"].clone(),
        "retained_total": sample["retained_total"].clone(),
        "alice": endpoint_summary(&sample["alice"]),
        "bob": endpoint_summary(&sample["bob"]),
    })
}

fn endpoint_summary(snapshot: &serde_json::Value) -> serde_json::Value {
    json!({
        "session_store": snapshot["session_store"].clone(),
        "session_registry": snapshot["session_registry"].clone(),
        "lifecycle": snapshot["lifecycle"].clone(),
        "state_machine_helpers": snapshot["state_machine_helpers"].clone(),
        "transaction_manager": snapshot["transaction_manager"].clone(),
        "dialog_manager": snapshot["dialog_manager"].clone(),
        "dialog_adapter": snapshot["dialog_adapter"].clone(),
        "media_adapter": snapshot["media_adapter"].clone(),
        "sip_dialog_diagnostics": snapshot["sip_dialog_diagnostics"].clone(),
        "cleanup": snapshot["cleanup"].clone(),
    })
}

fn endpoint_retained_total(snapshot: &serde_json::Value) -> u64 {
    const POINTERS: &[&str] = &[
        "/session_store/total",
        "/session_registry/sessions",
        "/state_machine_helpers/active_sessions",
        "/state_machine_helpers/subscriber_sessions",
        "/dialog_adapter/session_to_dialog",
        "/dialog_adapter/dialog_to_session",
        "/dialog_adapter/callid_to_session",
        "/dialog_adapter/outgoing_invite_tx",
        "/dialog_adapter/registration_refresh_tasks",
        "/lifecycle/expired_terminal_entries",
        "/transaction_manager/total",
        "/transaction_manager/terminated_transactions",
        "/transaction_manager/server_invite_dialog_index",
        "/transaction_manager/server_invite_dialog_keys_by_tx",
        "/transaction_manager/invite_2xx_response_cache",
        "/transaction_manager/invite_2xx_response_due_queue",
        "/transaction_manager/transaction_destinations",
        "/transaction_manager/subscriber_to_transactions",
        "/transaction_manager/transaction_to_subscribers",
        "/transaction_manager/pending_inbound_bytes",
        "/transaction_manager/pending_inbound_timing",
        "/dialog_manager/dialogs",
        "/dialog_manager/dialog_lookup",
        "/dialog_manager/early_dialog_lookup",
        "/dialog_manager/terminated_bye_lookup",
        "/dialog_manager/transaction_to_dialog",
        "/dialog_manager/transaction_dialog_route_hash",
        "/dialog_manager/dialog_invite_transactions",
        "/dialog_manager/dialog_server_transactions",
        "/dialog_manager/pending_response_transaction_by_dialog",
        "/dialog_manager/session_to_dialog",
        "/dialog_manager/dialog_to_session",
        "/dialog_manager/reliable_provisional_tasks",
        "/dialog_manager/session_refresh_tasks",
        "/dialog_manager/outbound_flows",
        "/dialog_manager/outbound_flow_tasks",
        "/dialog_manager/flow_by_destination",
        "/dialog_manager/flow_by_aor",
        "/media_adapter/session_to_dialog",
        "/media_adapter/dialog_to_session",
        "/media_adapter/media_sessions",
        "/media_adapter/audio_receivers",
        "/media_adapter/pending_srtp_offerers",
        "/media_adapter/negotiated_srtp",
        "/media_adapter/audio_mixers",
        "/media_adapter/controller/sessions",
        "/media_adapter/controller/rtp_sessions",
        "/media_adapter/controller/session_to_media",
        "/media_adapter/controller/media_to_session",
        "/media_adapter/controller/audio_frame_callbacks",
        "/media_adapter/controller/dtmf_callbacks",
        "/media_adapter/controller/bridge_partners",
        "/media_adapter/controller/cn_gate_state",
        "/media_adapter/controller/advanced_processors",
        "/media_adapter/controller/media_directions",
        "/cleanup/active_total",
    ];

    POINTERS
        .iter()
        .map(|pointer| {
            snapshot
                .pointer(pointer)
                .and_then(|value| value.as_u64())
                .unwrap_or(0)
        })
        .sum()
}

#[derive(Default)]
struct SoakCounters {
    offered: AtomicU64,
    succeeded: AtomicU64,
    failed: AtomicU64,
    media_setup_failed: AtomicU64,
}

// Global anchors so the bucketing classifier doesn't need plumbing.
// First call from the test sets them; subsequent calls return the
// stored values.
fn started_global() -> std::time::Instant {
    use std::sync::OnceLock;
    static ANCHOR: OnceLock<std::time::Instant> = OnceLock::new();
    *ANCHOR.get_or_init(std::time::Instant::now)
}
fn started_minute_anchor() -> std::time::Instant {
    started_global()
}

trait WriteJsonFallback {
    fn write_resources_first_then_write_json_if_supported(&self) -> std::path::PathBuf;
}

impl WriteJsonFallback for ScenarioReport {
    fn write_resources_first_then_write_json_if_supported(&self) -> std::path::PathBuf {
        self.write_json()
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

fn rss_growth_mb_per_min(samples: &[ResourceSample]) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }

    let n = samples.len() as f64;
    let sum_x: f64 = samples.iter().map(|sample| sample.t_secs).sum();
    let sum_y: f64 = samples.iter().map(|sample| sample.rss_mb).sum();
    let sum_xy: f64 = samples
        .iter()
        .map(|sample| sample.t_secs * sample.rss_mb)
        .sum();
    let sum_xx: f64 = samples
        .iter()
        .map(|sample| sample.t_secs * sample.t_secs)
        .sum();
    let denom = n * sum_xx - sum_x * sum_x;
    if denom.abs() < f64::EPSILON {
        return 0.0;
    }

    ((n * sum_xy - sum_x * sum_y) / denom) * 60.0
}

fn rss_window_summaries(
    samples: &[ResourceSample],
    active_secs: f64,
    drain_secs: f64,
) -> Vec<serde_json::Value> {
    let total_secs = active_secs + drain_secs;
    let mut windows = Vec::new();
    let mut start = 0.0;

    while start < total_secs {
        let end = (start + 60.0).min(total_secs);
        let window_samples: Vec<ResourceSample> = samples
            .iter()
            .filter(|sample| sample.t_secs >= start && sample.t_secs <= end)
            .cloned()
            .collect();
        if let (Some(first), Some(last)) = (window_samples.first(), window_samples.last()) {
            windows.push(json!({
                "label": if start >= active_secs { "drain" } else { "active" },
                "start_secs": round2(start),
                "end_secs": round2(end),
                "sample_count": window_samples.len(),
                "first_rss_mb": round2(first.rss_mb),
                "last_rss_mb": round2(last.rss_mb),
                "delta_mb": round2(last.rss_mb - first.rss_mb),
                "growth_mb_per_hr": round2(rss_growth_mb_per_min(&window_samples) * 60.0),
            }));
        }
        start += 60.0;
    }

    let drain_samples: Vec<ResourceSample> = samples
        .iter()
        .filter(|sample| sample.t_secs >= active_secs)
        .cloned()
        .collect();
    if let (Some(first), Some(last)) = (drain_samples.first(), drain_samples.last()) {
        windows.push(json!({
            "label": "post_drain",
            "start_secs": round2(active_secs),
            "end_secs": round2(active_secs + drain_secs),
            "sample_count": drain_samples.len(),
            "first_rss_mb": round2(first.rss_mb),
            "last_rss_mb": round2(last.rss_mb),
            "delta_mb": round2(last.rss_mb - first.rss_mb),
            "growth_mb_per_hr": round2(rss_growth_mb_per_min(&drain_samples) * 60.0),
        }));
    }

    windows
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn with_rss_env<T>(value: Option<&str>, f: impl FnOnce() -> T) -> T {
        let _guard = env_lock().lock().expect("env lock");
        let key = "RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR";
        let old = std::env::var(key).ok();
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
        let out = f();
        match old {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
        out
    }

    #[test]
    fn rss_gate_uses_default_without_env_or_config() {
        with_rss_env(None, || {
            let alice = Config::local("alice", 5060);
            let bob = Config::local("bob", 5062);
            let gate = RssGrowthGate::resolve(&alice, &bob);
            assert_eq!(
                gate.effective_mb_per_hr,
                Config::DEFAULT_PERF_MAX_RSS_GROWTH_MB_PER_HR
            );
            assert_eq!(gate.source, "config:default");
        });
    }

    #[test]
    fn rss_gate_uses_strictest_endpoint_config() {
        with_rss_env(None, || {
            let alice = Config::local("alice", 5060).with_perf_max_rss_growth_mb_per_hr(8.0);
            let bob = Config::local("bob", 5062).with_perf_max_rss_growth_mb_per_hr(3.0);
            let gate = RssGrowthGate::resolve(&alice, &bob);
            assert_eq!(gate.effective_mb_per_hr, 3.0);
            assert_eq!(gate.source, "config:strictest_endpoint");
        });
    }

    #[test]
    fn rss_gate_env_override_wins_over_config() {
        with_rss_env(Some("25"), || {
            let alice = Config::local("alice", 5060).with_perf_max_rss_growth_mb_per_hr(8.0);
            let bob = Config::local("bob", 5062).with_perf_max_rss_growth_mb_per_hr(3.0);
            let gate = RssGrowthGate::resolve(&alice, &bob);
            assert_eq!(gate.effective_mb_per_hr, 25.0);
            assert_eq!(gate.source, "env:RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR");
        });
    }
}

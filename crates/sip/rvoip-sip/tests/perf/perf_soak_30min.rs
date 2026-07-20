//! Scenario 8 — long-duration soak (default 30 min, `#[ignore]`).
//!
//! Maintains a cycling pool of active RTP calls carrying PCMU, with an
//! optional immediate setup/teardown signalling churn stream. Run for
//! `RVOIP_PERF_SOAK_DURATION_SECS` (default 30 min). Marked `#[ignore]`
//! so it opts in via `-- --ignored`.
//!
//! The headline output is **latency drift** and **memory growth**:
//!
//! - `latency_drift_pct` — `(setup p99 in last minute) / (setup p99 in
//!   first minute) - 1`. Should stay well under 50 % on a healthy run.
//! - `rss_gate_growth_mb_per_hr` — projection of the final 600 seconds under
//!   active load for a long soak. Short diagnostic runs fall back to the
//!   post-drain or final tail slope. Qualifying the active window prevents a
//!   per-call leak from being hidden by the plateau after load stops.
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
//! - `RVOIP_PERF_SOAK_ACTIVE_CALLS`  (default 30 — cycling RTP calls)
//! - `RVOIP_PERF_SOAK_MIN_HOLD_SECS` (default 10)
//! - `RVOIP_PERF_SOAK_MAX_HOLD_SECS` (default 360)
//! - `RVOIP_PERF_SOAK_CPS`           (default 0 — optional immediate hangup churn)
//! - `RVOIP_PERF_SOAK_DRAIN_CPS`     (default 10 — paced end-of-soak hangups)
//! - `RVOIP_PERF_SOAK_ERROR_SAMPLE_LIMIT` (default 32)
//! - `RVOIP_PERF_SOAK_MEDIA_CALLS`   (legacy alias for active calls)
//! - `RVOIP_PERF_CALL_TIMEOUT_SECS`  (default 30)
//! - `RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR` (default from `Config`)
//! - `RVOIP_PERF_APP_EVENT_CHANNEL_CAPACITY` (default 256)
//! - `RVOIP_PERF_SIP_TRANSACTION_COMMAND_CHANNEL_CAPACITY` (default from `Config`)
//! - `RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS` (default 40; covers UDP Timer J)
//! - `RVOIP_PERF_RSS_TAIL_WINDOW_SECS` (default 60)

#![allow(clippy::needless_return)]

use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rvoip_sip::api::callback_peer::{
    CallHandler, CallHandlerDecision, CallbackPeer, ShutdownHandle,
};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::{AudioSource, Config, UnifiedCoordinator};
use rvoip_sip::{SessionError, SessionId};
use serde_json::json;
use tokio::task::{JoinHandle, JoinSet};

#[path = "support/mod.rs"]
mod support;
use support::{LatencyHistogram, LoadProfile, ResourceSample, ResourceSampler, ScenarioReport};

const DEFAULT_PERF_APP_EVENT_CHANNEL_CAPACITY: usize = Config::DEFAULT_APP_EVENT_CHANNEL_CAPACITY;
const DEFAULT_RETENTION_DRAIN_WAIT_SECS: usize = 40;
const DEFAULT_CONTROLLED_DRAIN_CPS: f64 = 10.0;
const DEFAULT_ERROR_SAMPLE_LIMIT: usize = 32;
const MAX_ERROR_MESSAGE_CHARS: usize = 256;
const LONG_SOAK_ACTIVE_RSS_WINDOW_SECS: f64 = 600.0;
const LONG_SOAK_MIN_ACTIVE_RSS_COVERAGE_SECS: f64 = 590.0;
const LONG_SOAK_MIN_ACTIVE_RSS_SAMPLES: usize = 110;

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

#[derive(Default)]
struct FailureDiagnosticsState {
    total: u64,
    by_phase_and_class: BTreeMap<(String, String), u64>,
    samples: Vec<serde_json::Value>,
}

#[derive(Clone)]
struct FailureDiagnostics {
    sample_limit: usize,
    state: Arc<Mutex<FailureDiagnosticsState>>,
}

impl FailureDiagnostics {
    fn new(sample_limit: usize) -> Self {
        Self {
            sample_limit,
            state: Arc::new(Mutex::new(FailureDiagnosticsState::default())),
        }
    }

    fn record(
        &self,
        phase: &'static str,
        error: &SessionError,
        call_id: Option<&SessionId>,
        elapsed: Duration,
    ) {
        let error_class = session_error_class(error);
        self.record_class(phase, error_class, call_id, elapsed, error_class);
    }

    fn record_class(
        &self,
        phase: &'static str,
        error_class: &'static str,
        call_id: Option<&SessionId>,
        elapsed: Duration,
        message: &str,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("failure diagnostics lock poisoned");
        state.total += 1;
        *state
            .by_phase_and_class
            .entry((phase.to_string(), error_class.to_string()))
            .or_default() += 1;
        if state.samples.len() < self.sample_limit {
            let sequence = state.total;
            let sample = json!({
                "sequence": sequence,
                "phase": phase,
                "error_class": error_class,
                "call_id_hash": call_id.map(hash_session_id),
                "elapsed_ms": round2(elapsed.as_secs_f64() * 1_000.0),
                "message": truncate_chars(message, MAX_ERROR_MESSAGE_CHARS),
            });
            state.samples.push(sample);
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        let state = self
            .state
            .lock()
            .expect("failure diagnostics lock poisoned");
        let counts = state
            .by_phase_and_class
            .iter()
            .map(|((phase, error_class), count)| {
                json!({
                    "phase": phase,
                    "error_class": error_class,
                    "count": count,
                })
            })
            .collect::<Vec<_>>();
        json!({
            "total": state.total,
            "sample_limit": self.sample_limit,
            "sampled": state.samples.len(),
            "dropped_samples": state.total.saturating_sub(state.samples.len() as u64),
            "by_phase_and_class": counts,
            "samples": state.samples.clone(),
        })
    }
}

fn session_error_class(error: &SessionError) -> &'static str {
    match error {
        SessionError::SessionNotFound(_) => "session_not_found",
        SessionError::InvalidTransition(_) => "invalid_transition",
        SessionError::DialogError(_) => "dialog_error",
        SessionError::MediaError(detail) => classify_media_detail(detail, "media_error"),
        SessionError::MediaIntegration { reason } => {
            classify_media_detail(reason, "media_integration")
        }
        SessionError::SDPNegotiationFailed(_) => "sdp_negotiation",
        SessionError::ConfigurationError(_) | SessionError::ConfigError(_) => "configuration",
        SessionError::InvalidInput(_) => "invalid_input",
        SessionError::Timeout(_) => "timeout",
        SessionError::NetworkError(_) => "network_error",
        SessionError::ProtocolError(_) => "protocol_error",
        SessionError::IoError(error) => match error.kind() {
            std::io::ErrorKind::AddrInUse => "io_address_in_use",
            std::io::ErrorKind::AddrNotAvailable => "io_address_not_available",
            std::io::ErrorKind::ConnectionRefused => "io_connection_refused",
            std::io::ErrorKind::TimedOut => "io_timeout",
            _ => "io_error",
        },
        SessionError::InternalError(_) => "internal_error",
        SessionError::Other(_) => "other",
        _ => "session_error",
    }
}

fn classify_media_detail(detail: &str, fallback: &'static str) -> &'static str {
    if detail.contains("[kind=port_pool_exhausted]") {
        "port_pool_exhausted"
    } else if detail.contains("[kind=rtp_bind_collision]") {
        "rtp_bind_collision"
    } else if detail.contains("[kind=rtp_session_creation]") {
        "rtp_session_creation"
    } else {
        fallback
    }
}

fn hash_session_id(session_id: &SessionId) -> String {
    let mut hasher = DefaultHasher::new();
    session_id.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
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
    if let Some(seconds) = read_nonnegative_u64_env("RVOIP_PERF_SETUP_TEARDOWN_TIMEOUT_SECS") {
        config = config.with_setup_teardown_timeout_secs(seconds);
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
    let soak_cps: f64 = read_nonnegative_f64_env("RVOIP_PERF_SOAK_CPS").unwrap_or(0.0);
    let active_calls: u64 = std::env::var("RVOIP_PERF_SOAK_ACTIVE_CALLS")
        .or_else(|_| std::env::var("RVOIP_PERF_SOAK_MEDIA_CALLS"))
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    assert!(
        active_calls > 0,
        "RVOIP_PERF_SOAK_ACTIVE_CALLS must be greater than 0"
    );
    let min_hold_secs: u64 = std::env::var("RVOIP_PERF_SOAK_MIN_HOLD_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let max_hold_secs: u64 = std::env::var("RVOIP_PERF_SOAK_MAX_HOLD_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(360);
    assert!(
        min_hold_secs > 0 && max_hold_secs >= min_hold_secs,
        "RVOIP_PERF_SOAK_MIN_HOLD_SECS must be > 0 and <= RVOIP_PERF_SOAK_MAX_HOLD_SECS"
    );
    let call_timeout = Duration::from_secs(
        std::env::var("RVOIP_PERF_CALL_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30),
    );
    let controlled_drain_cps =
        read_positive_f64_env("RVOIP_PERF_SOAK_DRAIN_CPS").unwrap_or(DEFAULT_CONTROLLED_DRAIN_CPS);
    let error_sample_limit = read_positive_usize_env("RVOIP_PERF_SOAK_ERROR_SAMPLE_LIMIT")
        .unwrap_or(DEFAULT_ERROR_SAMPLE_LIMIT);

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
    let failures = Arc::new(FailureDiagnostics::new(error_sample_limit));

    let total = Duration::from_secs(duration_secs);
    let started = std::time::Instant::now();
    let active_deadline = started + total;

    let sampler = ResourceSampler::start(Duration::from_secs(5));
    let retention_sampler = RetentionSampler::start(
        Arc::clone(&alice),
        Arc::clone(&bob.coordinator),
        Duration::from_secs(5),
    );

    // Cycling active media pool. Replenishment stops early enough to avoid
    // starting calls that cannot complete inside the active window. Calls
    // still active at the deadline are handed to the paced drain below rather
    // than all dispatching BYE at the same instant.
    let mut active_tasks = JoinSet::<Option<PendingDrain>>::new();
    for slot in 0..active_calls {
        let alice = Arc::clone(&alice);
        let from = from.clone();
        let target_uri = target_uri.clone();
        let counters = Arc::clone(&counters);
        let failures = Arc::clone(&failures);
        let setup_hist = Arc::clone(&setup_hist);
        let first_minute_hist = Arc::clone(&first_minute_hist);
        let last_minute_hist = Arc::clone(&last_minute_hist);
        active_tasks.spawn(async move {
            let mut cycle = 0u64;
            loop {
                let now = std::time::Instant::now();
                if now >= active_deadline {
                    return None;
                }
                let remaining_before_stop = active_deadline.saturating_duration_since(now);
                if remaining_before_stop <= setup_teardown_budget(call_timeout) {
                    if !remaining_before_stop.is_zero() {
                        tokio::time::sleep(remaining_before_stop).await;
                    }
                    return None;
                }

                let dispatch_at = std::time::Instant::now();
                counters.offered.fetch_add(1, Ordering::Relaxed);
                counters.active_offered.fetch_add(1, Ordering::Relaxed);
                let call_id = match alice
                    .invite(Some(from.clone()), target_uri.clone())
                    .send()
                    .await
                {
                    Ok(id) => id,
                    Err(error) => {
                        counters.failed.fetch_add(1, Ordering::Relaxed);
                        counters.invite_send_failed.fetch_add(1, Ordering::Relaxed);
                        failures.record("active_invite_send", &error, None, dispatch_at.elapsed());
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                let handle = alice.session(&call_id);
                if let Err(error) = handle.wait_for_answered(Some(call_timeout)).await {
                    counters.failed.fetch_add(1, Ordering::Relaxed);
                    counters.answer_failed.fetch_add(1, Ordering::Relaxed);
                    if matches!(&error, SessionError::Timeout(_)) {
                        counters.answer_timeout.fetch_add(1, Ordering::Relaxed);
                    }
                    failures.record(
                        "active_wait_answered",
                        &error,
                        Some(&call_id),
                        dispatch_at.elapsed(),
                    );
                    let cleanup_started = std::time::Instant::now();
                    if let Err(error) = handle.hangup_and_wait(Some(call_timeout)).await {
                        counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
                        failures.record(
                            "active_setup_failure_cleanup",
                            &error,
                            Some(&call_id),
                            cleanup_started.elapsed(),
                        );
                    }
                    continue;
                }

                let ns = dispatch_at.elapsed().as_nanos() as u64;
                setup_hist.record_nanos(ns);
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

                if let Err(error) = alice
                    .set_audio_source(
                        &call_id,
                        AudioSource::Tone {
                            frequency: 440.0,
                            amplitude: 0.25,
                        },
                    )
                    .await
                {
                    counters.failed.fetch_add(1, Ordering::Relaxed);
                    counters.media_setup_failed.fetch_add(1, Ordering::Relaxed);
                    failures.record(
                        "active_audio_source",
                        &error,
                        Some(&call_id),
                        dispatch_at.elapsed(),
                    );
                    let cleanup_started = std::time::Instant::now();
                    if let Err(error) = handle.hangup_and_wait(Some(call_timeout)).await {
                        counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
                        failures.record(
                            "active_media_failure_cleanup",
                            &error,
                            Some(&call_id),
                            cleanup_started.elapsed(),
                        );
                    }
                    continue;
                }

                let hold = cycling_hold_duration(slot, cycle, min_hold_secs, max_hold_secs);
                let natural_hold_deadline = std::time::Instant::now() + hold;
                let hold_deadline = natural_hold_deadline.min(active_deadline);
                let remaining = hold_deadline.saturating_duration_since(std::time::Instant::now());
                if !remaining.is_zero() {
                    tokio::time::sleep(remaining).await;
                }

                if natural_hold_deadline >= active_deadline {
                    return Some(PendingDrain {
                        slot,
                        cycle,
                        call_id,
                    });
                }

                let teardown_started = std::time::Instant::now();
                match handle.hangup_and_wait(Some(call_timeout)).await {
                    Ok(_) => {
                        counters.succeeded.fetch_add(1, Ordering::Relaxed);
                        counters.active_succeeded.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error) => {
                        counters.failed.fetch_add(1, Ordering::Relaxed);
                        counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
                        failures.record(
                            "active_natural_hangup",
                            &error,
                            Some(&call_id),
                            teardown_started.elapsed(),
                        );
                    }
                }
                cycle += 1;
            }
        });
    }

    // Optional signalling churn: continuously dispatch INVITE-BYE cycles at
    // `soak_cps`. The cycling active-call pool above is the normal soak load;
    // this path is retained as an explicit additional stress knob.
    let mut churn_tasks = JoinSet::<()>::new();
    if soak_cps > 0.0 {
        let tick = Duration::from_secs_f64(1.0 / soak_cps);
        loop {
            while let Some(result) = churn_tasks.try_join_next() {
                if let Err(_error) = result {
                    counters.failed.fetch_add(1, Ordering::Relaxed);
                    counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
                    failures.record_class(
                        "churn_task_join",
                        "task_join_error",
                        None,
                        started.elapsed(),
                        "task join failed before producing a result",
                    );
                }
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
            let failures = Arc::clone(&failures);
            churn_tasks.spawn(async move {
                let dispatch_at = std::time::Instant::now();
                let t_send = dispatch_at;
                counters.offered.fetch_add(1, Ordering::Relaxed);
                counters.churn_offered.fetch_add(1, Ordering::Relaxed);
                let call_id = match alice.invite(Some(from), target_uri).send().await {
                    Ok(id) => id,
                    Err(error) => {
                        counters.failed.fetch_add(1, Ordering::Relaxed);
                        counters.invite_send_failed.fetch_add(1, Ordering::Relaxed);
                        failures.record("churn_invite_send", &error, None, dispatch_at.elapsed());
                        return;
                    }
                };
                let handle = alice.session(&call_id);
                if let Err(error) = handle.wait_for_answered(Some(call_timeout)).await {
                    counters.failed.fetch_add(1, Ordering::Relaxed);
                    counters.answer_failed.fetch_add(1, Ordering::Relaxed);
                    if matches!(&error, SessionError::Timeout(_)) {
                        counters.answer_timeout.fetch_add(1, Ordering::Relaxed);
                    }
                    failures.record(
                        "churn_wait_answered",
                        &error,
                        Some(&call_id),
                        dispatch_at.elapsed(),
                    );
                    let cleanup_started = std::time::Instant::now();
                    if let Err(error) = handle.hangup_and_wait(Some(call_timeout)).await {
                        counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
                        failures.record(
                            "churn_setup_failure_cleanup",
                            &error,
                            Some(&call_id),
                            cleanup_started.elapsed(),
                        );
                    }
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
                let teardown_started = std::time::Instant::now();
                match handle.hangup_and_wait(Some(call_timeout)).await {
                    Ok(_) => {
                        counters.succeeded.fetch_add(1, Ordering::Relaxed);
                        counters.churn_succeeded.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error) => {
                        counters.failed.fetch_add(1, Ordering::Relaxed);
                        counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
                        failures.record(
                            "churn_hangup",
                            &error,
                            Some(&call_id),
                            teardown_started.elapsed(),
                        );
                    }
                }
            });
            tokio::time::sleep(tick).await;
        }
    } else {
        tokio::time::sleep(total).await;
    }

    // Drain churn calls.
    let drain_result = tokio::time::timeout(drain_join_timeout(call_timeout), async {
        while let Some(result) = churn_tasks.join_next().await {
            if let Err(_error) = result {
                counters.failed.fetch_add(1, Ordering::Relaxed);
                counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
                failures.record_class(
                    "churn_task_join",
                    "task_join_error",
                    None,
                    started.elapsed(),
                    "task join failed before producing a result",
                );
            }
        }
    })
    .await;
    if drain_result.is_err() {
        churn_tasks.abort_all();
        while let Some(result) = churn_tasks.join_next().await {
            let _ = result;
        }
        counters.failed.fetch_add(1, Ordering::Relaxed);
        counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
        failures.record_class(
            "churn_task_drain",
            "join_timeout",
            None,
            drain_join_timeout(call_timeout),
            "churn tasks did not finish inside the drain deadline",
        );
    }

    // Collect calls that were still active at the steady-load deadline, then
    // dispatch their hangups in stable slot/cycle order at the configured
    // rate. This keeps steady-state soak teardown distinct from burst teardown.
    let mut pending_drains = Vec::new();
    let active_collection_result = tokio::time::timeout(drain_join_timeout(call_timeout), async {
        while let Some(result) = active_tasks.join_next().await {
            match result {
                Ok(Some(pending)) => pending_drains.push(pending),
                Ok(None) => {}
                Err(_error) => {
                    counters.failed.fetch_add(1, Ordering::Relaxed);
                    counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
                    failures.record_class(
                        "active_task_join",
                        "task_join_error",
                        None,
                        Duration::ZERO,
                        "task join failed before producing a result",
                    );
                }
            }
        }
    })
    .await;
    let controlled_drain = if active_collection_result.is_err() {
        active_tasks.abort_all();
        while let Some(result) = active_tasks.join_next().await {
            let _ = result;
        }
        counters.failed.fetch_add(1, Ordering::Relaxed);
        counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
        failures.record_class(
            "active_task_collection",
            "join_timeout",
            None,
            drain_join_timeout(call_timeout),
            "active slot tasks did not hand off inside the collection deadline",
        );
        force_teardown_remaining_sessions(Arc::clone(&alice), call_timeout, &counters, &failures)
            .await;
        ControlledDrainSummary::default()
    } else {
        controlled_drain_calls(
            Arc::clone(&alice),
            pending_drains,
            controlled_drain_cps,
            call_timeout,
            &counters,
            &failures,
        )
        .await
    };

    // Anchor the post-drain window to observed wall time after churn joins,
    // active-task handoff, and the controlled BYE drain have all completed.
    // Using nominal duration + paced-drain time alone can misclassify task
    // collection/cleanup samples as post-drain stability evidence.
    let rss_post_drain_start_secs = started.elapsed().as_secs_f64();
    let teardown_phase_elapsed_secs = (rss_post_drain_start_secs - duration_secs as f64).max(0.0);
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
    let rss_active_tail = rss_active_tail_metrics(
        &resources.samples,
        duration_secs as f64,
        resources.sample_interval_estimate_secs,
    );
    let rss_post_drain_samples: Vec<ResourceSample> = resources
        .samples
        .iter()
        .filter(|sample| sample.t_secs >= rss_post_drain_start_secs)
        .cloned()
        .collect();
    let rss_post_drain_growth_mb_per_hr = rss_growth_mb_per_min(&rss_post_drain_samples) * 60.0;
    let (rss_gate_growth_mb_per_hr, rss_gate_window) =
        if duration_secs as f64 >= LONG_SOAK_ACTIVE_RSS_WINDOW_SECS {
            (
                rss_active_tail.growth_mb_per_hr,
                if rss_active_tail.complete {
                    "active_tail_600s"
                } else {
                    "active_tail_600s_incomplete"
                },
            )
        } else if rss_post_drain_samples.len() >= 2 {
            (rss_post_drain_growth_mb_per_hr, "post_drain")
        } else {
            (rss_sustained_growth_mb_per_hr, "tail")
        };
    let rss_windows = rss_window_summaries(
        &resources.samples,
        duration_secs as f64,
        teardown_phase_elapsed_secs,
        retention_drain_wait.as_secs_f64(),
    );
    let offered = counters.offered.load(Ordering::Relaxed);
    let succeeded = counters.succeeded.load(Ordering::Relaxed);
    let failed = counters.failed.load(Ordering::Relaxed);
    let invite_send_failed = counters.invite_send_failed.load(Ordering::Relaxed);
    let answer_failed = counters.answer_failed.load(Ordering::Relaxed);
    let answer_timeout = counters.answer_timeout.load(Ordering::Relaxed);
    let media_setup_failed = counters.media_setup_failed.load(Ordering::Relaxed);
    let teardown_failed = counters.teardown_failed.load(Ordering::Relaxed);
    let active_offered = counters.active_offered.load(Ordering::Relaxed);
    let active_succeeded = counters.active_succeeded.load(Ordering::Relaxed);
    let churn_offered = counters.churn_offered.load(Ordering::Relaxed);
    let churn_succeeded = counters.churn_succeeded.load(Ordering::Relaxed);
    let active_audio_receivers = bob_diagnostics
        .active_audio_receivers
        .load(Ordering::Relaxed);
    let completed_audio_receivers = bob_diagnostics
        .completed_audio_receivers
        .load(Ordering::Relaxed);
    let received_frames = bob_diagnostics.received_frames.load(Ordering::Relaxed);
    let failure_diagnostics = failures.snapshot();
    let asr = if offered > 0 {
        succeeded as f64 / offered as f64
    } else {
        0.0
    };

    let load = LoadProfile {
        target_cps: soak_cps,
        ramp_secs: 0,
        steady_secs: duration_secs,
        cooldown_secs: controlled_drain.elapsed.as_secs() + retention_drain_wait.as_secs(),
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
        .result("active_calls_target", active_calls)
        .result("media_calls_held", active_calls)
        .result("active_call_min_hold_secs", min_hold_secs)
        .result("active_call_max_hold_secs", max_hold_secs)
        .result("controlled_drain_cps", controlled_drain_cps)
        .result("controlled_drain_calls", controlled_drain.queued)
        .result("controlled_drain_succeeded", controlled_drain.succeeded)
        .result("controlled_drain_failed", controlled_drain.failed)
        .result(
            "controlled_drain_elapsed_secs",
            round2(controlled_drain.elapsed.as_secs_f64()),
        )
        .result(
            "teardown_phase_elapsed_secs",
            round2(teardown_phase_elapsed_secs),
        )
        .result(
            "replenishment_stop_budget_secs",
            setup_teardown_budget(call_timeout).as_secs(),
        )
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
        .result("active_calls_offered", active_offered)
        .result("active_calls_succeeded", active_succeeded)
        .result("churn_calls_offered", churn_offered)
        .result("churn_calls_succeeded", churn_succeeded)
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
            "rss_active_tail_growth_mb_per_hr",
            round2(rss_active_tail.growth_mb_per_hr),
        )
        .result(
            "rss_active_tail_sample_count",
            rss_active_tail.sample_count as u64,
        )
        .result(
            "rss_active_tail_window_secs",
            round2(rss_active_tail.window_secs),
        )
        .result("rss_active_tail_window_complete", rss_active_tail.complete)
        .result("rss_active_tail_estimator", rss_active_tail.estimator)
        .result(
            "rss_active_tail_endpoint_band_secs",
            round2(rss_active_tail.endpoint_band_secs),
        )
        .result(
            "rss_active_tail_endpoint_separation_secs",
            round2(rss_active_tail.endpoint_separation_secs),
        )
        .result(
            "rss_active_tail_start_sample_count",
            rss_active_tail.start_sample_count as u64,
        )
        .result(
            "rss_active_tail_end_sample_count",
            rss_active_tail.end_sample_count as u64,
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
            "rss_post_drain_start_secs",
            round2(rss_post_drain_start_secs),
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
                "active_tail_window_secs": round2(rss_active_tail.window_secs),
                "active_tail_window_complete": rss_active_tail.complete,
                "active_tail_growth_mb_per_hr": round2(rss_active_tail.growth_mb_per_hr),
                "gate_window": rss_gate_window,
                "gate_growth_mb_per_hr": round2(rss_gate_growth_mb_per_hr),
            }),
        )
        .diagnostic_block("call_failures", failure_diagnostics)
        .result(
            "errors",
            json!({
                "call_failed":          failed,
                "invite_send_failed":   invite_send_failed,
                "answer_failed":        answer_failed,
                "answer_timeout":       answer_timeout,
                "media_setup_failed":   media_setup_failed,
                "teardown_failed":      teardown_failed,
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
    if duration_secs as f64 >= LONG_SOAK_ACTIVE_RSS_WINDOW_SECS && !rss_active_tail.complete {
        gate_failures.push(format!(
            "active RSS gate window incomplete: measured {:.2}s with {} samples; required {:.0}s",
            rss_active_tail.window_secs,
            rss_active_tail.sample_count,
            LONG_SOAK_ACTIVE_RSS_WINDOW_SECS
        ));
    }
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
        gate_failures.push(format!("call_failed={failed}"));
    }
    if invite_send_failed != 0 {
        gate_failures.push(format!("invite_send_failed={invite_send_failed}"));
    }
    if answer_failed != 0 {
        gate_failures.push(format!(
            "answer_failed={answer_failed} (timeouts={answer_timeout})"
        ));
    }
    if media_setup_failed != 0 {
        gate_failures.push(format!("media_setup_failed={media_setup_failed}"));
    }
    if teardown_failed != 0 {
        gate_failures.push(format!("teardown_failed={teardown_failed}"));
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
                    let _ = handle.hangup_and_wait(Some(call_timeout)).await;
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

/// Deliberately releases every established call through one barrier. The
/// steady soak uses a paced drain; this separate scenario retains coverage for
/// the worst-case synchronized BYE burst without conflating the two workloads.
#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_mass_teardown_stress() {
    let requested_calls = read_positive_usize_env("RVOIP_PERF_MASS_TEARDOWN_CALLS").unwrap_or(500);
    let setup_cps = read_positive_f64_env("RVOIP_PERF_MASS_TEARDOWN_SETUP_CPS").unwrap_or(30.0);
    let call_timeout = Duration::from_secs(
        std::env::var("RVOIP_PERF_CALL_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30),
    );
    let retention_drain_wait = retention_drain_wait();
    let failures = Arc::new(FailureDiagnostics::new(
        read_positive_usize_env("RVOIP_PERF_SOAK_ERROR_SAMPLE_LIMIT")
            .unwrap_or(DEFAULT_ERROR_SAMPLE_LIMIT),
    ));

    let bob_port = support::ports::next_sip_port();
    let alice_port = support::ports::next_sip_port();
    let bob_diagnostics = BobHandlerDiagnostics::default();
    let bob = boot_bob(
        perf_config("perf-mass-teardown-bob", bob_port),
        bob_diagnostics.clone(),
    )
    .await;
    let alice = boot_alice(perf_config("perf-mass-teardown-alice", alice_port)).await;
    let from = format!("sip:alice@127.0.0.1:{alice_port}");
    let target_uri = format!("sip:bob@127.0.0.1:{bob_port}");
    let sampler = ResourceSampler::start(Duration::from_secs(1));
    let started = std::time::Instant::now();

    let setup_interval = Duration::from_secs_f64(1.0 / setup_cps);
    let setup_started = std::time::Instant::now();
    let mut setup_tasks = JoinSet::new();
    for sequence in 0..requested_calls {
        let scheduled_at = setup_started + setup_interval.mul_f64(sequence as f64);
        let remaining = scheduled_at.saturating_duration_since(std::time::Instant::now());
        if !remaining.is_zero() {
            tokio::time::sleep(remaining).await;
        }
        let alice = Arc::clone(&alice);
        let from = from.clone();
        let target_uri = target_uri.clone();
        let failures = Arc::clone(&failures);
        setup_tasks.spawn(async move {
            let dispatched_at = std::time::Instant::now();
            let call_id = match alice.invite(Some(from), target_uri).send().await {
                Ok(call_id) => call_id,
                Err(error) => {
                    failures.record(
                        "mass_teardown_invite_send",
                        &error,
                        None,
                        dispatched_at.elapsed(),
                    );
                    return None;
                }
            };
            let handle = alice.session(&call_id);
            if let Err(error) = handle.wait_for_answered(Some(call_timeout)).await {
                failures.record(
                    "mass_teardown_wait_answered",
                    &error,
                    Some(&call_id),
                    dispatched_at.elapsed(),
                );
                let cleanup_started = std::time::Instant::now();
                if let Err(error) = handle.hangup_and_wait(Some(call_timeout)).await {
                    failures.record(
                        "mass_teardown_setup_cleanup",
                        &error,
                        Some(&call_id),
                        cleanup_started.elapsed(),
                    );
                }
                return None;
            }
            if let Err(error) = alice
                .set_audio_source(
                    &call_id,
                    AudioSource::Tone {
                        frequency: 440.0,
                        amplitude: 0.25,
                    },
                )
                .await
            {
                failures.record(
                    "mass_teardown_audio_source",
                    &error,
                    Some(&call_id),
                    dispatched_at.elapsed(),
                );
                let cleanup_started = std::time::Instant::now();
                if let Err(error) = handle.hangup_and_wait(Some(call_timeout)).await {
                    failures.record(
                        "mass_teardown_media_cleanup",
                        &error,
                        Some(&call_id),
                        cleanup_started.elapsed(),
                    );
                }
                return None;
            }
            Some(call_id)
        });
    }

    let mut established = Vec::with_capacity(requested_calls);
    while let Some(result) = setup_tasks.join_next().await {
        match result {
            Ok(Some(call_id)) => established.push(call_id),
            Ok(None) => {}
            Err(_error) => failures.record_class(
                "mass_teardown_setup_join",
                "task_join_error",
                None,
                setup_started.elapsed(),
                "task join failed before producing a result",
            ),
        }
    }
    established.sort_by_key(hash_session_id);
    let setup_failed = requested_calls.saturating_sub(established.len());

    let barrier = Arc::new(tokio::sync::Barrier::new(established.len() + 1));
    let release_anchor = std::time::Instant::now();
    let mut teardown_tasks = JoinSet::new();
    for call_id in established {
        let barrier = Arc::clone(&barrier);
        let handle = alice.session(&call_id);
        teardown_tasks.spawn(async move {
            barrier.wait().await;
            let dispatch_offset = release_anchor.elapsed();
            let teardown_started = std::time::Instant::now();
            let result = handle.hangup_and_wait(Some(call_timeout)).await;
            (call_id, dispatch_offset, teardown_started.elapsed(), result)
        });
    }
    barrier.wait().await;

    let mut teardown_succeeded = 0_usize;
    let mut teardown_failed = 0_usize;
    let mut first_dispatch = None::<Duration>;
    let mut last_dispatch = Duration::ZERO;
    while let Some(result) = teardown_tasks.join_next().await {
        match result {
            Ok((_call_id, dispatch_offset, _elapsed, Ok(_))) => {
                teardown_succeeded += 1;
                first_dispatch = Some(
                    first_dispatch.map_or(dispatch_offset, |first| first.min(dispatch_offset)),
                );
                last_dispatch = last_dispatch.max(dispatch_offset);
            }
            Ok((call_id, dispatch_offset, elapsed, Err(error))) => {
                teardown_failed += 1;
                first_dispatch = Some(
                    first_dispatch.map_or(dispatch_offset, |first| first.min(dispatch_offset)),
                );
                last_dispatch = last_dispatch.max(dispatch_offset);
                failures.record("mass_teardown_hangup", &error, Some(&call_id), elapsed);
            }
            Err(_error) => {
                teardown_failed += 1;
                failures.record_class(
                    "mass_teardown_join",
                    "task_join_error",
                    None,
                    release_anchor.elapsed(),
                    "task join failed before producing a result",
                );
            }
        }
    }
    let dispatch_spread = first_dispatch
        .map(|first| last_dispatch.saturating_sub(first))
        .unwrap_or_default();

    tokio::time::sleep(retention_drain_wait).await;
    let final_retention =
        capture_retention_sample("after_mass_teardown", started, &alice, &bob.coordinator).await;
    let retained_after_drain = retained_total(&final_retention);
    let active_audio_receivers = bob_diagnostics
        .active_audio_receivers
        .load(Ordering::Relaxed);
    let resources = sampler.stop().await;
    let failure_diagnostics = failures.snapshot();

    let load = LoadProfile {
        target_cps: setup_cps,
        ramp_secs: 0,
        steady_secs: release_anchor.duration_since(started).as_secs(),
        cooldown_secs: retention_drain_wait.as_secs(),
    };
    let mut report = ScenarioReport::new("perf_mass_teardown_stress", load);
    report
        .result("calls_requested", requested_calls as u64)
        .result(
            "calls_established",
            (teardown_succeeded + teardown_failed) as u64,
        )
        .result("setup_failed", setup_failed as u64)
        .result("teardown_succeeded", teardown_succeeded as u64)
        .result("teardown_failed", teardown_failed as u64)
        .result(
            "teardown_dispatch_spread_ms",
            round2(dispatch_spread.as_secs_f64() * 1_000.0),
        )
        .result("retained_objects_after_drain", retained_after_drain)
        .result("bob_active_audio_receivers", active_audio_receivers)
        .diagnostic_block("call_failures", failure_diagnostics)
        .diagnostic_block("retention_final", final_retention)
        .with_resources(resources);
    let json_path = report.write_resources_first_then_write_json_if_supported();
    report.print_summary(&json_path);

    bob.shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(3), bob.task).await;
    drop(alice);

    assert_eq!(setup_failed, 0, "mass teardown setup failures");
    assert_eq!(teardown_failed, 0, "mass teardown hangup failures");
    assert_eq!(retained_after_drain, 0, "mass teardown retained objects");
    assert_eq!(
        active_audio_receivers, 0,
        "mass teardown left audio receiver tasks active"
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

fn read_nonnegative_f64_env(name: &str) -> Option<f64> {
    let raw = match std::env::var(name) {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return None,
        Err(err) => panic!("{name} could not be read: {err}"),
    };
    let value: f64 = raw
        .parse()
        .unwrap_or_else(|_| panic!("{name} must be a finite number >= 0, got {raw:?}"));
    assert!(
        value.is_finite() && value >= 0.0,
        "{name} must be a finite number >= 0, got {raw:?}"
    );
    Some(value)
}

fn read_nonnegative_u64_env(name: &str) -> Option<u64> {
    let raw = match std::env::var(name) {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return None,
        Err(err) => panic!("{name} could not be read: {err}"),
    };
    Some(
        raw.parse()
            .unwrap_or_else(|_| panic!("{name} must be a non-negative integer, got {raw:?}")),
    )
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
        "/dialog_adapter/outgoing_bye_tx",
        "/dialog_adapter/outgoing_bye_generation_watch",
        "/dialog_adapter/outgoing_bye_wait_intents",
        "/dialog_adapter/outbound_initial_invites",
        "/dialog_adapter/registration_refresh_tasks",
        "/app_event_publisher/dispatcher/queued_current",
        "/app_event_publisher/dispatcher/in_flight_current",
        "/app_event_publisher/exact_terminal_claims/pending",
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
        "/dialog_manager/terminated_bye_deadlines",
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
    active_offered: AtomicU64,
    active_succeeded: AtomicU64,
    churn_offered: AtomicU64,
    churn_succeeded: AtomicU64,
    invite_send_failed: AtomicU64,
    answer_failed: AtomicU64,
    answer_timeout: AtomicU64,
    media_setup_failed: AtomicU64,
    teardown_failed: AtomicU64,
}

struct PendingDrain {
    slot: u64,
    cycle: u64,
    call_id: SessionId,
}

#[derive(Default)]
struct ControlledDrainSummary {
    queued: u64,
    succeeded: u64,
    failed: u64,
    elapsed: Duration,
}

fn cycling_hold_duration(slot: u64, cycle: u64, min_secs: u64, max_secs: u64) -> Duration {
    let span = max_secs - min_secs + 1;
    let offset = if span == 1 {
        0
    } else {
        slot.wrapping_mul(1_103_515_245)
            .wrapping_add(cycle.wrapping_mul(12_345))
            .wrapping_add(slot.rotate_left((cycle % 63) as u32))
            % span
    };
    Duration::from_secs(min_secs + offset)
}

fn setup_teardown_budget(call_timeout: Duration) -> Duration {
    call_timeout + call_timeout + Duration::from_secs(5)
}

fn drain_join_timeout(call_timeout: Duration) -> Duration {
    call_timeout + call_timeout + Duration::from_secs(60)
}

async fn controlled_drain_calls(
    alice: Arc<UnifiedCoordinator>,
    mut pending: Vec<PendingDrain>,
    drain_cps: f64,
    call_timeout: Duration,
    counters: &Arc<SoakCounters>,
    failures: &Arc<FailureDiagnostics>,
) -> ControlledDrainSummary {
    sort_pending_drains(&mut pending);
    let queued = pending.len() as u64;
    let interval = Duration::from_secs_f64(1.0 / drain_cps);
    let started = std::time::Instant::now();
    let mut tasks = JoinSet::new();

    for (index, pending) in pending.into_iter().enumerate() {
        let scheduled_at = started + drain_dispatch_offset(index, interval);
        let remaining = scheduled_at.saturating_duration_since(std::time::Instant::now());
        if !remaining.is_zero() {
            tokio::time::sleep(remaining).await;
        }
        let handle = alice.session(&pending.call_id);
        tasks.spawn(async move {
            let teardown_started = std::time::Instant::now();
            let result = handle.hangup_and_wait(Some(call_timeout)).await;
            (pending, teardown_started.elapsed(), result)
        });
    }

    let mut succeeded = 0_u64;
    let mut failed = 0_u64;
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok((_pending, _elapsed, Ok(_))) => {
                succeeded += 1;
                counters.succeeded.fetch_add(1, Ordering::Relaxed);
                counters.active_succeeded.fetch_add(1, Ordering::Relaxed);
            }
            Ok((pending, elapsed, Err(error))) => {
                failed += 1;
                counters.failed.fetch_add(1, Ordering::Relaxed);
                counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
                failures.record(
                    "active_controlled_drain",
                    &error,
                    Some(&pending.call_id),
                    elapsed,
                );
            }
            Err(_error) => {
                failed += 1;
                counters.failed.fetch_add(1, Ordering::Relaxed);
                counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
                failures.record_class(
                    "active_controlled_drain",
                    "task_join_error",
                    None,
                    started.elapsed(),
                    "task join failed before producing a result",
                );
            }
        }
    }

    ControlledDrainSummary {
        queued,
        succeeded,
        failed,
        elapsed: started.elapsed(),
    }
}

fn sort_pending_drains(pending: &mut [PendingDrain]) {
    pending.sort_by_key(|call| (call.slot, call.cycle));
}

fn drain_dispatch_offset(index: usize, interval: Duration) -> Duration {
    interval.mul_f64(index as f64)
}

async fn force_teardown_remaining_sessions(
    alice: Arc<UnifiedCoordinator>,
    call_timeout: Duration,
    counters: &Arc<SoakCounters>,
    failures: &Arc<FailureDiagnostics>,
) {
    let mut tasks = JoinSet::new();
    for session in alice.list_sessions().await {
        if session.state.is_final() {
            continue;
        }
        let call_id = session.session_id;
        let handle = alice.session(&call_id);
        tasks.spawn(async move {
            let started = std::time::Instant::now();
            let result = handle.hangup_and_wait(Some(call_timeout)).await;
            (call_id, started.elapsed(), result)
        });
    }

    while let Some(result) = tasks.join_next().await {
        match result {
            Ok((_call_id, _elapsed, Ok(_))) => {}
            Ok((call_id, elapsed, Err(error))) => {
                counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
                failures.record("forced_cleanup", &error, Some(&call_id), elapsed);
            }
            Err(_error) => {
                counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
                failures.record_class(
                    "forced_cleanup",
                    "task_join_error",
                    None,
                    Duration::ZERO,
                    "task join failed before producing a result",
                );
            }
        }
    }
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

struct ActiveTailRssMetrics {
    growth_mb_per_hr: f64,
    sample_count: usize,
    window_secs: f64,
    complete: bool,
    estimator: &'static str,
    endpoint_band_secs: f64,
    endpoint_separation_secs: f64,
    start_sample_count: usize,
    end_sample_count: usize,
}

fn rss_active_tail_metrics(
    samples: &[ResourceSample],
    active_secs: f64,
    _sample_interval_secs: f64,
) -> ActiveTailRssMetrics {
    let start_secs = (active_secs - LONG_SOAK_ACTIVE_RSS_WINDOW_SECS).max(0.0);
    let selected: Vec<ResourceSample> = samples
        .iter()
        .filter(|sample| sample.t_secs >= start_secs && sample.t_secs <= active_secs)
        .cloned()
        .collect();
    let actual_secs = match (selected.first(), selected.last()) {
        (Some(first), Some(last)) => (last.t_secs - first.t_secs).max(0.0),
        _ => 0.0,
    };
    let endpoint = support::soak::rss_endpoint_median_growth_mb_per_hr(&selected);
    let complete = active_secs >= LONG_SOAK_ACTIVE_RSS_WINDOW_SECS
        && actual_secs >= LONG_SOAK_MIN_ACTIVE_RSS_COVERAGE_SECS
        && selected.len() >= LONG_SOAK_MIN_ACTIVE_RSS_SAMPLES
        && endpoint.is_some();
    ActiveTailRssMetrics {
        growth_mb_per_hr: endpoint.as_ref().map_or_else(
            || rss_growth_mb_per_min(&selected) * 60.0,
            |estimate| estimate.growth_mb_per_hr,
        ),
        sample_count: selected.len(),
        window_secs: actual_secs,
        complete,
        estimator: if endpoint.is_some() {
            "median_first_last_sixth_capped_60s"
        } else {
            "unavailable_ols_diagnostic_only"
        },
        endpoint_band_secs: endpoint.as_ref().map_or(0.0, |estimate| estimate.band_secs),
        endpoint_separation_secs: endpoint
            .as_ref()
            .map_or(0.0, |estimate| estimate.separation_secs),
        start_sample_count: endpoint
            .as_ref()
            .map_or(0, |estimate| estimate.start_sample_count),
        end_sample_count: endpoint
            .as_ref()
            .map_or(0, |estimate| estimate.end_sample_count),
    }
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
    controlled_drain_secs: f64,
    post_drain_secs: f64,
) -> Vec<serde_json::Value> {
    let post_drain_start_secs = active_secs + controlled_drain_secs;
    let total_secs = post_drain_start_secs + post_drain_secs;
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
            let label = if start >= post_drain_start_secs {
                "post_drain"
            } else if start >= active_secs {
                "controlled_drain"
            } else {
                "active"
            };
            windows.push(json!({
                "label": label,
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
        .filter(|sample| sample.t_secs >= post_drain_start_secs)
        .cloned()
        .collect();
    if let (Some(first), Some(last)) = (drain_samples.first(), drain_samples.last()) {
        windows.push(json!({
            "label": "post_drain",
            "start_secs": round2(post_drain_start_secs),
            "end_secs": round2(total_secs),
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

    fn linear_rss_samples(end_secs: u64) -> Vec<ResourceSample> {
        (0..=end_secs)
            .step_by(5)
            .map(|second| ResourceSample {
                t_secs: second as f64,
                rss_mb: 100.0 + second as f64 * 0.01,
                cpu_pct: 0.0,
            })
            .collect()
    }

    #[test]
    fn active_tail_measurement_requires_and_covers_ten_minutes() {
        let samples = linear_rss_samples(640);
        let metrics = rss_active_tail_metrics(&samples, 640.0, 5.0);
        assert!(metrics.complete);
        assert_eq!(metrics.sample_count, 121);
        assert_eq!(metrics.window_secs, 600.0);
        assert_eq!(metrics.estimator, "median_first_last_sixth_capped_60s");
        assert!((metrics.growth_mb_per_hr - 36.0).abs() < 0.000_001);

        assert!(!rss_active_tail_metrics(&samples, 599.0, 5.0).complete);

        let sparse = vec![
            ResourceSample {
                t_secs: 0.0,
                rss_mb: 100.0,
                cpu_pct: 0.0,
            },
            ResourceSample {
                t_secs: 400.0,
                rss_mb: 101.0,
                cpu_pct: 0.0,
            },
        ];
        assert!(!rss_active_tail_metrics(&sparse, 600.0, 400.0).complete);

        let endpoint_gap = std::iter::once(ResourceSample {
            t_secs: 0.0,
            rss_mb: 100.0,
            cpu_pct: 0.0,
        })
        .chain((12..=120).map(|index| ResourceSample {
            t_secs: index as f64 * 5.0,
            rss_mb: 100.0,
            cpu_pct: 0.0,
        }))
        .collect::<Vec<_>>();
        assert_eq!(endpoint_gap.len(), 110);
        let endpoint_gap_metrics = rss_active_tail_metrics(&endpoint_gap, 600.0, 5.0);
        assert!(!endpoint_gap_metrics.complete);
        assert_eq!(
            endpoint_gap_metrics.estimator,
            "unavailable_ols_diagnostic_only"
        );
    }

    #[test]
    fn shared_long_soak_gate_selects_active_tail_not_post_drain() {
        let mut resources = support::ResourceSummary::empty();
        resources.samples = linear_rss_samples(680);
        resources.sample_interval_estimate_secs = 5.0;
        let rss = support::soak::rss_result_metrics(
            &resources,
            640.0,
            640.0,
            40.0,
            support::soak::RssGatePolicy::ActiveTail600,
        );
        assert!(rss.active_tail_window_complete);
        assert_eq!(rss.gate_window, "active_tail_600s");
        assert!((rss.gate_growth_mb_per_hr - 36.0).abs() < 0.000_001);
    }

    #[test]
    fn shared_burst_policy_keeps_post_drain_gate_for_long_wall_time() {
        let mut resources = support::ResourceSummary::empty();
        resources.samples = linear_rss_samples(680);
        for sample in &mut resources.samples {
            if sample.t_secs >= 640.0 {
                sample.rss_mb = 106.4;
            }
        }
        resources.sample_interval_estimate_secs = 5.0;
        let rss = support::soak::rss_result_metrics(
            &resources,
            640.0,
            640.0,
            40.0,
            support::soak::RssGatePolicy::PostDrainOrTail,
        );
        assert_eq!(rss.gate_window, "post_drain");
        assert!(rss.gate_growth_mb_per_hr.abs() < 0.000_001);
        assert!(rss.active_tail_growth_mb_per_hr > 30.0);
    }

    #[test]
    fn active_tail_endpoint_medians_ignore_one_sample_spike_but_keep_growth() {
        let mut spike_samples = (0..=120)
            .map(|index| ResourceSample {
                t_secs: index as f64 * 5.0,
                rss_mb: 100.0,
                cpu_pct: 0.0,
            })
            .collect::<Vec<_>>();
        spike_samples.last_mut().unwrap().rss_mb += 100.0;
        let spike_rate = support::soak::rss_endpoint_median_growth_mb_per_hr(&spike_samples)
            .expect("endpoint rate");
        assert!(spike_rate.growth_mb_per_hr.abs() < 0.000_001);

        let growth_samples = (0..=120)
            .map(|index| {
                let t_secs = index as f64 * 5.0;
                ResourceSample {
                    t_secs,
                    rss_mb: 100.0 + 10.01 * t_secs / 3600.0,
                    cpu_pct: 0.0,
                }
            })
            .collect::<Vec<_>>();
        let growth_rate = support::soak::rss_endpoint_median_growth_mb_per_hr(&growth_samples)
            .expect("endpoint rate");
        assert!((growth_rate.growth_mb_per_hr - 10.01).abs() < 0.000_001);
    }

    #[test]
    fn cycling_hold_duration_stays_inside_configured_range() {
        for slot in 0..64 {
            for cycle in 0..64 {
                let hold = cycling_hold_duration(slot, cycle, 10, 360);
                assert!(hold >= Duration::from_secs(10));
                assert!(hold <= Duration::from_secs(360));
            }
        }
    }

    #[test]
    fn cycling_hold_duration_allows_fixed_hold_time() {
        assert_eq!(
            cycling_hold_duration(42, 7, 30, 30),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn failure_diagnostics_preserve_classes_and_bound_samples() {
        let diagnostics = FailureDiagnostics::new(2);
        let call_id = SessionId::from_string("sensitive-call-id");
        diagnostics.record(
            "invite_send",
            &SessionError::MediaError("bind failed for sensitive-call-id".to_string()),
            Some(&call_id),
            Duration::from_millis(12),
        );
        diagnostics.record(
            "wait_answered",
            &SessionError::Timeout("answer".to_string()),
            Some(&call_id),
            Duration::from_secs(1),
        );
        diagnostics.record(
            "hangup",
            &SessionError::DialogError("bye".to_string()),
            Some(&call_id),
            Duration::from_secs(2),
        );

        let snapshot = diagnostics.snapshot();
        assert_eq!(snapshot["total"], 3);
        assert_eq!(snapshot["sampled"], 2);
        assert_eq!(snapshot["dropped_samples"], 1);
        assert_eq!(snapshot["samples"][0]["phase"], "invite_send");
        assert_eq!(snapshot["samples"][0]["error_class"], "media_error");
        assert_ne!(snapshot["samples"][0]["call_id_hash"], call_id.as_str());
        assert_eq!(snapshot["by_phase_and_class"].as_array().unwrap().len(), 3);
        let encoded = snapshot.to_string();
        assert!(!encoded.contains("sensitive-call-id"));
        assert!(!encoded.contains("bind failed"));
        assert!(!encoded.contains("\"message\":\"answer\""));
    }

    #[test]
    fn session_error_classifier_distinguishes_bind_collisions() {
        let error = SessionError::IoError(std::io::Error::from(std::io::ErrorKind::AddrInUse));
        assert_eq!(session_error_class(&error), "io_address_in_use");
        assert_eq!(
            session_error_class(&SessionError::MediaError(
                "[kind=rtp_bind_collision] bind failed".to_string()
            )),
            "rtp_bind_collision"
        );
        assert_eq!(
            session_error_class(&SessionError::MediaIntegration {
                reason: "[kind=port_pool_exhausted] no ports".to_string(),
            }),
            "port_pool_exhausted"
        );
        assert_eq!(
            session_error_class(&SessionError::Timeout("bye".to_string())),
            "timeout"
        );
    }

    #[test]
    fn controlled_drain_order_is_stable_by_slot_then_cycle() {
        let mut pending = vec![
            PendingDrain {
                slot: 9,
                cycle: 1,
                call_id: SessionId::from_string("third"),
            },
            PendingDrain {
                slot: 2,
                cycle: 7,
                call_id: SessionId::from_string("second"),
            },
            PendingDrain {
                slot: 2,
                cycle: 3,
                call_id: SessionId::from_string("first"),
            },
        ];
        sort_pending_drains(&mut pending);
        assert_eq!(
            pending
                .iter()
                .map(|call| call.call_id.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second", "third"]
        );
    }

    #[test]
    fn controlled_drain_schedule_respects_configured_rate() {
        let interval = Duration::from_secs_f64(1.0 / 10.0);
        assert_eq!(drain_dispatch_offset(0, interval), Duration::ZERO);
        assert_eq!(
            drain_dispatch_offset(1, interval),
            Duration::from_millis(100)
        );
        assert_eq!(
            drain_dispatch_offset(9, interval),
            Duration::from_millis(900)
        );
    }

    #[test]
    fn retention_gate_counts_bye_owners_and_event_work() {
        let snapshot = json!({
            "dialog_adapter": {
                "outgoing_bye_tx": 1,
                "outgoing_bye_generation_watch": 1,
                "outgoing_bye_wait_intents": 1,
                "outbound_initial_invites": 1,
            },
            "app_event_publisher": {
                "dispatcher": {
                    "queued_current": 1,
                    "in_flight_current": 1,
                },
                "exact_terminal_claims": {
                    "pending": 1,
                },
            },
        });
        assert_eq!(endpoint_retained_total(&snapshot), 7);
    }
}

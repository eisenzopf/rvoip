//! Scenario 1 — sustained call-setup CPS (with concurrency-sweep support).
//!
//! Drives INVITE → 200 OK → ACK → BYE → 200 OK on loopback at the
//! offered CPS dictated by [`LoadProfile`]. Reports the **headline**
//! VoIP signalling number: **ASR** (Answer-Seizure Ratio, ITU E.411) at
//! the operating point, plus setup / full-cycle latency
//! p50/p95/p99/p99.9.
//!
//! Two run modes:
//!
//! - **Single point (default)**: `cargo test -p rvoip-sip --features
//!   perf-tests --test perf_call_setup_cps --release -- --nocapture`
//!   writes `target/perf-results/perf_call_setup_cps.json`.
//! - **Sweep** (industry pattern from OpenSIPS / Kamailio / SBC perf
//!   reports): set `RVOIP_PERF_SWEEP_CPS=10,50,100,500,1000` and the
//!   test loops once per point, sharing the booted peers. Writes
//!   per-point JSONs plus aggregated `_sweep.json` and a
//!   publication-ready `_sweep.md` table under
//!   `target/perf-results/perf_call_setup_cps/`.
//!
//! Env knobs:
//! - `RVOIP_PERF_SWEEP_CPS`     (comma-separated points; enables sweep mode)
//! - `RVOIP_PERF_TARGET_CPS`    (single-point default; 100)
//! - `RVOIP_PERF_RAMP_SECS`     (default 5)
//! - `RVOIP_PERF_STEADY_SECS`   (default 30)
//! - `RVOIP_PERF_COOLDOWN_SECS` (default 5)
//! - `RVOIP_PERF_CALL_TIMEOUT_SECS` (default 15) — per-call timeout
//! - `RVOIP_PERF_WORKER_THREADS`   (default 8)
//! - `RVOIP_PERF_PROFILE`          (Bob/server YAML recipe; default `pbx-media-server`)
//! - `RVOIP_PERF_CLIENT_PROFILE`   (Alice/client YAML recipe; default `endpoint`)
//! - `RVOIP_PERF_ALICE_SHARDS`     (client endpoint instances; default 4 for server profiles)
//! - `RVOIP_PERF_RECIPE_FILE`      (optional YAML recipe book path)
//! - `RVOIP_PERF_MAX_IN_FLIGHT`    (optional emergency harness safety cap)
//! - `RVOIP_PERF_RETENTION_SNAPSHOT` (default 0; append endpoint retention counts
//!   after resource sampling stops, for bounded A/B diagnostics)
//! - `RVOIP_PERF_BOUNDARY_SNAPSHOT` (default 0; capture endpoint and allocator
//!   state once at `calls_drained` without waiting out the retention horizon)
//! - `RVOIP_PERF_POST_DRAIN_SETTLE_SECS` (default 0; wait before the final
//!   post-drain RSS gate window)
//! - `RVOIP_PERF_POST_DRAIN_SAMPLE_SECS` (default 0; length of the final
//!   post-settle RSS gate window)
//!
//! See `docs/BENCHMARKING.md` for full interpretation.

#![allow(clippy::needless_return)]
#![recursion_limit = "256"]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rvoip_sip::api::callback_peer::{
    CallHandler, CallHandlerDecision, CallbackPeer, ShutdownHandle,
};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::PerformanceConfig;
use rvoip_sip_dialog::transaction::{
    DEFAULT_INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK, DEFAULT_TRANSACTION_DISPATCH_PRIORITY_BURST_MAX,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tokio::task::{JoinHandle, JoinSet};

#[path = "support/mod.rs"]
mod support;
use support::{
    parse_sweep_env, LatencyHistogram, LoadProfile, ResourceSampler, ResourceWindowSpec,
    ScenarioReport, SweepRunner,
};

const SAME_HOST_BOB_MEDIA_START: u16 = 16_384;
const SAME_HOST_BOB_MEDIA_END: u16 = 40_999;
const SAME_HOST_ALICE_MEDIA_START: u16 = 51_000;
const SAME_HOST_ALICE_MEDIA_END: u16 = 65_535;
const SIP_SESSION_ANTI_REUSE_HORIZON_SECS: usize = 64;
const RETAINED_LIFECYCLE_CHURN_HEADROOM_PERCENT: usize = 25;
const CLEANUP_RSS_INTENT_MB_PER_HOUR: f64 = 10.0;
const CLEANUP_RSS_ENDPOINT_ESTIMATOR: &str = "median_first_last_sixth_capped_60s";
const CLEANUP_RSS_MINIMUM_REPRESENTATIVE_SEPARATION_SECS: u64 = 360;
const BUNDLED_PERFORMANCE_RECIPES: &str = include_str!("../../config/performance-recipes.yaml");

/// Auto-accept handler — every inbound INVITE answered immediately
/// (no provisional 180; real PDD measurement is Phase 3's job).
struct AutoAccept;

#[async_trait::async_trait]
impl CallHandler for AutoAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let _ = call.accept().await;
        CallHandlerDecision::Accept
    }
}

struct Counters {
    offered: AtomicU64,
    succeeded: AtomicU64,
    invite_send_failed: AtomicU64,
    answer_failed: AtomicU64,
    bye_failed: AtomicU64,
    timeout: AtomicU64,
    harness_backpressure_rejected: AtomicU64,
    max_in_flight_observed: AtomicU64,
    invite_send_errors: Mutex<BTreeMap<String, u64>>,
    bye_failures: Mutex<ByeFailureDiagnostics>,
}

#[derive(Default)]
struct ByeFailureDiagnostics {
    error_counts: BTreeMap<&'static str, u64>,
    stage_counts: BTreeMap<&'static str, u64>,
    hangup_elapsed_bucket_counts: BTreeMap<&'static str, u64>,
    call_elapsed_bucket_counts: BTreeMap<&'static str, u64>,
    load_phase_counts: BTreeMap<&'static str, u64>,
}

#[derive(Clone, Copy)]
struct LoadPhaseBoundaries {
    ramp_end: Duration,
    steady_end: Duration,
}

impl LoadPhaseBoundaries {
    fn from_load(load: &LoadProfile) -> Self {
        let ramp_end = Duration::from_secs(load.ramp_secs);
        Self {
            ramp_end,
            steady_end: ramp_end + Duration::from_secs(load.steady_secs),
        }
    }
}

#[derive(Clone)]
struct LoadClient {
    peer: Arc<UnifiedCoordinator>,
    from: String,
}

impl Default for Counters {
    fn default() -> Self {
        Self {
            offered: AtomicU64::new(0),
            succeeded: AtomicU64::new(0),
            invite_send_failed: AtomicU64::new(0),
            answer_failed: AtomicU64::new(0),
            bye_failed: AtomicU64::new(0),
            timeout: AtomicU64::new(0),
            harness_backpressure_rejected: AtomicU64::new(0),
            max_in_flight_observed: AtomicU64::new(0),
            invite_send_errors: Mutex::new(BTreeMap::new()),
            bye_failures: Mutex::new(ByeFailureDiagnostics::default()),
        }
    }
}

impl Counters {
    fn record_invite_send_error(&self, error: &rvoip_sip::SessionError) {
        let mut errors = self
            .invite_send_errors
            .lock()
            .expect("invite send error map poisoned");
        *errors.entry(error.to_string()).or_insert(0) += 1;
    }

    fn invite_send_error_counts(&self) -> BTreeMap<String, u64> {
        self.invite_send_errors
            .lock()
            .expect("invite send error map poisoned")
            .clone()
    }

    fn record_bye_failure(
        &self,
        error: &rvoip_sip::SessionError,
        hangup_elapsed: Duration,
        call_elapsed: Duration,
        load_elapsed: Duration,
        load_phases: LoadPhaseBoundaries,
    ) {
        let (error_class, stage) = classify_bye_failure(error);
        let mut failures = self
            .bye_failures
            .lock()
            .expect("BYE failure diagnostic map poisoned");
        increment_static_count(&mut failures.error_counts, error_class);
        increment_static_count(&mut failures.stage_counts, stage);
        increment_static_count(
            &mut failures.hangup_elapsed_bucket_counts,
            elapsed_bucket(hangup_elapsed),
        );
        increment_static_count(
            &mut failures.call_elapsed_bucket_counts,
            elapsed_bucket(call_elapsed),
        );
        increment_static_count(
            &mut failures.load_phase_counts,
            load_phase(load_elapsed, load_phases),
        );
    }

    fn bye_failure_diagnostics(&self) -> Value {
        let failures = self
            .bye_failures
            .lock()
            .expect("BYE failure diagnostic map poisoned");
        json!({
            "schema": "rvoip-sip-bye-failure-diagnostics-v1",
            "error_counts": failures.error_counts,
            "stage_counts": failures.stage_counts,
            "hangup_elapsed_bucket_counts": failures.hangup_elapsed_bucket_counts,
            "call_elapsed_bucket_counts": failures.call_elapsed_bucket_counts,
            "load_phase_counts": failures.load_phase_counts,
        })
    }
}

fn increment_static_count(counts: &mut BTreeMap<&'static str, u64>, key: &'static str) {
    *counts.entry(key).or_insert(0) += 1;
}

fn classify_bye_failure(error: &rvoip_sip::SessionError) -> (&'static str, &'static str) {
    use rvoip_sip::SessionError;

    match error {
        SessionError::Timeout(detail) => match detail.as_str() {
            "SIP BYE transaction was not available before its deadline" => (
                "confirmation_transaction_unavailable_timeout",
                "confirmation",
            ),
            "SIP BYE final response timed out" => {
                ("confirmation_final_response_timeout", "confirmation")
            }
            "hangup_and_wait timed out" => ("terminal_lifecycle_observation_timeout", "terminal"),
            _ => ("timeout_other", "other"),
        },
        SessionError::DialogError(detail)
            if detail == "SIP BYE final response could not be observed" =>
        {
            ("confirmation_unobservable", "confirmation")
        }
        SessionError::DialogError(detail) if detail == "SIP BYE failed (class=dialog-dispatch)" => {
            ("dialog_dispatch_failed", "dispatch")
        }
        SessionError::ProtocolError(detail)
            if detail == "SIP BYE received a non-success final response" =>
        {
            ("confirmation_non_2xx", "confirmation")
        }
        SessionError::InvalidTransition(detail)
            if detail == "SIP BYE exact dialog is no longer available" =>
        {
            ("exact_dialog_unavailable", "dispatch")
        }
        SessionError::InternalError(detail) => match detail.as_str() {
            "exact terminal publication failed" => ("terminal_publication_failed", "terminal"),
            "exact terminal resource release failed" => ("terminal_release_failed", "terminal"),
            "exact terminal publication and resource release failed" => {
                ("terminal_publication_and_release_failed", "terminal")
            }
            "exact terminal publication owner stopped before completion" => {
                ("terminal_owner_dropped", "terminal")
            }
            _ => ("internal_other", "other"),
        },
        SessionError::SessionNotFound(_) => ("session_not_found", "dispatch"),
        SessionError::InvalidTransition(_) => ("invalid_transition_other", "dispatch"),
        SessionError::DialogError(_) => ("dialog_other", "dispatch"),
        SessionError::ProtocolError(_) => ("protocol_other", "confirmation"),
        SessionError::NetworkError(_) | SessionError::IoError(_) => {
            ("transport_failed", "dispatch")
        }
        SessionError::MediaError(_) | SessionError::MediaIntegration { .. } => {
            ("media_cleanup_failed", "terminal")
        }
        SessionError::Other(detail)
            if detail.starts_with("Failed to publish app-level event (class=") =>
        {
            ("terminal_publication_failed", "terminal")
        }
        SessionError::Other(detail)
            if detail == "lower-layer operation failed (class=opaque-erased)" =>
        {
            ("opaque_lower_layer_failure", "dispatch")
        }
        SessionError::AuthError(_)
        | SessionError::MissingCredentialsForInviteAuth
        | SessionError::MissingCredentialsForRequestAuth { .. }
        | SessionError::InviteAuthRetryExhausted
        | SessionError::RequestAuthRetryExhausted { .. }
        | SessionError::InviteAuthConstructionFailed
        | SessionError::RequestAuthConstructionFailed => ("authentication_failed", "dispatch"),
        SessionError::Conflict { .. } => ("concurrent_control_conflict", "dispatch"),
        SessionError::SDPNegotiationFailed(_)
        | SessionError::ConfigurationError(_)
        | SessionError::ConfigError(_)
        | SessionError::InvalidInput(_)
        | SessionError::UnreliableProvisionalsNotSupported
        | SessionError::RegisterAuthConstructionFailed
        | SessionError::NotImplemented(_)
        | SessionError::TransferFailed(_)
        | SessionError::RegistrationFailed(_)
        | SessionError::Other(_)
        | SessionError::HeaderPolicy { .. }
        | SessionError::MissingRequiredHeader { .. } => ("other", "other"),
    }
}

fn elapsed_bucket(elapsed: Duration) -> &'static str {
    match elapsed.as_millis() {
        0 => "lt_1_ms",
        1..=4 => "1_to_4_ms",
        5..=24 => "5_to_24_ms",
        25..=99 => "25_to_99_ms",
        100..=499 => "100_to_499_ms",
        500..=999 => "500_to_999_ms",
        1_000..=1_999 => "1_to_1_999_s",
        2_000..=4_999 => "2_to_4_999_s",
        5_000..=14_999 => "5_to_14_999_s",
        _ => "ge_15_s",
    }
}

fn load_phase(elapsed: Duration, boundaries: LoadPhaseBoundaries) -> &'static str {
    if elapsed < boundaries.ramp_end {
        "ramp"
    } else if elapsed < boundaries.steady_end {
        "steady"
    } else {
        "cooldown_or_drain"
    }
}

struct InFlightGuard {
    in_flight: Arc<AtomicU64>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.in_flight.fetch_sub(1, Ordering::Relaxed);
    }
}

struct BobReceiver {
    _coord: Arc<UnifiedCoordinator>,
    task: JoinHandle<()>,
    shutdown: ShutdownHandle,
}

async fn boot_bob(cfg: Config) -> BobReceiver {
    let bob = CallbackPeer::new(AutoAccept, cfg)
        .await
        .expect("perf bob: CallbackPeer::new");
    let coord = bob.coordinator().clone();
    let shutdown = bob.shutdown_handle();
    let task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    BobReceiver {
        _coord: coord,
        task,
        shutdown,
    }
}

async fn boot_alice(cfg: Config) -> Arc<UnifiedCoordinator> {
    let coord = UnifiedCoordinator::new(cfg)
        .await
        .expect("perf alice: UnifiedCoordinator::new");
    tokio::time::sleep(Duration::from_millis(200)).await;
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
    in_flight: Arc<AtomicU64>,
    point_started: Instant,
    load_phases: LoadPhaseBoundaries,
) {
    let _guard = InFlightGuard { in_flight };
    let t_send = std::time::Instant::now();

    let call_id = match alice.invite(Some(from), target).send().await {
        Ok(id) => id,
        Err(e) => {
            counters.invite_send_failed.fetch_add(1, Ordering::Relaxed);
            counters.record_invite_send_error(&e);
            return;
        }
    };
    let handle = alice.session(&call_id);

    let answered_elapsed = match handle.wait_for_answered(Some(per_call_timeout)).await {
        Ok(_) => {
            let elapsed = t_send.elapsed();
            setup_hist.record_nanos(elapsed.as_nanos() as u64);
            elapsed
        }
        Err(e) => {
            if matches!(e, rvoip_sip::SessionError::Timeout(_)) {
                counters.timeout.fetch_add(1, Ordering::Relaxed);
            } else {
                counters.answer_failed.fetch_add(1, Ordering::Relaxed);
            }
            return;
        }
    };

    match handle.hangup_and_wait(Some(per_call_timeout)).await {
        Ok(_) => {
            full_hist.record_nanos(t_send.elapsed().as_nanos() as u64);
            counters.succeeded.fetch_add(1, Ordering::Relaxed);
        }
        Err(e) => {
            let call_elapsed = t_send.elapsed();
            counters.record_bye_failure(
                &e,
                call_elapsed.saturating_sub(answered_elapsed),
                call_elapsed,
                point_started.elapsed(),
                load_phases,
            );
            if matches!(e, rvoip_sip::SessionError::Timeout(_)) {
                counters.timeout.fetch_add(1, Ordering::Relaxed);
            } else {
                counters.bye_failed.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// One sweep point: fresh histograms + counters, run the load profile
/// once, return a populated `ScenarioReport`. Peers stay shared across
/// sweep points so the bind cost is paid once per test.
async fn run_one_point(
    report_scenario: String,
    clients: Arc<Vec<LoadClient>>,
    bob: Arc<UnifiedCoordinator>,
    target: String,
    load: LoadProfile,
    per_call_timeout: Duration,
    max_in_flight: Option<u64>,
    post_drain_settle: Duration,
    post_drain_sample: Duration,
    effective_config: Value,
) -> ScenarioReport {
    let point_started = Instant::now();
    let mut phase_markers = vec![phase_marker("point_start", point_started, "actual")];
    phase_markers.push(json!({
        "phase": "ramp_end",
        "kind": "planned",
        "elapsed_ms": load.ramp_secs.saturating_mul(1_000),
    }));
    phase_markers.push(json!({
        "phase": "steady_end",
        "kind": "planned",
        "elapsed_ms": load
            .ramp_secs
            .saturating_add(load.steady_secs)
            .saturating_mul(1_000),
    }));
    phase_markers.push(json!({
        "phase": "cooldown_end",
        "kind": "planned",
        "elapsed_ms": load
            .ramp_secs
            .saturating_add(load.steady_secs)
            .saturating_add(load.cooldown_secs)
            .saturating_mul(1_000),
    }));
    // Diagnostic retention scans deliberately run outside clean controls. The
    // task observes the same active workload, then keeps the endpoints alive
    // through the complete 64-second identifier fence for a post-retention
    // snapshot. Clean/cpu/timing modes do not create this task.
    let retention_timeline_task = env_enabled("RVOIP_PERF_RETENTION_SNAPSHOT").then(|| {
        tokio::spawn(capture_retention_timeline(
            point_started,
            load.clone(),
            Arc::clone(&bob),
            Arc::clone(&clients),
        ))
    });
    let setup_hist = Arc::new(LatencyHistogram::new("setup_latency"));
    let full_hist = Arc::new(LatencyHistogram::new("full_cycle"));
    let counters = Arc::new(Counters::default());
    let in_flight = Arc::new(AtomicU64::new(0));
    let mut tasks = JoinSet::<()>::new();
    let load_phases = LoadPhaseBoundaries::from_load(&load);

    // ChatGPT guidance §1.5.B + §1.5.C: sample CPU% + RSS every 500 ms
    // during the active phase so the report carries the leak indicator
    // (rss_growth_mb_per_min) and a populated avg_cpu_pct field.
    let sampler = ResourceSampler::start(Duration::from_millis(500));

    let active_wall = {
        let setup_hist = Arc::clone(&setup_hist);
        let full_hist = Arc::clone(&full_hist);
        let counters = Arc::clone(&counters);
        let in_flight = Arc::clone(&in_flight);
        let clients = Arc::clone(&clients);
        load.run(|seq| {
            while tasks.try_join_next().is_some() {}
            counters.offered.fetch_add(1, Ordering::Relaxed);
            if let Some(max_in_flight) = max_in_flight {
                let current = in_flight.load(Ordering::Relaxed);
                if current >= max_in_flight {
                    counters
                        .harness_backpressure_rejected
                        .fetch_add(1, Ordering::Relaxed);
                    return;
                }
            }
            let observed = in_flight.fetch_add(1, Ordering::Relaxed) + 1;
            update_atomic_max(&counters.max_in_flight_observed, observed);
            let client = &clients[(seq as usize) % clients.len()];
            let alice = Arc::clone(&client.peer);
            let setup_hist = Arc::clone(&setup_hist);
            let full_hist = Arc::clone(&full_hist);
            let counters = Arc::clone(&counters);
            let in_flight = Arc::clone(&in_flight);
            let from = client.from.clone();
            let target = target.clone();
            tasks.spawn(async move {
                run_one_call(
                    alice,
                    from,
                    target,
                    setup_hist,
                    full_hist,
                    counters,
                    per_call_timeout,
                    in_flight,
                    point_started,
                    load_phases,
                )
                .await;
            });
        })
        .await
    };
    phase_markers.push(phase_marker("dispatch_complete", point_started, "actual"));

    // Cooldown drain — outstanding calls must finish (or time out)
    // before we snapshot histograms for this point.
    let cooldown_budget = Duration::from_secs(load.cooldown_secs) + per_call_timeout;
    let drain_deadline = tokio::time::sleep(cooldown_budget);
    tokio::pin!(drain_deadline);
    let mut drain_timed_out = false;
    loop {
        if tasks.is_empty() {
            break;
        }
        tokio::select! {
            _ = &mut drain_deadline => {
                drain_timed_out = true;
                break;
            }
            joined = tasks.join_next() => {
                if joined.is_none() {
                    break;
                }
            }
        }
    }
    if drain_timed_out {
        let outstanding = tasks.len() as u64;
        counters.timeout.fetch_add(outstanding, Ordering::Relaxed);
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        in_flight.store(0, Ordering::Relaxed);
    }
    phase_markers.push(phase_marker("calls_drained", point_started, "actual"));
    // This diagnostic deliberately preserves the production sweep's
    // conditioning overlap: unlike `RVOIP_PERF_RETENTION_SNAPSHOT`, it does
    // not wait for the 64-second anti-reuse horizon before the next point.
    // Capture after all call tasks drain so scans cannot perturb signaling
    // latency, and keep it opt-in because walking every endpoint allocates.
    let boundary_snapshot = if env_enabled("RVOIP_PERF_BOUNDARY_SNAPSHOT") {
        Some(
            capture_retention_snapshot(
                "calls_drained",
                elapsed_millis(point_started),
                point_started,
                bob.as_ref(),
                clients.as_ref(),
            )
            .await,
        )
    } else {
        None
    };
    let active_resource_end = sampler.elapsed();
    // The canonical cleanup measurement has two distinct phases. First wait
    // through the retained-lifecycle horizon so bounded retirement work and
    // allocator settling cannot be projected into a false hourly leak rate.
    // Then start a fresh, longer idle window with enough time resolution to
    // distinguish sub-MB quantization from sustained 10 MB/hour growth.
    let post_drain_settle_deadline = Instant::now() + post_drain_settle;
    if !post_drain_sample.is_zero() {
        phase_markers.push(phase_marker(
            "post_drain_settle_start",
            point_started,
            "actual",
        ));
    }
    phase_markers.push(phase_marker("cooldown_start", point_started, "actual"));

    // `LoadProfile::run` stops dispatch at ramp + steady. Keep sampling the
    // idle/retention tail through the declared cooldown boundary even when all
    // calls drain immediately; otherwise the canonical RSS slope silently
    // measures only allocation growth and omits the configured cooldown.
    wait_until_cooldown_deadline(planned_cooldown_deadline(point_started, &load)).await;
    phase_markers.push(phase_marker("cooldown_end", point_started, "actual"));

    let mut resource_windows = vec![ResourceWindowSpec::new(
        "active_load",
        "point_start",
        "calls_drained",
        Duration::ZERO,
        active_resource_end,
    )];
    let mut cleanup_convergence_at_settle = None;
    if !post_drain_sample.is_zero() {
        tokio::time::sleep_until(tokio::time::Instant::from_std(post_drain_settle_deadline)).await;
        phase_markers.push(phase_marker(
            "post_drain_settle_end",
            point_started,
            "actual",
        ));

        cleanup_convergence_at_settle =
            Some(capture_cleanup_convergence(bob.as_ref(), clients.as_ref()).await);
        phase_markers.push(phase_marker(
            "cleanup_convergence_at_settle_captured",
            point_started,
            "actual_before_resource_window",
        ));

        // Capture the exact sampler-clock boundaries after the structural
        // scan. The returned convergence JSON remains live for the duration
        // of the window, so any memory it owns is part of both endpoint bands
        // rather than a mid-window allocation.
        let post_drain_resource_start = sampler.elapsed();
        phase_markers.push(phase_marker(
            "post_drain_cleanup_start",
            point_started,
            "actual",
        ));
        tokio::time::sleep(post_drain_sample).await;
        let post_drain_resource_end = sampler.elapsed();
        phase_markers.push(phase_marker(
            "post_drain_cleanup_end",
            point_started,
            "actual",
        ));
        resource_windows.push(ResourceWindowSpec::with_requested_coverage(
            "post_drain_cleanup",
            "post_drain_cleanup_start",
            "post_drain_cleanup_end",
            post_drain_resource_start,
            post_drain_resource_end,
            post_drain_sample,
        ));
    }

    let resources = sampler.stop_with_windows(resource_windows).await;
    phase_markers.push(phase_marker(
        "resource_sampling_stopped",
        point_started,
        "actual",
    ));
    let cleanup_convergence = if post_drain_sample.is_zero() {
        None
    } else {
        let convergence = capture_cleanup_convergence(bob.as_ref(), clients.as_ref()).await;
        phase_markers.push(phase_marker(
            "cleanup_convergence_captured",
            point_started,
            "actual_after_resource_sampling",
        ));
        Some(convergence)
    };
    // Keep the canonical resource window free of diagnostic scan/allocation
    // overhead. This opt-in snapshot is intended for short A/B investigations,
    // and records every endpoint because client-side retained INVITE state is
    // sharded in the qualified server profiles.
    let retention_snapshots = match retention_timeline_task {
        Some(task) => {
            let mut snapshots = task.await.expect("retention timeline task");
            // Anchor the final fence only after every call task has drained and
            // the planned cooldown snapshot has completed. This guarantees a
            // full horizon even when profiler overhead delays call teardown.
            let retention_anchor = Instant::now();
            let retention_anchor_elapsed_ms = elapsed_millis(point_started);
            let post_retention_scheduled_ms = retention_anchor_elapsed_ms
                .saturating_add((SIP_SESSION_ANTI_REUSE_HORIZON_SECS as u64).saturating_mul(1_000));
            tokio::time::sleep(Duration::from_secs(
                SIP_SESSION_ANTI_REUSE_HORIZON_SECS as u64,
            ))
            .await;
            let mut post_retention = capture_retention_snapshot(
                "post_retention",
                post_retention_scheduled_ms,
                point_started,
                bob.as_ref(),
                clients.as_ref(),
            )
            .await;
            if let Some(object) = post_retention.as_object_mut() {
                object.insert(
                    "retention_anchor_elapsed_ms".to_string(),
                    json!(retention_anchor_elapsed_ms),
                );
                object.insert(
                    "retention_fence_elapsed_ms".to_string(),
                    json!(elapsed_millis(retention_anchor)),
                );
            }
            snapshots.push(post_retention);
            Some(snapshots)
        }
        None => None,
    };
    if let Some(snapshots) = retention_snapshots.as_ref() {
        for snapshot in snapshots {
            phase_markers.push(json!({
                "phase": snapshot["phase"].clone(),
                "kind": "retention_snapshot_actual",
                "elapsed_ms": snapshot["captured_elapsed_ms"].clone(),
            }));
        }
    }

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

    let mut report = ScenarioReport::new(report_scenario, load);
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
        // `ner` (Network Efficiency Ratio) excludes user-driven
        // rejections (busy / no-answer). With AutoAccept on the bob
        // side there are no user rejections in the denominator, so
        // NER == ASR here. The placeholder slot makes the JSON shape
        // forward-compatible with the user-rejection scenarios in
        // Phase 3 of the perf plan.
        .result("ner", round4(asr))
        .result("calls_offered", offered)
        .result("calls_succeeded", succeeded)
        .result(
            "errors",
            json!({
                "invite_send_failed": counters.invite_send_failed.load(Ordering::Relaxed),
                "answer_failed":      counters.answer_failed.load(Ordering::Relaxed),
                "bye_failed":         counters.bye_failed.load(Ordering::Relaxed),
                "timeout":            counters.timeout.load(Ordering::Relaxed),
                "harness_backpressure_rejected": counters.harness_backpressure_rejected.load(Ordering::Relaxed),
                "invite_send_error_counts": counters.invite_send_error_counts(),
            }),
        )
        .result(
            "harness_backpressure_rejected",
            counters
                .harness_backpressure_rejected
                .load(Ordering::Relaxed),
        )
        .result(
            "max_in_flight_observed",
            counters.max_in_flight_observed.load(Ordering::Relaxed),
        )
        .result("max_in_flight_limit", max_in_flight)
        .latency(&setup_hist)
        .latency(&full_hist)
        .diagnostic_block("effective_config", effective_config)
        .diagnostic_block("bye_failures", counters.bye_failure_diagnostics())
        .diagnostic_block("phase_markers", Value::Array(phase_markers))
        .with_resources(resources);
    if let Some(cleanup_convergence_at_settle) = cleanup_convergence_at_settle {
        report.diagnostic_block(
            "cleanup_convergence_at_settle",
            cleanup_convergence_at_settle,
        );
    }
    if let Some(cleanup_convergence) = cleanup_convergence {
        report.diagnostic_block("cleanup_convergence", cleanup_convergence);
    }
    if let Some(snapshots) = retention_snapshots {
        report.diagnostic_block("retention_snapshots", Value::Array(snapshots));
    }
    if let Some(boundary_snapshot) = boundary_snapshot {
        report.diagnostic_block("boundary_snapshot", boundary_snapshot);
    }
    report
}

async fn capture_retention_timeline(
    point_started: Instant,
    load: LoadProfile,
    bob: Arc<UnifiedCoordinator>,
    clients: Arc<Vec<LoadClient>>,
) -> Vec<Value> {
    let phases = retention_phase_schedule(&load);
    let mut snapshots = Vec::with_capacity(phases.len() + 1);

    for (phase, scheduled_secs) in phases {
        tokio::time::sleep_until(tokio::time::Instant::from_std(
            point_started + Duration::from_secs(scheduled_secs),
        ))
        .await;
        snapshots.push(
            capture_retention_snapshot(
                phase,
                scheduled_secs.saturating_mul(1_000),
                point_started,
                bob.as_ref(),
                clients.as_ref(),
            )
            .await,
        );
    }

    snapshots
}

async fn capture_retention_snapshot(
    phase: &'static str,
    scheduled_elapsed_ms: u64,
    point_started: Instant,
    bob: &UnifiedCoordinator,
    clients: &[LoadClient],
) -> Value {
    let scan_started_ms = elapsed_millis(point_started);
    let mut alice = Vec::with_capacity(clients.len());
    for client in clients {
        alice.push(client.peer.perf_diagnostic_snapshot().await);
    }
    let bob_snapshot = bob.perf_diagnostic_snapshot().await;
    json!({
        "phase": phase,
        "scheduled_elapsed_ms": scheduled_elapsed_ms,
        "scan_started_elapsed_ms": scan_started_ms,
        "captured_elapsed_ms": elapsed_millis(point_started),
        "anti_reuse_horizon_secs": SIP_SESSION_ANTI_REUSE_HORIZON_SECS,
        "bob": bob_snapshot,
        "alice": alice,
    })
}

const CLEANUP_CONVERGENCE_POINTERS: &[&str] = &[
    "/app_event_publisher/dispatcher/in_flight_current",
    "/app_event_publisher/dispatcher/queued_current",
    "/app_event_publisher/dispatcher/terminal_queued_current",
    "/dialog_adapter/callid_to_session",
    "/dialog_adapter/dialog_to_session",
    "/dialog_adapter/outbound_initial_invites",
    "/dialog_adapter/outgoing_bye_generation_watch",
    "/dialog_adapter/outgoing_bye_tx",
    "/dialog_adapter/outgoing_bye_wait_intents",
    "/dialog_adapter/outgoing_invite_tx",
    "/dialog_adapter/session_to_dialog",
    "/dialog_manager/dialog_invite_transactions",
    "/dialog_manager/dialog_lookup",
    "/dialog_manager/dialog_server_transactions",
    "/dialog_manager/dialog_to_session",
    "/dialog_manager/dialogs",
    "/dialog_manager/early_dialog_lookup",
    "/dialog_manager/invite_failover_attempt_reservations",
    "/dialog_manager/invite_failover_attempts",
    "/dialog_manager/invite_failover_attempts_by_dialog",
    "/dialog_manager/invite_failover_plan_reservations",
    "/dialog_manager/invite_failover_plans",
    "/dialog_manager/invite_failover_plans_by_dialog",
    "/dialog_manager/pending_response_transaction_by_dialog",
    "/dialog_manager/session_to_dialog",
    "/dialog_manager/terminated_bye_deadlines",
    "/dialog_manager/terminated_bye_lookup",
    "/dialog_manager/transaction_dialog_route_hash",
    "/dialog_manager/transaction_to_dialog",
    "/global_event_bus/broadcast_retained_total",
    "/global_event_bus/observational_handlers/in_flight_current",
    "/global_event_bus/observational_handlers/queued_current",
    "/global_event_bus/subscriber_queued_total",
    "/lifecycle/entries",
    "/lifecycle/terminal_entries",
    "/lifecycle/waiters",
    "/media_adapter/audio_receivers",
    "/media_adapter/dialog_to_session",
    "/media_adapter/media_create_reservations",
    "/media_adapter/media_resources",
    "/media_adapter/media_sessions",
    "/media_adapter/session_to_dialog",
    "/session_registry/sessions",
    "/session_store/lifecycle/authority/active",
    "/session_store/lifecycle/authority/active_capacity_in_use",
    "/session_store/lifecycle/authority/index_blocked",
    "/session_store/lifecycle/authority/index_live",
    "/session_store/lifecycle/authority/retained_total",
    "/session_store/lifecycle/authority/retired",
    "/session_store/total",
    "/state_machine_helpers/active_sessions",
    "/state_machine_helpers/subscriber_sessions",
    "/transaction_manager/breakdown/client_completions/active",
    "/transaction_manager/breakdown/client_completions/compact",
    "/transaction_manager/breakdown/client_completions/deadlines",
    "/transaction_manager/breakdown/client_completions/retained",
    "/transaction_manager/breakdown/compact_non_invite_deadlines",
    "/transaction_manager/breakdown/compact_non_invite_tombstones",
    "/transaction_manager/breakdown/retired_client_deadlines",
    "/transaction_manager/client_transactions",
    "/transaction_manager/invite_2xx_response_cache",
    "/transaction_manager/invite_2xx_response_due_queue",
    "/transaction_manager/retired_client_transactions",
    "/transaction_manager/server_invite_dialog_index",
    "/transaction_manager/server_invite_dialog_keys_by_tx",
    "/transaction_manager/server_transactions",
    "/transaction_manager/terminated_transactions",
    "/transaction_manager/total",
    "/transaction_manager/transaction_destinations",
];

async fn capture_cleanup_convergence(bob: &UnifiedCoordinator, clients: &[LoadClient]) -> Value {
    let mut endpoints = Vec::with_capacity(clients.len() + 1);
    endpoints.push(cleanup_endpoint_summary(
        "bob",
        0,
        bob.perf_diagnostic_snapshot().await,
    ));
    for (index, client) in clients.iter().enumerate() {
        endpoints.push(cleanup_endpoint_summary(
            "alice",
            index,
            client.peer.perf_diagnostic_snapshot().await,
        ));
    }
    let total_retained = endpoints
        .iter()
        .filter_map(|endpoint| endpoint["retained_total"].as_u64())
        .sum::<u64>();
    let missing_count = endpoints
        .iter()
        .filter_map(|endpoint| endpoint["missing_count"].as_u64())
        .sum::<u64>();
    json!({
        "schema": "rvoip-sip-cleanup-convergence-v1",
        "required_zero_pointer_count_per_endpoint": CLEANUP_CONVERGENCE_POINTERS.len(),
        "endpoint_count": endpoints.len(),
        "retained_total": total_retained,
        "missing_count": missing_count,
        "converged": total_retained == 0 && missing_count == 0,
        "endpoints": endpoints,
    })
}

fn cleanup_endpoint_summary(role: &str, index: usize, snapshot: Value) -> Value {
    let mut retained = Map::new();
    let mut missing = Vec::new();
    let mut retained_total = 0_u64;
    for pointer in CLEANUP_CONVERGENCE_POINTERS {
        match snapshot.pointer(pointer).and_then(Value::as_u64) {
            Some(value) => {
                retained_total = retained_total.saturating_add(value);
                if value != 0 {
                    retained.insert((*pointer).to_string(), json!(value));
                }
            }
            None => missing.push(*pointer),
        }
    }
    json!({
        "role": role,
        "index": index,
        "retained_total": retained_total,
        "nonzero": retained,
        "missing_count": missing.len(),
        "missing": missing,
    })
}

fn retention_phase_schedule(load: &LoadProfile) -> [(&'static str, u64); 3] {
    let ramp_end = load.ramp_secs;
    let steady_end = ramp_end.saturating_add(load.steady_secs);
    let cooldown_end = steady_end.saturating_add(load.cooldown_secs);
    [
        ("ramp_end", ramp_end),
        ("steady_end", steady_end),
        ("cooldown_end", cooldown_end),
    ]
}

fn measurement_identity(
    points: &[f64],
    point_index: usize,
    completed_points: &[Value],
    post_drain_settle: Duration,
    post_drain_sample: Duration,
) -> Value {
    json!({
        "schema": "rvoip-sip-perf-measurement-identity-v2",
        "peer_lifecycle": "shared_for_entire_sweep",
        "sweep_points_cps": points,
        "point_index": point_index,
        "measured_point_cps": points[point_index],
        "conditioning": {
            "points": completed_points,
            "point_count": completed_points.len(),
            "calls_offered": completed_points
                .iter()
                .filter_map(|point| point["calls_offered"].as_u64())
                .sum::<u64>(),
            "calls_succeeded": completed_points
                .iter()
                .filter_map(|point| point["calls_succeeded"].as_u64())
                .sum::<u64>(),
        },
        "resource_window": {
            "metric": "resources.rss_active_growth_mb_per_min",
            "kind": "active_load",
            "start_phase": "point_start",
            "end_phase": "calls_drained",
            "sample_interval_ms": 500,
        },
        "post_drain_cleanup": {
            "settle_secs": post_drain_settle.as_secs(),
            "requested_secs": post_drain_sample.as_secs(),
            "start_phase": if post_drain_sample.is_zero() {
                Value::Null
            } else {
                json!("post_drain_cleanup_start")
            },
            "end_phase": if post_drain_sample.is_zero() {
                Value::Null
            } else {
                json!("post_drain_cleanup_end")
            },
            "rss_metric": if post_drain_sample.is_zero() {
                Value::Null
            } else {
                json!("resources.rss_cleanup_endpoint_growth_mb_per_hour")
            },
            "rss_retained_delta_metric": if post_drain_sample.is_zero() {
                Value::Null
            } else {
                json!("resources.rss_cleanup_retained_growth_mb")
            },
            "rss_trend_metric": if post_drain_sample.is_zero() {
                Value::Null
            } else {
                json!("resources.rss_cleanup_growth_mb_per_hour")
            },
            "rss_intent_mb_per_hour": if post_drain_sample.is_zero() {
                Value::Null
            } else {
                json!(CLEANUP_RSS_INTENT_MB_PER_HOUR)
            },
            "rss_endpoint_estimator": if post_drain_sample.is_zero() {
                Value::Null
            } else {
                json!(CLEANUP_RSS_ENDPOINT_ESTIMATOR)
            },
            "minimum_representative_separation_secs": if post_drain_sample.is_zero() {
                Value::Null
            } else {
                json!(CLEANUP_RSS_MINIMUM_REPRESENTATIVE_SEPARATION_SECS)
            },
            "structural_metrics": if post_drain_sample.is_zero() {
                Value::Array(Vec::new())
            } else {
                json!([
                    "diagnostics.cleanup_convergence_at_settle",
                    "diagnostics.cleanup_convergence",
                ])
            },
        },
    })
}

fn planned_cooldown_deadline(point_started: Instant, load: &LoadProfile) -> Instant {
    point_started
        + Duration::from_secs(
            load.ramp_secs
                .saturating_add(load.steady_secs)
                .saturating_add(load.cooldown_secs),
        )
}

async fn wait_until_cooldown_deadline(deadline: Instant) {
    tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
}

#[test]
fn perf_call_setup_cps() {
    let worker_threads = env_usize("RVOIP_PERF_WORKER_THREADS", 8).max(1);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .enable_all()
        .build()
        .expect("perf runtime");
    runtime.block_on(perf_call_setup_cps_inner());
}

async fn perf_call_setup_cps_inner() {
    // Sweep points: env-driven list, or fall back to a single-point
    // run pinned at RVOIP_PERF_TARGET_CPS (default 100).
    let points = parse_sweep_env("RVOIP_PERF_SWEEP_CPS").unwrap_or_else(|| {
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
    let profile =
        std::env::var("RVOIP_PERF_PROFILE").unwrap_or_else(|_| "pbx-media-server".to_string());
    let report_scenario = std::env::var("RVOIP_PERF_REPORT_SCENARIO")
        .unwrap_or_else(|_| "perf_call_setup_cps".to_string());
    let client_profile =
        std::env::var("RVOIP_PERF_CLIENT_PROFILE").unwrap_or_else(|_| "endpoint".to_string());
    let recipe_path = std::env::var("RVOIP_PERF_RECIPE_FILE").ok();
    let recipe_provenance = recipe_provenance(recipe_path.as_deref());
    let runtime_switches = runtime_switch_snapshot();
    let channel_capacity = perf_channel_capacity(&points);
    let max_offered_cps = max_offered_cps(&points);
    let first_point = points.first().copied().unwrap_or(0.0);
    let max_in_flight_override = env_usize_opt("RVOIP_PERF_MAX_IN_FLIGHT");
    let alice_shards = env_usize("RVOIP_PERF_ALICE_SHARDS", default_alice_shards(&profile)).max(1);
    assert!(
        alice_shards
            <= media_port_range_capacity(SAME_HOST_ALICE_MEDIA_START, SAME_HOST_ALICE_MEDIA_END),
        "RVOIP_PERF_ALICE_SHARDS={} exceeds available Alice RTP ports {}-{}",
        alice_shards,
        SAME_HOST_ALICE_MEDIA_START,
        SAME_HOST_ALICE_MEDIA_END
    );

    let bob_port = support::ports::next_sip_port();
    let bob_cfg = with_high_cps_retained_lifecycle_capacity(
        apply_same_host_media_carveout(
            perf_config(
                Config::local("perf-bob", bob_port),
                channel_capacity,
                &profile,
                recipe_path.as_deref(),
            ),
            SAME_HOST_BOB_MEDIA_START,
            SAME_HOST_BOB_MEDIA_END,
            channel_capacity,
        ),
        max_offered_cps,
    );
    let alice_recipe = if profile == "legacy" {
        "legacy"
    } else {
        client_profile.as_str()
    };
    let alice_capacity = channel_capacity.div_ceil(alice_shards).max(1);
    let mut alice_configs = Vec::with_capacity(alice_shards);
    for shard in 0..alice_shards {
        let alice_port = support::ports::next_sip_port();
        let (media_start, media_end) = alice_media_subrange(shard, alice_shards);
        let alice_name = format!("perf-alice-{shard}");
        let config = apply_same_host_media_carveout(
            perf_config(
                Config::local(&alice_name, alice_port),
                alice_capacity,
                alice_recipe,
                recipe_path.as_deref(),
            ),
            media_start,
            media_end,
            alice_capacity,
        );
        let from = format!("sip:alice{shard}@127.0.0.1:{alice_port}");
        alice_configs.push((config, from));
    }
    let effective_config = json!({
        "profile": profile.clone(),
        "report_scenario": report_scenario.clone(),
        "client_profile": alice_recipe,
        "alice_shards": alice_shards,
        "recipe_file": recipe_path.clone(),
        "recipe": recipe_provenance,
        "runtime_switches": runtime_switches,
        "channel_capacity": channel_capacity,
        "retained_lifecycle_sizing": {
            "max_offered_cps": max_offered_cps,
            "anti_reuse_horizon_secs": SIP_SESSION_ANTI_REUSE_HORIZON_SECS,
            "churn_headroom_percent": RETAINED_LIFECYCLE_CHURN_HEADROOM_PERCENT,
        },
        "alice_channel_capacity_per_shard": alice_capacity,
        "max_in_flight_override": max_in_flight_override,
        "same_host_media_carveout": {
            "bob": {
                "start": SAME_HOST_BOB_MEDIA_START,
                "end": SAME_HOST_BOB_MEDIA_END,
            },
            "alice": {
                "start": SAME_HOST_ALICE_MEDIA_START,
                "end": SAME_HOST_ALICE_MEDIA_END,
            },
        },
        "bob": config_snapshot(&bob_cfg),
        "alice": alice_configs
            .first()
            .map(|(config, _)| config_snapshot(config))
            .unwrap_or_else(|| json!(null)),
        "alice_shard_configs": alice_configs
            .iter()
            .map(|(config, from)| json!({
                "from": from,
                "config": config_snapshot(config),
            }))
            .collect::<Vec<_>>(),
    });
    let bob = boot_bob(bob_cfg).await;
    let mut clients = Vec::with_capacity(alice_configs.len());
    for (config, from) in alice_configs {
        clients.push(LoadClient {
            peer: boot_alice(config).await,
            from,
        });
    }
    let clients = Arc::new(clients);
    let target = format!("sip:bob@127.0.0.1:{}", bob_port);

    let mut sweep = SweepRunner::new(
        report_scenario.clone(),
        points.clone(),
        "CPS target",
        "achieved_cps",
        "ASR",
    );
    let mut first_asr: Option<f64> = None;
    let min_asr = env_f64_opt("RVOIP_PERF_MIN_ASR").unwrap_or(match profile.as_str() {
        "legacy" if first_point > 1000.0 => 0.0,
        "legacy" => 0.95,
        _ => 0.999,
    });
    let require_all_points = std::env::var("RVOIP_PERF_REQUIRE_ALL_POINTS")
        .ok()
        .map(|value| value != "0")
        .unwrap_or(profile != "legacy");
    let require_zero_errors = std::env::var("RVOIP_PERF_REQUIRE_ZERO_ERRORS")
        .ok()
        .map(|value| value != "0")
        .unwrap_or(false);
    let final_point_post_drain_settle =
        Duration::from_secs(env_u64("RVOIP_PERF_POST_DRAIN_SETTLE_SECS", 0));
    let final_point_post_drain_sample =
        Duration::from_secs(env_u64("RVOIP_PERF_POST_DRAIN_SAMPLE_SECS", 0));
    let mut point_failures = Vec::new();
    let mut completed_points = Vec::new();

    for (point_index, &point) in points.iter().enumerate() {
        let load = LoadProfile::for_point(point, default_steady);
        let max_in_flight = max_in_flight_override.map(|value| value as u64);
        let mut point_effective_config = effective_config.clone();
        if let Some(obj) = point_effective_config.as_object_mut() {
            obj.insert("max_in_flight_limit".to_string(), json!(max_in_flight));
        }
        let post_drain_sample = if point_index + 1 == points.len() {
            final_point_post_drain_sample
        } else {
            Duration::ZERO
        };
        let post_drain_settle = if post_drain_sample.is_zero() {
            Duration::ZERO
        } else {
            final_point_post_drain_settle
        };
        let mut report = run_one_point(
            report_scenario.clone(),
            Arc::clone(&clients),
            Arc::clone(&bob._coord),
            target.clone(),
            load,
            per_call_timeout,
            max_in_flight,
            post_drain_settle,
            post_drain_sample,
            point_effective_config,
        )
        .await;
        report.diagnostic_block(
            "measurement_identity",
            measurement_identity(
                &points,
                point_index,
                &completed_points,
                post_drain_settle,
                post_drain_sample,
            ),
        );
        let report_json = report.to_json();
        let asr = report_json
            .pointer("/results/asr")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let harness_backpressure = report_json
            .pointer("/results/harness_backpressure_rejected")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let calls_offered = report_json
            .pointer("/results/calls_offered")
            .and_then(|value| value.as_u64());
        let calls_succeeded = report_json
            .pointer("/results/calls_succeeded")
            .and_then(|value| value.as_u64());
        let error_counts = report_json
            .pointer("/results/errors")
            .ok_or_else(|| "missing /results/errors".to_string())
            .and_then(validate_error_counts)
            .and_then(|(numeric_leaves, total)| {
                if numeric_leaves == 0 {
                    Err("/results/errors has no numeric leaves".to_string())
                } else {
                    Ok(total)
                }
            });
        if first_asr.is_none() {
            first_asr = Some(asr);
        }
        if require_all_points && (asr < min_asr || harness_backpressure > 0) {
            point_failures.push(format!(
                "point {}: asr {:.4} below {:.4} or harness_backpressure_rejected={}",
                point, asr, min_asr, harness_backpressure
            ));
        }
        if require_zero_errors {
            match (calls_offered, calls_succeeded, error_counts) {
                (Some(offered), Some(succeeded), Ok(0)) if offered == succeeded => {}
                (offered, succeeded, Ok(errors)) => point_failures.push(format!(
                    "point {}: zero-error gate failed: calls_offered={offered:?} calls_succeeded={succeeded:?} errors={errors}",
                    point
                )),
                (offered, succeeded, Err(error)) => point_failures.push(format!(
                    "point {}: zero-error gate failed: calls_offered={offered:?} calls_succeeded={succeeded:?} error_schema={error}",
                    point
                )),
            }
        }
        completed_points.push(json!({
            "target_cps": point,
            "calls_offered": calls_offered,
            "calls_succeeded": calls_succeeded,
        }));
        sweep.add_point(point, report);
    }

    let _written = sweep.finalize();

    bob.shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(3), bob.task).await;
    drop(clients);

    // Smoke acceptance always checks the first point. Non-legacy profiles also
    // require every sweep point to meet the ASR gate unless explicitly disabled
    // for exploratory knee-finding.
    let first = first_asr.unwrap_or(0.0);
    assert!(
        first >= min_asr,
        "first-point ASR {:.3} below {:.3} for profile {} — likely a perf regression or env issue",
        first,
        min_asr,
        profile
    );
    assert!(
        point_failures.is_empty(),
        "perf_call_setup_cps profile {} failed required sweep points: {}",
        profile,
        point_failures.join("; ")
    );
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

fn validate_error_counts(value: &Value) -> std::result::Result<(usize, u64), String> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .map(|count| (1, count))
            .ok_or_else(|| format!("error count must be a non-negative integer, got {number}")),
        Value::Object(values) => {
            values
                .values()
                .try_fold((0usize, 0u64), |(leaf_count, total), child| {
                    let (child_leaves, child_total) = validate_error_counts(child)?;
                    Ok((
                        leaf_count.saturating_add(child_leaves),
                        total.saturating_add(child_total),
                    ))
                })
        }
        other => Err(format!(
            "error count tree must contain only objects and integers, got {other}"
        )),
    }
}

fn perf_channel_capacity(points: &[f64]) -> usize {
    max_offered_cps(points).saturating_mul(4).max(1000)
}

fn max_offered_cps(points: &[f64]) -> usize {
    points
        .iter()
        .copied()
        .fold(0.0_f64, f64::max)
        .ceil()
        .max(1.0) as usize
}

fn with_high_cps_retained_lifecycle_capacity(config: Config, max_cps: usize) -> Config {
    let Some(active_capacity) = config.server_call_capacity else {
        // The endpoint recipe intentionally keeps the library defaults. The
        // beta matrix qualifies it only at its 30-CPS compatibility point.
        return config;
    };
    let required = retained_lifecycle_capacity(active_capacity, max_cps);
    let retained = config
        .server_retained_lifecycle_capacity
        .unwrap_or(0)
        .max(required);
    config.with_server_retained_lifecycle_capacity(retained)
}

fn retained_lifecycle_capacity(active_capacity: usize, max_cps: usize) -> usize {
    // A retired identifier remains fenced for the full SIP anti-reuse
    // horizon. Size that churn independently from active concurrency and add
    // an explicit 25% arrival-rate margin so scheduler catch-up bursts do not
    // turn a throughput measurement into a lifecycle-capacity test.
    let horizon_churn = max_cps.saturating_mul(SIP_SESSION_ANTI_REUSE_HORIZON_SECS);
    active_capacity.saturating_add(with_churn_headroom(horizon_churn))
}

fn with_churn_headroom(value: usize) -> usize {
    let multiplier = 100 + RETAINED_LIFECYCLE_CHURN_HEADROOM_PERCENT;
    let whole = (value / 100).saturating_mul(multiplier);
    let remainder = (value % 100).saturating_mul(multiplier).div_ceil(100);
    whole.saturating_add(remainder)
}

fn default_alice_shards(profile: &str) -> usize {
    match profile {
        "pbx-media-server" | "signaling-only-server-high-performance" => 4,
        _ => 1,
    }
}

fn perf_config(
    config: Config,
    channel_capacity: usize,
    profile: &str,
    recipe_path: Option<&str>,
) -> Config {
    if profile == "legacy" {
        config.with_high_cps_udp_auto_answer(channel_capacity)
    } else {
        let mut performance = PerformanceConfig::profile(profile)
            .with_capacity(channel_capacity)
            .with_signaling_only_rtp_port(9);
        if let Some(path) = recipe_path {
            performance = performance.with_recipe_path(path);
        }
        config
            .try_with_performance_config(performance)
            .unwrap_or_else(|e| panic!("perf harness performance recipe '{profile}' failed: {e}"))
    }
}

fn apply_same_host_media_carveout(
    config: Config,
    start: u16,
    end: u16,
    channel_capacity: usize,
) -> Config {
    let port_capacity = media_port_range_capacity(start, end);
    config
        .with_media_port_capacity(start, port_capacity)
        .with_media_session_capacity(channel_capacity.min(port_capacity))
}

fn media_port_range_capacity(start: u16, end: u16) -> usize {
    usize::from(end.saturating_sub(start)) + 1
}

fn alice_media_subrange(shard: usize, shards: usize) -> (u16, u16) {
    assert!(shards > 0, "Alice shard count must be at least 1");
    assert!(shard < shards, "Alice shard index out of range");

    let total = media_port_range_capacity(SAME_HOST_ALICE_MEDIA_START, SAME_HOST_ALICE_MEDIA_END);
    assert!(
        shards <= total,
        "Alice shard count {} exceeds available media ports {}",
        shards,
        total
    );

    let base = total / shards;
    let remainder = total % shards;
    let len = base + usize::from(shard < remainder);
    let offset = shard * base + shard.min(remainder);
    let start = SAME_HOST_ALICE_MEDIA_START + u16::try_from(offset).expect("Alice offset fits u16");
    let end = start + u16::try_from(len - 1).expect("Alice range length fits u16");
    (start, end)
}

fn config_snapshot(config: &Config) -> Value {
    json!({
        "media_mode": media_mode_name(&config.media_mode),
        "incoming_call_channel_capacity": config.incoming_call_channel_capacity,
        "state_event_channel_capacity": config.state_event_channel_capacity,
        "sip_transport_channel_capacity": config.sip_transport_channel_capacity,
        "sip_transport_dispatch_workers": config.sip_transport_dispatch_workers,
        "sip_transport_dispatch_queue_capacity": config.sip_transport_dispatch_queue_capacity,
        "sip_udp_recv_buffer_size": config.sip_udp_recv_buffer_size,
        "sip_udp_send_buffer_size": config.sip_udp_send_buffer_size,
        "sip_udp_parse_workers": config.sip_udp_parse_workers,
        "sip_udp_parse_queue_capacity": config.sip_udp_parse_queue_capacity,
        "sip_udp_parse_dispatch": config.sip_udp_parse_dispatch.map(|d| format!("{d:?}")),
        "transaction_event_channel_capacity": config.transaction_event_channel_capacity,
        "sip_transaction_dispatch_workers": config.sip_transaction_dispatch_workers,
        "sip_transaction_dispatch_queue_capacity": config.sip_transaction_dispatch_queue_capacity,
        "sip_transaction_command_channel_capacity": config.sip_transaction_command_channel_capacity,
        "effective_sip_transaction_command_channel_capacity": config
            .sip_transaction_command_channel_capacity
            .unwrap_or(Config::DEFAULT_SIP_TRANSACTION_COMMAND_CHANNEL_CAPACITY),
        "sip_transaction_dispatch_priority_burst_max": config.sip_transaction_dispatch_priority_burst_max,
        "effective_sip_transaction_dispatch_priority_burst_max": config
            .sip_transaction_dispatch_priority_burst_max
            .unwrap_or(DEFAULT_TRANSACTION_DISPATCH_PRIORITY_BURST_MAX),
        "sip_invite_2xx_retransmit_max_due_per_tick": config.sip_invite_2xx_retransmit_max_due_per_tick,
        "effective_sip_invite_2xx_retransmit_max_due_per_tick": config
            .sip_invite_2xx_retransmit_max_due_per_tick
            .unwrap_or(DEFAULT_INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK),
        "sip_dialog_dispatch_workers": config.sip_dialog_dispatch_workers,
        "sip_dialog_dispatch_queue_capacity": config.sip_dialog_dispatch_queue_capacity,
        "global_event_channel_capacity": config.global_event_channel_capacity,
        "session_event_dispatcher_workers": config.session_event_dispatcher_workers,
        "session_event_dispatcher_channel_capacity": config.session_event_dispatcher_channel_capacity,
        "server_call_capacity": config.server_call_capacity,
        "server_retained_lifecycle_capacity": config.server_retained_lifecycle_capacity,
        "server_call_admission_limit": config.server_call_admission_limit,
        "server_call_admission_soft_limit": config.server_call_admission_soft_limit,
        "server_call_admission_pacing_delay_ms": config.server_call_admission_pacing_delay_ms,
        "server_overload_retry_after_secs": config.server_overload_retry_after_secs,
        "media_port_start": config.media_port_start,
        "media_port_end": config.media_port_end,
        "media_port_capacity": config.media_port_capacity,
        "media_session_capacity": config.media_session_capacity,
        "auto_180_ringing": config.auto_180_ringing,
        "auto_100_trying": config.auto_100_trying,
        "fast_auto_accept_incoming_calls": config.fast_auto_accept_incoming_calls,
        "diagnostics": {
            "sip_udp": config.sip_udp_diagnostics,
            "transaction_timing": config.sip_transaction_timing_diagnostics,
            "dialog_timing": config.sip_dialog_timing_diagnostics,
            "media_setup": config.media_setup_diagnostics,
            "cleanup": config.cleanup_diagnostics,
        },
    })
}

fn media_mode_name(mode: &rvoip_sip::MediaMode) -> Value {
    match mode {
        rvoip_sip::MediaMode::Enabled => json!({"kind": "enabled"}),
        rvoip_sip::MediaMode::SignalingOnly { sdp_rtp_port } => {
            json!({"kind": "signaling-only", "sdp_rtp_port": sdp_rtp_port})
        }
    }
}

fn phase_marker(phase: &str, started: Instant, kind: &str) -> Value {
    json!({
        "phase": phase,
        "kind": kind,
        "elapsed_ms": elapsed_millis(started),
    })
}

fn elapsed_millis(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn recipe_provenance(recipe_path: Option<&str>) -> Value {
    match recipe_path {
        Some(path) => {
            let requested = Path::new(path);
            let resolved = requested
                .canonicalize()
                .unwrap_or_else(|_| requested.to_path_buf());
            match fs::read(&resolved) {
                Ok(bytes) => json!({
                    "source": "external",
                    "requested_path": path,
                    "resolved_path": resolved,
                    "sha256": sha256_hex(&bytes),
                }),
                Err(error) => json!({
                    "source": "external",
                    "requested_path": path,
                    "resolved_path": resolved,
                    "sha256": null,
                    "read_error": error.to_string(),
                }),
            }
        }
        None => json!({
            "source": "bundled",
            "requested_path": null,
            "resolved_path": null,
            "sha256": sha256_hex(BUNDLED_PERFORMANCE_RECIPES.as_bytes()),
        }),
    }
}

fn runtime_switch_snapshot() -> Value {
    const ENV_NAMES: &[&str] = &[
        "RVOIP_PERF_BUILD_FEATURES",
        "RVOIP_PERF_SWEEP_CPS",
        "RVOIP_PERF_TARGET_CPS",
        "RVOIP_PERF_RAMP_SECS",
        "RVOIP_PERF_STEADY_SECS",
        "RVOIP_PERF_COOLDOWN_SECS",
        "RVOIP_PERF_CALL_TIMEOUT_SECS",
        "RVOIP_PERF_WORKER_THREADS",
        "RVOIP_PERF_PROFILE",
        "RVOIP_PERF_CLIENT_PROFILE",
        "RVOIP_PERF_ALICE_SHARDS",
        "RVOIP_PERF_RECIPE_FILE",
        "RVOIP_PERF_MAX_IN_FLIGHT",
        "RVOIP_PERF_SCHED_TICK_MS",
        "RVOIP_PERF_REPORT_SCENARIO",
        "RVOIP_PERF_MIN_ASR",
        "RVOIP_PERF_REQUIRE_ALL_POINTS",
        "RVOIP_PERF_REQUIRE_ZERO_ERRORS",
        "RVOIP_PERF_RETENTION_SNAPSHOT",
        "RVOIP_PERF_BOUNDARY_SNAPSHOT",
        "RVOIP_PERF_EMBED_RESOURCE_SAMPLES",
        "RVOIP_PERF_RSS_TAIL_WINDOW_SECS",
        "RVOIP_PERF_POST_DRAIN_SETTLE_SECS",
        "RVOIP_PERF_POST_DRAIN_SAMPLE_SECS",
        "RVOIP_PERF_CALL_SETUP_DIAGNOSTICS",
        "RVOIP_PERF_MEMORY_DIAGNOSTICS",
        "RVOIP_PERF_ALLOCATOR_DIAGNOSTICS",
        "RVOIP_PERF_SYSTEM_ALLOCATOR",
        "RVOIP_PERF_DHAT",
        "RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY",
        "RVOIP_MEDIA_AUDIO_TX_PACING",
        "RVOIP_MEDIA_AUDIO_TX_PACING_TARGET_ACTIVE",
        "RVOIP_MEDIA_AUDIO_TX_SHARED_SCHEDULER",
        "RVOIP_MEDIA_AUDIO_TX_SHARED_BATCH_SIZE",
        "RVOIP_MEDIA_AUDIO_QUALITY_DIAGNOSTICS",
        "RVOIP_MEDIA_DIAGNOSTICS",
        "RVOIP_RTP_DIAGNOSTICS",
        "RVOIP_SIP_DIAGNOSTICS",
        "RVOIP_SRTP_DIAGNOSTICS",
        "RVOIP_PERF_RUN_MODE",
        "RVOIP_PERF_OUTPUT_ROOT",
        "RVOIP_TEST",
        "RVOIP_TEST_TRANSACTION_TIMEOUT_MS",
    ];

    let mut environment = Map::new();
    for name in ENV_NAMES {
        environment.insert(
            (*name).to_string(),
            std::env::var(name)
                .map(Value::String)
                .unwrap_or(Value::Null),
        );
    }

    json!({
        "environment": environment,
        "effective": {
            "audio_tx_pacing": env_enabled("RVOIP_MEDIA_AUDIO_TX_PACING"),
            "audio_tx_shared_scheduler": env_enabled("RVOIP_MEDIA_AUDIO_TX_SHARED_SCHEDULER"),
            "skip_audio_frame_delivery": env_enabled("RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY"),
            "retention_snapshot": env_enabled("RVOIP_PERF_RETENTION_SNAPSHOT"),
            "boundary_snapshot": env_enabled("RVOIP_PERF_BOUNDARY_SNAPSHOT"),
            "call_setup_diagnostics": env_enabled("RVOIP_PERF_CALL_SETUP_DIAGNOSTICS"),
            "memory_diagnostics": env_enabled("RVOIP_PERF_MEMORY_DIAGNOSTICS"),
            "allocator_diagnostics": env_enabled("RVOIP_PERF_ALLOCATOR_DIAGNOSTICS"),
            "compiled_diagnostic_features": {
                "call_setup": cfg!(feature = "perf-call-setup-diagnostics"),
                "infra_memory": cfg!(feature = "perf-infra-memory-diagnostics"),
                "media": cfg!(feature = "perf-media-diagnostics"),
                "media_memory": cfg!(feature = "perf-media-memory-diagnostics"),
                "rtp_memory": cfg!(feature = "perf-rtp-memory-diagnostics"),
            },
        },
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn update_atomic_max(target: &AtomicU64, value: u64) {
    let mut current = target.load(Ordering::Relaxed);
    while value > current {
        match target.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(next) => current = next,
        }
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    env_usize_opt(name).unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_usize_opt(name: &str) -> Option<usize> {
    std::env::var(name).ok().and_then(|s| s.parse().ok())
}

fn env_f64_opt(name: &str) -> Option<f64> {
    std::env::var(name).ok().and_then(|s| s.parse().ok())
}

fn env_enabled(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| !matches!(value.as_str(), "" | "0" | "false" | "FALSE"))
}

#[test]
fn retained_lifecycle_sizing_covers_qualified_two_thousand_cps() {
    // The default full server sweep uses capacity=max_cps*4=8,000.
    // 8,000 active + (2,000 CPS * 64 seconds * 125%) = 168,000.
    assert_eq!(retained_lifecycle_capacity(8_000, 2_000), 168_000);
}

#[test]
fn retained_lifecycle_sizing_rounds_fractional_headroom_up() {
    assert_eq!(with_churn_headroom(1), 2);
    assert_eq!(with_churn_headroom(100), 125);
    assert_eq!(with_churn_headroom(101), 127);
}

#[test]
fn retained_lifecycle_sizing_applies_only_to_server_profiles() {
    let server = with_high_cps_retained_lifecycle_capacity(
        Config::local("sized-server", 0).with_server_capacity(8_000),
        2_000,
    );
    assert_eq!(server.server_retained_lifecycle_capacity, Some(168_000));

    let endpoint = with_high_cps_retained_lifecycle_capacity(Config::local("endpoint", 0), 2_000);
    assert_eq!(endpoint.server_retained_lifecycle_capacity, None);
}

#[test]
fn zero_error_validator_counts_nested_non_negative_integers() {
    let errors = json!({
        "invite": 0,
        "bye": 2,
        "by_reason": {"timeout": 1},
        "empty_breakdown": {},
    });
    assert_eq!(validate_error_counts(&errors), Ok((3, 3)));
}

#[test]
fn zero_error_validator_rejects_non_counter_leaves() {
    for invalid in [json!(-1), json!(0.5), json!(false), json!("0"), json!([])] {
        assert!(
            validate_error_counts(&invalid).is_err(),
            "invalid error leaf was accepted: {invalid}"
        );
    }
    assert_eq!(validate_error_counts(&json!({})), Ok((0, 0)));
}

#[test]
fn bye_failure_classifier_distinguishes_release_and_wire_failures() {
    use rvoip_sip::SessionError;

    assert_eq!(
        classify_bye_failure(&SessionError::DialogError(
            "SIP BYE final response could not be observed".to_string(),
        )),
        ("confirmation_unobservable", "confirmation")
    );
    assert_eq!(
        classify_bye_failure(&SessionError::Timeout(
            "SIP BYE final response timed out".to_string(),
        )),
        ("confirmation_final_response_timeout", "confirmation")
    );
    assert_eq!(
        classify_bye_failure(&SessionError::ProtocolError(
            "SIP BYE received a non-success final response".to_string(),
        )),
        ("confirmation_non_2xx", "confirmation")
    );
    assert_eq!(
        classify_bye_failure(&SessionError::InternalError(
            "exact terminal resource release failed".to_string(),
        )),
        ("terminal_release_failed", "terminal")
    );
    assert_eq!(
        classify_bye_failure(&SessionError::Other(
            "Failed to publish app-level event (class=coordinator)".to_string(),
        )),
        ("terminal_publication_failed", "terminal")
    );
    assert_eq!(
        classify_bye_failure(&SessionError::Other(
            "lower-layer operation failed (class=opaque-erased)".to_string(),
        )),
        ("opaque_lower_layer_failure", "dispatch")
    );
}

#[test]
fn bye_failure_diagnostics_are_bounded_redacted_and_report_serializable() {
    let load = LoadProfile {
        target_cps: 2_000.0,
        ramp_secs: 5,
        steady_secs: 30,
        cooldown_secs: 5,
    };
    let counters = Counters::default();
    counters.record_bye_failure(
        &rvoip_sip::SessionError::SessionNotFound("caller-owned-secret-session-id".to_string()),
        Duration::from_millis(3),
        Duration::from_millis(7),
        Duration::from_secs(6),
        LoadPhaseBoundaries::from_load(&load),
    );

    let diagnostics = counters.bye_failure_diagnostics();
    assert_eq!(
        diagnostics.pointer("/error_counts/session_not_found"),
        Some(&json!(1))
    );
    assert_eq!(
        diagnostics.pointer("/stage_counts/dispatch"),
        Some(&json!(1))
    );
    assert_eq!(
        diagnostics.pointer("/hangup_elapsed_bucket_counts/1_to_4_ms"),
        Some(&json!(1))
    );
    assert_eq!(
        diagnostics.pointer("/call_elapsed_bucket_counts/5_to_24_ms"),
        Some(&json!(1))
    );
    assert_eq!(
        diagnostics.pointer("/load_phase_counts/steady"),
        Some(&json!(1))
    );
    assert!(!diagnostics.to_string().contains("caller-owned-secret"));

    // Keep this schema test runnable in debug builds. `ScenarioReport::new`
    // correctly rejects non-release perf runs, while the diagnostic block is
    // ordinary JSON and can be verified independently here.
    let report_json = json!({
        "diagnostics": {
            "bye_failures": diagnostics,
        },
    });
    assert_eq!(
        report_json.pointer("/diagnostics/bye_failures/schema"),
        Some(&json!("rvoip-sip-bye-failure-diagnostics-v1"))
    );
    assert!(report_json.pointer("/results/errors").is_none());
}

#[test]
fn bye_failure_time_buckets_and_load_phases_have_exact_boundaries() {
    assert_eq!(elapsed_bucket(Duration::from_nanos(999_999)), "lt_1_ms");
    assert_eq!(elapsed_bucket(Duration::from_millis(1)), "1_to_4_ms");
    assert_eq!(elapsed_bucket(Duration::from_millis(2_000)), "2_to_4_999_s");
    assert_eq!(elapsed_bucket(Duration::from_secs(15)), "ge_15_s");

    let boundaries = LoadPhaseBoundaries {
        ramp_end: Duration::from_secs(5),
        steady_end: Duration::from_secs(35),
    };
    assert_eq!(load_phase(Duration::from_millis(4_999), boundaries), "ramp");
    assert_eq!(load_phase(Duration::from_secs(5), boundaries), "steady");
    assert_eq!(
        load_phase(Duration::from_secs(35), boundaries),
        "cooldown_or_drain"
    );
}

#[test]
fn planned_cooldown_deadline_includes_every_profile_phase() {
    let started = Instant::now();
    let load = LoadProfile {
        target_cps: 2_000.0,
        ramp_secs: 5,
        steady_secs: 30,
        cooldown_secs: 5,
    };
    assert_eq!(
        planned_cooldown_deadline(started, &load).duration_since(started),
        Duration::from_secs(40)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cooldown_wait_reaches_a_short_explicit_deadline() {
    let deadline = Instant::now() + Duration::from_millis(5);
    wait_until_cooldown_deadline(deadline).await;
    assert!(Instant::now() >= deadline);
}

#[test]
fn config_snapshot_resolves_canonical_transaction_defaults() {
    let snapshot = config_snapshot(&Config::local("snapshot", 0));
    assert_eq!(
        snapshot["effective_sip_transaction_dispatch_priority_burst_max"],
        json!(64)
    );
    assert_eq!(
        snapshot["effective_sip_invite_2xx_retransmit_max_due_per_tick"],
        json!(2048)
    );
}

#[test]
fn retention_timeline_covers_every_diagnostic_phase_and_full_fence() {
    let load = LoadProfile {
        target_cps: 2000.0,
        ramp_secs: 5,
        steady_secs: 30,
        cooldown_secs: 5,
    };
    assert_eq!(
        retention_phase_schedule(&load),
        [("ramp_end", 5), ("steady_end", 35), ("cooldown_end", 40),]
    );
}

#[test]
fn measurement_identity_records_shared_peer_conditioning() {
    let points = [30.0, 100.0, 300.0, 1000.0, 2000.0];
    let conditioning = vec![
        json!({"target_cps": 30.0, "calls_offered": 975, "calls_succeeded": 975}),
        json!({"target_cps": 100.0, "calls_offered": 3250, "calls_succeeded": 3250}),
        json!({"target_cps": 300.0, "calls_offered": 9750, "calls_succeeded": 9750}),
        json!({"target_cps": 1000.0, "calls_offered": 32500, "calls_succeeded": 32500}),
    ];
    let identity = measurement_identity(
        &points,
        4,
        &conditioning,
        Duration::from_secs(95),
        Duration::from_secs(600),
    );
    assert_eq!(identity["schema"], "rvoip-sip-perf-measurement-identity-v2");
    assert_eq!(identity["peer_lifecycle"], "shared_for_entire_sweep");
    assert_eq!(identity["conditioning"]["calls_offered"], 46_475);
    assert_eq!(identity["conditioning"]["calls_succeeded"], 46_475);
    assert_eq!(identity["resource_window"]["kind"], "active_load");
    assert_eq!(identity["post_drain_cleanup"]["settle_secs"], 95);
    assert_eq!(identity["post_drain_cleanup"]["requested_secs"], 600);
    assert_eq!(
        identity["post_drain_cleanup"]["start_phase"],
        "post_drain_cleanup_start"
    );
    assert_eq!(
        identity["post_drain_cleanup"]["end_phase"],
        "post_drain_cleanup_end"
    );
    assert_eq!(
        identity["post_drain_cleanup"]["rss_metric"],
        "resources.rss_cleanup_endpoint_growth_mb_per_hour"
    );
    assert_eq!(
        identity["post_drain_cleanup"]["rss_retained_delta_metric"],
        "resources.rss_cleanup_retained_growth_mb"
    );
    assert_eq!(
        identity["post_drain_cleanup"]["rss_intent_mb_per_hour"],
        10.0
    );
    assert_eq!(
        identity["post_drain_cleanup"]["rss_endpoint_estimator"],
        "median_first_last_sixth_capped_60s"
    );
    assert_eq!(
        identity["post_drain_cleanup"]["minimum_representative_separation_secs"],
        360
    );
    assert_eq!(
        identity["post_drain_cleanup"]["structural_metrics"],
        json!([
            "diagnostics.cleanup_convergence_at_settle",
            "diagnostics.cleanup_convergence",
        ])
    );
}

#[test]
fn cleanup_convergence_treats_missing_schema_as_failure() {
    let summary = cleanup_endpoint_summary("alice", 0, json!({}));
    assert_eq!(summary["retained_total"], 0);
    assert_eq!(
        summary["missing_count"],
        u64::try_from(CLEANUP_CONVERGENCE_POINTERS.len()).expect("pointer count fits u64")
    );
}

#[test]
fn cleanup_convergence_counts_live_event_delivery_backlogs() {
    let summary = cleanup_endpoint_summary(
        "bob",
        0,
        json!({
            "app_event_publisher": {
                "dispatcher": {
                    "in_flight_current": 1,
                    "queued_current": 2,
                    "terminal_queued_current": 3,
                },
            },
            "global_event_bus": {
                "broadcast_retained_total": 4,
                "subscriber_queued_total": 5,
                "observational_handlers": {
                    "in_flight_current": 6,
                    "queued_current": 7,
                },
            },
        }),
    );

    assert_eq!(summary["retained_total"], 28);
    for pointer in [
        "/app_event_publisher/dispatcher/in_flight_current",
        "/app_event_publisher/dispatcher/queued_current",
        "/app_event_publisher/dispatcher/terminal_queued_current",
        "/global_event_bus/broadcast_retained_total",
        "/global_event_bus/observational_handlers/in_flight_current",
        "/global_event_bus/observational_handlers/queued_current",
        "/global_event_bus/subscriber_queued_total",
    ] {
        assert!(
            summary["nonzero"].get(pointer).is_some(),
            "cleanup convergence omitted {pointer}"
        );
    }
}

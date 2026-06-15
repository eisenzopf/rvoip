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
//!
//! See `docs/BENCHMARKING.md` for full interpretation.

#![allow(clippy::needless_return)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rvoip_sip::api::callback_peer::{
    CallHandler, CallHandlerDecision, CallbackPeer, ShutdownHandle,
};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::PerformanceConfig;
use serde_json::{json, Value};
use tokio::task::{JoinHandle, JoinSet};

#[path = "support/mod.rs"]
mod support;
use support::{
    parse_sweep_env, LatencyHistogram, LoadProfile, ResourceSampler, ScenarioReport, SweepRunner,
};

const SAME_HOST_BOB_MEDIA_START: u16 = 16_384;
const SAME_HOST_BOB_MEDIA_END: u16 = 40_999;
const SAME_HOST_ALICE_MEDIA_START: u16 = 51_000;
const SAME_HOST_ALICE_MEDIA_END: u16 = 65_535;

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

/// One sweep point: fresh histograms + counters, run the load profile
/// once, return a populated `ScenarioReport`. Peers stay shared across
/// sweep points so the bind cost is paid once per test.
async fn run_one_point(
    report_scenario: String,
    clients: Arc<Vec<LoadClient>>,
    target: String,
    load: LoadProfile,
    per_call_timeout: Duration,
    max_in_flight: Option<u64>,
    effective_config: Value,
) -> ScenarioReport {
    let setup_hist = Arc::new(LatencyHistogram::new("setup_latency"));
    let full_hist = Arc::new(LatencyHistogram::new("full_cycle"));
    let counters = Arc::new(Counters::default());
    let in_flight = Arc::new(AtomicU64::new(0));
    let mut tasks = JoinSet::<()>::new();

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
                )
                .await;
            });
        })
        .await
    };

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
        .with_resources(resources);
    report
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
    let channel_capacity = perf_channel_capacity(&points);
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
    let bob_cfg = apply_same_host_media_carveout(
        perf_config(
            Config::local("perf-bob", bob_port),
            channel_capacity,
            &profile,
            recipe_path.as_deref(),
        ),
        SAME_HOST_BOB_MEDIA_START,
        SAME_HOST_BOB_MEDIA_END,
        channel_capacity,
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
        "recipe_file": recipe_path,
        "channel_capacity": channel_capacity,
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
    let mut point_failures = Vec::new();

    for &point in &points {
        let load = LoadProfile::for_point(point, default_steady);
        let max_in_flight = max_in_flight_override.map(|value| value as u64);
        let mut point_effective_config = effective_config.clone();
        if let Some(obj) = point_effective_config.as_object_mut() {
            obj.insert("max_in_flight_limit".to_string(), json!(max_in_flight));
        }
        let report = run_one_point(
            report_scenario.clone(),
            Arc::clone(&clients),
            target.clone(),
            load,
            per_call_timeout,
            max_in_flight,
            point_effective_config,
        )
        .await;
        let report_json = report.to_json();
        let asr = report_json
            .pointer("/results/asr")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let harness_backpressure = report_json
            .pointer("/results/harness_backpressure_rejected")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if first_asr.is_none() {
            first_asr = Some(asr);
        }
        if require_all_points && (asr < min_asr || harness_backpressure > 0) {
            point_failures.push(format!(
                "point {}: asr {:.4} below {:.4} or harness_backpressure_rejected={}",
                point, asr, min_asr, harness_backpressure
            ));
        }
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

fn perf_channel_capacity(points: &[f64]) -> usize {
    let max_point = points
        .iter()
        .copied()
        .fold(0.0_f64, f64::max)
        .ceil()
        .max(1.0) as usize;
    max_point.saturating_mul(4).max(1000)
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
        "sip_invite_2xx_retransmit_max_due_per_tick": config.sip_invite_2xx_retransmit_max_due_per_tick,
        "sip_dialog_dispatch_workers": config.sip_dialog_dispatch_workers,
        "sip_dialog_dispatch_queue_capacity": config.sip_dialog_dispatch_queue_capacity,
        "global_event_channel_capacity": config.global_event_channel_capacity,
        "session_event_dispatcher_workers": config.session_event_dispatcher_workers,
        "session_event_dispatcher_channel_capacity": config.session_event_dispatcher_channel_capacity,
        "server_call_capacity": config.server_call_capacity,
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

fn env_usize_opt(name: &str) -> Option<usize> {
    std::env::var(name).ok().and_then(|s| s.parse().ok())
}

fn env_f64_opt(name: &str) -> Option<f64> {
    std::env::var(name).ok().and_then(|s| s.parse().ok())
}

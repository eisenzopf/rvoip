//! Split-process carrier burst caller.

#![allow(clippy::needless_return)]

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rvoip_sip::api::unified::AudioSource;
use rvoip_sip::{Config, PerformanceConfig, UnifiedCoordinator};
use serde_json::{json, Value};
use tokio::task::JoinSet;

#[cfg(feature = "dhat")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[path = "support/mod.rs"]
mod support;
use support::burst::{BurstPhase, BurstScenario, BurstScenarioBook};
use support::soak::{
    admission_diagnostics, diagnostic_artifact_path, diagnostic_sample_path,
    endpoint_global_retained_total, endpoint_metric, endpoint_retained_total,
    endpoint_retention_summary, in_process_resource_sampler_enabled, media_receive_diagnostics,
    media_setup_raw_diagnostics, media_setup_timing_diagnostics, memory_diagnostic_interval,
    memory_diagnostic_summary, read_required_u16_env, resource_sampling_diagnostics,
    retention_drain_wait, round2, round4, sip_dialog_raw_diagnostics,
    sip_dialog_timing_diagnostics, sip_udp_diagnostics, DhatProfile, EndpointRetentionSampler,
    MemoryDiagnosticSampler, RssGrowthGate,
};
use support::{LatencyHistogram, LoadProfile, ResourceSampler, ResourceSummary, ScenarioReport};

const BOB_PORT_ENV: &str = "RVOIP_PERF_BURST_BOB_PORT";
const ALICE_PORT_ENV: &str = "RVOIP_PERF_BURST_ALICE_PORT";
const RUN_DIR_ENV: &str = "RVOIP_PERF_BURST_RUN_DIR";
const SKIP_AUDIO_SOURCE_ENV: &str = "RVOIP_PERF_BURST_SKIP_AUDIO_SOURCE";

const BOB_MEDIA_START: u16 = 16_384;
const BOB_MEDIA_END: u16 = 40_999;
const ALICE_MEDIA_START: u16 = 51_000;
const ALICE_MEDIA_END: u16 = 65_535;

#[derive(Clone)]
struct LoadClient {
    peer: Arc<UnifiedCoordinator>,
    from: String,
}

struct BurstCounters {
    offered: AtomicU64,
    succeeded: AtomicU64,
    invite_send_failed: AtomicU64,
    answer_failed: AtomicU64,
    media_setup_failed: AtomicU64,
    teardown_failed: AtomicU64,
    timeout: AtomicU64,
    overload_rejected: AtomicU64,
    active_calls: AtomicU64,
    pending_setups: AtomicU64,
    max_in_flight_observed: AtomicU64,
}

impl Default for BurstCounters {
    fn default() -> Self {
        Self {
            offered: AtomicU64::new(0),
            succeeded: AtomicU64::new(0),
            invite_send_failed: AtomicU64::new(0),
            answer_failed: AtomicU64::new(0),
            media_setup_failed: AtomicU64::new(0),
            teardown_failed: AtomicU64::new(0),
            timeout: AtomicU64::new(0),
            overload_rejected: AtomicU64::new(0),
            active_calls: AtomicU64::new(0),
            pending_setups: AtomicU64::new(0),
            max_in_flight_observed: AtomicU64::new(0),
        }
    }
}

#[derive(Clone)]
struct CallFailureTrace {
    path: PathBuf,
    writer: Arc<Mutex<BufWriter<File>>>,
}

impl CallFailureTrace {
    fn new() -> Self {
        let path = diagnostic_artifact_path("burst_caller", "call_failures", "jsonl");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create burst caller diagnostics dir");
        }
        let writer = BufWriter::new(File::create(&path).expect("create call failure trace JSONL"));
        Self {
            path,
            writer: Arc::new(Mutex::new(writer)),
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn record(&self, value: Value) {
        let mut writer = self
            .writer
            .lock()
            .expect("call failure trace lock poisoned");
        serde_json::to_writer(&mut *writer, &value).expect("write call failure trace JSONL");
        writer
            .write_all(b"\n")
            .expect("write call failure trace newline");
        writer.flush().expect("flush call failure trace JSONL");
    }
}

struct PhaseMetrics {
    label: String,
    offered: AtomicU64,
    succeeded: AtomicU64,
    failed: AtomicU64,
    setup_hist: Arc<LatencyHistogram>,
    teardown_hist: Arc<LatencyHistogram>,
}

impl PhaseMetrics {
    fn new(phase: &BurstPhase) -> Self {
        let label = phase.label.clone();
        Self {
            setup_hist: Arc::new(LatencyHistogram::new(format!("setup_latency_{label}"))),
            teardown_hist: Arc::new(LatencyHistogram::new(format!("teardown_latency_{label}"))),
            label,
            offered: AtomicU64::new(0),
            succeeded: AtomicU64::new(0),
            failed: AtomicU64::new(0),
        }
    }

    fn to_json(&self) -> Value {
        let offered = self.offered.load(Ordering::Relaxed);
        let succeeded = self.succeeded.load(Ordering::Relaxed);
        json!({
            "label": self.label,
            "offered": offered,
            "succeeded": succeeded,
            "failed": self.failed.load(Ordering::Relaxed),
            "asr": if offered > 0 { round4(succeeded as f64 / offered as f64) } else { 0.0 },
            "setup_latency": self.setup_hist.snapshot(),
            "teardown_latency": self.teardown_hist.snapshot(),
        })
    }
}

struct ActiveSeries {
    samples_path: PathBuf,
    samples: Vec<(f64, u64, u64)>,
}

impl ActiveSeries {
    fn summary(&self) -> Value {
        let active_values = self
            .samples
            .iter()
            .map(|(_, active, _)| *active)
            .collect::<Vec<_>>();
        let pending_values = self
            .samples
            .iter()
            .map(|(_, _, pending)| *pending)
            .collect::<Vec<_>>();
        json!({
            "samples_path": self.samples_path.display().to_string(),
            "sample_count": self.samples.len(),
            "peak_active_calls": active_values.iter().copied().max().unwrap_or(0),
            "avg_active_calls": round2(avg_u64(&active_values)),
            "p95_active_calls": percentile_u64(active_values, 0.95),
            "peak_pending_setups": pending_values.iter().copied().max().unwrap_or(0),
        })
    }
}

struct AllCallerRetention {
    retained_total: u64,
    transaction_manager_active: u64,
    shards: Vec<Value>,
}

impl AllCallerRetention {
    fn to_json(&self) -> Value {
        json!({
            "retained_total": self.retained_total,
            "transaction_manager_active": self.transaction_manager_active,
            "shards": self.shards,
        })
    }
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_burst_caller() {
    let dhat_profile = DhatProfile::start("burst_caller");
    let scenario = load_scenario();
    let bob_port = read_required_u16_env(BOB_PORT_ENV);
    let alice_base_port = read_required_u16_env(ALICE_PORT_ENV);
    let run_dir = burst_run_dir(&scenario);
    let receiver_cfg = burst_config(
        &format!("burst-bob-{}", scenario.name),
        bob_port,
        &scenario.server_profile,
        scenario.capacity,
        BOB_MEDIA_START,
        BOB_MEDIA_END,
    );
    let rss_gate = {
        let first_alice_cfg = burst_config(
            &format!("burst-alice-{}-0", scenario.name),
            alice_base_port,
            &scenario.client_profile,
            scenario.capacity.div_ceil(scenario.alice_shards).max(1),
            ALICE_MEDIA_START,
            ALICE_MEDIA_END,
        );
        RssGrowthGate::resolve(&first_alice_cfg, &receiver_cfg)
    };
    let retention_drain_wait = retention_drain_wait();
    let skip_audio_source = read_bool_env(SKIP_AUDIO_SOURCE_ENV);
    let call_timeout = Duration::from_secs(
        std::env::var("RVOIP_PERF_CALL_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(30),
    );

    let mut clients = Vec::with_capacity(scenario.alice_shards);
    let alice_capacity = scenario.capacity.div_ceil(scenario.alice_shards).max(1);
    for shard in 0..scenario.alice_shards {
        let sip_port = alice_base_port
            .checked_add(u16::try_from(shard).expect("shard index fits u16") * 2)
            .expect("Alice SIP port range fits u16");
        let (media_start, media_end) = alice_media_subrange(shard, scenario.alice_shards);
        let name = format!("burst-alice-{}-{shard}", scenario.name);
        let cfg = burst_config(
            &name,
            sip_port,
            &scenario.client_profile,
            alice_capacity,
            media_start,
            media_end,
        );
        let from = format!("sip:alice{shard}@127.0.0.1:{sip_port}");
        clients.push(LoadClient {
            peer: boot_caller(cfg).await,
            from,
        });
    }
    let clients = Arc::new(clients);
    let target_uri = format!("sip:bob@127.0.0.1:{bob_port}");
    let counters = Arc::new(BurstCounters::default());
    let setup_hist = Arc::new(LatencyHistogram::new("setup_latency"));
    let teardown_hist = Arc::new(LatencyHistogram::new("teardown_latency"));
    let full_cycle_hist = Arc::new(LatencyHistogram::new("full_cycle"));
    let phase_metrics = Arc::new(
        scenario
            .phases
            .iter()
            .map(PhaseMetrics::new)
            .collect::<Vec<_>>(),
    );
    let in_process_resource_sampling = in_process_resource_sampler_enabled();
    let sampler = if in_process_resource_sampling {
        Some(ResourceSampler::start_with_output(
            Duration::from_secs(5),
            diagnostic_sample_path("burst_caller", "resource"),
        ))
    } else {
        None
    };
    let retention_sampler = EndpointRetentionSampler::start(
        "burst_caller",
        clients[0].peer.clone(),
        Duration::from_secs(5),
    );
    let memory_sampler = MemoryDiagnosticSampler::start(
        "burst_caller",
        &soak_like_settings(&scenario),
        memory_diagnostic_interval(),
    );
    let active_sampler = start_active_sampler(&run_dir, Arc::clone(&counters));
    let call_failure_trace = Arc::new(CallFailureTrace::new());

    let started = Instant::now();
    run_burst_load(
        Arc::clone(&clients),
        target_uri,
        scenario.clone(),
        call_timeout,
        Arc::clone(&counters),
        Arc::clone(&phase_metrics),
        Arc::clone(&setup_hist),
        Arc::clone(&teardown_hist),
        Arc::clone(&full_cycle_hist),
        Arc::clone(&call_failure_trace),
        skip_audio_source,
    )
    .await;
    let active_wall = started.elapsed();

    tokio::time::sleep(retention_drain_wait).await;
    let final_retention_all = capture_all_caller_retention(clients.as_slice()).await;
    let active_series = stop_active_sampler(active_sampler).await;
    let retention_series = retention_sampler.stop().await;
    let memory_series = match memory_sampler {
        Some(sampler) => Some(sampler.stop().await),
        None => None,
    };
    let retained_after_drain = final_retention_all.retained_total;
    let mut resources = match sampler {
        Some(sampler) => sampler.stop().await,
        None => ResourceSummary::empty(),
    };
    let rss = support::soak::rss_result_metrics(
        &resources,
        active_wall.as_secs_f64(),
        retention_drain_wait.as_secs_f64(),
    );
    let rss_gate_enforced =
        rss.post_drain_window_secs >= scenario.acceptance.min_rss_gate_window_secs;
    let rss_gate_reason = if rss_gate_enforced {
        "post_drain_window_meets_minimum"
    } else {
        "reported_only_short_post_drain_window"
    };
    resources.samples.clear();
    let dhat_diagnostics = dhat_profile.finish();

    let offered = counters.offered.load(Ordering::Relaxed);
    let succeeded = counters.succeeded.load(Ordering::Relaxed);
    let failed = offered.saturating_sub(succeeded);
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
    let accepted_cps = if active_wall.as_secs_f64() > 0.0 {
        (succeeded + counters.answer_failed.load(Ordering::Relaxed)) as f64
            / active_wall.as_secs_f64()
    } else {
        0.0
    };
    let timeout = counters.timeout.load(Ordering::Relaxed);
    let overload_rejected = counters.overload_rejected.load(Ordering::Relaxed);
    let media_setup_failed = counters.media_setup_failed.load(Ordering::Relaxed);
    let teardown_failed = counters.teardown_failed.load(Ordering::Relaxed);

    let load = LoadProfile {
        target_cps: max_phase_cps(&scenario),
        ramp_secs: 0,
        steady_secs: scenario.duration_secs(),
        cooldown_secs: retention_drain_wait.as_secs(),
    };
    let mut report = ScenarioReport::new(format!("perf_burst_caller_{}", scenario.name), load);
    let cores = report.environment().cpu_count_physical() as f64;
    let cpu_per_achieved_cps = if achieved_cps > 0.0 {
        Some(resources.avg_cpu_pct / achieved_cps)
    } else {
        None
    };
    let peak_active_calls = active_series
        .samples
        .iter()
        .map(|(_, active, _)| *active)
        .max()
        .unwrap_or(0);
    let rss_per_peak_active_call = if peak_active_calls > 0 {
        Some(
            (resources.peak_rss_mb - resources.baseline_rss_mb).max(0.0) / peak_active_calls as f64,
        )
    } else {
        None
    };
    report
        .result("process_role", "caller")
        .result("scenario", scenario.name.clone())
        .result("scenario_seed", scenario.seed)
        .result("offered_cps_peak", max_phase_cps(&scenario))
        .result("achieved_cps", round2(achieved_cps))
        .result("accepted_cps", round2(accepted_cps))
        .result(
            "cps_per_core",
            if cores > 0.0 {
                round2(achieved_cps / cores)
            } else {
                0.0
            },
        )
        .result("cpu_per_achieved_cps", cpu_per_achieved_cps.map(round2))
        .result(
            "rss_mb_per_peak_active_call",
            rss_per_peak_active_call.map(round2),
        )
        .result("asr", round4(asr))
        .result("ner", round4(asr))
        .result("calls_offered", offered)
        .result("calls_succeeded", succeeded)
        .result("calls_failed", failed)
        .result(
            "overload_rejection_ratio",
            if offered > 0 {
                round4(overload_rejected as f64 / offered as f64)
            } else {
                0.0
            },
        )
        .result(
            "timeout_ratio",
            if offered > 0 {
                round4(timeout as f64 / offered as f64)
            } else {
                0.0
            },
        )
        .result(
            "max_in_flight_observed",
            counters.max_in_flight_observed.load(Ordering::Relaxed),
        )
        .result_block("active_call_occupancy", active_series.summary())
        .result_block(
            "phase_results",
            json!(phase_metrics
                .iter()
                .map(PhaseMetrics::to_json)
                .collect::<Vec<_>>()),
        )
        .result("rss_growth_mb_per_hr", round2(rss.full_growth_mb_per_hr))
        .result(
            "rss_sustained_growth_mb_per_hr",
            round2(rss.sustained_growth_mb_per_hr),
        )
        .result(
            "rss_post_drain_growth_mb_per_hr",
            round2(rss.post_drain_growth_mb_per_hr),
        )
        .result(
            "rss_post_drain_sample_count",
            rss.post_drain_sample_count as u64,
        )
        .result(
            "rss_post_drain_window_secs",
            round2(rss.post_drain_window_secs),
        )
        .result(
            "rss_gate_growth_mb_per_hr",
            round2(rss.gate_growth_mb_per_hr),
        )
        .result("rss_gate_window", rss.gate_window)
        .result("rss_gate_enforced", rss_gate_enforced)
        .result("rss_gate_reason", rss_gate_reason)
        .result_block("rss_gate", rss_gate.to_json())
        .result("retained_objects_after_drain", retained_after_drain)
        .result(
            "transaction_manager_active_after_drain",
            final_retention_all.transaction_manager_active,
        )
        .result(
            "call_failure_trace_path",
            call_failure_trace.path().display().to_string(),
        )
        .result_block("all_caller_retention", final_retention_all.to_json())
        .result_block("scenario_definition", json!(scenario))
        .result_block(
            "effective_config",
            json!({
                "server_profile": scenario.server_profile,
                "client_profile": scenario.client_profile,
                "capacity": scenario.capacity,
                "alice_shards": scenario.alice_shards,
                "receiver": config_snapshot(&receiver_cfg),
                "caller_shards": clients
                    .iter()
                    .map(|client| json!({
                        "from": client.from,
                    }))
                    .collect::<Vec<_>>(),
            }),
        )
        .result_block("retention", endpoint_retention_summary(&retention_series))
        .result_block("sip_dialog_timing", sip_dialog_timing_diagnostics())
        .result_block("sip_udp", sip_udp_diagnostics())
        .result_block("media_setup_timing", media_setup_timing_diagnostics())
        .result_block("server_call_admission", admission_diagnostics())
        .result(
            "errors",
            json!({
                "invite_send_failed": counters.invite_send_failed.load(Ordering::Relaxed),
                "answer_failed": counters.answer_failed.load(Ordering::Relaxed),
                "media_setup_failed": media_setup_failed,
                "teardown_failed": teardown_failed,
                "timeout": timeout,
                "overload_rejected": overload_rejected,
            }),
        )
        .diagnostic_block(
            "retention_samples",
            json!({
                "sample_count": retention_series.sample_count,
                "samples_path": retention_series.samples_path.display().to_string(),
                "final_retained_objects": retained_after_drain,
            }),
        )
        .diagnostic_block(
            "rss_windows",
            json!({
                "windows": rss.windows,
                "gate_window": rss.gate_window,
                "gate_growth_mb_per_hr": round2(rss.gate_growth_mb_per_hr),
                "gate_enforced": rss_gate_enforced,
                "gate_reason": rss_gate_reason,
                "min_gate_window_secs": scenario.acceptance.min_rss_gate_window_secs,
                "post_drain_window_secs": round2(rss.post_drain_window_secs),
            }),
        )
        .diagnostic_block(
            "memory_diagnostics",
            memory_diagnostic_summary(memory_series.as_ref()),
        )
        .diagnostic_block(
            "resource_sampling",
            resource_sampling_diagnostics("burst_caller", in_process_resource_sampling),
        )
        .diagnostic_block(
            "call_failure_trace",
            json!({
                "path": call_failure_trace.path().display().to_string(),
                "expected_records": failed,
            }),
        )
        .diagnostic_block("sip_dialog_diagnostics", sip_dialog_raw_diagnostics())
        .diagnostic_block("media_setup_diagnostics", media_setup_raw_diagnostics())
        .diagnostic_block("media_receive", media_receive_diagnostics())
        .diagnostic_block("dhat", dhat_diagnostics)
        .latency(&setup_hist)
        .latency(&teardown_hist)
        .latency(&full_cycle_hist)
        .with_resources(resources);
    let json_path = write_report(
        &run_dir,
        &format!("perf_burst_caller_{}", scenario.name),
        &report,
    );
    report.print_summary(&json_path);
    write_burst_markdown(&run_dir, &scenario, &report.to_json());
    drop(clients);

    let mut gate_failures = Vec::new();
    let rss_limit = scenario
        .acceptance
        .max_rss_growth_mb_per_hr
        .unwrap_or(rss_gate.effective_mb_per_hr);
    if asr < scenario.acceptance.min_asr {
        gate_failures.push(format!(
            "ASR {:.4} below {:.4}",
            asr, scenario.acceptance.min_asr
        ));
    }
    if media_setup_failed > scenario.acceptance.max_media_setup_failed {
        gate_failures.push(format!("media_setup_failed={media_setup_failed}"));
    }
    if teardown_failed > scenario.acceptance.max_teardown_failed {
        gate_failures.push(format!("teardown_failed={teardown_failed}"));
    }
    if !scenario.acceptance.allow_overload_rejections && overload_rejected > 0 {
        gate_failures.push(format!("overload_rejected={overload_rejected}"));
    }
    if retained_after_drain > scenario.acceptance.max_retained_after_drain {
        gate_failures.push(format!(
            "caller_retained_objects_after_drain={retained_after_drain}"
        ));
    }
    if rss_gate_enforced && rss.gate_growth_mb_per_hr > rss_limit {
        gate_failures.push(format!(
            "caller RSS gate growth {:.2} MB/hr over {} window exceeded threshold {:.2} MB/hr",
            rss.gate_growth_mb_per_hr, rss.gate_window, rss_limit
        ));
    }
    assert!(
        gate_failures.is_empty(),
        "perf_burst_caller gate failed:\n{}",
        gate_failures.join("\n")
    );
}

async fn capture_all_caller_retention(clients: &[LoadClient]) -> AllCallerRetention {
    let mut retained_total = 0u64;
    let mut transaction_manager_active = 0u64;
    let mut shards = Vec::new();
    for (index, client) in clients.iter().enumerate() {
        let snapshot = client.peer.perf_diagnostic_snapshot().await;
        let retained =
            endpoint_retained_total(&snapshot) + endpoint_global_retained_total(&snapshot);
        let tx_active = endpoint_metric(&snapshot, "/transaction_manager/total");
        retained_total += retained;
        transaction_manager_active += tx_active;
        shards.push(json!({
            "index": index,
            "from": client.from,
            "retained_total": retained,
            "transaction_manager_active": tx_active,
            "summary": support::soak::endpoint_summary(&snapshot),
        }));
    }
    AllCallerRetention {
        retained_total,
        transaction_manager_active,
        shards,
    }
}

fn lifecycle_snapshot_json(snapshot: &rvoip_sip::CallLifecycleSnapshot) -> Value {
    json!({
        "call_id": snapshot.call_id.to_string(),
        "state": snapshot.state.map(|state| state.to_string()),
        "progress": snapshot
            .progress
            .iter()
            .map(|progress| {
                json!({
                    "call_id": progress.call_id.to_string(),
                    "status_code": progress.status_code,
                    "reason": progress.reason,
                    "sdp_present": progress.sdp.is_some(),
                    "sdp_bytes": progress.sdp.as_ref().map(|sdp| sdp.len()).unwrap_or(0),
                })
            })
            .collect::<Vec<_>>(),
        "answered": snapshot.answered.as_ref().map(|answered| {
            json!({
                "call_id": answered.call_id.to_string(),
                "sdp_present": answered.sdp.is_some(),
                "sdp_bytes": answered.sdp.as_ref().map(|sdp| sdp.len()).unwrap_or(0),
            })
        }),
        "media_security": snapshot
            .media_security
            .as_ref()
            .map(|security| format!("{security:?}")),
        "terminal": snapshot.terminal.as_ref().map(terminal_json),
        "latest_transfer_outcome": snapshot
            .latest_transfer_outcome
            .as_ref()
            .map(|outcome| format!("{outcome:?}")),
    })
}

fn terminal_json(terminal: &rvoip_sip::CallTerminalInfo) -> Value {
    match terminal {
        rvoip_sip::CallTerminalInfo::Ended { reason } => {
            json!({"kind": "ended", "reason": reason})
        }
        rvoip_sip::CallTerminalInfo::Failed {
            status_code,
            reason,
        } => {
            json!({"kind": "failed", "status_code": status_code, "reason": reason})
        }
        rvoip_sip::CallTerminalInfo::Cancelled => json!({"kind": "cancelled"}),
    }
}

async fn run_burst_load(
    clients: Arc<Vec<LoadClient>>,
    target_uri: String,
    scenario: BurstScenario,
    call_timeout: Duration,
    counters: Arc<BurstCounters>,
    phase_metrics: Arc<Vec<PhaseMetrics>>,
    setup_hist: Arc<LatencyHistogram>,
    teardown_hist: Arc<LatencyHistogram>,
    full_cycle_hist: Arc<LatencyHistogram>,
    call_failure_trace: Arc<CallFailureTrace>,
    skip_audio_source: bool,
) {
    let mut tasks = JoinSet::new();
    let mut call_seq = 0u64;
    for (phase_index, phase) in scenario.phases.iter().enumerate() {
        let phase_started = Instant::now();
        let phase_duration = Duration::from_secs(phase.duration_secs);
        let phase_deadline = phase_started + phase_duration;
        let target_calls = phase.expected_calls();
        let mut emitted = 0u64;
        let tick = Duration::from_millis(
            std::env::var("RVOIP_PERF_BURST_SCHED_TICK_MS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(10)
                .max(1),
        );
        while Instant::now() < phase_deadline {
            while tasks.try_join_next().is_some() {}
            let elapsed = phase_started.elapsed().min(phase_duration);
            let desired = (elapsed.as_secs_f64() * phase.cps).floor() as u64;
            while emitted < desired.min(target_calls) {
                spawn_call(
                    &mut tasks,
                    Arc::clone(&clients),
                    target_uri.clone(),
                    scenario.clone(),
                    call_seq,
                    phase_index,
                    call_timeout,
                    Arc::clone(&counters),
                    Arc::clone(&phase_metrics),
                    Arc::clone(&setup_hist),
                    Arc::clone(&teardown_hist),
                    Arc::clone(&full_cycle_hist),
                    Arc::clone(&call_failure_trace),
                    skip_audio_source,
                );
                call_seq += 1;
                emitted += 1;
            }
            tokio::time::sleep(tick).await;
        }
        while emitted < target_calls {
            spawn_call(
                &mut tasks,
                Arc::clone(&clients),
                target_uri.clone(),
                scenario.clone(),
                call_seq,
                phase_index,
                call_timeout,
                Arc::clone(&counters),
                Arc::clone(&phase_metrics),
                Arc::clone(&setup_hist),
                Arc::clone(&teardown_hist),
                Arc::clone(&full_cycle_hist),
                Arc::clone(&call_failure_trace),
                skip_audio_source,
            );
            call_seq += 1;
            emitted += 1;
        }
    }

    let max_hold = max_hold(&scenario);
    let drain_budget = max_hold + call_timeout + call_timeout + Duration::from_secs(60);
    let drain_deadline = tokio::time::sleep(drain_budget);
    tokio::pin!(drain_deadline);
    loop {
        if tasks.is_empty() {
            break;
        }
        tokio::select! {
            _ = &mut drain_deadline => {
                let outstanding = tasks.len() as u64;
                counters.timeout.fetch_add(outstanding, Ordering::Relaxed);
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                break;
            }
            joined = tasks.join_next() => {
                if joined.is_none() {
                    break;
                }
            }
        }
    }
}

fn spawn_call(
    tasks: &mut JoinSet<()>,
    clients: Arc<Vec<LoadClient>>,
    target_uri: String,
    scenario: BurstScenario,
    call_seq: u64,
    phase_index: usize,
    call_timeout: Duration,
    counters: Arc<BurstCounters>,
    phase_metrics: Arc<Vec<PhaseMetrics>>,
    setup_hist: Arc<LatencyHistogram>,
    teardown_hist: Arc<LatencyHistogram>,
    full_cycle_hist: Arc<LatencyHistogram>,
    call_failure_trace: Arc<CallFailureTrace>,
    skip_audio_source: bool,
) {
    let client = clients[(call_seq as usize) % clients.len()].clone();
    tasks.spawn(async move {
        run_one_call(
            client,
            target_uri,
            scenario,
            call_seq,
            phase_index,
            call_timeout,
            counters,
            phase_metrics,
            setup_hist,
            teardown_hist,
            full_cycle_hist,
            call_failure_trace,
            skip_audio_source,
        )
        .await;
    });
}

async fn run_one_call(
    client: LoadClient,
    target_uri: String,
    scenario: BurstScenario,
    call_seq: u64,
    phase_index: usize,
    call_timeout: Duration,
    counters: Arc<BurstCounters>,
    phase_metrics: Arc<Vec<PhaseMetrics>>,
    setup_hist: Arc<LatencyHistogram>,
    teardown_hist: Arc<LatencyHistogram>,
    full_cycle_hist: Arc<LatencyHistogram>,
    call_failure_trace: Arc<CallFailureTrace>,
    skip_audio_source: bool,
) {
    counters.offered.fetch_add(1, Ordering::Relaxed);
    counters.pending_setups.fetch_add(1, Ordering::Relaxed);
    update_atomic_max(
        &counters.max_in_flight_observed,
        counters.pending_setups.load(Ordering::Relaxed)
            + counters.active_calls.load(Ordering::Relaxed),
    );
    let phase = &phase_metrics[phase_index];
    phase.offered.fetch_add(1, Ordering::Relaxed);
    let t_start = Instant::now();
    let from = client.from.clone();
    let call_id = match client
        .peer
        .invite(Some(from.clone()), target_uri.clone())
        .send()
        .await
    {
        Ok(call_id) => call_id,
        Err(err) => {
            counters.pending_setups.fetch_sub(1, Ordering::Relaxed);
            counters.invite_send_failed.fetch_add(1, Ordering::Relaxed);
            if looks_like_overload(&err) {
                counters.overload_rejected.fetch_add(1, Ordering::Relaxed);
            }
            phase.failed.fetch_add(1, Ordering::Relaxed);
            call_failure_trace.record(json!({
                "kind": "invite_send_failed",
                "call_seq": call_seq,
                "phase_index": phase_index,
                "phase": phase.label,
                "from": from,
                "to": target_uri,
                "elapsed_ms": round2(t_start.elapsed().as_secs_f64() * 1000.0),
                "error": err.to_string(),
                "looks_like_overload": looks_like_overload(&err),
            }));
            return;
        }
    };
    let handle = client.peer.session(&call_id);
    match handle.wait_for_answered(Some(call_timeout)).await {
        Ok(_) => {
            let setup_ns = t_start.elapsed().as_nanos() as u64;
            setup_hist.record_nanos(setup_ns);
            phase.setup_hist.record_nanos(setup_ns);
        }
        Err(err) => {
            counters.pending_setups.fetch_sub(1, Ordering::Relaxed);
            let is_timeout = matches!(&err, rvoip_sip::SessionError::Timeout(_));
            let looks_like_overload = looks_like_overload(&err);
            if is_timeout {
                counters.timeout.fetch_add(1, Ordering::Relaxed);
            } else {
                counters.answer_failed.fetch_add(1, Ordering::Relaxed);
                if looks_like_overload {
                    counters.overload_rejected.fetch_add(1, Ordering::Relaxed);
                }
            }
            phase.failed.fetch_add(1, Ordering::Relaxed);
            let hangup_started = Instant::now();
            let hangup_result = handle.hangup_and_wait(Some(call_timeout)).await;
            let lifecycle_after_hangup = handle
                .lifecycle()
                .await
                .ok()
                .map(|snapshot| lifecycle_snapshot_json(&snapshot));
            call_failure_trace.record(json!({
                "kind": if is_timeout { "answer_timeout" } else { "answer_failed" },
                "call_seq": call_seq,
                "phase_index": phase_index,
                "phase": phase.label,
                "from": from,
                "to": target_uri,
                "call_id": call_id.to_string(),
                "elapsed_ms": round2(t_start.elapsed().as_secs_f64() * 1000.0),
                "error": err.to_string(),
                "looks_like_overload": looks_like_overload,
                "hangup_elapsed_ms": round2(hangup_started.elapsed().as_secs_f64() * 1000.0),
                "hangup_result": match hangup_result {
                    Ok(reason) => json!({"ok": true, "reason": reason}),
                    Err(err) => json!({"ok": false, "error": err.to_string()}),
                },
                "lifecycle_after_hangup": lifecycle_after_hangup,
            }));
            return;
        }
    }
    counters.pending_setups.fetch_sub(1, Ordering::Relaxed);
    counters.active_calls.fetch_add(1, Ordering::Relaxed);
    update_atomic_max(
        &counters.max_in_flight_observed,
        counters.pending_setups.load(Ordering::Relaxed)
            + counters.active_calls.load(Ordering::Relaxed),
    );
    if !skip_audio_source {
        if client
            .peer
            .set_audio_source(
                &call_id,
                AudioSource::Tone {
                    frequency: 440.0,
                    amplitude: 0.25,
                },
            )
            .await
            .is_err()
        {
            counters.media_setup_failed.fetch_add(1, Ordering::Relaxed);
            phase.failed.fetch_add(1, Ordering::Relaxed);
            counters.active_calls.fetch_sub(1, Ordering::Relaxed);
            let hangup_started = Instant::now();
            let hangup_result = handle.hangup_and_wait(Some(call_timeout)).await;
            let lifecycle_after_hangup = handle
                .lifecycle()
                .await
                .ok()
                .map(|snapshot| lifecycle_snapshot_json(&snapshot));
            call_failure_trace.record(json!({
                "kind": "media_setup_failed",
                "call_seq": call_seq,
                "phase_index": phase_index,
                "phase": phase.label,
                "from": from,
                "to": target_uri,
                "call_id": call_id.to_string(),
                "elapsed_ms": round2(t_start.elapsed().as_secs_f64() * 1000.0),
                "hangup_elapsed_ms": round2(hangup_started.elapsed().as_secs_f64() * 1000.0),
                "hangup_result": match hangup_result {
                    Ok(reason) => json!({"ok": true, "reason": reason}),
                    Err(err) => json!({"ok": false, "error": err.to_string()}),
                },
                "lifecycle_after_hangup": lifecycle_after_hangup,
            }));
            return;
        }
    }

    tokio::time::sleep(scenario.hold_duration(call_seq)).await;
    let teardown_start = Instant::now();
    match handle.hangup_and_wait(Some(call_timeout)).await {
        Ok(_) => {
            let teardown_ns = teardown_start.elapsed().as_nanos() as u64;
            let full_ns = t_start.elapsed().as_nanos() as u64;
            teardown_hist.record_nanos(teardown_ns);
            phase.teardown_hist.record_nanos(teardown_ns);
            full_cycle_hist.record_nanos(full_ns);
            counters.succeeded.fetch_add(1, Ordering::Relaxed);
            phase.succeeded.fetch_add(1, Ordering::Relaxed);
        }
        Err(err) => {
            if matches!(err, rvoip_sip::SessionError::Timeout(_)) {
                counters.timeout.fetch_add(1, Ordering::Relaxed);
            }
            counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
            phase.failed.fetch_add(1, Ordering::Relaxed);
            let lifecycle_after_hangup = handle
                .lifecycle()
                .await
                .ok()
                .map(|snapshot| lifecycle_snapshot_json(&snapshot));
            call_failure_trace.record(json!({
                "kind": "teardown_failed",
                "call_seq": call_seq,
                "phase_index": phase_index,
                "phase": phase.label,
                "from": from,
                "to": target_uri,
                "call_id": call_id.to_string(),
                "elapsed_ms": round2(t_start.elapsed().as_secs_f64() * 1000.0),
                "teardown_elapsed_ms": round2(teardown_start.elapsed().as_secs_f64() * 1000.0),
                "error": err.to_string(),
                "lifecycle_after_hangup": lifecycle_after_hangup,
            }));
        }
    }
    counters.active_calls.fetch_sub(1, Ordering::Relaxed);
}

async fn boot_caller(cfg: Config) -> Arc<UnifiedCoordinator> {
    let coord = UnifiedCoordinator::new(cfg)
        .await
        .expect("perf-burst caller");
    tokio::time::sleep(Duration::from_millis(200)).await;
    coord
}

fn load_scenario() -> BurstScenario {
    let name = std::env::var("RVOIP_PERF_BURST_SCENARIO")
        .or_else(|_| std::env::var("BETA_BURST_SCENARIO"))
        .unwrap_or_else(|_| "carrier-smoke".to_string());
    BurstScenarioBook::load_default_or_env().scenario(&name)
}

fn burst_config(
    name: &str,
    sip_port: u16,
    profile: &str,
    capacity: usize,
    media_start: u16,
    media_end: u16,
) -> Config {
    let media_capacity = media_port_range_capacity(media_start, media_end);
    let mut performance = PerformanceConfig::profile(profile)
        .with_capacity(capacity)
        .with_signaling_only_rtp_port(9);
    if let Some(path) = std::env::var("RVOIP_PERF_RECIPE_FILE")
        .or_else(|_| std::env::var("BETA_PERFORMANCE_RECIPE_FILE"))
        .ok()
    {
        performance = performance.with_recipe_path(path);
    }
    Config::local(name, sip_port)
        .try_with_performance_config(performance)
        .unwrap_or_else(|err| panic!("burst performance profile '{profile}' failed: {err}"))
        .with_media_port_capacity(media_start, media_capacity)
        .with_media_session_capacity(capacity.min(media_capacity))
}

fn start_active_sampler(
    run_dir: &Path,
    counters: Arc<BurstCounters>,
) -> (
    tokio::sync::watch::Sender<bool>,
    tokio::task::JoinHandle<ActiveSeries>,
) {
    let (tx, mut rx) = tokio::sync::watch::channel(false);
    let path = run_dir.join("diagnostics").join(format!(
        "burst_caller_active_calls_{}.jsonl",
        std::process::id()
    ));
    let task = tokio::spawn(async move {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create burst diagnostics dir");
        }
        let mut writer = BufWriter::new(File::create(&path).expect("create active sampler JSONL"));
        let started = Instant::now();
        let mut samples = Vec::new();
        loop {
            let t_secs = started.elapsed().as_secs_f64();
            let active = counters.active_calls.load(Ordering::Relaxed);
            let pending = counters.pending_setups.load(Ordering::Relaxed);
            let sample = json!({
                "t_secs": round2(t_secs),
                "active_calls": active,
                "pending_setups": pending,
            });
            serde_json::to_writer(&mut writer, &sample).expect("write active sampler JSONL");
            writer
                .write_all(b"\n")
                .expect("write active sampler newline");
            writer.flush().expect("flush active sampler JSONL");
            samples.push((t_secs, active, pending));
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                _ = rx.changed() => break,
            }
        }
        ActiveSeries {
            samples_path: path,
            samples,
        }
    });
    (tx, task)
}

async fn stop_active_sampler(
    sampler: (
        tokio::sync::watch::Sender<bool>,
        tokio::task::JoinHandle<ActiveSeries>,
    ),
) -> ActiveSeries {
    let _ = sampler.0.send(true);
    sampler.1.await.unwrap_or_else(|_| ActiveSeries {
        samples_path: diagnostic_sample_path("burst_caller", "active_calls"),
        samples: Vec::new(),
    })
}

fn config_snapshot(config: &Config) -> Value {
    let mut snapshot = serde_json::Map::new();
    snapshot.insert(
        "media_mode".to_string(),
        match &config.media_mode {
            rvoip_sip::MediaMode::Enabled => json!({"kind": "enabled"}),
            rvoip_sip::MediaMode::SignalingOnly { sdp_rtp_port } => {
                json!({"kind": "signaling-only", "sdp_rtp_port": sdp_rtp_port})
            }
        },
    );
    snapshot.insert(
        "auto_180_ringing".to_string(),
        json!(config.auto_180_ringing),
    );
    snapshot.insert("auto_100_trying".to_string(), json!(config.auto_100_trying));
    snapshot.insert(
        "fast_auto_accept_incoming_calls".to_string(),
        json!(config.fast_auto_accept_incoming_calls),
    );
    snapshot.insert(
        "sip_udp_diagnostics".to_string(),
        json!(config.sip_udp_diagnostics),
    );
    snapshot.insert(
        "sip_transaction_timing_diagnostics".to_string(),
        json!(config.sip_transaction_timing_diagnostics),
    );
    snapshot.insert(
        "sip_dialog_timing_diagnostics".to_string(),
        json!(config.sip_dialog_timing_diagnostics),
    );
    snapshot.insert(
        "media_setup_diagnostics".to_string(),
        json!(config.media_setup_diagnostics),
    );
    snapshot.insert(
        "cleanup_diagnostics".to_string(),
        json!(config.cleanup_diagnostics),
    );
    snapshot.insert(
        "cleanup_diagnostic_events".to_string(),
        json!(config.cleanup_diagnostic_events),
    );
    snapshot.insert(
        "incoming_call_channel_capacity".to_string(),
        json!(config.incoming_call_channel_capacity),
    );
    snapshot.insert(
        "state_event_channel_capacity".to_string(),
        json!(config.state_event_channel_capacity),
    );
    snapshot.insert(
        "sip_transport_channel_capacity".to_string(),
        json!(config.sip_transport_channel_capacity),
    );
    snapshot.insert(
        "transaction_event_channel_capacity".to_string(),
        json!(config.transaction_event_channel_capacity),
    );
    snapshot.insert(
        "global_event_channel_capacity".to_string(),
        json!(config.global_event_channel_capacity),
    );
    snapshot.insert(
        "session_event_dispatcher_workers".to_string(),
        json!(config.session_event_dispatcher_workers),
    );
    snapshot.insert(
        "session_event_dispatcher_channel_capacity".to_string(),
        json!(config.session_event_dispatcher_channel_capacity),
    );
    snapshot.insert(
        "sip_udp_recv_buffer_size".to_string(),
        json!(config.sip_udp_recv_buffer_size),
    );
    snapshot.insert(
        "sip_udp_send_buffer_size".to_string(),
        json!(config.sip_udp_send_buffer_size),
    );
    snapshot.insert(
        "sip_udp_parse_workers".to_string(),
        json!(config.sip_udp_parse_workers),
    );
    snapshot.insert(
        "sip_udp_parse_queue_capacity".to_string(),
        json!(config.sip_udp_parse_queue_capacity),
    );
    snapshot.insert(
        "sip_udp_parse_dispatch".to_string(),
        json!(config
            .sip_udp_parse_dispatch
            .map(|dispatch| format!("{dispatch:?}"))),
    );
    snapshot.insert(
        "sip_transport_dispatch_workers".to_string(),
        json!(config.sip_transport_dispatch_workers),
    );
    snapshot.insert(
        "sip_transport_dispatch_queue_capacity".to_string(),
        json!(config.sip_transport_dispatch_queue_capacity),
    );
    snapshot.insert(
        "sip_transaction_dispatch_workers".to_string(),
        json!(config.sip_transaction_dispatch_workers),
    );
    snapshot.insert(
        "sip_transaction_dispatch_queue_capacity".to_string(),
        json!(config.sip_transaction_dispatch_queue_capacity),
    );
    snapshot.insert(
        "sip_transaction_dispatch_priority_burst_max".to_string(),
        json!(config.sip_transaction_dispatch_priority_burst_max),
    );
    snapshot.insert(
        "sip_invite_2xx_retransmit_max_due_per_tick".to_string(),
        json!(config.sip_invite_2xx_retransmit_max_due_per_tick),
    );
    snapshot.insert(
        "sip_dialog_dispatch_workers".to_string(),
        json!(config.sip_dialog_dispatch_workers),
    );
    snapshot.insert(
        "sip_dialog_dispatch_queue_capacity".to_string(),
        json!(config.sip_dialog_dispatch_queue_capacity),
    );
    snapshot.insert(
        "sip_transaction_command_channel_capacity".to_string(),
        json!(config.sip_transaction_command_channel_capacity),
    );
    snapshot.insert(
        "active_call_no_media_timeout_secs".to_string(),
        json!(config.active_call_no_media_timeout_secs),
    );
    snapshot.insert(
        "active_call_media_idle_timeout_secs".to_string(),
        json!(config.active_call_media_idle_timeout_secs),
    );
    snapshot.insert(
        "server_call_capacity".to_string(),
        json!(config.server_call_capacity),
    );
    snapshot.insert(
        "server_call_admission_limit".to_string(),
        json!(config.server_call_admission_limit),
    );
    snapshot.insert(
        "server_call_admission_soft_limit".to_string(),
        json!(config.server_call_admission_soft_limit),
    );
    snapshot.insert(
        "server_call_admission_pacing_delay_ms".to_string(),
        json!(config.server_call_admission_pacing_delay_ms),
    );
    snapshot.insert(
        "server_overload_retry_after_secs".to_string(),
        json!(config.server_overload_retry_after_secs),
    );
    snapshot.insert(
        "media_port_start".to_string(),
        json!(config.media_port_start),
    );
    snapshot.insert("media_port_end".to_string(), json!(config.media_port_end));
    snapshot.insert(
        "media_port_capacity".to_string(),
        json!(config.media_port_capacity),
    );
    snapshot.insert(
        "media_session_capacity".to_string(),
        json!(config.media_session_capacity),
    );
    snapshot.insert(
        "rtp_session_buffer_config".to_string(),
        json!({
            "sender_channel_capacity": config.rtp_session_buffer_config.sender_channel_capacity,
            "receiver_channel_capacity": config.rtp_session_buffer_config.receiver_channel_capacity,
            "event_channel_capacity": config.rtp_session_buffer_config.event_channel_capacity,
        }),
    );
    snapshot.insert(
        "rtp_transport_buffer_config".to_string(),
        json!({
            "event_channel_capacity": config.rtp_transport_buffer_config.event_channel_capacity,
            "recv_buffer_size": config.rtp_transport_buffer_config.recv_buffer_size,
            "rtcp_recv_buffer_size": config.rtp_transport_buffer_config.rtcp_recv_buffer_size,
        }),
    );
    snapshot.insert(
        "media_session_controller_config".to_string(),
        json!({
            "audio_frame_pool": {
                "initial_size": config.media_session_controller_config.audio_frame_pool.initial_size,
                "max_size": config.media_session_controller_config.audio_frame_pool.max_size,
                "sample_rate": config.media_session_controller_config.audio_frame_pool.sample_rate,
                "channels": config.media_session_controller_config.audio_frame_pool.channels,
                "samples_per_frame": config.media_session_controller_config.audio_frame_pool.samples_per_frame,
            },
            "rtp_buffer_size": config.media_session_controller_config.rtp_buffer_size,
            "rtp_buffer_initial_count": config.media_session_controller_config.rtp_buffer_initial_count,
            "rtp_buffer_max_count": config.media_session_controller_config.rtp_buffer_max_count,
        }),
    );
    Value::Object(snapshot)
}

fn read_bool_env(name: &str) -> bool {
    matches!(
        std::env::var(name).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

fn burst_run_dir(scenario: &BurstScenario) -> PathBuf {
    std::env::var(RUN_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(|path| path.parent())
                .unwrap()
                .join("target")
                .join("perf-results")
                .join("perf_burst_matrix")
                .join(&scenario.name)
        })
}

fn write_report(run_dir: &Path, name: &str, report: &ScenarioReport) -> PathBuf {
    std::fs::create_dir_all(run_dir).expect("create burst run dir");
    let path = run_dir.join(format!("{name}.json"));
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&report.to_json()).expect("serialize burst report"),
    )
    .expect("write burst report");
    path
}

fn write_burst_markdown(run_dir: &Path, scenario: &BurstScenario, report_json: &Value) {
    let results = &report_json["results"];
    let latency = &report_json["latency_ns"]["setup_latency"];
    let md = format!(
        "# Burst Scenario: {}\n\n\
         {}\n\n\
         | Metric | Value |\n\
         | --- | ---: |\n\
         | Calls offered | {} |\n\
         | Calls succeeded | {} |\n\
         | ASR | {} |\n\
         | Achieved CPS | {} |\n\
         | Setup p95 ns | {} |\n\
         | Setup p99 ns | {} |\n\
         | Peak active calls | {} |\n\
         | RSS gate MB/hr | {} |\n\
         | RSS gate enforced | {} |\n\
         | RSS gate reason | {} |\n\
         | Retained after drain | {} |\n",
        scenario.name,
        scenario.description.as_deref().unwrap_or(""),
        results["calls_offered"],
        results["calls_succeeded"],
        results["asr"],
        results["achieved_cps"],
        latency["p95"],
        latency["p99"],
        results["active_call_occupancy"]["peak_active_calls"],
        results["rss_gate_growth_mb_per_hr"],
        results["rss_gate_enforced"],
        results["rss_gate_reason"],
        results["retained_objects_after_drain"],
    );
    std::fs::write(run_dir.join("_burst.md"), md).expect("write burst markdown");
}

fn alice_media_subrange(shard: usize, shards: usize) -> (u16, u16) {
    let total = media_port_range_capacity(ALICE_MEDIA_START, ALICE_MEDIA_END);
    let base = total / shards;
    let remainder = total % shards;
    let len = base + usize::from(shard < remainder);
    let offset = shard * base + shard.min(remainder);
    let start = ALICE_MEDIA_START + u16::try_from(offset).expect("Alice offset fits u16");
    let end = start + u16::try_from(len - 1).expect("Alice range length fits u16");
    (start, end)
}

fn media_port_range_capacity(start: u16, end: u16) -> usize {
    usize::from(end.saturating_sub(start)) + 1
}

fn max_phase_cps(scenario: &BurstScenario) -> f64 {
    scenario
        .phases
        .iter()
        .map(|phase| phase.cps)
        .fold(0.0_f64, f64::max)
}

fn max_hold(scenario: &BurstScenario) -> Duration {
    Duration::from_secs(
        scenario
            .hold_distribution
            .iter()
            .map(|bucket| bucket.max_secs)
            .max()
            .unwrap_or(1),
    )
}

fn avg_u64(values: &[u64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<u64>() as f64 / values.len() as f64
}

fn percentile_u64(mut values: Vec<u64>, quantile: f64) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let idx = ((values.len() - 1) as f64 * quantile).round() as usize;
    values[idx.min(values.len() - 1)]
}

fn looks_like_overload(err: &rvoip_sip::SessionError) -> bool {
    let text = err.to_string().to_ascii_lowercase();
    text.contains("503") || text.contains("service unavailable") || text.contains("overload")
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

fn soak_like_settings(scenario: &BurstScenario) -> support::soak::SoakLoadSettings {
    support::soak::SoakLoadSettings {
        duration_secs: scenario.duration_secs(),
        soak_cps: 0.0,
        active_calls: scenario.capacity as u64,
        active_phases: vec![support::soak::SoakActivePhase {
            start_secs: 0,
            duration_secs: scenario.duration_secs(),
            active_calls: scenario.capacity as u64,
        }],
        min_hold_secs: 1,
        max_hold_secs: max_hold(scenario).as_secs(),
        call_timeout: Duration::from_secs(30),
    }
}

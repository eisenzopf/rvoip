//! Split-process carrier burst receiver.

#![allow(clippy::needless_return)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::callback_peer::{
    CallHandler, CallHandlerDecision, CallbackPeer, ShutdownHandle,
};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::{Config, PerformanceConfig};
use serde_json::{json, Value};
use tokio::task::JoinHandle;

#[cfg(feature = "dhat")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[path = "support/mod.rs"]
mod support;
use support::burst::{BurstScenario, BurstScenarioBook};
use support::soak::{
    admission_diagnostics, diagnostic_artifact_path, diagnostic_sample_path, endpoint_metric,
    endpoint_retention_summary, in_process_resource_sampler_enabled, media_receive_diagnostics,
    media_setup_raw_diagnostics, media_setup_timing_diagnostics, memory_diagnostic_interval,
    memory_diagnostic_summary, read_required_u16_env, resource_sampling_diagnostics,
    retention_drain_wait, round2, sip_dialog_raw_diagnostics, sip_dialog_timing_diagnostics,
    sip_udp_diagnostics, DhatProfile, EndpointRetentionSampler, MemoryDiagnosticSampler,
    RssGrowthGate,
};
use support::{LoadProfile, ResourceSampler, ResourceSummary, ScenarioReport};

const BOB_PORT_ENV: &str = "RVOIP_PERF_BURST_BOB_PORT";
const ALICE_PORT_ENV: &str = "RVOIP_PERF_BURST_ALICE_PORT";
const READY_FILE_ENV: &str = "RVOIP_PERF_BURST_READY_FILE";
const STOP_FILE_ENV: &str = "RVOIP_PERF_BURST_STOP_FILE";
const RUN_DIR_ENV: &str = "RVOIP_PERF_BURST_RUN_DIR";

const BOB_MEDIA_START: u16 = 16_384;
const BOB_MEDIA_END: u16 = 40_999;
const ALICE_MEDIA_START: u16 = 51_000;
const ALICE_MEDIA_END: u16 = 65_535;

#[derive(Clone)]
struct BurstAccept {
    scenario: BurstScenario,
    received_frames: Arc<AtomicU64>,
    active_audio_receivers: Arc<AtomicU64>,
    completed_audio_receivers: Arc<AtomicU64>,
    incoming_calls: Arc<AtomicU64>,
}

#[async_trait::async_trait]
impl CallHandler for BurstAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let seq = self.incoming_calls.fetch_add(1, Ordering::Relaxed);
        let delay = self
            .scenario
            .answer_delay
            .duration_for(seq, self.scenario.seed);
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
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

struct ReceiverEndpoint {
    task: JoinHandle<()>,
    shutdown: ShutdownHandle,
    coordinator: Arc<rvoip_sip::UnifiedCoordinator>,
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn perf_burst_receiver() {
    let dhat_profile = DhatProfile::start("burst_receiver");
    let scenario = load_scenario();
    let bob_port = read_required_u16_env(BOB_PORT_ENV);
    let alice_port = read_required_u16_env(ALICE_PORT_ENV);
    let ready_file = PathBuf::from(
        std::env::var(READY_FILE_ENV)
            .unwrap_or_else(|_| panic!("{READY_FILE_ENV} must be set for burst receiver")),
    );
    let stop_file = PathBuf::from(
        std::env::var(STOP_FILE_ENV)
            .unwrap_or_else(|_| panic!("{STOP_FILE_ENV} must be set for burst receiver")),
    );
    let run_dir = burst_run_dir(&scenario);
    let receiver_cfg = burst_config(
        &format!("burst-bob-{}", scenario.name),
        bob_port,
        &scenario.server_profile,
        scenario.capacity,
        BOB_MEDIA_START,
        BOB_MEDIA_END,
    );
    let caller_cfg = burst_config(
        &format!("burst-alice-{}", scenario.name),
        alice_port,
        &scenario.client_profile,
        scenario.capacity.div_ceil(scenario.alice_shards).max(1),
        ALICE_MEDIA_START,
        ALICE_MEDIA_END,
    );
    let rss_gate = RssGrowthGate::resolve(&caller_cfg, &receiver_cfg);
    let retention_drain_wait = retention_drain_wait();

    let received_frames = Arc::new(AtomicU64::new(0));
    let active_audio_receivers = Arc::new(AtomicU64::new(0));
    let completed_audio_receivers = Arc::new(AtomicU64::new(0));
    let incoming_calls = Arc::new(AtomicU64::new(0));
    let receiver = boot_receiver(
        receiver_cfg.clone(),
        BurstAccept {
            scenario: scenario.clone(),
            received_frames: Arc::clone(&received_frames),
            active_audio_receivers: Arc::clone(&active_audio_receivers),
            completed_audio_receivers: Arc::clone(&completed_audio_receivers),
            incoming_calls: Arc::clone(&incoming_calls),
        },
    )
    .await;

    let in_process_resource_sampling = in_process_resource_sampler_enabled();
    let sampler = if in_process_resource_sampling {
        Some(ResourceSampler::start_with_output(
            Duration::from_secs(5),
            diagnostic_sample_path("burst_receiver", "resource"),
        ))
    } else {
        None
    };
    let retention_sampler = EndpointRetentionSampler::start(
        "burst_receiver",
        receiver.coordinator.clone(),
        Duration::from_secs(5),
    );
    let memory_sampler = MemoryDiagnosticSampler::start(
        "burst_receiver",
        &soak_like_settings(&scenario),
        memory_diagnostic_interval(),
    );
    std::fs::write(&ready_file, "ready\n").expect("write burst receiver ready file");

    let started = std::time::Instant::now();
    let max_wait = Duration::from_secs(scenario.duration_secs())
        + max_hold(&scenario)
        + retention_drain_wait
        + Duration::from_secs(300);
    let mut stop_seen = false;
    loop {
        if stop_file.exists() {
            stop_seen = true;
            break;
        }
        if started.elapsed() >= max_wait {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let active_secs = started.elapsed().as_secs_f64();

    tokio::time::sleep(retention_drain_wait).await;
    let retention_series = retention_sampler.stop().await;
    let memory_series = match memory_sampler {
        Some(sampler) => Some(sampler.stop().await),
        None => None,
    };
    let final_retention = retention_series
        .final_sample
        .clone()
        .unwrap_or_else(|| json!({}));
    let retained_after_drain = retention_series.final_retained_objects;
    let active_audio_receivers_after_drain = active_audio_receivers.load(Ordering::Relaxed);
    let completed_audio_receivers = completed_audio_receivers.load(Ordering::Relaxed);
    let received_frames = received_frames.load(Ordering::Relaxed);
    let incoming_calls = incoming_calls.load(Ordering::Relaxed);
    let mut resources = match sampler {
        Some(sampler) => sampler.stop().await,
        None => ResourceSummary::empty(),
    };
    let rss = support::soak::rss_result_metrics(
        &resources,
        active_secs,
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
    let retained_session_trace_path =
        if retained_after_drain > 0 || active_audio_receivers_after_drain > 0 {
            write_retained_session_trace(&receiver.coordinator, &final_retention).await
        } else {
            None
        };

    let load = LoadProfile {
        target_cps: 0.0,
        ramp_secs: 0,
        steady_secs: active_secs.round() as u64,
        cooldown_secs: retention_drain_wait.as_secs(),
    };
    let mut report = ScenarioReport::new(format!("perf_burst_receiver_{}", scenario.name), load);
    report
        .result("process_role", "receiver")
        .result("scenario", scenario.name.clone())
        .result("stop_seen", stop_seen)
        .result("active_secs", round2(active_secs))
        .result("configured_duration_secs", scenario.duration_secs())
        .result("incoming_calls_observed", incoming_calls)
        .result("bob_received_frames", received_frames)
        .result(
            "bob_active_audio_receivers",
            active_audio_receivers_after_drain,
        )
        .result("bob_completed_audio_receivers", completed_audio_receivers)
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
            endpoint_metric(
                &final_retention["burst_receiver"],
                "/transaction_manager/total",
            ),
        )
        .result(
            "retained_session_trace_path",
            retained_session_trace_path
                .as_ref()
                .map(|path| path.display().to_string()),
        )
        .result_block("scenario_definition", json!(scenario))
        .result_block("effective_config", config_snapshot(&receiver_cfg))
        .result_block("retention", endpoint_retention_summary(&retention_series))
        .result_block("sip_dialog_timing", sip_dialog_timing_diagnostics())
        .result_block("sip_udp", sip_udp_diagnostics())
        .result_block("media_setup_timing", media_setup_timing_diagnostics())
        .result_block("server_call_admission", admission_diagnostics())
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
            resource_sampling_diagnostics("burst_receiver", in_process_resource_sampling),
        )
        .diagnostic_block(
            "retained_session_trace",
            json!({
                "path": retained_session_trace_path
                    .as_ref()
                    .map(|path| path.display().to_string()),
                "written": retained_session_trace_path.is_some(),
                "trigger_retained_objects_after_drain": retained_after_drain,
                "trigger_active_audio_receivers_after_drain": active_audio_receivers_after_drain,
            }),
        )
        .diagnostic_block("media_receive", media_receive_diagnostics())
        .diagnostic_block("sip_dialog_diagnostics", sip_dialog_raw_diagnostics())
        .diagnostic_block("media_setup_diagnostics", media_setup_raw_diagnostics())
        .diagnostic_block("dhat", dhat_diagnostics)
        .with_resources(resources);
    let json_path = write_report(
        &run_dir,
        &format!("perf_burst_receiver_{}", scenario.name),
        &report,
    );
    report.print_summary(&json_path);

    receiver.shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(3), receiver.task).await;

    let mut gate_failures = Vec::new();
    let rss_limit = scenario
        .acceptance
        .max_rss_growth_mb_per_hr
        .unwrap_or(rss_gate.effective_mb_per_hr);
    if !stop_seen {
        gate_failures.push("receiver stop file was not observed".to_string());
    }
    if rss_gate_enforced && rss.gate_growth_mb_per_hr > rss_limit {
        gate_failures.push(format!(
            "receiver RSS gate growth {:.2} MB/hr over {} window exceeded threshold {:.2} MB/hr",
            rss.gate_growth_mb_per_hr, rss.gate_window, rss_limit
        ));
    }
    if retained_after_drain > scenario.acceptance.max_retained_after_drain {
        gate_failures.push(format!(
            "receiver_retained_objects_after_drain={retained_after_drain}"
        ));
    }
    if active_audio_receivers_after_drain
        > scenario.acceptance.max_active_audio_receivers_after_drain
    {
        gate_failures.push(format!(
            "bob_active_audio_receivers={active_audio_receivers_after_drain}"
        ));
    }
    assert!(
        gate_failures.is_empty(),
        "perf_burst_receiver gate failed:\n{}",
        gate_failures.join("\n")
    );
}

async fn write_retained_session_trace(
    coordinator: &Arc<rvoip_sip::UnifiedCoordinator>,
    final_retention: &Value,
) -> Option<PathBuf> {
    let path = diagnostic_artifact_path("burst_receiver", "retained_sessions", "json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create retained session diagnostics dir");
    }

    let sessions = coordinator.list_sessions().await;
    let mut session_entries = Vec::with_capacity(sessions.len());
    for session in sessions {
        let handle = coordinator.session(&session.session_id);
        let lifecycle = handle
            .lifecycle()
            .await
            .ok()
            .map(|snapshot| lifecycle_snapshot_json(&snapshot));
        session_entries.push(json!({
            "session_id": session.session_id.to_string(),
            "from": session.from,
            "to": session.to,
            "state": session.state.to_string(),
            "media_active": session.media_active,
            "lifecycle": lifecycle,
        }));
    }

    let perf_snapshot = coordinator.perf_diagnostic_snapshot().await;
    let session_count = session_entries.len();
    let artifact = json!({
        "sessions": session_entries,
        "session_count": session_count,
        "final_retention": final_retention,
        "perf_snapshot": perf_snapshot,
    });
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&artifact).expect("serialize retained session trace"),
    )
    .expect("write retained session trace");
    Some(path)
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

async fn boot_receiver(cfg: Config, handler: BurstAccept) -> ReceiverEndpoint {
    let peer = CallbackPeer::new(handler, cfg)
        .await
        .expect("perf-burst receiver");
    let shutdown = peer.shutdown_handle();
    let coordinator = peer.coordinator().clone();
    let task = tokio::spawn(async move {
        let _ = peer.run().await;
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    ReceiverEndpoint {
        task,
        shutdown,
        coordinator,
    }
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

fn config_snapshot(config: &Config) -> Value {
    let mut snapshot = serde_json::Map::new();
    snapshot.insert(
        "profile_media_mode".to_string(),
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

fn burst_run_dir(scenario: &BurstScenario) -> PathBuf {
    std::env::var(RUN_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(|path| path.parent())
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

fn media_port_range_capacity(start: u16, end: u16) -> usize {
    usize::from(end.saturating_sub(start)) + 1
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

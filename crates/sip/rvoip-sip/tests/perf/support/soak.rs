use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::callback_peer::{
    CallHandler, CallHandlerDecision, CallbackPeer, ShutdownHandle,
};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::{AudioSource, Config, UnifiedCoordinator};
use serde_json::json;
use tokio::task::{JoinHandle, JoinSet};

use super::{LatencyHistogram, ResourceSample, ResourceSummary};

pub const DEFAULT_PERF_APP_EVENT_CHANNEL_CAPACITY: usize =
    Config::DEFAULT_APP_EVENT_CHANNEL_CAPACITY;
pub const DEFAULT_RETENTION_DRAIN_WAIT_SECS: usize = 40;
pub const BOB_PORT_ENV: &str = "RVOIP_PERF_SOAK_BOB_PORT";
pub const ALICE_PORT_ENV: &str = "RVOIP_PERF_SOAK_ALICE_PORT";
pub const READY_FILE_ENV: &str = "RVOIP_PERF_SOAK_READY_FILE";
pub const STOP_FILE_ENV: &str = "RVOIP_PERF_SOAK_STOP_FILE";
pub const RUN_DIR_ENV: &str = "RVOIP_PERF_SOAK_RUN_DIR";
pub const ACTIVE_PHASES_ENV: &str = "RVOIP_PERF_SOAK_ACTIVE_CALL_PHASES";
pub const DISABLE_IN_PROCESS_RESOURCE_SAMPLER_ENV: &str =
    "RVOIP_PERF_DISABLE_IN_PROCESS_RESOURCE_SAMPLER";
pub const EXTERNAL_RESOURCE_DIAGNOSTICS_DIR_ENV: &str = "RVOIP_PERF_PROFILE_EXTERNAL_RESOURCE_DIR";
pub const MEMORY_DIAGNOSTICS_ENV: &str = "RVOIP_PERF_MEMORY_DIAGNOSTICS";
pub const MEMORY_DIAG_INTERVAL_SECS_ENV: &str = "RVOIP_PERF_MEMORY_DIAG_INTERVAL_SECS";
pub const ALLOCATOR_DIAGNOSTICS_ENV: &str = "RVOIP_PERF_ALLOCATOR_DIAGNOSTICS";
pub const MIMALLOC_COLLECT_AT_ENV: &str = "RVOIP_PERF_MIMALLOC_COLLECT_AT";
pub const SKIP_AUDIO_FRAME_DELIVERY_ENV: &str = "RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY";
pub const DHAT_ENV: &str = "RVOIP_PERF_DHAT";

pub fn diagnostic_sample_path(role: &str, kind: &str) -> PathBuf {
    diagnostic_artifact_path(role, &format!("{kind}_samples"), "jsonl")
}

pub fn diagnostic_artifact_path(role: &str, kind: &str, extension: &str) -> PathBuf {
    let base_dir = std::env::var(RUN_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| perf_results_dir().join("perf_soak_split"));
    base_dir.join("diagnostics").join(format!(
        "{role}_{kind}_{}.{}",
        std::process::id(),
        extension
    ))
}

pub fn in_process_resource_sampler_enabled() -> bool {
    !read_bool_env(DISABLE_IN_PROCESS_RESOURCE_SAMPLER_ENV)
}

pub fn memory_diagnostics_enabled() -> bool {
    read_bool_env(MEMORY_DIAGNOSTICS_ENV)
}

pub fn memory_diagnostic_interval() -> Duration {
    Duration::from_secs(
        read_positive_usize_env(MEMORY_DIAG_INTERVAL_SECS_ENV)
            .unwrap_or(5)
            .try_into()
            .unwrap_or(u64::MAX),
    )
}

pub fn resource_sampling_diagnostics(role: &str, in_process_enabled: bool) -> serde_json::Value {
    json!({
        "role": role,
        "in_process_enabled": in_process_enabled,
        "disable_env": DISABLE_IN_PROCESS_RESOURCE_SAMPLER_ENV,
        "external_diagnostics_dir": std::env::var(EXTERNAL_RESOURCE_DIAGNOSTICS_DIR_ENV).ok(),
    })
}

pub fn media_receive_diagnostics() -> serde_json::Value {
    json!({
        "skip_audio_frame_delivery": read_bool_env(SKIP_AUDIO_FRAME_DELIVERY_ENV),
        "skip_audio_frame_delivery_env": SKIP_AUDIO_FRAME_DELIVERY_ENV,
    })
}

pub struct DhatProfile {
    enabled: bool,
    path: Option<PathBuf>,
    #[cfg(feature = "dhat")]
    profiler: Option<dhat::Profiler>,
}

impl DhatProfile {
    pub fn start(role: &'static str) -> Self {
        #[cfg(not(feature = "dhat"))]
        let _ = role;

        if !read_bool_env(DHAT_ENV) {
            return Self {
                enabled: false,
                path: None,
                #[cfg(feature = "dhat")]
                profiler: None,
            };
        }

        #[cfg(feature = "dhat")]
        {
            let path = diagnostic_artifact_path(role, "dhat_heap", "json");
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create dhat diagnostics dir");
            }
            let profiler = dhat::Profiler::builder().file_name(&path).build();
            Self {
                enabled: true,
                path: Some(path),
                profiler: Some(profiler),
            }
        }

        #[cfg(not(feature = "dhat"))]
        {
            panic!("{DHAT_ENV}=1 requires building rvoip-sip with --features dhat");
        }
    }

    pub fn finish(self) -> serde_json::Value {
        #[cfg(feature = "dhat")]
        {
            let enabled = self.enabled;
            let path = self.path;
            let stats = if enabled && self.profiler.is_some() {
                let stats = dhat::HeapStats::get();
                Some(json!({
                    "total_blocks": stats.total_blocks,
                    "total_bytes": stats.total_bytes,
                    "curr_blocks": stats.curr_blocks,
                    "curr_bytes": stats.curr_bytes,
                    "max_blocks": stats.max_blocks,
                    "max_bytes": stats.max_bytes,
                }))
            } else {
                None
            };
            drop(self.profiler);
            json!({
                "enabled": enabled,
                "enable_env": DHAT_ENV,
                "feature_enabled": true,
                "profile_path": path.as_ref().map(|path| path.display().to_string()),
                "heap_stats_before_drop": stats,
                "viewer": "https://nnethercote.github.io/dh_view/dh_view.html",
            })
        }

        #[cfg(not(feature = "dhat"))]
        {
            json!({
                "enabled": false,
                "enable_env": DHAT_ENV,
                "feature_enabled": false,
                "profile_path": null,
                "heap_stats_before_drop": null,
                "viewer": "https://nnethercote.github.io/dh_view/dh_view.html",
            })
        }
    }
}

fn perf_results_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set under cargo"),
    );
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("target").join("perf-results"))
        .unwrap_or_else(|| PathBuf::from("target/perf-results"))
}

#[derive(Clone)]
pub struct SoakLoadSettings {
    pub duration_secs: u64,
    pub soak_cps: f64,
    pub active_calls: u64,
    pub active_phases: Vec<SoakActivePhase>,
    pub min_hold_secs: u64,
    pub max_hold_secs: u64,
    pub call_timeout: Duration,
}

#[derive(Clone, Copy)]
pub struct SoakActivePhase {
    pub start_secs: u64,
    pub duration_secs: u64,
    pub active_calls: u64,
}

impl SoakActivePhase {
    pub fn end_secs(self) -> u64 {
        self.start_secs + self.duration_secs
    }
}

impl SoakLoadSettings {
    pub fn from_env() -> Self {
        let soak_cps: f64 = read_nonnegative_f64_env("RVOIP_PERF_SOAK_CPS").unwrap_or(0.0);
        let configured_duration_secs =
            std::env::var("RVOIP_PERF_SOAK_DURATION_SECS")
                .ok()
                .map(|raw| {
                    raw.parse::<u64>()
                        .unwrap_or_else(|_| panic!("RVOIP_PERF_SOAK_DURATION_SECS must be a u64"))
                });
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
        let active_phases = if let Some(phases) = parse_active_phases_env() {
            let phase_duration_secs: u64 = phases.iter().map(|phase| phase.duration_secs).sum();
            if let Some(configured) = configured_duration_secs {
                assert_eq!(
                    configured, phase_duration_secs,
                    "{ACTIVE_PHASES_ENV} duration sum must match RVOIP_PERF_SOAK_DURATION_SECS when both are set"
                );
            }
            phases
        } else {
            let duration_secs = configured_duration_secs.unwrap_or(1800);
            let active_calls: u64 = std::env::var("RVOIP_PERF_SOAK_ACTIVE_CALLS")
                .or_else(|_| std::env::var("RVOIP_PERF_SOAK_MEDIA_CALLS"))
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(30);
            assert!(
                active_calls > 0,
                "RVOIP_PERF_SOAK_ACTIVE_CALLS must be greater than 0"
            );
            vec![SoakActivePhase {
                start_secs: 0,
                duration_secs,
                active_calls,
            }]
        };
        let duration_secs = active_phases
            .last()
            .map(|phase| phase.end_secs())
            .unwrap_or(0);
        let active_calls = active_phases
            .iter()
            .map(|phase| phase.active_calls)
            .max()
            .unwrap_or(0);

        Self {
            duration_secs,
            soak_cps,
            active_calls,
            active_phases,
            min_hold_secs,
            max_hold_secs,
            call_timeout,
        }
    }

    pub fn total(&self) -> Duration {
        Duration::from_secs(self.duration_secs)
    }

    pub fn max_active_calls(&self) -> u64 {
        self.active_calls
    }

    pub fn initial_active_calls(&self) -> u64 {
        self.active_phases
            .first()
            .map(|phase| phase.active_calls)
            .unwrap_or(self.active_calls)
    }

    pub fn final_active_calls(&self) -> u64 {
        self.active_phases
            .last()
            .map(|phase| phase.active_calls)
            .unwrap_or(self.active_calls)
    }

    pub fn active_calls_at(&self, elapsed: Duration) -> u64 {
        let secs = elapsed.as_secs();
        self.active_phases
            .iter()
            .find(|phase| secs >= phase.start_secs && secs < phase.end_secs())
            .map(|phase| phase.active_calls)
            .unwrap_or(0)
    }

    pub fn next_slot_activation_secs(&self, slot: u64, elapsed: Duration) -> Option<u64> {
        let secs = elapsed.as_secs();
        self.active_phases
            .iter()
            .find(|phase| phase.start_secs > secs && slot < phase.active_calls)
            .map(|phase| phase.start_secs)
    }

    pub fn next_slot_deactivation_secs(&self, slot: u64, elapsed: Duration) -> Option<u64> {
        let secs = elapsed.as_secs();
        self.active_phases
            .iter()
            .find(|phase| phase.start_secs > secs && slot >= phase.active_calls)
            .map(|phase| phase.start_secs)
    }

    pub fn active_phases_json(&self) -> serde_json::Value {
        json!(self
            .active_phases
            .iter()
            .map(|phase| {
                json!({
                    "start_secs": phase.start_secs,
                    "duration_secs": phase.duration_secs,
                    "end_secs": phase.end_secs(),
                    "active_calls": phase.active_calls,
                })
            })
            .collect::<Vec<_>>())
    }
}

#[derive(Default)]
pub struct SoakCounters {
    pub offered: AtomicU64,
    pub succeeded: AtomicU64,
    pub failed: AtomicU64,
    pub active_offered: AtomicU64,
    pub active_succeeded: AtomicU64,
    pub churn_offered: AtomicU64,
    pub churn_succeeded: AtomicU64,
    pub media_setup_failed: AtomicU64,
    pub teardown_failed: AtomicU64,
}

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
pub struct ReceiverDiagnostics {
    pub received_frames: Arc<AtomicU64>,
    pub active_audio_receivers: Arc<AtomicU64>,
    pub completed_audio_receivers: Arc<AtomicU64>,
}

pub struct ReceiverEndpoint {
    pub task: JoinHandle<()>,
    pub shutdown: ShutdownHandle,
    pub coordinator: Arc<UnifiedCoordinator>,
}

pub async fn boot_receiver(cfg: Config, diagnostics: ReceiverDiagnostics) -> ReceiverEndpoint {
    let peer = CallbackPeer::new(
        CountingAccept {
            received_frames: diagnostics.received_frames,
            active_audio_receivers: diagnostics.active_audio_receivers,
            completed_audio_receivers: diagnostics.completed_audio_receivers,
        },
        cfg,
    )
    .await
    .expect("perf-soak receiver");
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

pub async fn boot_caller(cfg: Config) -> Arc<UnifiedCoordinator> {
    let coord = UnifiedCoordinator::new(cfg)
        .await
        .expect("perf-soak caller");
    tokio::time::sleep(Duration::from_millis(200)).await;
    coord
}

pub fn perf_config(name: &str, port: u16) -> Config {
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

pub fn retention_drain_wait() -> Duration {
    Duration::from_secs(
        read_positive_usize_env("RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS")
            .unwrap_or(DEFAULT_RETENTION_DRAIN_WAIT_SECS)
            .try_into()
            .unwrap_or(u64::MAX),
    )
}

pub async fn run_caller_load(
    caller: Arc<UnifiedCoordinator>,
    from: String,
    target_uri: String,
    settings: SoakLoadSettings,
    counters: Arc<SoakCounters>,
    setup_hist: Arc<LatencyHistogram>,
    first_minute_hist: Arc<LatencyHistogram>,
    last_minute_hist: Arc<LatencyHistogram>,
) {
    let total = settings.total();
    let call_timeout = settings.call_timeout;
    let started = std::time::Instant::now();
    let active_deadline = started + total;
    let mut active_tasks = JoinSet::<()>::new();
    for slot in 0..settings.max_active_calls() {
        let caller = Arc::clone(&caller);
        let from = from.clone();
        let target_uri = target_uri.clone();
        let settings = settings.clone();
        let counters = Arc::clone(&counters);
        let setup_hist = Arc::clone(&setup_hist);
        let first_minute_hist = Arc::clone(&first_minute_hist);
        let last_minute_hist = Arc::clone(&last_minute_hist);
        active_tasks.spawn(async move {
            let mut cycle = 0u64;
            loop {
                let now = std::time::Instant::now();
                if now >= active_deadline {
                    break;
                }
                let elapsed = now.duration_since(started);
                if slot >= settings.active_calls_at(elapsed) {
                    let Some(next_activation_secs) =
                        settings.next_slot_activation_secs(slot, elapsed)
                    else {
                        break;
                    };
                    let wake_at =
                        (started + Duration::from_secs(next_activation_secs)).min(active_deadline);
                    let wait = wake_at.saturating_duration_since(std::time::Instant::now());
                    if !wait.is_zero() {
                        tokio::time::sleep(wait).await;
                    }
                    continue;
                }

                let slot_stop_at =
                    active_slot_stop_deadline(&settings, slot, started, elapsed, active_deadline);
                let remaining_before_stop =
                    slot_stop_at.saturating_duration_since(std::time::Instant::now());
                if remaining_before_stop <= setup_teardown_budget(settings.call_timeout) {
                    if !remaining_before_stop.is_zero() {
                        tokio::time::sleep(remaining_before_stop).await;
                    }
                    continue;
                }

                let dispatch_at = std::time::Instant::now();
                counters.offered.fetch_add(1, Ordering::Relaxed);
                counters.active_offered.fetch_add(1, Ordering::Relaxed);
                let call_id = match caller
                    .invite(Some(from.clone()), target_uri.clone())
                    .send()
                    .await
                {
                    Ok(id) => id,
                    Err(_) => {
                        counters.failed.fetch_add(1, Ordering::Relaxed);
                        counters.media_setup_failed.fetch_add(1, Ordering::Relaxed);
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                let handle = caller.session(&call_id);
                if handle
                    .wait_for_answered(Some(settings.call_timeout))
                    .await
                    .is_err()
                {
                    counters.failed.fetch_add(1, Ordering::Relaxed);
                    counters.media_setup_failed.fetch_add(1, Ordering::Relaxed);
                    if handle
                        .hangup_and_wait(Some(settings.call_timeout))
                        .await
                        .is_err()
                    {
                        counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
                    }
                    continue;
                }

                let ns = dispatch_at.elapsed().as_nanos() as u64;
                setup_hist.record_nanos(ns);
                let elapsed = dispatch_at.duration_since(started);
                if elapsed.as_secs() < 60 {
                    first_minute_hist.record_nanos(ns);
                }
                if total.saturating_sub(elapsed).as_secs() <= 60 {
                    last_minute_hist.record_nanos(ns);
                }

                if caller
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
                    counters.failed.fetch_add(1, Ordering::Relaxed);
                    counters.media_setup_failed.fetch_add(1, Ordering::Relaxed);
                    let _ = handle.hangup_and_wait(Some(settings.call_timeout)).await;
                    continue;
                }

                let hold = cycling_hold_duration(
                    slot,
                    cycle,
                    settings.min_hold_secs,
                    settings.max_hold_secs,
                );
                let mut hold_deadline = (std::time::Instant::now() + hold).min(active_deadline);
                if let Some(deactivation_secs) =
                    settings.next_slot_deactivation_secs(slot, dispatch_at.duration_since(started))
                {
                    hold_deadline =
                        hold_deadline.min(started + Duration::from_secs(deactivation_secs));
                }
                let remaining = hold_deadline.saturating_duration_since(std::time::Instant::now());
                if !remaining.is_zero() {
                    tokio::time::sleep(remaining).await;
                }

                if handle
                    .hangup_and_wait(Some(settings.call_timeout))
                    .await
                    .is_ok()
                {
                    counters.succeeded.fetch_add(1, Ordering::Relaxed);
                    counters.active_succeeded.fetch_add(1, Ordering::Relaxed);
                } else {
                    counters.failed.fetch_add(1, Ordering::Relaxed);
                    counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
                }
                cycle += 1;
            }
        });
    }

    let mut churn_tasks = JoinSet::<()>::new();
    if settings.soak_cps > 0.0 {
        let tick = Duration::from_secs_f64(1.0 / settings.soak_cps);
        loop {
            while let Some(result) = churn_tasks.try_join_next() {
                let _ = result;
            }

            let elapsed = started.elapsed();
            if elapsed >= total {
                break;
            }
            let caller = Arc::clone(&caller);
            let from = from.clone();
            let target_uri = target_uri.clone();
            let setup_hist = Arc::clone(&setup_hist);
            let first_minute_hist = Arc::clone(&first_minute_hist);
            let last_minute_hist = Arc::clone(&last_minute_hist);
            let counters = Arc::clone(&counters);
            churn_tasks.spawn(async move {
                let dispatch_at = std::time::Instant::now();
                counters.offered.fetch_add(1, Ordering::Relaxed);
                counters.churn_offered.fetch_add(1, Ordering::Relaxed);
                let call_id = match caller.invite(Some(from), target_uri).send().await {
                    Ok(id) => id,
                    Err(_) => {
                        counters.failed.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                };
                let handle = caller.session(&call_id);
                if handle.wait_for_answered(Some(call_timeout)).await.is_err() {
                    counters.failed.fetch_add(1, Ordering::Relaxed);
                    if handle.hangup_and_wait(Some(call_timeout)).await.is_err() {
                        counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
                    }
                    return;
                }
                let ns = dispatch_at.elapsed().as_nanos() as u64;
                setup_hist.record_nanos(ns);
                let elapsed = dispatch_at.duration_since(started);
                if elapsed.as_secs() < 60 {
                    first_minute_hist.record_nanos(ns);
                }
                if total.saturating_sub(elapsed).as_secs() <= 60 {
                    last_minute_hist.record_nanos(ns);
                }
                if handle.hangup_and_wait(Some(call_timeout)).await.is_ok() {
                    counters.succeeded.fetch_add(1, Ordering::Relaxed);
                    counters.churn_succeeded.fetch_add(1, Ordering::Relaxed);
                } else {
                    counters.failed.fetch_add(1, Ordering::Relaxed);
                    counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
                }
            });
            tokio::time::sleep(tick).await;
        }
    } else {
        tokio::time::sleep(total).await;
    }

    let drain_result = tokio::time::timeout(drain_join_timeout(call_timeout), async {
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

    let active_drain_result = tokio::time::timeout(drain_join_timeout(call_timeout), async {
        while let Some(result) = active_tasks.join_next().await {
            let _ = result;
        }
    })
    .await;
    if active_drain_result.is_err() {
        force_teardown_remaining_sessions(Arc::clone(&caller), call_timeout, &counters).await;
        active_tasks.abort_all();
        while let Some(result) = active_tasks.join_next().await {
            let _ = result;
        }
        counters.failed.fetch_add(1, Ordering::Relaxed);
        counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
    }
}

fn active_slot_stop_deadline(
    settings: &SoakLoadSettings,
    slot: u64,
    started: std::time::Instant,
    elapsed: Duration,
    active_deadline: std::time::Instant,
) -> std::time::Instant {
    settings
        .next_slot_deactivation_secs(slot, elapsed)
        .map(|secs| started + Duration::from_secs(secs))
        .unwrap_or(active_deadline)
        .min(active_deadline)
}

fn setup_teardown_budget(call_timeout: Duration) -> Duration {
    call_timeout + call_timeout + Duration::from_secs(5)
}

fn drain_join_timeout(call_timeout: Duration) -> Duration {
    call_timeout + call_timeout + Duration::from_secs(60)
}

async fn force_teardown_remaining_sessions(
    caller: Arc<UnifiedCoordinator>,
    call_timeout: Duration,
    counters: &Arc<SoakCounters>,
) {
    let mut tasks = JoinSet::new();
    for session in caller.list_sessions().await {
        if session.state.is_final() {
            continue;
        }
        let handle = caller.session(&session.session_id);
        tasks.spawn(async move { handle.hangup_and_wait(Some(call_timeout)).await.is_ok() });
    }

    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(true) => {}
            _ => {
                counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

pub struct EndpointRetentionSampler {
    stop_tx: tokio::sync::watch::Sender<bool>,
    task: JoinHandle<EndpointRetentionSeries>,
}

pub struct MemoryDiagnosticSampler {
    stop_tx: tokio::sync::watch::Sender<bool>,
    task: JoinHandle<MemoryDiagnosticSeries>,
}

pub struct MemoryDiagnosticSeries {
    pub samples_path: PathBuf,
    pub sample_count: usize,
    pub allocator_diagnostics_enabled: bool,
    pub collect_at: String,
    pub collect_count: usize,
    pub first: Option<serde_json::Value>,
    pub last: Option<serde_json::Value>,
}

pub struct EndpointRetentionSeries {
    pub samples_path: PathBuf,
    pub sample_count: usize,
    pub max_retained_objects: u64,
    pub final_retained_objects: u64,
    pub first: Option<serde_json::Value>,
    pub last: Option<serde_json::Value>,
    pub final_sample: Option<serde_json::Value>,
}

impl EndpointRetentionSampler {
    pub fn start(
        role: &'static str,
        endpoint: Arc<UnifiedCoordinator>,
        interval: Duration,
    ) -> Self {
        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        let samples_path = diagnostic_sample_path(role, "retention");
        let task = tokio::spawn(async move {
            let started = std::time::Instant::now();
            let mut series = EndpointRetentionSeries::new(samples_path);
            let mut writer = series.open_writer();
            loop {
                let sample =
                    capture_endpoint_retention_sample(role, "periodic", started, &endpoint).await;
                series.record(role, sample, &mut writer);
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {}
                    _ = stop_rx.changed() => break,
                }
            }
            let sample =
                capture_endpoint_retention_sample(role, "after_drain", started, &endpoint).await;
            series.record(role, sample, &mut writer);
            writer.flush().expect("flush retention diagnostics JSONL");
            series
        });
        Self { stop_tx, task }
    }

    pub async fn stop(self) -> EndpointRetentionSeries {
        let _ = self.stop_tx.send(true);
        self.task.await.unwrap_or_else(|_| {
            EndpointRetentionSeries::new(diagnostic_sample_path("unknown", "retention"))
        })
    }
}

impl MemoryDiagnosticSampler {
    pub fn start(
        role: &'static str,
        settings: &SoakLoadSettings,
        interval: Duration,
    ) -> Option<Self> {
        if !memory_diagnostics_enabled() {
            return None;
        }
        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        let samples_path = diagnostic_sample_path(role, "memory_diag");
        let allocator_diagnostics_enabled = read_bool_env(ALLOCATOR_DIAGNOSTICS_ENV);
        let collect_at = MimallocCollectAt::from_env();
        let phase_starts = settings
            .active_phases
            .iter()
            .filter_map(|phase| (phase.start_secs > 0).then_some(phase.start_secs))
            .collect::<Vec<_>>();
        let task = tokio::spawn(async move {
            let started = std::time::Instant::now();
            let mut series = MemoryDiagnosticSeries::new(
                samples_path,
                allocator_diagnostics_enabled,
                collect_at.as_str().to_string(),
            );
            let mut writer = series.open_writer();
            let mut next_phase_collect = 0usize;
            loop {
                while collect_at.includes_phase()
                    && next_phase_collect < phase_starts.len()
                    && started.elapsed().as_secs() >= phase_starts[next_phase_collect]
                {
                    rvoip_infra_common::memory_diagnostics::collect_allocator(true);
                    series.collect_count += 1;
                    let sample = capture_memory_diagnostic_sample(
                        role,
                        "phase_collect",
                        started,
                        allocator_diagnostics_enabled,
                    );
                    series.record(sample, &mut writer);
                    next_phase_collect += 1;
                }

                let sample = capture_memory_diagnostic_sample(
                    role,
                    "periodic",
                    started,
                    allocator_diagnostics_enabled,
                );
                series.record(sample, &mut writer);
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {}
                    _ = stop_rx.changed() => break,
                }
            }
            if collect_at.includes_drain() {
                rvoip_infra_common::memory_diagnostics::collect_allocator(true);
                series.collect_count += 1;
            }
            let sample = capture_memory_diagnostic_sample(
                role,
                "after_drain",
                started,
                allocator_diagnostics_enabled,
            );
            series.record(sample, &mut writer);
            writer.flush().expect("flush memory diagnostics JSONL");
            series
        });
        Some(Self { stop_tx, task })
    }

    pub async fn stop(self) -> MemoryDiagnosticSeries {
        let _ = self.stop_tx.send(true);
        self.task.await.unwrap_or_else(|_| {
            MemoryDiagnosticSeries::new(
                diagnostic_sample_path("unknown", "memory_diag"),
                read_bool_env(ALLOCATOR_DIAGNOSTICS_ENV),
                MimallocCollectAt::from_env().as_str().to_string(),
            )
        })
    }
}

impl MemoryDiagnosticSeries {
    fn new(samples_path: PathBuf, allocator_diagnostics_enabled: bool, collect_at: String) -> Self {
        Self {
            samples_path,
            sample_count: 0,
            allocator_diagnostics_enabled,
            collect_at,
            collect_count: 0,
            first: None,
            last: None,
        }
    }

    fn open_writer(&self) -> BufWriter<File> {
        if let Some(parent) = self.samples_path.parent() {
            std::fs::create_dir_all(parent).expect("create memory diagnostics dir");
        }
        BufWriter::new(File::create(&self.samples_path).expect("create memory diagnostics JSONL"))
    }

    fn record(&mut self, sample: serde_json::Value, writer: &mut BufWriter<File>) {
        serde_json::to_writer(&mut *writer, &sample).expect("write memory diagnostics JSONL");
        writer
            .write_all(b"\n")
            .expect("write memory diagnostics newline");
        writer.flush().expect("flush memory diagnostics JSONL");

        self.sample_count += 1;
        let summary = memory_diagnostic_sample_summary(&sample);
        if self.first.is_none() {
            self.first = Some(summary.clone());
        }
        self.last = Some(summary);
    }
}

#[derive(Clone, Copy)]
enum MimallocCollectAt {
    Off,
    Phase,
    Drain,
    Both,
}

impl MimallocCollectAt {
    fn from_env() -> Self {
        match std::env::var(MIMALLOC_COLLECT_AT_ENV)
            .unwrap_or_else(|_| "off".to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "off" => Self::Off,
            "phase" => Self::Phase,
            "drain" => Self::Drain,
            "both" => Self::Both,
            other => panic!("{MIMALLOC_COLLECT_AT_ENV} must be off|phase|drain|both, got {other}"),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Phase => "phase",
            Self::Drain => "drain",
            Self::Both => "both",
        }
    }

    fn includes_phase(self) -> bool {
        matches!(self, Self::Phase | Self::Both)
    }

    fn includes_drain(self) -> bool {
        matches!(self, Self::Drain | Self::Both)
    }
}

pub fn memory_diagnostic_summary(series: Option<&MemoryDiagnosticSeries>) -> serde_json::Value {
    match series {
        Some(series) => json!({
            "enabled": true,
            "sample_count": series.sample_count,
            "samples_path": series.samples_path.display().to_string(),
            "allocator_diagnostics_enabled": series.allocator_diagnostics_enabled,
            "mimalloc_collect_at": series.collect_at,
            "mimalloc_collect_count": series.collect_count,
            "first": series.first.clone(),
            "last": series.last.clone(),
        }),
        None => json!({
            "enabled": false,
            "enable_env": MEMORY_DIAGNOSTICS_ENV,
            "allocator_enable_env": ALLOCATOR_DIAGNOSTICS_ENV,
            "mimalloc_collect_at_env": MIMALLOC_COLLECT_AT_ENV,
        }),
    }
}

fn capture_memory_diagnostic_sample(
    role: &'static str,
    label: &'static str,
    started: std::time::Instant,
    allocator_diagnostics_enabled: bool,
) -> serde_json::Value {
    let allocator = allocator_diagnostics_enabled
        .then(rvoip_infra_common::memory_diagnostics::allocator_snapshot);
    json!({
        "role": role,
        "label": label,
        "t_secs": round2(started.elapsed().as_secs_f64()),
        "memory": rvoip_infra_common::memory_diagnostics::snapshot(),
        "allocator": allocator,
    })
}

fn memory_diagnostic_sample_summary(sample: &serde_json::Value) -> serde_json::Value {
    let kinds = sample["memory"]["kinds"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut live_kinds = kinds
        .iter()
        .filter(|kind| {
            kind["live"].as_u64().unwrap_or(0) > 0 || kind["bytes_live"].as_u64().unwrap_or(0) > 0
        })
        .map(|kind| {
            json!({
                "kind": kind["kind"].clone(),
                "live": kind["live"].clone(),
                "bytes_live": kind["bytes_live"].clone(),
                "peak_live": kind["peak_live"].clone(),
                "peak_bytes": kind["peak_bytes"].clone(),
                "checkouts": kind["checkouts"].clone(),
                "returns": kind["returns"].clone(),
                "dropped_full": kind["dropped_full"].clone(),
            })
        })
        .collect::<Vec<_>>();
    let mut activity_kinds = kinds
        .iter()
        .filter(|kind| {
            kind["created"].as_u64().unwrap_or(0) > 0
                || kind["checkouts"].as_u64().unwrap_or(0) > 0
                || kind["dropped_full"].as_u64().unwrap_or(0) > 0
        })
        .map(|kind| {
            json!({
                "kind": kind["kind"].clone(),
                "created": kind["created"].clone(),
                "dropped": kind["dropped"].clone(),
                "bytes_created": kind["bytes_created"].clone(),
                "bytes_dropped": kind["bytes_dropped"].clone(),
                "peak_live": kind["peak_live"].clone(),
                "peak_bytes": kind["peak_bytes"].clone(),
                "checkouts": kind["checkouts"].clone(),
                "returns": kind["returns"].clone(),
                "dropped_full": kind["dropped_full"].clone(),
            })
        })
        .collect::<Vec<_>>();
    live_kinds.sort_by(|a, b| {
        b["bytes_live"]
            .as_u64()
            .cmp(&a["bytes_live"].as_u64())
            .then_with(|| a["kind"].as_str().cmp(&b["kind"].as_str()))
    });
    activity_kinds.sort_by(|a, b| {
        b["bytes_created"]
            .as_u64()
            .cmp(&a["bytes_created"].as_u64())
            .then_with(|| b["created"].as_u64().cmp(&a["created"].as_u64()))
            .then_with(|| b["checkouts"].as_u64().cmp(&a["checkouts"].as_u64()))
            .then_with(|| a["kind"].as_str().cmp(&b["kind"].as_str()))
    });
    if live_kinds.len() > 32 {
        live_kinds.truncate(32);
    }
    if activity_kinds.len() > 32 {
        activity_kinds.truncate(32);
    }
    json!({
        "label": sample["label"].clone(),
        "t_secs": sample["t_secs"].clone(),
        "live_kinds": live_kinds,
        "activity_kinds": activity_kinds,
        "allocator_active": sample["allocator"]["active_allocator"].clone(),
        "allocator_process": sample["allocator"]["process"].clone(),
        "allocator_unsupported_reason": sample["allocator"]["unsupported_reason"].clone(),
    })
}

impl EndpointRetentionSeries {
    fn new(samples_path: PathBuf) -> Self {
        Self {
            samples_path,
            sample_count: 0,
            max_retained_objects: 0,
            final_retained_objects: 0,
            first: None,
            last: None,
            final_sample: None,
        }
    }

    fn open_writer(&self) -> BufWriter<File> {
        if let Some(parent) = self.samples_path.parent() {
            std::fs::create_dir_all(parent).expect("create retention diagnostics dir");
        }
        BufWriter::new(
            File::create(&self.samples_path).expect("create retention diagnostics JSONL"),
        )
    }

    fn record(
        &mut self,
        role: &'static str,
        sample: serde_json::Value,
        writer: &mut BufWriter<File>,
    ) {
        serde_json::to_writer(&mut *writer, &sample).expect("write retention diagnostics JSONL");
        writer
            .write_all(b"\n")
            .expect("write retention diagnostics newline");
        writer.flush().expect("flush retention diagnostics JSONL");

        self.sample_count += 1;
        let retained = sample["retained_total"].as_u64().unwrap_or(0);
        self.max_retained_objects = self.max_retained_objects.max(retained);
        self.final_retained_objects = retained;

        let summary = endpoint_retention_sample_summary(&sample, role);
        if self.first.is_none() {
            self.first = Some(summary.clone());
        }
        self.last = Some(summary);
        self.final_sample = Some(sample);
    }
}

pub async fn capture_endpoint_retention_sample(
    role: &'static str,
    label: &'static str,
    started: std::time::Instant,
    endpoint: &Arc<UnifiedCoordinator>,
) -> serde_json::Value {
    let snapshot = endpoint.perf_diagnostic_snapshot().await;
    let retained = endpoint_retained_total(&snapshot) + endpoint_global_retained_total(&snapshot);
    json!({
        "role": role,
        "label": label,
        "t_secs": round2(started.elapsed().as_secs_f64()),
        "retained_total": retained,
        role: snapshot,
    })
}

pub fn endpoint_retention_summary(series: &EndpointRetentionSeries) -> serde_json::Value {
    json!({
        "sample_count": series.sample_count,
        "samples_path": series.samples_path.display().to_string(),
        "max_retained_objects": series.max_retained_objects,
        "final_retained_objects": series.final_retained_objects,
        "first": series.first.clone(),
        "last": series.last.clone(),
    })
}

fn endpoint_retention_sample_summary(
    sample: &serde_json::Value,
    role: &'static str,
) -> serde_json::Value {
    json!({
        "label": sample["label"].clone(),
        "t_secs": sample["t_secs"].clone(),
        "retained_total": sample["retained_total"].clone(),
        role: endpoint_summary(&sample[role]),
    })
}

pub fn endpoint_summary(snapshot: &serde_json::Value) -> serde_json::Value {
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

pub fn endpoint_retained_total(snapshot: &serde_json::Value) -> u64 {
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
        .map(|pointer| endpoint_metric(snapshot, pointer))
        .sum()
}

pub fn endpoint_global_retained_total(snapshot: &serde_json::Value) -> u64 {
    const POINTERS: &[&str] = &[
        "/sip_dialog_diagnostics/transaction_runner/active",
        "/sip_dialog_diagnostics/transaction_cleanup/in_flight",
    ];

    POINTERS
        .iter()
        .map(|pointer| endpoint_metric(snapshot, pointer))
        .sum()
}

pub fn endpoint_metric(snapshot: &serde_json::Value, pointer: &str) -> u64 {
    snapshot
        .pointer(pointer)
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
}

pub struct RssGrowthGate {
    pub effective_mb_per_hr: f64,
    pub source: &'static str,
    pub env_override_mb_per_hr: Option<f64>,
    pub caller_config_mb_per_hr: Option<f64>,
    pub receiver_config_mb_per_hr: Option<f64>,
}

impl RssGrowthGate {
    pub fn resolve(caller: &Config, receiver: &Config) -> Self {
        let env_override = read_positive_f64_env("RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR");
        let caller_config = caller.perf_max_rss_growth_mb_per_hr;
        let receiver_config = receiver.perf_max_rss_growth_mb_per_hr;

        let (effective, source) = if let Some(env) = env_override {
            (env, "env:RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR")
        } else {
            match (caller_config, receiver_config) {
                (Some(a), Some(b)) => (a.min(b), "config:strictest_endpoint"),
                (Some(a), None) => (a, "config:caller"),
                (None, Some(b)) => (b, "config:receiver"),
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
            caller_config_mb_per_hr: caller_config,
            receiver_config_mb_per_hr: receiver_config,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "effective_mb_per_hr": self.effective_mb_per_hr,
            "source": self.source,
            "env_override_mb_per_hr": self.env_override_mb_per_hr,
            "caller_config_mb_per_hr": self.caller_config_mb_per_hr,
            "receiver_config_mb_per_hr": self.receiver_config_mb_per_hr,
            "default_mb_per_hr": Config::DEFAULT_PERF_MAX_RSS_GROWTH_MB_PER_HR,
        })
    }
}

pub struct RssResultMetrics {
    pub full_growth_mb_per_hr: f64,
    pub sustained_growth_mb_per_hr: f64,
    pub post_drain_growth_mb_per_hr: f64,
    pub post_drain_sample_count: usize,
    pub gate_growth_mb_per_hr: f64,
    pub gate_window: &'static str,
    pub windows: Vec<serde_json::Value>,
}

pub fn rss_result_metrics(
    resources: &ResourceSummary,
    active_secs: f64,
    drain_secs: f64,
) -> RssResultMetrics {
    let full_growth_mb_per_hr = resources.rss_growth_mb_per_min * 60.0;
    let sustained_growth_mb_per_hr = resources.rss_tail_growth_mb_per_min * 60.0;
    let post_drain_samples: Vec<ResourceSample> = resources
        .samples
        .iter()
        .filter(|sample| sample.t_secs >= active_secs)
        .cloned()
        .collect();
    let post_drain_growth_mb_per_hr = rss_growth_mb_per_min(&post_drain_samples) * 60.0;
    let (gate_growth_mb_per_hr, gate_window) = if post_drain_samples.len() >= 2 {
        (post_drain_growth_mb_per_hr, "post_drain")
    } else {
        (sustained_growth_mb_per_hr, "tail")
    };
    let windows = rss_window_summaries(&resources.samples, active_secs, drain_secs);

    RssResultMetrics {
        full_growth_mb_per_hr,
        sustained_growth_mb_per_hr,
        post_drain_growth_mb_per_hr,
        post_drain_sample_count: post_drain_samples.len(),
        gate_growth_mb_per_hr,
        gate_window,
        windows,
    }
}

pub fn rss_growth_mb_per_min(samples: &[ResourceSample]) -> f64 {
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

pub fn rss_window_summaries(
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

pub fn cycling_hold_duration(slot: u64, cycle: u64, min_secs: u64, max_secs: u64) -> Duration {
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

fn parse_active_phases_env() -> Option<Vec<SoakActivePhase>> {
    let raw = match std::env::var(ACTIVE_PHASES_ENV) {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return None,
        Err(err) => panic!("{ACTIVE_PHASES_ENV} could not be read: {err}"),
    };
    let mut start_secs = 0u64;
    let mut phases = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (active_raw, duration_raw) = part.split_once(':').unwrap_or_else(|| {
            panic!("{ACTIVE_PHASES_ENV} entries must be active_calls:duration_secs, got {part:?}")
        });
        let active_calls: u64 = active_raw.trim().parse().unwrap_or_else(|_| {
            panic!(
                "{ACTIVE_PHASES_ENV} active call count must be a positive u64, got {active_raw:?}"
            )
        });
        let duration_secs: u64 = duration_raw.trim().parse().unwrap_or_else(|_| {
            panic!("{ACTIVE_PHASES_ENV} duration must be a positive u64, got {duration_raw:?}")
        });
        assert!(
            active_calls > 0,
            "{ACTIVE_PHASES_ENV} active call count must be greater than 0"
        );
        assert!(
            duration_secs > 0,
            "{ACTIVE_PHASES_ENV} phase duration must be greater than 0"
        );
        phases.push(SoakActivePhase {
            start_secs,
            duration_secs,
            active_calls,
        });
        start_secs = start_secs
            .checked_add(duration_secs)
            .unwrap_or_else(|| panic!("{ACTIVE_PHASES_ENV} total duration overflowed u64"));
    }
    assert!(
        !phases.is_empty(),
        "{ACTIVE_PHASES_ENV} must include at least one active_calls:duration_secs entry"
    );
    Some(phases)
}

pub fn read_positive_f64_env(name: &str) -> Option<f64> {
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

pub fn read_nonnegative_f64_env(name: &str) -> Option<f64> {
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

pub fn read_nonnegative_u64_env(name: &str) -> Option<u64> {
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

pub fn read_positive_usize_env(name: &str) -> Option<usize> {
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

pub fn read_required_u16_env(name: &str) -> u16 {
    let raw = std::env::var(name).unwrap_or_else(|err| panic!("{name} must be set: {err}"));
    raw.parse()
        .unwrap_or_else(|_| panic!("{name} must be a valid u16 port, got {raw:?}"))
}

fn read_bool_env(name: &str) -> bool {
    let raw = match std::env::var(name) {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return false,
        Err(err) => panic!("{name} could not be read: {err}"),
    };
    match raw.as_str() {
        "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON" => true,
        "0" | "false" | "FALSE" | "no" | "NO" | "off" | "OFF" => false,
        _ => panic!("{name} must be boolean-like, got {raw:?}"),
    }
}

pub fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

pub fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

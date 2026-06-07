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

#[derive(Clone, Copy)]
pub struct SoakLoadSettings {
    pub duration_secs: u64,
    pub soak_cps: f64,
    pub active_calls: u64,
    pub min_hold_secs: u64,
    pub max_hold_secs: u64,
    pub call_timeout: Duration,
}

impl SoakLoadSettings {
    pub fn from_env() -> Self {
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

        Self {
            duration_secs,
            soak_cps,
            active_calls,
            min_hold_secs,
            max_hold_secs,
            call_timeout,
        }
    }

    pub fn total(self) -> Duration {
        Duration::from_secs(self.duration_secs)
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
    let started = std::time::Instant::now();
    let active_deadline = started + total;
    let mut active_tasks = JoinSet::<()>::new();
    for slot in 0..settings.active_calls {
        let caller = Arc::clone(&caller);
        let from = from.clone();
        let target_uri = target_uri.clone();
        let counters = Arc::clone(&counters);
        let setup_hist = Arc::clone(&setup_hist);
        let first_minute_hist = Arc::clone(&first_minute_hist);
        let last_minute_hist = Arc::clone(&last_minute_hist);
        active_tasks.spawn(async move {
            let mut cycle = 0u64;
            loop {
                if std::time::Instant::now() >= active_deadline {
                    break;
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
                let hold_deadline = (std::time::Instant::now() + hold).min(active_deadline);
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
                if handle
                    .wait_for_answered(Some(settings.call_timeout))
                    .await
                    .is_err()
                {
                    counters.failed.fetch_add(1, Ordering::Relaxed);
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
                if handle
                    .hangup_and_wait(Some(settings.call_timeout))
                    .await
                    .is_ok()
                {
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

    let drain_result =
        tokio::time::timeout(settings.call_timeout + Duration::from_secs(30), async {
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

    let active_drain_result =
        tokio::time::timeout(settings.call_timeout + Duration::from_secs(30), async {
            while let Some(result) = active_tasks.join_next().await {
                let _ = result;
            }
        })
        .await;
    if active_drain_result.is_err() {
        active_tasks.abort_all();
        while let Some(result) = active_tasks.join_next().await {
            let _ = result;
        }
        counters.failed.fetch_add(1, Ordering::Relaxed);
        counters.teardown_failed.fetch_add(1, Ordering::Relaxed);
    }
}

pub struct EndpointRetentionSampler {
    stop_tx: tokio::sync::watch::Sender<bool>,
    task: JoinHandle<Vec<serde_json::Value>>,
}

impl EndpointRetentionSampler {
    pub fn start(
        role: &'static str,
        endpoint: Arc<UnifiedCoordinator>,
        interval: Duration,
    ) -> Self {
        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(async move {
            let started = std::time::Instant::now();
            let mut samples = Vec::new();
            loop {
                samples.push(
                    capture_endpoint_retention_sample(role, "periodic", started, &endpoint).await,
                );
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {}
                    _ = stop_rx.changed() => break,
                }
            }
            samples.push(
                capture_endpoint_retention_sample(role, "after_drain", started, &endpoint).await,
            );
            samples
        });
        Self { stop_tx, task }
    }

    pub async fn stop(self) -> Vec<serde_json::Value> {
        let _ = self.stop_tx.send(true);
        self.task.await.unwrap_or_default()
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

pub fn endpoint_retention_summary(
    samples: &[serde_json::Value],
    final_retained: u64,
    role: &'static str,
) -> serde_json::Value {
    let max_retained_objects = samples
        .iter()
        .filter_map(|sample| sample["retained_total"].as_u64())
        .max()
        .unwrap_or(0);
    json!({
        "sample_count": samples.len(),
        "max_retained_objects": max_retained_objects,
        "final_retained_objects": final_retained,
        "first": samples.first().map(|sample| endpoint_retention_sample_summary(sample, role)),
        "last": samples.last().map(|sample| endpoint_retention_sample_summary(sample, role)),
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

pub fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

pub fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

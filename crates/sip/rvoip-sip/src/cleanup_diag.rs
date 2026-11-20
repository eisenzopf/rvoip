//! Perf-only cleanup-stage diagnostics.
//!
//! This module is intentionally process-global and config-gated. The SIPp
//! cleanup backlog investigation needs low-friction counters from several
//! subsystems without changing public call semantics or threading a metrics
//! object through the session stack.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const ENABLE_OFF: u8 = 1;
const ENABLE_ON: u8 = 2;

const BUCKET_UPPER_US: [u64; 18] = [
    10,
    25,
    50,
    100,
    250,
    500,
    1_000,
    2_500,
    5_000,
    10_000,
    25_000,
    50_000,
    100_000,
    250_000,
    500_000,
    1_000_000,
    2_500_000,
    u64::MAX,
];

/// Cleanup and high-rate call-progress subpaths measured by perf diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupStage {
    DialogCleanup,
    MediaCleanup,
    SessionStoreRemoval,
    TerminalEventPublish,
    TerminalRelease,
    StateMachineEventPublish,
    SessionEventDispatch,
    TimerTaskShutdown,
    IncomingCallSetup,
    CallbackIncomingDispatch,
    CallbackAcceptCall,
    CallbackEventDispatch,
    StateMachineIncomingCall,
    StateMachineAcceptCall,
    StateMachineTerminalEvent,
    StateMachineOtherEvent,
    ActionGenerateLocalSdp,
    ActionNegotiateSdpUas,
    ActionSend200Ok,
    ByeReceivedHandling,
}

impl CleanupStage {
    pub const ALL: [CleanupStage; 20] = [
        CleanupStage::DialogCleanup,
        CleanupStage::MediaCleanup,
        CleanupStage::SessionStoreRemoval,
        CleanupStage::TerminalEventPublish,
        CleanupStage::TerminalRelease,
        CleanupStage::StateMachineEventPublish,
        CleanupStage::SessionEventDispatch,
        CleanupStage::TimerTaskShutdown,
        CleanupStage::IncomingCallSetup,
        CleanupStage::CallbackIncomingDispatch,
        CleanupStage::CallbackAcceptCall,
        CleanupStage::CallbackEventDispatch,
        CleanupStage::StateMachineIncomingCall,
        CleanupStage::StateMachineAcceptCall,
        CleanupStage::StateMachineTerminalEvent,
        CleanupStage::StateMachineOtherEvent,
        CleanupStage::ActionGenerateLocalSdp,
        CleanupStage::ActionNegotiateSdpUas,
        CleanupStage::ActionSend200Ok,
        CleanupStage::ByeReceivedHandling,
    ];

    fn as_index(self) -> usize {
        match self {
            CleanupStage::DialogCleanup => 0,
            CleanupStage::MediaCleanup => 1,
            CleanupStage::SessionStoreRemoval => 2,
            CleanupStage::TerminalEventPublish => 3,
            CleanupStage::TerminalRelease => 4,
            CleanupStage::StateMachineEventPublish => 5,
            CleanupStage::SessionEventDispatch => 6,
            CleanupStage::TimerTaskShutdown => 7,
            CleanupStage::IncomingCallSetup => 8,
            CleanupStage::CallbackIncomingDispatch => 9,
            CleanupStage::CallbackAcceptCall => 10,
            CleanupStage::CallbackEventDispatch => 11,
            CleanupStage::StateMachineIncomingCall => 12,
            CleanupStage::StateMachineAcceptCall => 13,
            CleanupStage::StateMachineTerminalEvent => 14,
            CleanupStage::StateMachineOtherEvent => 15,
            CleanupStage::ActionGenerateLocalSdp => 16,
            CleanupStage::ActionNegotiateSdpUas => 17,
            CleanupStage::ActionSend200Ok => 18,
            CleanupStage::ByeReceivedHandling => 19,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CleanupStage::DialogCleanup => "dialog_cleanup",
            CleanupStage::MediaCleanup => "media_cleanup",
            CleanupStage::SessionStoreRemoval => "session_store_removal",
            CleanupStage::TerminalEventPublish => "terminal_event_publish",
            CleanupStage::TerminalRelease => "terminal_release",
            CleanupStage::StateMachineEventPublish => "state_machine_event_publish",
            CleanupStage::SessionEventDispatch => "session_event_dispatch",
            CleanupStage::TimerTaskShutdown => "timer_task_shutdown",
            CleanupStage::IncomingCallSetup => "incoming_call_setup",
            CleanupStage::CallbackIncomingDispatch => "callback_incoming_dispatch",
            CleanupStage::CallbackAcceptCall => "callback_accept_call",
            CleanupStage::CallbackEventDispatch => "callback_event_dispatch",
            CleanupStage::StateMachineIncomingCall => "state_machine_incoming_call",
            CleanupStage::StateMachineAcceptCall => "state_machine_accept_call",
            CleanupStage::StateMachineTerminalEvent => "state_machine_terminal_event",
            CleanupStage::StateMachineOtherEvent => "state_machine_other_event",
            CleanupStage::ActionGenerateLocalSdp => "action_generate_local_sdp",
            CleanupStage::ActionNegotiateSdpUas => "action_negotiate_sdp_uas",
            CleanupStage::ActionSend200Ok => "action_send_200_ok",
            CleanupStage::ByeReceivedHandling => "bye_received_handling",
        }
    }
}

/// Point-in-time counters for a cleanup stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupStageSnapshot {
    pub stage: CleanupStage,
    pub started: u64,
    pub completed: u64,
    pub failed: u64,
    pub active: u64,
    pub avg_us: u64,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
    pub max_queue_depth: u64,
}

/// Point-in-time cleanup diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupDiagSnapshot {
    pub enabled: bool,
    pub active_total: u64,
    pub setup_teardown_watchdog_armed: u64,
    pub setup_teardown_watchdog_disarmed: u64,
    pub setup_teardown_watchdog_fired: u64,
    pub setup_teardown_watchdog_transition_failed: u64,
    pub setup_teardown_watchdog_release_completed: u64,
    pub setup_teardown_watchdog_release_failed: u64,
    pub session_event_dispatch_saturated: u64,
    pub session_event_dispatch_dropped: u64,
    pub session_event_dispatch_closed: u64,
    pub session_event_publication_failed: u64,
    pub session_event_publication_timed_out: u64,
    pub session_event_dispatch_shutdown_timeouts: u64,
    pub session_event_dispatch_aborted_workers: u64,
    pub stages: Vec<CleanupStageSnapshot>,
}

struct StageMetrics {
    started: AtomicU64,
    completed: AtomicU64,
    failed: AtomicU64,
    active: AtomicU64,
    sum_us: AtomicU64,
    max_us: AtomicU64,
    max_queue_depth: AtomicU64,
    buckets: Vec<AtomicU64>,
}

impl StageMetrics {
    fn new() -> Self {
        Self {
            started: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            active: AtomicU64::new(0),
            sum_us: AtomicU64::new(0),
            max_us: AtomicU64::new(0),
            max_queue_depth: AtomicU64::new(0),
            buckets: (0..BUCKET_UPPER_US.len())
                .map(|_| AtomicU64::new(0))
                .collect(),
        }
    }

    #[cfg(test)]
    fn reset(&self) {
        self.started.store(0, Ordering::Relaxed);
        self.completed.store(0, Ordering::Relaxed);
        self.failed.store(0, Ordering::Relaxed);
        self.active.store(0, Ordering::Relaxed);
        self.sum_us.store(0, Ordering::Relaxed);
        self.max_us.store(0, Ordering::Relaxed);
        self.max_queue_depth.store(0, Ordering::Relaxed);
        for bucket in &self.buckets {
            bucket.store(0, Ordering::Relaxed);
        }
    }
}

/// RAII timer for one cleanup-stage operation.
pub struct CleanupStageGuard {
    stage: CleanupStage,
    start: Option<Instant>,
    session_id: Option<String>,
    finished: bool,
}

impl CleanupStageGuard {
    pub fn finish_success(mut self) {
        self.finish(true);
    }

    pub fn finish_failure(mut self) {
        self.finish(false);
    }

    fn finish(&mut self, success: bool) {
        if self.finished {
            return;
        }
        self.finished = true;
        let Some(start) = self.start.take() else {
            return;
        };
        let duration_us = micros_u64(start.elapsed().as_micros());
        record_stage_finish(self.stage, duration_us, success);
        if per_operation_logs_enabled() {
            tracing::info!(
                target: "rvoip_sip::cleanup_diag",
                "cleanup_stage_end stage={} session={} ok={} duration_us={} ts_ms={}",
                self.stage.as_str(),
                self.session_id.as_deref().unwrap_or("-"),
                success,
                duration_us,
                unix_ms()
            );
        }
    }
}

impl Drop for CleanupStageGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.finish(false);
        }
    }
}

static ENABLE_OVERRIDE: AtomicU8 = AtomicU8::new(ENABLE_OFF);
static EVENT_LOGS_OVERRIDE: AtomicU8 = AtomicU8::new(ENABLE_OFF);
static METRICS: OnceLock<Vec<StageMetrics>> = OnceLock::new();
static SETUP_TEARDOWN_WATCHDOG_ARMED: AtomicU64 = AtomicU64::new(0);
static SETUP_TEARDOWN_WATCHDOG_DISARMED: AtomicU64 = AtomicU64::new(0);
static SETUP_TEARDOWN_WATCHDOG_FIRED: AtomicU64 = AtomicU64::new(0);
static SETUP_TEARDOWN_WATCHDOG_TRANSITION_FAILED: AtomicU64 = AtomicU64::new(0);
static SETUP_TEARDOWN_WATCHDOG_RELEASE_COMPLETED: AtomicU64 = AtomicU64::new(0);
static SETUP_TEARDOWN_WATCHDOG_RELEASE_FAILED: AtomicU64 = AtomicU64::new(0);
static SESSION_EVENT_DISPATCH_SATURATED: AtomicU64 = AtomicU64::new(0);
static SESSION_EVENT_DISPATCH_DROPPED: AtomicU64 = AtomicU64::new(0);
static SESSION_EVENT_DISPATCH_CLOSED: AtomicU64 = AtomicU64::new(0);
static SESSION_EVENT_PUBLICATION_FAILED: AtomicU64 = AtomicU64::new(0);
static SESSION_EVENT_PUBLICATION_TIMED_OUT: AtomicU64 = AtomicU64::new(0);
static SESSION_EVENT_DISPATCH_SHUTDOWN_TIMEOUTS: AtomicU64 = AtomicU64::new(0);
static SESSION_EVENT_DISPATCH_ABORTED_WORKERS: AtomicU64 = AtomicU64::new(0);

/// Enable or disable cleanup diagnostics for this process.
pub fn set_enabled(enabled: bool) {
    ENABLE_OVERRIDE.store(
        if enabled { ENABLE_ON } else { ENABLE_OFF },
        Ordering::Relaxed,
    );
}

/// Enable or disable per-operation cleanup diagnostic event logs.
pub fn set_event_logs_enabled(enabled: bool) {
    EVENT_LOGS_OVERRIDE.store(
        if enabled { ENABLE_ON } else { ENABLE_OFF },
        Ordering::Relaxed,
    );
}

/// Whether cleanup diagnostics are enabled for this process.
pub fn enabled() -> bool {
    ENABLE_OVERRIDE.load(Ordering::Relaxed) == ENABLE_ON
}

/// Start timing a cleanup stage. Returns an inert guard when diagnostics are off.
pub fn stage_guard(stage: CleanupStage, session_id: impl ToString) -> CleanupStageGuard {
    if !enabled() {
        return CleanupStageGuard {
            stage,
            start: None,
            session_id: None,
            finished: true,
        };
    }

    let session_id = session_id.to_string();
    let metric = metric(stage);
    metric.started.fetch_add(1, Ordering::Relaxed);
    metric.active.fetch_add(1, Ordering::Relaxed);
    if per_operation_logs_enabled() {
        tracing::info!(
            target: "rvoip_sip::cleanup_diag",
            "cleanup_stage_start stage={} session={} ts_ms={}",
            stage.as_str(),
            session_id,
            unix_ms()
        );
    }

    CleanupStageGuard {
        stage,
        start: Some(Instant::now()),
        session_id: Some(session_id),
        finished: false,
    }
}

/// Record current queue depth for stages backed by a bounded channel.
pub fn record_queue_depth(stage: CleanupStage, depth: usize) {
    if !enabled() {
        return;
    }
    update_max(&metric(stage).max_queue_depth, depth as u64);
}

/// Read all cleanup diagnostic counters.
pub fn snapshot() -> CleanupDiagSnapshot {
    let stages = CleanupStage::ALL
        .iter()
        .copied()
        .map(stage_snapshot)
        .collect::<Vec<_>>();
    let active_total = stages.iter().map(|s| s.active).sum();
    CleanupDiagSnapshot {
        enabled: enabled(),
        active_total,
        setup_teardown_watchdog_armed: SETUP_TEARDOWN_WATCHDOG_ARMED.load(Ordering::Relaxed),
        setup_teardown_watchdog_disarmed: SETUP_TEARDOWN_WATCHDOG_DISARMED.load(Ordering::Relaxed),
        setup_teardown_watchdog_fired: SETUP_TEARDOWN_WATCHDOG_FIRED.load(Ordering::Relaxed),
        setup_teardown_watchdog_transition_failed: SETUP_TEARDOWN_WATCHDOG_TRANSITION_FAILED
            .load(Ordering::Relaxed),
        setup_teardown_watchdog_release_completed: SETUP_TEARDOWN_WATCHDOG_RELEASE_COMPLETED
            .load(Ordering::Relaxed),
        setup_teardown_watchdog_release_failed: SETUP_TEARDOWN_WATCHDOG_RELEASE_FAILED
            .load(Ordering::Relaxed),
        session_event_dispatch_saturated: SESSION_EVENT_DISPATCH_SATURATED.load(Ordering::Relaxed),
        session_event_dispatch_dropped: SESSION_EVENT_DISPATCH_DROPPED.load(Ordering::Relaxed),
        session_event_dispatch_closed: SESSION_EVENT_DISPATCH_CLOSED.load(Ordering::Relaxed),
        session_event_publication_failed: SESSION_EVENT_PUBLICATION_FAILED.load(Ordering::Relaxed),
        session_event_publication_timed_out: SESSION_EVENT_PUBLICATION_TIMED_OUT
            .load(Ordering::Relaxed),
        session_event_dispatch_shutdown_timeouts: SESSION_EVENT_DISPATCH_SHUTDOWN_TIMEOUTS
            .load(Ordering::Relaxed),
        session_event_dispatch_aborted_workers: SESSION_EVENT_DISPATCH_ABORTED_WORKERS
            .load(Ordering::Relaxed),
        stages,
    }
}

/// Render a compact single-line summary suitable for periodic perf logs.
pub fn format_summary(snapshot: &CleanupDiagSnapshot) -> String {
    let mut out = format!("[cleanup_diag] active_total={}", snapshot.active_total);
    let watchdog_total = snapshot
        .setup_teardown_watchdog_armed
        .saturating_add(snapshot.setup_teardown_watchdog_disarmed)
        .saturating_add(snapshot.setup_teardown_watchdog_fired)
        .saturating_add(snapshot.setup_teardown_watchdog_transition_failed)
        .saturating_add(snapshot.setup_teardown_watchdog_release_completed)
        .saturating_add(snapshot.setup_teardown_watchdog_release_failed);
    if watchdog_total > 0 {
        out.push_str(&format!(
            " setup_teardown_watchdog:armed={} disarmed={} fired={} transition_failed={} release_done={} release_failed={}",
            snapshot.setup_teardown_watchdog_armed,
            snapshot.setup_teardown_watchdog_disarmed,
            snapshot.setup_teardown_watchdog_fired,
            snapshot.setup_teardown_watchdog_transition_failed,
            snapshot.setup_teardown_watchdog_release_completed,
            snapshot.setup_teardown_watchdog_release_failed,
        ));
    }
    let session_event_total = snapshot
        .session_event_dispatch_saturated
        .saturating_add(snapshot.session_event_dispatch_dropped)
        .saturating_add(snapshot.session_event_dispatch_closed)
        .saturating_add(snapshot.session_event_publication_failed)
        .saturating_add(snapshot.session_event_publication_timed_out)
        .saturating_add(snapshot.session_event_dispatch_shutdown_timeouts)
        .saturating_add(snapshot.session_event_dispatch_aborted_workers);
    if session_event_total > 0 {
        out.push_str(&format!(
            " session_event_dispatch:saturated={} dropped={} closed={} publish_failed={} publish_timed_out={} shutdown_timeouts={} aborted_workers={}",
            snapshot.session_event_dispatch_saturated,
            snapshot.session_event_dispatch_dropped,
            snapshot.session_event_dispatch_closed,
            snapshot.session_event_publication_failed,
            snapshot.session_event_publication_timed_out,
            snapshot.session_event_dispatch_shutdown_timeouts,
            snapshot.session_event_dispatch_aborted_workers,
        ));
    }
    for stage in &snapshot.stages {
        if stage.started == 0 && stage.max_queue_depth == 0 {
            continue;
        }
        out.push_str(&format!(
            " {}:started={} done={} fail={} active={} p50_us={} p95_us={} p99_us={} max_us={} qmax={}",
            stage.stage.as_str(),
            stage.started,
            stage.completed,
            stage.failed,
            stage.active,
            stage.p50_us,
            stage.p95_us,
            stage.p99_us,
            stage.max_us,
            stage.max_queue_depth,
        ));
    }
    out
}

fn stage_snapshot(stage: CleanupStage) -> CleanupStageSnapshot {
    let metric = metric(stage);
    let completed = metric.completed.load(Ordering::Relaxed);
    let failed = metric.failed.load(Ordering::Relaxed);
    let observed = completed.saturating_add(failed);
    let sum_us = metric.sum_us.load(Ordering::Relaxed);
    CleanupStageSnapshot {
        stage,
        started: metric.started.load(Ordering::Relaxed),
        completed,
        failed,
        active: metric.active.load(Ordering::Relaxed),
        avg_us: sum_us.checked_div(observed).unwrap_or(0),
        p50_us: percentile_us(metric, observed, 50),
        p95_us: percentile_us(metric, observed, 95),
        p99_us: percentile_us(metric, observed, 99),
        max_us: metric.max_us.load(Ordering::Relaxed),
        max_queue_depth: metric.max_queue_depth.load(Ordering::Relaxed),
    }
}

fn percentile_us(metric: &StageMetrics, observed: u64, percentile: u64) -> u64 {
    if observed == 0 {
        return 0;
    }
    let rank = observed.saturating_mul(percentile).saturating_add(99) / 100;
    let mut seen = 0;
    for (idx, bucket) in metric.buckets.iter().enumerate() {
        seen += bucket.load(Ordering::Relaxed);
        if seen >= rank {
            return BUCKET_UPPER_US[idx];
        }
    }
    metric.max_us.load(Ordering::Relaxed)
}

fn record_stage_finish(stage: CleanupStage, duration_us: u64, success: bool) {
    let metric = metric(stage);
    metric.active.fetch_sub(1, Ordering::Relaxed);
    if success {
        metric.completed.fetch_add(1, Ordering::Relaxed);
    } else {
        metric.failed.fetch_add(1, Ordering::Relaxed);
    }
    metric.sum_us.fetch_add(duration_us, Ordering::Relaxed);
    update_max(&metric.max_us, duration_us);
    metric.buckets[bucket_index(duration_us)].fetch_add(1, Ordering::Relaxed);
}

fn bucket_index(duration_us: u64) -> usize {
    BUCKET_UPPER_US
        .iter()
        .position(|upper| duration_us <= *upper)
        .unwrap_or(BUCKET_UPPER_US.len() - 1)
}

fn metric(stage: CleanupStage) -> &'static StageMetrics {
    &metrics()[stage.as_index()]
}

pub(crate) fn record_setup_teardown_watchdog_armed() {
    SETUP_TEARDOWN_WATCHDOG_ARMED.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_setup_teardown_watchdog_disarmed() {
    SETUP_TEARDOWN_WATCHDOG_DISARMED.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_setup_teardown_watchdog_fired() {
    SETUP_TEARDOWN_WATCHDOG_FIRED.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_setup_teardown_watchdog_transition_failed() {
    SETUP_TEARDOWN_WATCHDOG_TRANSITION_FAILED.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_setup_teardown_watchdog_release_completed() {
    SETUP_TEARDOWN_WATCHDOG_RELEASE_COMPLETED.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_setup_teardown_watchdog_release_failed() {
    SETUP_TEARDOWN_WATCHDOG_RELEASE_FAILED.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_session_event_dispatch_saturated() {
    SESSION_EVENT_DISPATCH_SATURATED.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_session_event_dispatch_dropped() {
    SESSION_EVENT_DISPATCH_DROPPED.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_session_event_dispatch_dropped_by(count: usize) {
    SESSION_EVENT_DISPATCH_DROPPED.fetch_add(count as u64, Ordering::Relaxed);
}

pub(crate) fn record_session_event_dispatch_closed() {
    SESSION_EVENT_DISPATCH_CLOSED.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_session_event_publication_failed() {
    SESSION_EVENT_PUBLICATION_FAILED.fetch_add(1, Ordering::Relaxed);
}

#[allow(dead_code)]
pub(crate) fn record_session_event_publication_timed_out() {
    SESSION_EVENT_PUBLICATION_TIMED_OUT.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_session_event_dispatch_shutdown_timeout() {
    SESSION_EVENT_DISPATCH_SHUTDOWN_TIMEOUTS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_session_event_dispatch_aborted_workers(count: usize) {
    SESSION_EVENT_DISPATCH_ABORTED_WORKERS.fetch_add(count as u64, Ordering::Relaxed);
}

fn metrics() -> &'static [StageMetrics] {
    METRICS
        .get_or_init(|| {
            CleanupStage::ALL
                .iter()
                .map(|_| StageMetrics::new())
                .collect()
        })
        .as_slice()
}

fn update_max(slot: &AtomicU64, value: u64) {
    let mut current = slot.load(Ordering::Relaxed);
    while value > current {
        match slot.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(next) => current = next,
        }
    }
}

fn per_operation_logs_enabled() -> bool {
    enabled() && EVENT_LOGS_OVERRIDE.load(Ordering::Relaxed) == ENABLE_ON
}

fn micros_u64(value: u128) -> u64 {
    value.min(u64::MAX as u128) as u64
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
pub(crate) fn set_enabled_for_tests(enabled: bool) {
    set_enabled(enabled);
    set_event_logs_enabled(false);
}

#[cfg(test)]
pub(crate) fn reset_for_tests() {
    for metric in metrics() {
        metric.reset();
    }
    SETUP_TEARDOWN_WATCHDOG_ARMED.store(0, Ordering::Relaxed);
    SETUP_TEARDOWN_WATCHDOG_DISARMED.store(0, Ordering::Relaxed);
    SETUP_TEARDOWN_WATCHDOG_FIRED.store(0, Ordering::Relaxed);
    SETUP_TEARDOWN_WATCHDOG_TRANSITION_FAILED.store(0, Ordering::Relaxed);
    SETUP_TEARDOWN_WATCHDOG_RELEASE_COMPLETED.store(0, Ordering::Relaxed);
    SETUP_TEARDOWN_WATCHDOG_RELEASE_FAILED.store(0, Ordering::Relaxed);
    SESSION_EVENT_DISPATCH_SATURATED.store(0, Ordering::Relaxed);
    SESSION_EVENT_DISPATCH_DROPPED.store(0, Ordering::Relaxed);
    SESSION_EVENT_DISPATCH_CLOSED.store(0, Ordering::Relaxed);
    SESSION_EVENT_PUBLICATION_FAILED.store(0, Ordering::Relaxed);
    SESSION_EVENT_PUBLICATION_TIMED_OUT.store(0, Ordering::Relaxed);
    SESSION_EVENT_DISPATCH_SHUTDOWN_TIMEOUTS.store(0, Ordering::Relaxed);
    SESSION_EVENT_DISPATCH_ABORTED_WORKERS.store(0, Ordering::Relaxed);
}

#[cfg(test)]
pub(crate) fn record_duration_for_tests(stage: CleanupStage, duration_us: u64, success: bool) {
    metric(stage).started.fetch_add(1, Ordering::Relaxed);
    metric(stage).active.fetch_add(1, Ordering::Relaxed);
    record_stage_finish(stage, duration_us, success);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    fn test_lock() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap()
    }

    #[test]
    fn guard_records_successful_stage() {
        let _lock = test_lock();
        set_enabled_for_tests(true);
        reset_for_tests();

        stage_guard(CleanupStage::MediaCleanup, "s1").finish_success();

        let media = snapshot()
            .stages
            .into_iter()
            .find(|stage| stage.stage == CleanupStage::MediaCleanup)
            .unwrap();
        assert_eq!(media.started, 1);
        assert_eq!(media.completed, 1);
        assert_eq!(media.failed, 0);
        assert_eq!(media.active, 0);
    }

    #[test]
    fn dropped_guard_records_failure() {
        let _lock = test_lock();
        set_enabled_for_tests(true);
        reset_for_tests();

        let _guard = stage_guard(CleanupStage::DialogCleanup, "s2");
        drop(_guard);

        let dialog = snapshot()
            .stages
            .into_iter()
            .find(|stage| stage.stage == CleanupStage::DialogCleanup)
            .unwrap();
        assert_eq!(dialog.started, 1);
        assert_eq!(dialog.completed, 0);
        assert_eq!(dialog.failed, 1);
        assert_eq!(dialog.active, 0);
    }

    #[test]
    fn histogram_and_queue_depth_are_reported() {
        let _lock = test_lock();
        set_enabled_for_tests(true);
        reset_for_tests();

        record_duration_for_tests(CleanupStage::SessionStoreRemoval, 1_200, true);
        record_duration_for_tests(CleanupStage::SessionStoreRemoval, 40_000, true);
        record_queue_depth(CleanupStage::SessionEventDispatch, 10);
        record_queue_depth(CleanupStage::SessionEventDispatch, 4);

        let snap = snapshot();
        let store = snap
            .stages
            .iter()
            .find(|stage| stage.stage == CleanupStage::SessionStoreRemoval)
            .unwrap();
        assert_eq!(store.completed, 2);
        assert_eq!(store.p50_us, 2_500);
        assert_eq!(store.p95_us, 50_000);

        let dispatch = snap
            .stages
            .iter()
            .find(|stage| stage.stage == CleanupStage::SessionEventDispatch)
            .unwrap();
        assert_eq!(dispatch.max_queue_depth, 10);
    }

    #[test]
    fn disabled_guard_is_noop() {
        let _lock = test_lock();
        set_enabled_for_tests(false);
        reset_for_tests();

        stage_guard(CleanupStage::MediaCleanup, "s3").finish_success();

        let media = snapshot()
            .stages
            .into_iter()
            .find(|stage| stage.stage == CleanupStage::MediaCleanup)
            .unwrap();
        assert_eq!(media.started, 0);
    }

    #[test]
    fn setup_teardown_watchdog_counters_are_reported() {
        let _lock = test_lock();
        reset_for_tests();

        record_setup_teardown_watchdog_armed();
        record_setup_teardown_watchdog_disarmed();
        record_setup_teardown_watchdog_fired();
        record_setup_teardown_watchdog_transition_failed();
        record_setup_teardown_watchdog_release_completed();
        record_setup_teardown_watchdog_release_failed();

        let snap = snapshot();
        assert_eq!(snap.setup_teardown_watchdog_armed, 1);
        assert_eq!(snap.setup_teardown_watchdog_disarmed, 1);
        assert_eq!(snap.setup_teardown_watchdog_fired, 1);
        assert_eq!(snap.setup_teardown_watchdog_transition_failed, 1);
        assert_eq!(snap.setup_teardown_watchdog_release_completed, 1);
        assert_eq!(snap.setup_teardown_watchdog_release_failed, 1);
        assert!(format_summary(&snap).contains("setup_teardown_watchdog"));
    }
}

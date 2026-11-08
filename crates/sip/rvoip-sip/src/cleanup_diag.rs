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

// Session cleanup accounting. Unlike the stage diagnostics above these are
// always on: each increment marks a bounded-failure decision that operators
// must be able to reconcile against released sessions and connections.
static CANCELLED_REJECT_TERMINALS: AtomicU64 = AtomicU64::new(0);
static INLINE_HANGUP_DEADLINE_EXPIRED: AtomicU64 = AtomicU64::new(0);
static INBOUND_END_BACKSTOP_EXPIRED: AtomicU64 = AtomicU64::new(0);
static INBOUND_END_RECLAIMS: AtomicU64 = AtomicU64::new(0);
static RECLAIM_RELEASED: AtomicU64 = AtomicU64::new(0);
static RECLAIM_FINALIZED: AtomicU64 = AtomicU64::new(0);
static RECLAIM_FORCED: AtomicU64 = AtomicU64::new(0);
static RECLAIM_TIMED_OUT: AtomicU64 = AtomicU64::new(0);
static FORCED_SESSION_RELEASES: AtomicU64 = AtomicU64::new(0);
static FORCED_SESSION_RELEASE_FAILURES: AtomicU64 = AtomicU64::new(0);
static PENDING_HANGUP_WATCHDOGS_ARMED: AtomicU64 = AtomicU64::new(0);
static ORPHAN_RECLAIMS_STARTED: AtomicU64 = AtomicU64::new(0);
static ORPHAN_RECLAIM_ALREADY_RELEASED: AtomicU64 = AtomicU64::new(0);
static ORPHAN_RECLAIM_GRACEFUL: AtomicU64 = AtomicU64::new(0);
static ORPHAN_RECLAIM_FORCED: AtomicU64 = AtomicU64::new(0);
static ORPHAN_RECLAIM_TIMED_OUT: AtomicU64 = AtomicU64::new(0);
static ORPHAN_RECLAIM_LOG: RateLimitedLog = RateLimitedLog::new();
static MEDIA_PORT_ADMISSION_REJECTS: AtomicU64 = AtomicU64::new(0);
static MEDIA_PORT_ADMISSION_LOG: RateLimitedLog = RateLimitedLog::new();
static SETUP_FAILURE_REJECTS: AtomicU64 = AtomicU64::new(0);
static SETUP_FAILURE_REJECT_LOG: RateLimitedLog = RateLimitedLog::new();
static SETUP_FAILURE_UNANSWERED: AtomicU64 = AtomicU64::new(0);
static SETUP_FAILURE_UNANSWERED_LOG: RateLimitedLog = RateLimitedLog::new();
static INBOUND_END_BACKSTOP_LOG: RateLimitedLog = RateLimitedLog::new();
static RECLAIM_TIMED_OUT_LOG: RateLimitedLog = RateLimitedLog::new();
static FORCED_RELEASE_LOG: RateLimitedLog = RateLimitedLog::new();
static FORCED_RELEASE_FAILURE_LOG: RateLimitedLog = RateLimitedLog::new();

/// Always-on counters for the bounded session cleanup paths.
///
/// Every field is a monotonic count since process start.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SessionCleanupCounters {
    /// Inbound rejects cancelled after their route was retired. Each one had
    /// its terminal delivered by a retained compensation task instead of
    /// leaving the orchestrator's connection lifecycle unresolved.
    pub cancelled_reject_terminals: u64,
    /// Inbound ends whose hangup join passed
    /// `SessionCleanupConfig::inline_hangup_deadline` before a BYE
    /// transaction existed.
    pub inline_hangup_deadline_expiries: u64,
    /// Inbound ends whose hangup join passed
    /// `SessionCleanupConfig::inbound_end_backstop`.
    pub inbound_end_backstop_expiries: u64,
    /// Inbound ends whose hangup failed (including both bounds above) and
    /// that handed the ended call to the retained reclaim.
    pub inbound_end_reclaims: u64,
    /// Reclaims whose call left the session store on its own.
    pub inbound_end_reclaims_released: u64,
    /// Reclaims whose call left after its local BYE was finalized.
    pub inbound_end_reclaims_finalized: u64,
    /// Reclaims that ran the forced local reclaim to completion.
    pub inbound_end_reclaims_forced: u64,
    /// Reclaims whose forced local reclaim exceeded its bound, so the session
    /// slot release was attempted directly. `released + finalized + forced +
    /// timed_out` is the finished subset of `inbound_end_reclaims`.
    pub inbound_end_reclaims_timed_out: u64,
    /// Ended lifetimes whose session-store slot was released after their
    /// bounded exact cleanup attempts failed.
    pub forced_session_releases: u64,
    /// Forced slot releases the lifecycle authority refused. Each one is a
    /// slot still held, so a nonzero value at quiescence is a leak.
    pub forced_session_release_failures: u64,
    /// Local hangups that waited for an ACK in `AnsweringHangupPending` and
    /// armed the accepted-call watchdog
    /// (`SessionCleanupConfig::pending_hangup_watchdog`).
    pub pending_hangup_watchdogs_armed: u64,
    /// Ownerless call sessions handed to an orphan reclaim
    /// (`SessionCleanupConfig::orphan_session_ttl`).
    pub orphan_session_reclaims: u64,
    /// Orphan reclaims that found the session already released by another
    /// owner.
    pub orphan_session_reclaims_already_released: u64,
    /// Orphan reclaims of established calls that left the session store
    /// after their bounded BYE.
    pub orphan_session_reclaims_graceful: u64,
    /// Orphan reclaims whose forced local reclaim (dialog, media port,
    /// session slot) ran to completion.
    pub orphan_session_reclaims_forced: u64,
    /// Orphan reclaims whose forced reclaim exceeded its bound, or was cut
    /// off by an adapter drain, so the slot release was attempted directly.
    /// `already_released + graceful + forced + timed_out` is the finished
    /// subset of `orphan_session_reclaims`.
    pub orphan_session_reclaims_timed_out: u64,
    /// Initial INVITEs shed with 503 because the RTP range had no free port
    /// slot for the call's reservation, before any session state existed.
    pub media_port_admission_rejects: u64,
    /// Initial INVITEs answered 503 after their setup failed before a final
    /// response (session creation, the dialog commit, media allocation or
    /// another setup action). Such INVITEs used to get no final response.
    pub setup_failure_rejects: u64,
    /// Failed setups whose 503 also stopped before any transport write.
    /// Each one is a caller that saw a timeout instead of a 503.
    pub setup_failure_unanswered: u64,
}

/// Read the session cleanup counters.
pub fn session_cleanup_counters() -> SessionCleanupCounters {
    SessionCleanupCounters {
        cancelled_reject_terminals: CANCELLED_REJECT_TERMINALS.load(Ordering::Relaxed),
        inline_hangup_deadline_expiries: INLINE_HANGUP_DEADLINE_EXPIRED.load(Ordering::Relaxed),
        inbound_end_backstop_expiries: INBOUND_END_BACKSTOP_EXPIRED.load(Ordering::Relaxed),
        inbound_end_reclaims: INBOUND_END_RECLAIMS.load(Ordering::Relaxed),
        inbound_end_reclaims_released: RECLAIM_RELEASED.load(Ordering::Relaxed),
        inbound_end_reclaims_finalized: RECLAIM_FINALIZED.load(Ordering::Relaxed),
        inbound_end_reclaims_forced: RECLAIM_FORCED.load(Ordering::Relaxed),
        inbound_end_reclaims_timed_out: RECLAIM_TIMED_OUT.load(Ordering::Relaxed),
        forced_session_releases: FORCED_SESSION_RELEASES.load(Ordering::Relaxed),
        forced_session_release_failures: FORCED_SESSION_RELEASE_FAILURES.load(Ordering::Relaxed),
        pending_hangup_watchdogs_armed: PENDING_HANGUP_WATCHDOGS_ARMED.load(Ordering::Relaxed),
        orphan_session_reclaims: ORPHAN_RECLAIMS_STARTED.load(Ordering::Relaxed),
        orphan_session_reclaims_already_released: ORPHAN_RECLAIM_ALREADY_RELEASED
            .load(Ordering::Relaxed),
        orphan_session_reclaims_graceful: ORPHAN_RECLAIM_GRACEFUL.load(Ordering::Relaxed),
        orphan_session_reclaims_forced: ORPHAN_RECLAIM_FORCED.load(Ordering::Relaxed),
        orphan_session_reclaims_timed_out: ORPHAN_RECLAIM_TIMED_OUT.load(Ordering::Relaxed),
        media_port_admission_rejects: MEDIA_PORT_ADMISSION_REJECTS.load(Ordering::Relaxed),
        setup_failure_rejects: SETUP_FAILURE_REJECTS.load(Ordering::Relaxed),
        setup_failure_unanswered: SETUP_FAILURE_UNANSWERED.load(Ordering::Relaxed),
    }
}

pub(crate) fn record_cancelled_reject_terminal() {
    CANCELLED_REJECT_TERMINALS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_inline_hangup_deadline_expired() {
    INLINE_HANGUP_DEADLINE_EXPIRED.fetch_add(1, Ordering::Relaxed);
}

/// Count one inbound end cut off by its backstop; `Some(occurrences)` when it
/// should log.
pub(crate) fn record_inbound_end_backstop_expired() -> Option<u64> {
    INBOUND_END_BACKSTOP_EXPIRED.fetch_add(1, Ordering::Relaxed);
    INBOUND_END_BACKSTOP_LOG.admit()
}

pub(crate) fn record_pending_hangup_watchdog_armed() {
    PENDING_HANGUP_WATCHDOGS_ARMED.fetch_add(1, Ordering::Relaxed);
}

/// Count one INVITE shed with 503 because no RTP port slot was free;
/// `Some(occurrences)` when it should log.
pub(crate) fn record_media_port_admission_reject() -> Option<u64> {
    MEDIA_PORT_ADMISSION_REJECTS.fetch_add(1, Ordering::Relaxed);
    MEDIA_PORT_ADMISSION_LOG.admit()
}

/// Count one INVITE answered 503 after its setup failed; `Some(occurrences)`
/// when it should log.
pub(crate) fn record_setup_failure_reject() -> Option<u64> {
    SETUP_FAILURE_REJECTS.fetch_add(1, Ordering::Relaxed);
    SETUP_FAILURE_REJECT_LOG.admit()
}

/// Count one failed setup whose 503 stopped before its first transport
/// write; `Some(occurrences)` when it should log.
pub(crate) fn record_setup_failure_unanswered() -> Option<u64> {
    SETUP_FAILURE_UNANSWERED.fetch_add(1, Ordering::Relaxed);
    SETUP_FAILURE_UNANSWERED_LOG.admit()
}

/// How one orphan session reclaim finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OrphanSessionReclaimOutcome {
    /// Another owner released the lifetime before the reclaim acted.
    AlreadyReleased,
    /// An established call left the store after its bounded BYE.
    Graceful,
    /// The forced local reclaim (dialog, media, slot) ran to completion.
    Forced,
    /// The forced reclaim exceeded its bound, or a drain stopped its waiter;
    /// the slot release was attempted directly.
    TimedOut,
}

/// Count one ownerless session handed to a reclaim; `Some(occurrences)` when
/// it should log.
pub(crate) fn record_orphan_session_reclaim_started() -> Option<u64> {
    ORPHAN_RECLAIMS_STARTED.fetch_add(1, Ordering::Relaxed);
    ORPHAN_RECLAIM_LOG.admit()
}

pub(crate) fn record_orphan_session_reclaim_outcome(outcome: OrphanSessionReclaimOutcome) {
    let counter = match outcome {
        OrphanSessionReclaimOutcome::AlreadyReleased => &ORPHAN_RECLAIM_ALREADY_RELEASED,
        OrphanSessionReclaimOutcome::Graceful => &ORPHAN_RECLAIM_GRACEFUL,
        OrphanSessionReclaimOutcome::Forced => &ORPHAN_RECLAIM_FORCED,
        OrphanSessionReclaimOutcome::TimedOut => &ORPHAN_RECLAIM_TIMED_OUT,
    };
    counter.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_inbound_end_reclaim() {
    INBOUND_END_RECLAIMS.fetch_add(1, Ordering::Relaxed);
}

/// How one inbound end reclaim finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InboundEndReclaimOutcome {
    /// The lifetime left the store on its own within the first wait.
    Released,
    /// It left after the local BYE was finalized.
    Finalized,
    /// The forced local reclaim ran to completion.
    Forced,
    /// The forced reclaim exceeded its bound; the slot release was attempted
    /// directly (its own counters say whether it succeeded).
    TimedOut,
}

/// Count one reclaim outcome; `Some(occurrences)` when a timeout should log.
pub(crate) fn record_inbound_end_reclaim_outcome(outcome: InboundEndReclaimOutcome) -> Option<u64> {
    match outcome {
        InboundEndReclaimOutcome::Released => RECLAIM_RELEASED.fetch_add(1, Ordering::Relaxed),
        InboundEndReclaimOutcome::Finalized => RECLAIM_FINALIZED.fetch_add(1, Ordering::Relaxed),
        InboundEndReclaimOutcome::Forced => RECLAIM_FORCED.fetch_add(1, Ordering::Relaxed),
        InboundEndReclaimOutcome::TimedOut => {
            RECLAIM_TIMED_OUT.fetch_add(1, Ordering::Relaxed);
            return RECLAIM_TIMED_OUT_LOG.admit();
        }
    };
    None
}

/// Count one forced slot release; `Some(occurrences)` when it should log.
pub(crate) fn record_forced_session_release() -> Option<u64> {
    FORCED_SESSION_RELEASES.fetch_add(1, Ordering::Relaxed);
    FORCED_RELEASE_LOG.admit()
}

/// Count one lifetime the authority would not retire even when forced.
pub(crate) fn record_forced_session_release_failed() -> Option<u64> {
    FORCED_SESSION_RELEASE_FAILURES.fetch_add(1, Ordering::Relaxed);
    FORCED_RELEASE_FAILURE_LOG.admit()
}

/// At most one log line per second for an event that can fire thousands of
/// times a second, carrying the occurrences since the previous line.
struct RateLimitedLog {
    occurrences: AtomicU64,
    last_logged_ms: AtomicU64,
}

impl RateLimitedLog {
    const INTERVAL_MS: u64 = 1_000;

    const fn new() -> Self {
        Self {
            occurrences: AtomicU64::new(0),
            last_logged_ms: AtomicU64::new(0),
        }
    }

    fn admit(&self) -> Option<u64> {
        let now_ms = u64::try_from(unix_ms()).unwrap_or(u64::MAX).max(1);
        self.occurrences.fetch_add(1, Ordering::AcqRel);
        let last = self.last_logged_ms.load(Ordering::Acquire);
        if last != 0 && now_ms.saturating_sub(last) < Self::INTERVAL_MS {
            return None;
        }
        self.last_logged_ms
            .compare_exchange(last, now_ms, Ordering::AcqRel, Ordering::Acquire)
            .ok()?;
        Some(self.occurrences.swap(0, Ordering::AcqRel))
    }
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

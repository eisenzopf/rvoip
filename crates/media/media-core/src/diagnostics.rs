//! Media setup diagnostics used by high-CPS SIP benchmarks.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;

static ENABLED_OVERRIDE: AtomicU8 = AtomicU8::new(0);

static MEDIA_START_TOTAL: AtomicU64 = AtomicU64::new(0);
static MEDIA_START_DONE: AtomicU64 = AtomicU64::new(0);
static MEDIA_START_FAIL: AtomicU64 = AtomicU64::new(0);
static MEDIA_START_ACTIVE: AtomicU64 = AtomicU64::new(0);
static MEDIA_START_NS: AtomicU64 = AtomicU64::new(0);
static MEDIA_START_MAX_NS: AtomicU64 = AtomicU64::new(0);

static RTP_PORT_ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static RTP_PORT_ALLOC_NS: AtomicU64 = AtomicU64::new(0);
static RTP_PORT_ALLOC_MAX_NS: AtomicU64 = AtomicU64::new(0);

static RTP_SESSION_NEW_COUNT: AtomicU64 = AtomicU64::new(0);
static RTP_SESSION_NEW_NS: AtomicU64 = AtomicU64::new(0);
static RTP_SESSION_NEW_MAX_NS: AtomicU64 = AtomicU64::new(0);

static RTP_EVENT_SUBSCRIBE_COUNT: AtomicU64 = AtomicU64::new(0);
static RTP_EVENT_SUBSCRIBE_NS: AtomicU64 = AtomicU64::new(0);
static RTP_EVENT_SUBSCRIBE_MAX_NS: AtomicU64 = AtomicU64::new(0);

static RTP_EVENT_HANDLER_SPAWN_COUNT: AtomicU64 = AtomicU64::new(0);
static RTP_EVENT_HANDLER_SPAWN_NS: AtomicU64 = AtomicU64::new(0);
static RTP_EVENT_HANDLER_SPAWN_MAX_NS: AtomicU64 = AtomicU64::new(0);

static STOP_MEDIA_COUNT: AtomicU64 = AtomicU64::new(0);
static STOP_MEDIA_NS: AtomicU64 = AtomicU64::new(0);
static STOP_MEDIA_MAX_NS: AtomicU64 = AtomicU64::new(0);

static PORT_RELEASE_COUNT: AtomicU64 = AtomicU64::new(0);
static PORT_RELEASE_NS: AtomicU64 = AtomicU64::new(0);
static PORT_RELEASE_MAX_NS: AtomicU64 = AtomicU64::new(0);

static AUDIO_TX_TASK_START_COUNT: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_START_PHASE_NS: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_START_PHASE_MAX_NS: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_TICK_GAP_COUNT: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_TICK_GAP_NS: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_TICK_GAP_MAX_NS: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_SEND_COUNT: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_SEND_FAIL: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_SEND_NS: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_SEND_MAX_NS: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_PACING_SKIP_COUNT: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_PACING_ACTIVE_MAX: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_PACING_DIVISOR_MAX: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_SHARED_DUE_COUNT: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_SHARED_SENT_COUNT: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_SHARED_SKIP_COUNT: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_SHARED_FAIL_COUNT: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_SHARED_ACTIVE_MAX: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_SHARED_BATCH_MAX: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    pub media_start_total: u64,
    pub media_start_done: u64,
    pub media_start_fail: u64,
    pub media_start_active: u64,
    pub media_start_ns: u64,
    pub media_start_max_ns: u64,
    pub rtp_port_allocate_count: u64,
    pub rtp_port_allocate_ns: u64,
    pub rtp_port_allocate_max_ns: u64,
    pub rtp_session_new_count: u64,
    pub rtp_session_new_ns: u64,
    pub rtp_session_new_max_ns: u64,
    pub rtp_event_subscription_count: u64,
    pub rtp_event_subscription_ns: u64,
    pub rtp_event_subscription_max_ns: u64,
    pub rtp_event_handler_spawn_count: u64,
    pub rtp_event_handler_spawn_ns: u64,
    pub rtp_event_handler_spawn_max_ns: u64,
    pub stop_media_count: u64,
    pub stop_media_ns: u64,
    pub stop_media_max_ns: u64,
    pub port_release_count: u64,
    pub port_release_ns: u64,
    pub port_release_max_ns: u64,
    pub audio_tx_task_start_count: u64,
    pub audio_tx_start_phase_ns: u64,
    pub audio_tx_start_phase_max_ns: u64,
    pub audio_tx_tick_gap_count: u64,
    pub audio_tx_tick_gap_ns: u64,
    pub audio_tx_tick_gap_max_ns: u64,
    pub audio_tx_send_count: u64,
    pub audio_tx_send_fail: u64,
    pub audio_tx_send_ns: u64,
    pub audio_tx_send_max_ns: u64,
    pub audio_tx_pacing_skip_count: u64,
    pub audio_tx_pacing_active_max: u64,
    pub audio_tx_pacing_divisor_max: u64,
    pub audio_tx_shared_due_count: u64,
    pub audio_tx_shared_sent_count: u64,
    pub audio_tx_shared_skip_count: u64,
    pub audio_tx_shared_fail_count: u64,
    pub audio_tx_shared_active_max: u64,
    pub audio_tx_shared_batch_max: u64,
}

pub struct MediaStartGuard {
    started: Instant,
    finished: bool,
}

impl MediaStartGuard {
    pub fn new() -> Self {
        if enabled() {
            MEDIA_START_TOTAL.fetch_add(1, Ordering::Relaxed);
            MEDIA_START_ACTIVE.fetch_add(1, Ordering::Relaxed);
        }
        Self {
            started: Instant::now(),
            finished: false,
        }
    }

    pub fn finish_success(mut self) {
        self.finished = true;
        record_media_start_finish(self.started.elapsed(), true);
    }
}

impl Drop for MediaStartGuard {
    fn drop(&mut self) {
        if !self.finished {
            record_media_start_finish(self.started.elapsed(), false);
        }
    }
}

pub fn enabled() -> bool {
    match ENABLED_OVERRIDE.load(Ordering::Relaxed) {
        2 => true,
        _ => false,
    }
}

pub fn set_enabled(enabled: bool) {
    ENABLED_OVERRIDE.store(if enabled { 2 } else { 1 }, Ordering::Relaxed);
}

#[cfg(test)]
fn set_enabled_for_tests(enabled: bool) {
    set_enabled(enabled);
}

pub fn reset() {
    for counter in all_counters() {
        counter.store(0, Ordering::Relaxed);
    }
}

pub fn snapshot() -> Snapshot {
    Snapshot {
        media_start_total: MEDIA_START_TOTAL.load(Ordering::Relaxed),
        media_start_done: MEDIA_START_DONE.load(Ordering::Relaxed),
        media_start_fail: MEDIA_START_FAIL.load(Ordering::Relaxed),
        media_start_active: MEDIA_START_ACTIVE.load(Ordering::Relaxed),
        media_start_ns: MEDIA_START_NS.load(Ordering::Relaxed),
        media_start_max_ns: MEDIA_START_MAX_NS.load(Ordering::Relaxed),
        rtp_port_allocate_count: RTP_PORT_ALLOC_COUNT.load(Ordering::Relaxed),
        rtp_port_allocate_ns: RTP_PORT_ALLOC_NS.load(Ordering::Relaxed),
        rtp_port_allocate_max_ns: RTP_PORT_ALLOC_MAX_NS.load(Ordering::Relaxed),
        rtp_session_new_count: RTP_SESSION_NEW_COUNT.load(Ordering::Relaxed),
        rtp_session_new_ns: RTP_SESSION_NEW_NS.load(Ordering::Relaxed),
        rtp_session_new_max_ns: RTP_SESSION_NEW_MAX_NS.load(Ordering::Relaxed),
        rtp_event_subscription_count: RTP_EVENT_SUBSCRIBE_COUNT.load(Ordering::Relaxed),
        rtp_event_subscription_ns: RTP_EVENT_SUBSCRIBE_NS.load(Ordering::Relaxed),
        rtp_event_subscription_max_ns: RTP_EVENT_SUBSCRIBE_MAX_NS.load(Ordering::Relaxed),
        rtp_event_handler_spawn_count: RTP_EVENT_HANDLER_SPAWN_COUNT.load(Ordering::Relaxed),
        rtp_event_handler_spawn_ns: RTP_EVENT_HANDLER_SPAWN_NS.load(Ordering::Relaxed),
        rtp_event_handler_spawn_max_ns: RTP_EVENT_HANDLER_SPAWN_MAX_NS.load(Ordering::Relaxed),
        stop_media_count: STOP_MEDIA_COUNT.load(Ordering::Relaxed),
        stop_media_ns: STOP_MEDIA_NS.load(Ordering::Relaxed),
        stop_media_max_ns: STOP_MEDIA_MAX_NS.load(Ordering::Relaxed),
        port_release_count: PORT_RELEASE_COUNT.load(Ordering::Relaxed),
        port_release_ns: PORT_RELEASE_NS.load(Ordering::Relaxed),
        port_release_max_ns: PORT_RELEASE_MAX_NS.load(Ordering::Relaxed),
        audio_tx_task_start_count: AUDIO_TX_TASK_START_COUNT.load(Ordering::Relaxed),
        audio_tx_start_phase_ns: AUDIO_TX_START_PHASE_NS.load(Ordering::Relaxed),
        audio_tx_start_phase_max_ns: AUDIO_TX_START_PHASE_MAX_NS.load(Ordering::Relaxed),
        audio_tx_tick_gap_count: AUDIO_TX_TICK_GAP_COUNT.load(Ordering::Relaxed),
        audio_tx_tick_gap_ns: AUDIO_TX_TICK_GAP_NS.load(Ordering::Relaxed),
        audio_tx_tick_gap_max_ns: AUDIO_TX_TICK_GAP_MAX_NS.load(Ordering::Relaxed),
        audio_tx_send_count: AUDIO_TX_SEND_COUNT.load(Ordering::Relaxed),
        audio_tx_send_fail: AUDIO_TX_SEND_FAIL.load(Ordering::Relaxed),
        audio_tx_send_ns: AUDIO_TX_SEND_NS.load(Ordering::Relaxed),
        audio_tx_send_max_ns: AUDIO_TX_SEND_MAX_NS.load(Ordering::Relaxed),
        audio_tx_pacing_skip_count: AUDIO_TX_PACING_SKIP_COUNT.load(Ordering::Relaxed),
        audio_tx_pacing_active_max: AUDIO_TX_PACING_ACTIVE_MAX.load(Ordering::Relaxed),
        audio_tx_pacing_divisor_max: AUDIO_TX_PACING_DIVISOR_MAX.load(Ordering::Relaxed),
        audio_tx_shared_due_count: AUDIO_TX_SHARED_DUE_COUNT.load(Ordering::Relaxed),
        audio_tx_shared_sent_count: AUDIO_TX_SHARED_SENT_COUNT.load(Ordering::Relaxed),
        audio_tx_shared_skip_count: AUDIO_TX_SHARED_SKIP_COUNT.load(Ordering::Relaxed),
        audio_tx_shared_fail_count: AUDIO_TX_SHARED_FAIL_COUNT.load(Ordering::Relaxed),
        audio_tx_shared_active_max: AUDIO_TX_SHARED_ACTIVE_MAX.load(Ordering::Relaxed),
        audio_tx_shared_batch_max: AUDIO_TX_SHARED_BATCH_MAX.load(Ordering::Relaxed),
    }
}

pub fn format_summary(snapshot: &Snapshot) -> String {
    format!(
        "[media_setup_diag] start_total={} start_done={} start_fail={} start_active={} \
         start_avg_us={:.1} start_max_us={} rtp_port_allocate={} rtp_port_avg_us={:.1} \
         rtp_port_max_us={} rtp_session_new={} rtp_session_avg_us={:.1} rtp_session_max_us={} \
         rtp_event_subscription={} rtp_event_sub_avg_us={:.1} rtp_event_sub_max_us={} \
         rtp_event_handler_spawn={} rtp_event_spawn_avg_us={:.1} rtp_event_spawn_max_us={} \
         stop_media={} stop_avg_us={:.1} stop_max_us={} port_release={} port_release_avg_us={:.1} \
         port_release_max_us={} audio_tx_starts={} audio_tx_start_phase_avg_us={:.1} \
         audio_tx_start_phase_max_us={} audio_tx_tick_gap_avg_us={:.1} audio_tx_tick_gap_max_us={} \
         audio_tx_send={} audio_tx_send_fail={} audio_tx_send_avg_us={:.1} audio_tx_send_max_us={} \
         audio_tx_pacing_skips={} audio_tx_pacing_active_max={} audio_tx_pacing_divisor_max={} \
         audio_tx_shared_due={} audio_tx_shared_sent={} audio_tx_shared_skip={} \
         audio_tx_shared_fail={} audio_tx_shared_active_max={} audio_tx_shared_batch_max={}",
        snapshot.media_start_total,
        snapshot.media_start_done,
        snapshot.media_start_fail,
        snapshot.media_start_active,
        avg_us(
            snapshot.media_start_ns,
            snapshot.media_start_done + snapshot.media_start_fail
        ),
        ns_to_us(snapshot.media_start_max_ns),
        snapshot.rtp_port_allocate_count,
        avg_us(
            snapshot.rtp_port_allocate_ns,
            snapshot.rtp_port_allocate_count
        ),
        ns_to_us(snapshot.rtp_port_allocate_max_ns),
        snapshot.rtp_session_new_count,
        avg_us(snapshot.rtp_session_new_ns, snapshot.rtp_session_new_count),
        ns_to_us(snapshot.rtp_session_new_max_ns),
        snapshot.rtp_event_subscription_count,
        avg_us(
            snapshot.rtp_event_subscription_ns,
            snapshot.rtp_event_subscription_count
        ),
        ns_to_us(snapshot.rtp_event_subscription_max_ns),
        snapshot.rtp_event_handler_spawn_count,
        avg_us(
            snapshot.rtp_event_handler_spawn_ns,
            snapshot.rtp_event_handler_spawn_count
        ),
        ns_to_us(snapshot.rtp_event_handler_spawn_max_ns),
        snapshot.stop_media_count,
        avg_us(snapshot.stop_media_ns, snapshot.stop_media_count),
        ns_to_us(snapshot.stop_media_max_ns),
        snapshot.port_release_count,
        avg_us(snapshot.port_release_ns, snapshot.port_release_count),
        ns_to_us(snapshot.port_release_max_ns),
        snapshot.audio_tx_task_start_count,
        avg_us(
            snapshot.audio_tx_start_phase_ns,
            snapshot.audio_tx_task_start_count
        ),
        ns_to_us(snapshot.audio_tx_start_phase_max_ns),
        avg_us(
            snapshot.audio_tx_tick_gap_ns,
            snapshot.audio_tx_tick_gap_count
        ),
        ns_to_us(snapshot.audio_tx_tick_gap_max_ns),
        snapshot.audio_tx_send_count,
        snapshot.audio_tx_send_fail,
        avg_us(snapshot.audio_tx_send_ns, snapshot.audio_tx_send_count),
        ns_to_us(snapshot.audio_tx_send_max_ns),
        snapshot.audio_tx_pacing_skip_count,
        snapshot.audio_tx_pacing_active_max,
        snapshot.audio_tx_pacing_divisor_max,
        snapshot.audio_tx_shared_due_count,
        snapshot.audio_tx_shared_sent_count,
        snapshot.audio_tx_shared_skip_count,
        snapshot.audio_tx_shared_fail_count,
        snapshot.audio_tx_shared_active_max,
        snapshot.audio_tx_shared_batch_max,
    )
}

pub fn record_rtp_port_allocate(duration: Duration) {
    record_duration(
        &RTP_PORT_ALLOC_COUNT,
        &RTP_PORT_ALLOC_NS,
        &RTP_PORT_ALLOC_MAX_NS,
        duration,
    );
}

pub fn record_rtp_session_new(duration: Duration) {
    record_duration(
        &RTP_SESSION_NEW_COUNT,
        &RTP_SESSION_NEW_NS,
        &RTP_SESSION_NEW_MAX_NS,
        duration,
    );
}

pub fn record_rtp_event_subscription(duration: Duration) {
    record_duration(
        &RTP_EVENT_SUBSCRIBE_COUNT,
        &RTP_EVENT_SUBSCRIBE_NS,
        &RTP_EVENT_SUBSCRIBE_MAX_NS,
        duration,
    );
}

pub fn record_rtp_event_handler_spawn(duration: Duration) {
    record_duration(
        &RTP_EVENT_HANDLER_SPAWN_COUNT,
        &RTP_EVENT_HANDLER_SPAWN_NS,
        &RTP_EVENT_HANDLER_SPAWN_MAX_NS,
        duration,
    );
}

pub fn record_stop_media(duration: Duration) {
    record_duration(
        &STOP_MEDIA_COUNT,
        &STOP_MEDIA_NS,
        &STOP_MEDIA_MAX_NS,
        duration,
    );
}

pub fn record_port_release(duration: Duration) {
    record_duration(
        &PORT_RELEASE_COUNT,
        &PORT_RELEASE_NS,
        &PORT_RELEASE_MAX_NS,
        duration,
    );
}

pub fn record_audio_tx_task_started(initial_delay: Duration) {
    if enabled() {
        AUDIO_TX_TASK_START_COUNT.fetch_add(1, Ordering::Relaxed);
        let ns = ns(initial_delay);
        AUDIO_TX_START_PHASE_NS.fetch_add(ns, Ordering::Relaxed);
        update_max(&AUDIO_TX_START_PHASE_MAX_NS, ns);
    }
}

pub fn record_audio_tx_tick_gap(duration: Duration) {
    record_duration(
        &AUDIO_TX_TICK_GAP_COUNT,
        &AUDIO_TX_TICK_GAP_NS,
        &AUDIO_TX_TICK_GAP_MAX_NS,
        duration,
    );
}

pub fn record_audio_tx_tick_gap_batch(count: u64, total: Duration, max: Duration) {
    record_duration_batch(
        &AUDIO_TX_TICK_GAP_COUNT,
        &AUDIO_TX_TICK_GAP_NS,
        &AUDIO_TX_TICK_GAP_MAX_NS,
        count,
        total,
        max,
    );
}

pub fn record_audio_tx_send(duration: Duration, success: bool) {
    if enabled() {
        AUDIO_TX_SEND_COUNT.fetch_add(1, Ordering::Relaxed);
        if !success {
            AUDIO_TX_SEND_FAIL.fetch_add(1, Ordering::Relaxed);
        }
        let ns = ns(duration);
        AUDIO_TX_SEND_NS.fetch_add(ns, Ordering::Relaxed);
        update_max(&AUDIO_TX_SEND_MAX_NS, ns);
    }
}

pub fn record_audio_tx_send_batch(count: u64, failures: u64, total: Duration, max: Duration) {
    if enabled() && count > 0 {
        AUDIO_TX_SEND_COUNT.fetch_add(count, Ordering::Relaxed);
        AUDIO_TX_SEND_FAIL.fetch_add(failures, Ordering::Relaxed);
        AUDIO_TX_SEND_NS.fetch_add(ns(total), Ordering::Relaxed);
        update_max(&AUDIO_TX_SEND_MAX_NS, ns(max));
    }
}

pub fn record_audio_tx_pacing_batch(skips: u64, active_max: u64, divisor_max: u64) {
    if skips > 0 {
        AUDIO_TX_PACING_SKIP_COUNT.fetch_add(skips, Ordering::Relaxed);
    }
    update_max(&AUDIO_TX_PACING_ACTIVE_MAX, active_max);
    update_max(&AUDIO_TX_PACING_DIVISOR_MAX, divisor_max);
}

pub fn record_audio_tx_shared_batch(
    due_count: u64,
    sent_count: u64,
    skip_count: u64,
    fail_count: u64,
    active_count: u64,
) {
    if due_count > 0 {
        AUDIO_TX_SHARED_DUE_COUNT.fetch_add(due_count, Ordering::Relaxed);
        AUDIO_TX_SHARED_SENT_COUNT.fetch_add(sent_count, Ordering::Relaxed);
        AUDIO_TX_SHARED_SKIP_COUNT.fetch_add(skip_count, Ordering::Relaxed);
        AUDIO_TX_SHARED_FAIL_COUNT.fetch_add(fail_count, Ordering::Relaxed);
    }
    update_max(&AUDIO_TX_SHARED_ACTIVE_MAX, active_count);
    update_max(&AUDIO_TX_SHARED_BATCH_MAX, due_count);
}

fn record_media_start_finish(duration: Duration, success: bool) {
    if enabled() {
        if success {
            MEDIA_START_DONE.fetch_add(1, Ordering::Relaxed);
        } else {
            MEDIA_START_FAIL.fetch_add(1, Ordering::Relaxed);
        }
        MEDIA_START_ACTIVE.fetch_sub(1, Ordering::Relaxed);
        let ns = ns(duration);
        MEDIA_START_NS.fetch_add(ns, Ordering::Relaxed);
        update_max(&MEDIA_START_MAX_NS, ns);
    }
}

fn record_duration(
    count: &AtomicU64,
    total_ns: &AtomicU64,
    max_ns: &AtomicU64,
    duration: Duration,
) {
    if enabled() {
        let ns = ns(duration);
        count.fetch_add(1, Ordering::Relaxed);
        total_ns.fetch_add(ns, Ordering::Relaxed);
        update_max(max_ns, ns);
    }
}

fn record_duration_batch(
    count: &AtomicU64,
    total_ns: &AtomicU64,
    max_ns: &AtomicU64,
    increment: u64,
    total: Duration,
    max: Duration,
) {
    if enabled() && increment > 0 {
        count.fetch_add(increment, Ordering::Relaxed);
        total_ns.fetch_add(ns(total), Ordering::Relaxed);
        update_max(max_ns, ns(max));
    }
}

fn update_max(counter: &AtomicU64, value: u64) {
    let mut current = counter.load(Ordering::Relaxed);
    while value > current {
        match counter.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(next) => current = next,
        }
    }
}

fn avg_us(total_ns: u64, count: u64) -> f64 {
    if count == 0 {
        0.0
    } else {
        total_ns as f64 / count as f64 / 1_000.0
    }
}

fn ns_to_us(ns: u64) -> u64 {
    ns / 1_000
}

fn ns(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

fn all_counters() -> [&'static AtomicU64; 43] {
    [
        &MEDIA_START_TOTAL,
        &MEDIA_START_DONE,
        &MEDIA_START_FAIL,
        &MEDIA_START_ACTIVE,
        &MEDIA_START_NS,
        &MEDIA_START_MAX_NS,
        &RTP_PORT_ALLOC_COUNT,
        &RTP_PORT_ALLOC_NS,
        &RTP_PORT_ALLOC_MAX_NS,
        &RTP_SESSION_NEW_COUNT,
        &RTP_SESSION_NEW_NS,
        &RTP_SESSION_NEW_MAX_NS,
        &RTP_EVENT_SUBSCRIBE_COUNT,
        &RTP_EVENT_SUBSCRIBE_NS,
        &RTP_EVENT_SUBSCRIBE_MAX_NS,
        &RTP_EVENT_HANDLER_SPAWN_COUNT,
        &RTP_EVENT_HANDLER_SPAWN_NS,
        &RTP_EVENT_HANDLER_SPAWN_MAX_NS,
        &STOP_MEDIA_COUNT,
        &STOP_MEDIA_NS,
        &STOP_MEDIA_MAX_NS,
        &PORT_RELEASE_COUNT,
        &PORT_RELEASE_NS,
        &PORT_RELEASE_MAX_NS,
        &AUDIO_TX_TASK_START_COUNT,
        &AUDIO_TX_START_PHASE_NS,
        &AUDIO_TX_START_PHASE_MAX_NS,
        &AUDIO_TX_TICK_GAP_COUNT,
        &AUDIO_TX_TICK_GAP_NS,
        &AUDIO_TX_TICK_GAP_MAX_NS,
        &AUDIO_TX_SEND_COUNT,
        &AUDIO_TX_SEND_FAIL,
        &AUDIO_TX_SEND_NS,
        &AUDIO_TX_SEND_MAX_NS,
        &AUDIO_TX_PACING_SKIP_COUNT,
        &AUDIO_TX_PACING_ACTIVE_MAX,
        &AUDIO_TX_PACING_DIVISOR_MAX,
        &AUDIO_TX_SHARED_DUE_COUNT,
        &AUDIO_TX_SHARED_SENT_COUNT,
        &AUDIO_TX_SHARED_SKIP_COUNT,
        &AUDIO_TX_SHARED_FAIL_COUNT,
        &AUDIO_TX_SHARED_ACTIVE_MAX,
        &AUDIO_TX_SHARED_BATCH_MAX,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_setup_summary_includes_phase_counts() {
        set_enabled_for_tests(true);
        reset();

        let guard = MediaStartGuard::new();
        record_rtp_port_allocate(Duration::from_micros(10));
        record_rtp_session_new(Duration::from_micros(20));
        record_rtp_event_subscription(Duration::from_micros(1));
        record_rtp_event_handler_spawn(Duration::from_micros(2));
        record_stop_media(Duration::from_micros(30));
        record_port_release(Duration::from_micros(3));
        record_audio_tx_task_started(Duration::from_micros(4));
        record_audio_tx_tick_gap(Duration::from_millis(20));
        record_audio_tx_send(Duration::from_micros(5), true);
        record_audio_tx_pacing_batch(7, 11, 3);
        record_audio_tx_shared_batch(13, 10, 2, 1, 17);
        guard.finish_success();

        let snapshot = snapshot();
        assert_eq!(snapshot.media_start_total, 1);
        assert_eq!(snapshot.media_start_done, 1);
        assert_eq!(snapshot.rtp_session_new_count, 1);
        assert_eq!(snapshot.audio_tx_task_start_count, 1);
        assert_eq!(snapshot.audio_tx_send_count, 1);
        assert_eq!(snapshot.audio_tx_pacing_skip_count, 7);
        assert_eq!(snapshot.audio_tx_pacing_active_max, 11);
        assert_eq!(snapshot.audio_tx_shared_due_count, 13);
        assert_eq!(snapshot.audio_tx_shared_sent_count, 10);
        assert_eq!(snapshot.audio_tx_shared_active_max, 17);
        let summary = format_summary(&snapshot);
        assert!(summary.contains("start_total=1"));
        assert!(summary.contains("rtp_session_new=1"));
        assert!(summary.contains("port_release=1"));
        assert!(summary.contains("audio_tx_starts=1"));
        assert!(summary.contains("audio_tx_pacing_skips=7"));
        assert!(summary.contains("audio_tx_shared_due=13"));
    }
}

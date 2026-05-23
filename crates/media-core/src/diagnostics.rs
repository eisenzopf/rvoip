//! Media setup diagnostics used by high-CPS SIP benchmarks.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

static ENABLED: OnceLock<bool> = OnceLock::new();
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
        1 => return false,
        2 => return true,
        _ => {}
    }
    *ENABLED.get_or_init(|| {
        env_flag("RVOIP_MEDIA_SETUP_DIAGNOSTICS") || env_flag("RVOIP_SIP_UDP_DIAGNOSTICS")
    })
}

#[cfg(test)]
fn set_enabled_for_tests(enabled: bool) {
    ENABLED_OVERRIDE.store(if enabled { 2 } else { 1 }, Ordering::Relaxed);
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
         port_release_max_us={}",
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

fn update_max(counter: &AtomicU64, value: u64) {
    let mut current = counter.load(Ordering::Relaxed);
    while value > current {
        match counter.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(next) => current = next,
        }
    }
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
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

fn all_counters() -> [&'static AtomicU64; 24] {
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
        guard.finish_success();

        let snapshot = snapshot();
        assert_eq!(snapshot.media_start_total, 1);
        assert_eq!(snapshot.media_start_done, 1);
        assert_eq!(snapshot.rtp_session_new_count, 1);
        let summary = format_summary(&snapshot);
        assert!(summary.contains("start_total=1"));
        assert!(summary.contains("rtp_session_new=1"));
        assert!(summary.contains("port_release=1"));
    }
}

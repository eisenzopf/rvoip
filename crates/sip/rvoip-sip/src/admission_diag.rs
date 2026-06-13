//! Perf-only server admission diagnostics.
//!
//! These counters explain whether inbound INVITEs are delayed, admitted, or
//! rejected by the rvoip-sip admission gate during burst tests. They are
//! compiled only with the `perf-tests` feature.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::time::Duration;

use serde::Serialize;

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

static ENABLE_OVERRIDE: AtomicU8 = AtomicU8::new(ENABLE_OFF);

static ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static NO_LIMIT_ADMITS: AtomicU64 = AtomicU64::new(0);
static ADMITS: AtomicU64 = AtomicU64::new(0);
static REJECTS: AtomicU64 = AtomicU64::new(0);
static HARD_LIMIT_REJECTS: AtomicU64 = AtomicU64::new(0);
static OVERLOAD_REJECTS: AtomicU64 = AtomicU64::new(0);
static OVERLOAD_ENTERED: AtomicU64 = AtomicU64::new(0);
static OVERLOAD_CLEARED: AtomicU64 = AtomicU64::new(0);
static PACING_DECISIONS: AtomicU64 = AtomicU64::new(0);
static OBSERVED_SESSIONS_MAX: AtomicU64 = AtomicU64::new(0);
static PENDING_MAX: AtomicU64 = AtomicU64::new(0);
static HARD_LIMIT_MAX: AtomicU64 = AtomicU64::new(0);
static SOFT_LIMIT_MIN: AtomicU64 = AtomicU64::new(0);
static SOFT_LIMIT_MAX: AtomicU64 = AtomicU64::new(0);

static LOCK_WAIT_COUNT: AtomicU64 = AtomicU64::new(0);
static LOCK_WAIT_SUM_US: AtomicU64 = AtomicU64::new(0);
static LOCK_WAIT_MAX_US: AtomicU64 = AtomicU64::new(0);
static LOCK_WAIT_BUCKETS: [AtomicU64; 18] = atomic_buckets();

static PACING_SLEEP_COUNT: AtomicU64 = AtomicU64::new(0);
static PACING_SLEEP_SUM_US: AtomicU64 = AtomicU64::new(0);
static PACING_SLEEP_MAX_US: AtomicU64 = AtomicU64::new(0);
static PACING_SLEEP_BUCKETS: [AtomicU64; 18] = atomic_buckets();

const fn atomic_buckets() -> [AtomicU64; 18] {
    [
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    ]
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AdmissionLatencySnapshot {
    pub count: u64,
    pub avg_us: u64,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
    pub over_500ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AdmissionDiagSnapshot {
    pub enabled: bool,
    pub attempts: u64,
    pub no_limit_admits: u64,
    pub admits: u64,
    pub rejects: u64,
    pub hard_limit_rejects: u64,
    pub overload_rejects: u64,
    pub overload_entered: u64,
    pub overload_cleared: u64,
    pub pacing_decisions: u64,
    pub observed_sessions_max: u64,
    pub pending_max: u64,
    pub hard_limit_max: u64,
    pub soft_limit_min: u64,
    pub soft_limit_max: u64,
    pub lock_wait: AdmissionLatencySnapshot,
    pub pacing_sleep: AdmissionLatencySnapshot,
}

pub fn set_enabled(enabled: bool) {
    ENABLE_OVERRIDE.store(
        if enabled { ENABLE_ON } else { ENABLE_OFF },
        Ordering::Relaxed,
    );
}

pub fn enabled() -> bool {
    ENABLE_OVERRIDE.load(Ordering::Relaxed) == ENABLE_ON
}

#[cfg(test)]
pub(crate) fn reset() {
    for counter in all_counters() {
        counter.store(0, Ordering::Relaxed);
    }
    for bucket in &LOCK_WAIT_BUCKETS {
        bucket.store(0, Ordering::Relaxed);
    }
    for bucket in &PACING_SLEEP_BUCKETS {
        bucket.store(0, Ordering::Relaxed);
    }
}

pub fn snapshot() -> AdmissionDiagSnapshot {
    AdmissionDiagSnapshot {
        enabled: enabled(),
        attempts: ATTEMPTS.load(Ordering::Relaxed),
        no_limit_admits: NO_LIMIT_ADMITS.load(Ordering::Relaxed),
        admits: ADMITS.load(Ordering::Relaxed),
        rejects: REJECTS.load(Ordering::Relaxed),
        hard_limit_rejects: HARD_LIMIT_REJECTS.load(Ordering::Relaxed),
        overload_rejects: OVERLOAD_REJECTS.load(Ordering::Relaxed),
        overload_entered: OVERLOAD_ENTERED.load(Ordering::Relaxed),
        overload_cleared: OVERLOAD_CLEARED.load(Ordering::Relaxed),
        pacing_decisions: PACING_DECISIONS.load(Ordering::Relaxed),
        observed_sessions_max: OBSERVED_SESSIONS_MAX.load(Ordering::Relaxed),
        pending_max: PENDING_MAX.load(Ordering::Relaxed),
        hard_limit_max: HARD_LIMIT_MAX.load(Ordering::Relaxed),
        soft_limit_min: SOFT_LIMIT_MIN.load(Ordering::Relaxed),
        soft_limit_max: SOFT_LIMIT_MAX.load(Ordering::Relaxed),
        lock_wait: latency_snapshot(
            &LOCK_WAIT_BUCKETS,
            &LOCK_WAIT_COUNT,
            &LOCK_WAIT_SUM_US,
            &LOCK_WAIT_MAX_US,
        ),
        pacing_sleep: latency_snapshot(
            &PACING_SLEEP_BUCKETS,
            &PACING_SLEEP_COUNT,
            &PACING_SLEEP_SUM_US,
            &PACING_SLEEP_MAX_US,
        ),
    }
}

pub(crate) fn record_attempt() {
    if enabled() {
        ATTEMPTS.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn record_no_limit_admit() {
    if enabled() {
        NO_LIMIT_ADMITS.fetch_add(1, Ordering::Relaxed);
        ADMITS.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn record_limits(hard_limit: usize, soft_limit: usize) {
    if !enabled() {
        return;
    }
    update_max(&HARD_LIMIT_MAX, hard_limit as u64);
    update_max(&SOFT_LIMIT_MAX, soft_limit as u64);
    update_min_nonzero(&SOFT_LIMIT_MIN, soft_limit as u64);
}

pub(crate) fn record_lock_wait(duration: Duration) {
    if enabled() {
        record_latency(
            &LOCK_WAIT_COUNT,
            &LOCK_WAIT_SUM_US,
            &LOCK_WAIT_MAX_US,
            &LOCK_WAIT_BUCKETS,
            duration,
        );
    }
}

pub(crate) fn record_observed(observed_sessions: usize, pending: usize) {
    if enabled() {
        update_max(&OBSERVED_SESSIONS_MAX, observed_sessions as u64);
        update_max(&PENDING_MAX, pending as u64);
    }
}

pub(crate) fn record_overload_entered() {
    if enabled() {
        OVERLOAD_ENTERED.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn record_overload_cleared() {
    if enabled() {
        OVERLOAD_CLEARED.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn record_pacing_decision() {
    if enabled() {
        PACING_DECISIONS.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn record_pacing_sleep(duration: Duration) {
    if enabled() {
        record_latency(
            &PACING_SLEEP_COUNT,
            &PACING_SLEEP_SUM_US,
            &PACING_SLEEP_MAX_US,
            &PACING_SLEEP_BUCKETS,
            duration,
        );
    }
}

pub(crate) fn record_admit(observed_sessions: usize, pending_after: usize) {
    if enabled() {
        ADMITS.fetch_add(1, Ordering::Relaxed);
        update_max(&OBSERVED_SESSIONS_MAX, observed_sessions as u64);
        update_max(&PENDING_MAX, pending_after as u64);
    }
}

pub(crate) fn record_reject_hard_limit(observed_sessions: usize) {
    if enabled() {
        REJECTS.fetch_add(1, Ordering::Relaxed);
        HARD_LIMIT_REJECTS.fetch_add(1, Ordering::Relaxed);
        update_max(&OBSERVED_SESSIONS_MAX, observed_sessions as u64);
    }
}

pub(crate) fn record_reject_overloaded(observed_sessions: usize) {
    if enabled() {
        REJECTS.fetch_add(1, Ordering::Relaxed);
        OVERLOAD_REJECTS.fetch_add(1, Ordering::Relaxed);
        update_max(&OBSERVED_SESSIONS_MAX, observed_sessions as u64);
    }
}

fn latency_snapshot(
    buckets: &[AtomicU64; 18],
    count: &AtomicU64,
    sum_us: &AtomicU64,
    max_us: &AtomicU64,
) -> AdmissionLatencySnapshot {
    let count = count.load(Ordering::Relaxed);
    let sum_us = sum_us.load(Ordering::Relaxed);
    AdmissionLatencySnapshot {
        count,
        avg_us: if count == 0 { 0 } else { sum_us / count },
        p50_us: percentile_us(buckets, count, 50),
        p95_us: percentile_us(buckets, count, 95),
        p99_us: percentile_us(buckets, count, 99),
        max_us: max_us.load(Ordering::Relaxed),
        over_500ms: buckets
            .iter()
            .enumerate()
            .filter(|(idx, _)| BUCKET_UPPER_US[*idx] > 500_000)
            .map(|(_, bucket)| bucket.load(Ordering::Relaxed))
            .sum(),
    }
}

fn record_latency(
    count: &AtomicU64,
    sum_us: &AtomicU64,
    max_us: &AtomicU64,
    buckets: &[AtomicU64; 18],
    duration: Duration,
) {
    let micros = micros_u64(duration.as_micros());
    count.fetch_add(1, Ordering::Relaxed);
    sum_us.fetch_add(micros, Ordering::Relaxed);
    update_max(max_us, micros);
    buckets[bucket_idx(micros)].fetch_add(1, Ordering::Relaxed);
}

fn percentile_us(buckets: &[AtomicU64; 18], count: u64, percentile: u64) -> u64 {
    if count == 0 {
        return 0;
    }
    let target = ((count * percentile) + 99) / 100;
    let mut seen = 0;
    for (idx, bucket) in buckets.iter().enumerate() {
        seen += bucket.load(Ordering::Relaxed);
        if seen >= target {
            return BUCKET_UPPER_US[idx];
        }
    }
    BUCKET_UPPER_US[BUCKET_UPPER_US.len() - 1]
}

fn bucket_idx(micros: u64) -> usize {
    BUCKET_UPPER_US
        .iter()
        .position(|upper| micros <= *upper)
        .unwrap_or(BUCKET_UPPER_US.len() - 1)
}

fn micros_u64(value: u128) -> u64 {
    value.try_into().unwrap_or(u64::MAX)
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

fn update_min_nonzero(counter: &AtomicU64, value: u64) {
    if value == 0 {
        return;
    }
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        if current != 0 && current <= value {
            return;
        }
        match counter.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(next) => current = next,
        }
    }
}

#[cfg(test)]
fn all_counters() -> [&'static AtomicU64; 20] {
    [
        &ATTEMPTS,
        &NO_LIMIT_ADMITS,
        &ADMITS,
        &REJECTS,
        &HARD_LIMIT_REJECTS,
        &OVERLOAD_REJECTS,
        &OVERLOAD_ENTERED,
        &OVERLOAD_CLEARED,
        &PACING_DECISIONS,
        &OBSERVED_SESSIONS_MAX,
        &PENDING_MAX,
        &HARD_LIMIT_MAX,
        &SOFT_LIMIT_MIN,
        &SOFT_LIMIT_MAX,
        &LOCK_WAIT_COUNT,
        &LOCK_WAIT_SUM_US,
        &LOCK_WAIT_MAX_US,
        &PACING_SLEEP_COUNT,
        &PACING_SLEEP_SUM_US,
        &PACING_SLEEP_MAX_US,
    ]
}

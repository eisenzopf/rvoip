//! SIP transaction/dialog diagnostics for duplicate recovery under UDP load.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::time::Duration;

const LATENCY_BUCKET_UPPER_US: [u64; 18] = [
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

static ENABLED_OVERRIDE: AtomicU8 = AtomicU8::new(0);

static DUP_INVITE_EXISTING_TX: AtomicU64 = AtomicU64::new(0);
static DUP_INVITE_CACHE_HIT: AtomicU64 = AtomicU64::new(0);
static DUP_INVITE_CACHE_MISS: AtomicU64 = AtomicU64::new(0);
static INVITE_2XX_CACHE_INSERT: AtomicU64 = AtomicU64::new(0);
static INVITE_2XX_CACHE_EXPIRED: AtomicU64 = AtomicU64::new(0);
static INVITE_2XX_PROACTIVE_RETRANSMIT: AtomicU64 = AtomicU64::new(0);
static INVITE_2XX_ACK_REMOVED: AtomicU64 = AtomicU64::new(0);
static INVITE_2XX_ACK_LATENCY_NS: AtomicU64 = AtomicU64::new(0);

static DUP_BYE_EXISTING_TX: AtomicU64 = AtomicU64::new(0);
static DUP_BYE_TOMBSTONE_HIT: AtomicU64 = AtomicU64::new(0);
static DUP_BYE_TOMBSTONE_MISS: AtomicU64 = AtomicU64::new(0);
static DUP_BYE_TERMINATED_DIALOG: AtomicU64 = AtomicU64::new(0);
static ACK_MATCHED_SESSION: AtomicU64 = AtomicU64::new(0);
static ACK_UNMATCHED_SESSION: AtomicU64 = AtomicU64::new(0);
static ACK_EVENT_DELIVERED: AtomicU64 = AtomicU64::new(0);
static BYE_200_SENT: AtomicU64 = AtomicU64::new(0);
static BYE_CLEANUP_EVENT_EMITTED: AtomicU64 = AtomicU64::new(0);
static BYE_CLEANUP_DELIVERED: AtomicU64 = AtomicU64::new(0);
static BYE_CLEANUP_SESSION_MISSING: AtomicU64 = AtomicU64::new(0);

static FIRST_INVITE_TO_200_COUNT: AtomicU64 = AtomicU64::new(0);
static FIRST_INVITE_TO_200_SUM_US: AtomicU64 = AtomicU64::new(0);
static FIRST_INVITE_TO_200_MAX_US: AtomicU64 = AtomicU64::new(0);
static FIRST_INVITE_TO_200_OVER_500MS: AtomicU64 = AtomicU64::new(0);
static FIRST_INVITE_TO_200_BUCKETS: [AtomicU64; 18] = [
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
];

static DIALOG_TO_SESSION_QUEUE_COUNT: AtomicU64 = AtomicU64::new(0);
static DIALOG_TO_SESSION_QUEUE_SUM_US: AtomicU64 = AtomicU64::new(0);
static DIALOG_TO_SESSION_QUEUE_MAX_US: AtomicU64 = AtomicU64::new(0);
static DIALOG_TO_SESSION_QUEUE_OVER_500MS: AtomicU64 = AtomicU64::new(0);
static DIALOG_TO_SESSION_QUEUE_INCOMING_CALL: AtomicU64 = AtomicU64::new(0);
static DIALOG_TO_SESSION_QUEUE_ACK_RECEIVED: AtomicU64 = AtomicU64::new(0);
static DIALOG_TO_SESSION_QUEUE_BYE_RECEIVED: AtomicU64 = AtomicU64::new(0);
static DIALOG_TO_SESSION_QUEUE_TERMINAL: AtomicU64 = AtomicU64::new(0);
static DIALOG_TO_SESSION_QUEUE_OTHER: AtomicU64 = AtomicU64::new(0);
static DIALOG_TO_SESSION_QUEUE_BUCKETS: [AtomicU64; 18] = [
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
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub duplicate_invite_existing_transaction: u64,
    pub duplicate_invite_cache_hit: u64,
    pub duplicate_invite_cache_miss: u64,
    pub invite_2xx_cache_insert: u64,
    pub invite_2xx_cache_expired: u64,
    pub invite_2xx_proactive_retransmit: u64,
    pub invite_2xx_ack_removed: u64,
    pub invite_2xx_ack_latency_ns: u64,
    pub duplicate_bye_existing_transaction: u64,
    pub duplicate_bye_tombstone_hit: u64,
    pub duplicate_bye_tombstone_miss: u64,
    pub duplicate_bye_terminated_dialog: u64,
    pub ack_matched_session: u64,
    pub ack_unmatched_session: u64,
    pub ack_event_delivered: u64,
    pub bye_200_sent: u64,
    pub bye_cleanup_event_emitted: u64,
    pub bye_cleanup_delivered: u64,
    pub bye_cleanup_session_missing: u64,
    pub first_invite_to_200_count: u64,
    pub first_invite_to_200_avg_us: u64,
    pub first_invite_to_200_p50_us: u64,
    pub first_invite_to_200_p95_us: u64,
    pub first_invite_to_200_p99_us: u64,
    pub first_invite_to_200_p999_us: u64,
    pub first_invite_to_200_max_us: u64,
    pub first_invite_to_200_over_500ms: u64,
    pub dialog_to_session_queue_count: u64,
    pub dialog_to_session_queue_avg_us: u64,
    pub dialog_to_session_queue_p50_us: u64,
    pub dialog_to_session_queue_p95_us: u64,
    pub dialog_to_session_queue_p99_us: u64,
    pub dialog_to_session_queue_p999_us: u64,
    pub dialog_to_session_queue_max_us: u64,
    pub dialog_to_session_queue_over_500ms: u64,
    pub dialog_to_session_queue_incoming_call: u64,
    pub dialog_to_session_queue_ack_received: u64,
    pub dialog_to_session_queue_bye_received: u64,
    pub dialog_to_session_queue_terminal: u64,
    pub dialog_to_session_queue_other: u64,
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
    for bucket in &FIRST_INVITE_TO_200_BUCKETS {
        bucket.store(0, Ordering::Relaxed);
    }
    for bucket in &DIALOG_TO_SESSION_QUEUE_BUCKETS {
        bucket.store(0, Ordering::Relaxed);
    }
}

pub fn snapshot() -> Snapshot {
    let first_count = FIRST_INVITE_TO_200_COUNT.load(Ordering::Relaxed);
    let first_sum = FIRST_INVITE_TO_200_SUM_US.load(Ordering::Relaxed);
    let dialog_queue_count = DIALOG_TO_SESSION_QUEUE_COUNT.load(Ordering::Relaxed);
    let dialog_queue_sum = DIALOG_TO_SESSION_QUEUE_SUM_US.load(Ordering::Relaxed);
    Snapshot {
        duplicate_invite_existing_transaction: DUP_INVITE_EXISTING_TX.load(Ordering::Relaxed),
        duplicate_invite_cache_hit: DUP_INVITE_CACHE_HIT.load(Ordering::Relaxed),
        duplicate_invite_cache_miss: DUP_INVITE_CACHE_MISS.load(Ordering::Relaxed),
        invite_2xx_cache_insert: INVITE_2XX_CACHE_INSERT.load(Ordering::Relaxed),
        invite_2xx_cache_expired: INVITE_2XX_CACHE_EXPIRED.load(Ordering::Relaxed),
        invite_2xx_proactive_retransmit: INVITE_2XX_PROACTIVE_RETRANSMIT.load(Ordering::Relaxed),
        invite_2xx_ack_removed: INVITE_2XX_ACK_REMOVED.load(Ordering::Relaxed),
        invite_2xx_ack_latency_ns: INVITE_2XX_ACK_LATENCY_NS.load(Ordering::Relaxed),
        duplicate_bye_existing_transaction: DUP_BYE_EXISTING_TX.load(Ordering::Relaxed),
        duplicate_bye_tombstone_hit: DUP_BYE_TOMBSTONE_HIT.load(Ordering::Relaxed),
        duplicate_bye_tombstone_miss: DUP_BYE_TOMBSTONE_MISS.load(Ordering::Relaxed),
        duplicate_bye_terminated_dialog: DUP_BYE_TERMINATED_DIALOG.load(Ordering::Relaxed),
        ack_matched_session: ACK_MATCHED_SESSION.load(Ordering::Relaxed),
        ack_unmatched_session: ACK_UNMATCHED_SESSION.load(Ordering::Relaxed),
        ack_event_delivered: ACK_EVENT_DELIVERED.load(Ordering::Relaxed),
        bye_200_sent: BYE_200_SENT.load(Ordering::Relaxed),
        bye_cleanup_event_emitted: BYE_CLEANUP_EVENT_EMITTED.load(Ordering::Relaxed),
        bye_cleanup_delivered: BYE_CLEANUP_DELIVERED.load(Ordering::Relaxed),
        bye_cleanup_session_missing: BYE_CLEANUP_SESSION_MISSING.load(Ordering::Relaxed),
        first_invite_to_200_count: first_count,
        first_invite_to_200_avg_us: if first_count == 0 {
            0
        } else {
            first_sum / first_count
        },
        first_invite_to_200_p50_us: percentile_us(&FIRST_INVITE_TO_200_BUCKETS, first_count, 50),
        first_invite_to_200_p95_us: percentile_us(&FIRST_INVITE_TO_200_BUCKETS, first_count, 95),
        first_invite_to_200_p99_us: percentile_us(&FIRST_INVITE_TO_200_BUCKETS, first_count, 99),
        first_invite_to_200_p999_us: percentile_per_mille_us(
            &FIRST_INVITE_TO_200_BUCKETS,
            first_count,
            999,
        ),
        first_invite_to_200_max_us: FIRST_INVITE_TO_200_MAX_US.load(Ordering::Relaxed),
        first_invite_to_200_over_500ms: FIRST_INVITE_TO_200_OVER_500MS.load(Ordering::Relaxed),
        dialog_to_session_queue_count: dialog_queue_count,
        dialog_to_session_queue_avg_us: if dialog_queue_count == 0 {
            0
        } else {
            dialog_queue_sum / dialog_queue_count
        },
        dialog_to_session_queue_p50_us: percentile_us(
            &DIALOG_TO_SESSION_QUEUE_BUCKETS,
            dialog_queue_count,
            50,
        ),
        dialog_to_session_queue_p95_us: percentile_us(
            &DIALOG_TO_SESSION_QUEUE_BUCKETS,
            dialog_queue_count,
            95,
        ),
        dialog_to_session_queue_p99_us: percentile_us(
            &DIALOG_TO_SESSION_QUEUE_BUCKETS,
            dialog_queue_count,
            99,
        ),
        dialog_to_session_queue_p999_us: percentile_per_mille_us(
            &DIALOG_TO_SESSION_QUEUE_BUCKETS,
            dialog_queue_count,
            999,
        ),
        dialog_to_session_queue_max_us: DIALOG_TO_SESSION_QUEUE_MAX_US.load(Ordering::Relaxed),
        dialog_to_session_queue_over_500ms: DIALOG_TO_SESSION_QUEUE_OVER_500MS
            .load(Ordering::Relaxed),
        dialog_to_session_queue_incoming_call: DIALOG_TO_SESSION_QUEUE_INCOMING_CALL
            .load(Ordering::Relaxed),
        dialog_to_session_queue_ack_received: DIALOG_TO_SESSION_QUEUE_ACK_RECEIVED
            .load(Ordering::Relaxed),
        dialog_to_session_queue_bye_received: DIALOG_TO_SESSION_QUEUE_BYE_RECEIVED
            .load(Ordering::Relaxed),
        dialog_to_session_queue_terminal: DIALOG_TO_SESSION_QUEUE_TERMINAL.load(Ordering::Relaxed),
        dialog_to_session_queue_other: DIALOG_TO_SESSION_QUEUE_OTHER.load(Ordering::Relaxed),
    }
}

pub fn format_summary(snapshot: &Snapshot) -> String {
    let avg_ack_ms = if snapshot.invite_2xx_ack_removed == 0 {
        0.0
    } else {
        snapshot.invite_2xx_ack_latency_ns as f64
            / snapshot.invite_2xx_ack_removed as f64
            / 1_000_000.0
    };
    format!(
        "[sip_retrans_diag] dup_invite_existing_tx={} dup_invite_cache_hit={} \
         dup_invite_cache_miss={} invite_2xx_cache_insert={} invite_2xx_cache_expired={} \
         invite_2xx_proactive_retx={} invite_2xx_ack_removed={} invite_2xx_ack_avg_ms={:.3} \
         dup_bye_existing_tx={} dup_bye_tombstone_hit={} dup_bye_tombstone_miss={} \
         dup_bye_terminated_dialog={} ack_matched={} ack_unmatched={} \
         ack_delivered={} bye_200_sent={} bye_cleanup_emitted={} bye_cleanup_delivered={} \
         bye_cleanup_missing={} first_invite_to_200=[count={} avg_us={} p50_us={} \
         p95_us={} p99_us={} p999_us={} max_us={} over_500ms={}] \
         dialog_to_session_queue=[count={} avg_us={} p50_us={} p95_us={} p99_us={} \
         p999_us={} max_us={} over_500ms={} incoming_call={} ack_received={} \
         bye_received={} terminal={} other={}]",
        snapshot.duplicate_invite_existing_transaction,
        snapshot.duplicate_invite_cache_hit,
        snapshot.duplicate_invite_cache_miss,
        snapshot.invite_2xx_cache_insert,
        snapshot.invite_2xx_cache_expired,
        snapshot.invite_2xx_proactive_retransmit,
        snapshot.invite_2xx_ack_removed,
        avg_ack_ms,
        snapshot.duplicate_bye_existing_transaction,
        snapshot.duplicate_bye_tombstone_hit,
        snapshot.duplicate_bye_tombstone_miss,
        snapshot.duplicate_bye_terminated_dialog,
        snapshot.ack_matched_session,
        snapshot.ack_unmatched_session,
        snapshot.ack_event_delivered,
        snapshot.bye_200_sent,
        snapshot.bye_cleanup_event_emitted,
        snapshot.bye_cleanup_delivered,
        snapshot.bye_cleanup_session_missing,
        snapshot.first_invite_to_200_count,
        snapshot.first_invite_to_200_avg_us,
        snapshot.first_invite_to_200_p50_us,
        snapshot.first_invite_to_200_p95_us,
        snapshot.first_invite_to_200_p99_us,
        snapshot.first_invite_to_200_p999_us,
        snapshot.first_invite_to_200_max_us,
        snapshot.first_invite_to_200_over_500ms,
        snapshot.dialog_to_session_queue_count,
        snapshot.dialog_to_session_queue_avg_us,
        snapshot.dialog_to_session_queue_p50_us,
        snapshot.dialog_to_session_queue_p95_us,
        snapshot.dialog_to_session_queue_p99_us,
        snapshot.dialog_to_session_queue_p999_us,
        snapshot.dialog_to_session_queue_max_us,
        snapshot.dialog_to_session_queue_over_500ms,
        snapshot.dialog_to_session_queue_incoming_call,
        snapshot.dialog_to_session_queue_ack_received,
        snapshot.dialog_to_session_queue_bye_received,
        snapshot.dialog_to_session_queue_terminal,
        snapshot.dialog_to_session_queue_other,
    )
}

pub(crate) fn record_duplicate_invite_existing_transaction() {
    increment(&DUP_INVITE_EXISTING_TX);
}

pub(crate) fn record_duplicate_invite_cache_hit() {
    increment(&DUP_INVITE_CACHE_HIT);
}

pub(crate) fn record_duplicate_invite_cache_miss() {
    increment(&DUP_INVITE_CACHE_MISS);
}

pub(crate) fn record_invite_2xx_cache_insert() {
    increment(&INVITE_2XX_CACHE_INSERT);
}

pub(crate) fn record_invite_2xx_cache_expired() {
    increment(&INVITE_2XX_CACHE_EXPIRED);
}

pub(crate) fn record_invite_2xx_proactive_retransmit() {
    increment(&INVITE_2XX_PROACTIVE_RETRANSMIT);
}

pub(crate) fn record_invite_2xx_ack_removed(latency: Duration) {
    if enabled() {
        INVITE_2XX_ACK_REMOVED.fetch_add(1, Ordering::Relaxed);
        INVITE_2XX_ACK_LATENCY_NS.fetch_add(ns(latency), Ordering::Relaxed);
    }
}

pub(crate) fn record_duplicate_bye_existing_transaction() {
    increment(&DUP_BYE_EXISTING_TX);
}

pub(crate) fn record_duplicate_bye_tombstone_hit() {
    increment(&DUP_BYE_TOMBSTONE_HIT);
}

pub(crate) fn record_duplicate_bye_tombstone_miss() {
    increment(&DUP_BYE_TOMBSTONE_MISS);
}

pub(crate) fn record_duplicate_bye_terminated_dialog() {
    increment(&DUP_BYE_TERMINATED_DIALOG);
}

pub(crate) fn record_ack_matched_session() {
    increment(&ACK_MATCHED_SESSION);
}

pub(crate) fn record_ack_unmatched_session() {
    increment(&ACK_UNMATCHED_SESSION);
}

pub fn record_ack_event_delivered() {
    increment(&ACK_EVENT_DELIVERED);
}

pub(crate) fn record_bye_200_sent() {
    increment(&BYE_200_SENT);
}

pub(crate) fn record_bye_cleanup_event_emitted() {
    increment(&BYE_CLEANUP_EVENT_EMITTED);
}

pub fn record_bye_cleanup_delivered() {
    increment(&BYE_CLEANUP_DELIVERED);
}

pub fn record_bye_cleanup_session_missing() {
    increment(&BYE_CLEANUP_SESSION_MISSING);
}

pub fn record_first_invite_to_200(elapsed: Duration) {
    if !enabled() {
        return;
    }
    let elapsed_us = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
    FIRST_INVITE_TO_200_COUNT.fetch_add(1, Ordering::Relaxed);
    FIRST_INVITE_TO_200_SUM_US.fetch_add(elapsed_us, Ordering::Relaxed);
    update_max(&FIRST_INVITE_TO_200_MAX_US, elapsed_us);
    if elapsed_us > 500_000 {
        FIRST_INVITE_TO_200_OVER_500MS.fetch_add(1, Ordering::Relaxed);
    }
    for (idx, upper) in LATENCY_BUCKET_UPPER_US.iter().enumerate() {
        if elapsed_us <= *upper {
            FIRST_INVITE_TO_200_BUCKETS[idx].fetch_add(1, Ordering::Relaxed);
            break;
        }
    }
}

fn increment(counter: &AtomicU64) {
    if enabled() {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn record_dialog_to_session_queue_delay(kind: &str, elapsed: Duration) {
    if !enabled() {
        return;
    }
    let elapsed_us = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
    DIALOG_TO_SESSION_QUEUE_COUNT.fetch_add(1, Ordering::Relaxed);
    DIALOG_TO_SESSION_QUEUE_SUM_US.fetch_add(elapsed_us, Ordering::Relaxed);
    update_max(&DIALOG_TO_SESSION_QUEUE_MAX_US, elapsed_us);
    if elapsed >= Duration::from_millis(500) {
        DIALOG_TO_SESSION_QUEUE_OVER_500MS.fetch_add(1, Ordering::Relaxed);
    }
    for (idx, upper) in LATENCY_BUCKET_UPPER_US.iter().enumerate() {
        if elapsed_us <= *upper {
            DIALOG_TO_SESSION_QUEUE_BUCKETS[idx].fetch_add(1, Ordering::Relaxed);
            break;
        }
    }

    match kind {
        "incoming_call" => DIALOG_TO_SESSION_QUEUE_INCOMING_CALL.fetch_add(1, Ordering::Relaxed),
        "ack_received" => DIALOG_TO_SESSION_QUEUE_ACK_RECEIVED.fetch_add(1, Ordering::Relaxed),
        "bye_received" => DIALOG_TO_SESSION_QUEUE_BYE_RECEIVED.fetch_add(1, Ordering::Relaxed),
        "call_terminated" | "call_failed" | "call_cancelled" => {
            DIALOG_TO_SESSION_QUEUE_TERMINAL.fetch_add(1, Ordering::Relaxed)
        }
        _ => DIALOG_TO_SESSION_QUEUE_OTHER.fetch_add(1, Ordering::Relaxed),
    };
}

fn update_max(counter: &AtomicU64, value: u64) {
    let mut current = counter.load(Ordering::Relaxed);
    while value > current {
        match counter.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}

fn ns(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

fn percentile_us(buckets: &[AtomicU64], observed: u64, percentile: u64) -> u64 {
    percentile_per_mille_us(buckets, observed, percentile * 10)
}

fn percentile_per_mille_us(buckets: &[AtomicU64], observed: u64, per_mille: u64) -> u64 {
    if observed == 0 {
        return 0;
    }
    let rank = observed.saturating_mul(per_mille).saturating_add(999) / 1000;
    let mut seen = 0;
    for (idx, bucket) in buckets.iter().enumerate() {
        seen += bucket.load(Ordering::Relaxed);
        if seen >= rank {
            return LATENCY_BUCKET_UPPER_US[idx];
        }
    }
    FIRST_INVITE_TO_200_MAX_US.load(Ordering::Relaxed)
}

fn all_counters() -> [&'static AtomicU64; 32] {
    [
        &DUP_INVITE_EXISTING_TX,
        &DUP_INVITE_CACHE_HIT,
        &DUP_INVITE_CACHE_MISS,
        &INVITE_2XX_CACHE_INSERT,
        &INVITE_2XX_CACHE_EXPIRED,
        &INVITE_2XX_PROACTIVE_RETRANSMIT,
        &INVITE_2XX_ACK_REMOVED,
        &INVITE_2XX_ACK_LATENCY_NS,
        &DUP_BYE_EXISTING_TX,
        &DUP_BYE_TOMBSTONE_HIT,
        &DUP_BYE_TOMBSTONE_MISS,
        &DUP_BYE_TERMINATED_DIALOG,
        &ACK_MATCHED_SESSION,
        &ACK_UNMATCHED_SESSION,
        &ACK_EVENT_DELIVERED,
        &BYE_200_SENT,
        &BYE_CLEANUP_EVENT_EMITTED,
        &BYE_CLEANUP_DELIVERED,
        &BYE_CLEANUP_SESSION_MISSING,
        &FIRST_INVITE_TO_200_COUNT,
        &FIRST_INVITE_TO_200_SUM_US,
        &FIRST_INVITE_TO_200_MAX_US,
        &FIRST_INVITE_TO_200_OVER_500MS,
        &DIALOG_TO_SESSION_QUEUE_COUNT,
        &DIALOG_TO_SESSION_QUEUE_SUM_US,
        &DIALOG_TO_SESSION_QUEUE_MAX_US,
        &DIALOG_TO_SESSION_QUEUE_OVER_500MS,
        &DIALOG_TO_SESSION_QUEUE_INCOMING_CALL,
        &DIALOG_TO_SESSION_QUEUE_ACK_RECEIVED,
        &DIALOG_TO_SESSION_QUEUE_BYE_RECEIVED,
        &DIALOG_TO_SESSION_QUEUE_TERMINAL,
        &DIALOG_TO_SESSION_QUEUE_OTHER,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retransmission_diagnostics_format_counts() {
        set_enabled_for_tests(true);
        reset();

        record_duplicate_invite_existing_transaction();
        record_duplicate_invite_cache_hit();
        record_duplicate_invite_cache_miss();
        record_invite_2xx_cache_insert();
        record_invite_2xx_cache_expired();
        record_invite_2xx_proactive_retransmit();
        record_invite_2xx_ack_removed(Duration::from_millis(5));
        record_duplicate_bye_existing_transaction();
        record_duplicate_bye_tombstone_hit();
        record_duplicate_bye_tombstone_miss();
        record_duplicate_bye_terminated_dialog();

        let snapshot = snapshot();
        assert_eq!(snapshot.duplicate_invite_existing_transaction, 1);
        assert_eq!(snapshot.duplicate_invite_cache_hit, 1);
        assert_eq!(snapshot.duplicate_bye_tombstone_hit, 1);
        let summary = format_summary(&snapshot);
        assert!(summary.contains("dup_invite_cache_hit=1"));
        assert!(summary.contains("dup_bye_tombstone_hit=1"));
    }
}

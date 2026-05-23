//! SIP transaction/dialog diagnostics for duplicate recovery under UDP load.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::time::Duration;

macro_rules! latency_buckets {
    () => {
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
    };
}

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
static TRANSACTION_TIMING_ENABLED: AtomicU8 = AtomicU8::new(0);

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

static UDP_RECEIVE_TO_INCOMING_CALL_EMIT_COUNT: AtomicU64 = AtomicU64::new(0);
static UDP_RECEIVE_TO_INCOMING_CALL_EMIT_SUM_US: AtomicU64 = AtomicU64::new(0);
static UDP_RECEIVE_TO_INCOMING_CALL_EMIT_MAX_US: AtomicU64 = AtomicU64::new(0);
static UDP_RECEIVE_TO_INCOMING_CALL_EMIT_OVER_500MS: AtomicU64 = AtomicU64::new(0);
static UDP_RECEIVE_TO_INCOMING_CALL_EMIT_BUCKETS: [AtomicU64; 18] = [
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

static BYE_RECEIVE_TO_200_COUNT: AtomicU64 = AtomicU64::new(0);
static BYE_RECEIVE_TO_200_SUM_US: AtomicU64 = AtomicU64::new(0);
static BYE_RECEIVE_TO_200_MAX_US: AtomicU64 = AtomicU64::new(0);
static BYE_RECEIVE_TO_200_OVER_500MS: AtomicU64 = AtomicU64::new(0);
static BYE_RECEIVE_TO_200_BUCKETS: [AtomicU64; 18] = [
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

struct LatencyMetric {
    count: AtomicU64,
    sum_us: AtomicU64,
    max_us: AtomicU64,
    over_500ms: AtomicU64,
    buckets: [AtomicU64; 18],
}

impl LatencyMetric {
    const fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
            sum_us: AtomicU64::new(0),
            max_us: AtomicU64::new(0),
            over_500ms: AtomicU64::new(0),
            buckets: latency_buckets!(),
        }
    }

    fn reset(&self) {
        self.count.store(0, Ordering::Relaxed);
        self.sum_us.store(0, Ordering::Relaxed);
        self.max_us.store(0, Ordering::Relaxed);
        self.over_500ms.store(0, Ordering::Relaxed);
        for bucket in &self.buckets {
            bucket.store(0, Ordering::Relaxed);
        }
    }

    fn record(&self, elapsed: Duration) {
        record_latency(
            elapsed,
            &self.count,
            &self.sum_us,
            &self.max_us,
            &self.over_500ms,
            &self.buckets,
        );
    }

    fn snapshot(&self) -> LatencySnapshot {
        latency_snapshot(
            &self.buckets,
            &self.count,
            &self.sum_us,
            &self.max_us,
            &self.over_500ms,
        )
    }
}

static TRANSACTION_DISPATCH_QUEUE: LatencyMetric = LatencyMetric::new();
static TRANSACTION_HANDLER_TOTAL: LatencyMetric = LatencyMetric::new();
static TRANSACTION_HANDLER_INVITE: LatencyMetric = LatencyMetric::new();
static TRANSACTION_HANDLER_ACK: LatencyMetric = LatencyMetric::new();
static TRANSACTION_HANDLER_BYE: LatencyMetric = LatencyMetric::new();
static TRANSACTION_HANDLER_CANCEL: LatencyMetric = LatencyMetric::new();
static TRANSACTION_HANDLER_OTHER: LatencyMetric = LatencyMetric::new();
static SERVER_TRANSACTION_CREATE: LatencyMetric = LatencyMetric::new();
static EXISTING_TRANSACTION_DISPATCH: LatencyMetric = LatencyMetric::new();
static TRANSACTION_EVENT_BROADCAST: LatencyMetric = LatencyMetric::new();

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LatencySnapshot {
    pub count: u64,
    pub avg_us: u64,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub p999_us: u64,
    pub max_us: u64,
    pub over_500ms: u64,
}

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
    pub udp_receive_to_incoming_call_emit: LatencySnapshot,
    pub bye_receive_to_200: LatencySnapshot,
    pub transaction_dispatch_queue: LatencySnapshot,
    pub transaction_handler_total: LatencySnapshot,
    pub transaction_handler_invite: LatencySnapshot,
    pub transaction_handler_ack: LatencySnapshot,
    pub transaction_handler_bye: LatencySnapshot,
    pub transaction_handler_cancel: LatencySnapshot,
    pub transaction_handler_other: LatencySnapshot,
    pub server_transaction_create: LatencySnapshot,
    pub existing_transaction_dispatch: LatencySnapshot,
    pub transaction_event_broadcast: LatencySnapshot,
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

pub fn transaction_timing_enabled() -> bool {
    enabled()
        && match TRANSACTION_TIMING_ENABLED.load(Ordering::Relaxed) {
            2 => true,
            _ => false,
        }
}

pub fn set_transaction_timing_enabled(enabled: bool) {
    TRANSACTION_TIMING_ENABLED.store(if enabled { 2 } else { 1 }, Ordering::Relaxed);
}

#[cfg(test)]
fn set_enabled_for_tests(enabled: bool) {
    set_enabled(enabled);
}

#[cfg(test)]
fn set_transaction_timing_enabled_for_tests(enabled: bool) {
    set_transaction_timing_enabled(enabled);
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
    for bucket in &UDP_RECEIVE_TO_INCOMING_CALL_EMIT_BUCKETS {
        bucket.store(0, Ordering::Relaxed);
    }
    for bucket in &BYE_RECEIVE_TO_200_BUCKETS {
        bucket.store(0, Ordering::Relaxed);
    }
    for metric in transaction_latency_metrics() {
        metric.reset();
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
        udp_receive_to_incoming_call_emit: latency_snapshot(
            &UDP_RECEIVE_TO_INCOMING_CALL_EMIT_BUCKETS,
            &UDP_RECEIVE_TO_INCOMING_CALL_EMIT_COUNT,
            &UDP_RECEIVE_TO_INCOMING_CALL_EMIT_SUM_US,
            &UDP_RECEIVE_TO_INCOMING_CALL_EMIT_MAX_US,
            &UDP_RECEIVE_TO_INCOMING_CALL_EMIT_OVER_500MS,
        ),
        bye_receive_to_200: latency_snapshot(
            &BYE_RECEIVE_TO_200_BUCKETS,
            &BYE_RECEIVE_TO_200_COUNT,
            &BYE_RECEIVE_TO_200_SUM_US,
            &BYE_RECEIVE_TO_200_MAX_US,
            &BYE_RECEIVE_TO_200_OVER_500MS,
        ),
        transaction_dispatch_queue: TRANSACTION_DISPATCH_QUEUE.snapshot(),
        transaction_handler_total: TRANSACTION_HANDLER_TOTAL.snapshot(),
        transaction_handler_invite: TRANSACTION_HANDLER_INVITE.snapshot(),
        transaction_handler_ack: TRANSACTION_HANDLER_ACK.snapshot(),
        transaction_handler_bye: TRANSACTION_HANDLER_BYE.snapshot(),
        transaction_handler_cancel: TRANSACTION_HANDLER_CANCEL.snapshot(),
        transaction_handler_other: TRANSACTION_HANDLER_OTHER.snapshot(),
        server_transaction_create: SERVER_TRANSACTION_CREATE.snapshot(),
        existing_transaction_dispatch: EXISTING_TRANSACTION_DISPATCH.snapshot(),
        transaction_event_broadcast: TRANSACTION_EVENT_BROADCAST.snapshot(),
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
         bye_received={} terminal={} other={}] \
         udp_receive_to_incoming_call_emit=[{}] bye_receive_to_200=[{}] \
         transaction_dispatch_queue=[{}] transaction_handler=[total=[{}] invite=[{}] \
         ack=[{}] bye=[{}] cancel=[{}] other=[{}]] server_transaction_create=[{}] \
         existing_transaction_dispatch=[{}] transaction_event_broadcast=[{}]",
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
        format_latency(&snapshot.udp_receive_to_incoming_call_emit),
        format_latency(&snapshot.bye_receive_to_200),
        format_latency(&snapshot.transaction_dispatch_queue),
        format_latency(&snapshot.transaction_handler_total),
        format_latency(&snapshot.transaction_handler_invite),
        format_latency(&snapshot.transaction_handler_ack),
        format_latency(&snapshot.transaction_handler_bye),
        format_latency(&snapshot.transaction_handler_cancel),
        format_latency(&snapshot.transaction_handler_other),
        format_latency(&snapshot.server_transaction_create),
        format_latency(&snapshot.existing_transaction_dispatch),
        format_latency(&snapshot.transaction_event_broadcast),
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

pub fn record_udp_receive_to_incoming_call_emit(elapsed: Duration) {
    record_latency(
        elapsed,
        &UDP_RECEIVE_TO_INCOMING_CALL_EMIT_COUNT,
        &UDP_RECEIVE_TO_INCOMING_CALL_EMIT_SUM_US,
        &UDP_RECEIVE_TO_INCOMING_CALL_EMIT_MAX_US,
        &UDP_RECEIVE_TO_INCOMING_CALL_EMIT_OVER_500MS,
        &UDP_RECEIVE_TO_INCOMING_CALL_EMIT_BUCKETS,
    );
}

pub fn record_bye_receive_to_200(elapsed: Duration) {
    record_latency(
        elapsed,
        &BYE_RECEIVE_TO_200_COUNT,
        &BYE_RECEIVE_TO_200_SUM_US,
        &BYE_RECEIVE_TO_200_MAX_US,
        &BYE_RECEIVE_TO_200_OVER_500MS,
        &BYE_RECEIVE_TO_200_BUCKETS,
    );
}

pub(crate) fn record_transaction_dispatch_queue_delay(elapsed: Duration) {
    if transaction_timing_enabled() {
        TRANSACTION_DISPATCH_QUEUE.record(elapsed);
    }
}

pub(crate) fn record_transaction_handler(kind: &str, elapsed: Duration) {
    if !transaction_timing_enabled() {
        return;
    }
    TRANSACTION_HANDLER_TOTAL.record(elapsed);
    match kind {
        "invite" => TRANSACTION_HANDLER_INVITE.record(elapsed),
        "ack" => TRANSACTION_HANDLER_ACK.record(elapsed),
        "bye" => TRANSACTION_HANDLER_BYE.record(elapsed),
        "cancel" => TRANSACTION_HANDLER_CANCEL.record(elapsed),
        _ => TRANSACTION_HANDLER_OTHER.record(elapsed),
    }
}

pub(crate) fn record_server_transaction_create(elapsed: Duration) {
    if transaction_timing_enabled() {
        SERVER_TRANSACTION_CREATE.record(elapsed);
    }
}

pub(crate) fn record_existing_transaction_dispatch(elapsed: Duration) {
    if transaction_timing_enabled() {
        EXISTING_TRANSACTION_DISPATCH.record(elapsed);
    }
}

pub(crate) fn record_transaction_event_broadcast(elapsed: Duration) {
    if transaction_timing_enabled() {
        TRANSACTION_EVENT_BROADCAST.record(elapsed);
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

fn record_latency(
    elapsed: Duration,
    count: &AtomicU64,
    sum_us: &AtomicU64,
    max_us: &AtomicU64,
    over_500ms: &AtomicU64,
    buckets: &[AtomicU64; 18],
) {
    if !enabled() {
        return;
    }
    let elapsed_us = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
    count.fetch_add(1, Ordering::Relaxed);
    sum_us.fetch_add(elapsed_us, Ordering::Relaxed);
    update_max(max_us, elapsed_us);
    if elapsed_us > 500_000 {
        over_500ms.fetch_add(1, Ordering::Relaxed);
    }
    for (idx, upper) in LATENCY_BUCKET_UPPER_US.iter().enumerate() {
        if elapsed_us <= *upper {
            buckets[idx].fetch_add(1, Ordering::Relaxed);
            break;
        }
    }
}

fn latency_snapshot(
    buckets: &[AtomicU64; 18],
    count: &AtomicU64,
    sum_us: &AtomicU64,
    max_us: &AtomicU64,
    over_500ms: &AtomicU64,
) -> LatencySnapshot {
    let count = count.load(Ordering::Relaxed);
    let sum_us = sum_us.load(Ordering::Relaxed);
    LatencySnapshot {
        count,
        avg_us: if count == 0 { 0 } else { sum_us / count },
        p50_us: percentile_us(buckets, count, 50),
        p95_us: percentile_us(buckets, count, 95),
        p99_us: percentile_us(buckets, count, 99),
        p999_us: percentile_per_mille_us(buckets, count, 999),
        max_us: max_us.load(Ordering::Relaxed),
        over_500ms: over_500ms.load(Ordering::Relaxed),
    }
}

fn format_latency(latency: &LatencySnapshot) -> String {
    format!(
        "count={} avg_us={} p50_us={} p95_us={} p99_us={} p999_us={} max_us={} over_500ms={}",
        latency.count,
        latency.avg_us,
        latency.p50_us,
        latency.p95_us,
        latency.p99_us,
        latency.p999_us,
        latency.max_us,
        latency.over_500ms,
    )
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

fn all_counters() -> [&'static AtomicU64; 40] {
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
        &UDP_RECEIVE_TO_INCOMING_CALL_EMIT_COUNT,
        &UDP_RECEIVE_TO_INCOMING_CALL_EMIT_SUM_US,
        &UDP_RECEIVE_TO_INCOMING_CALL_EMIT_MAX_US,
        &UDP_RECEIVE_TO_INCOMING_CALL_EMIT_OVER_500MS,
        &BYE_RECEIVE_TO_200_COUNT,
        &BYE_RECEIVE_TO_200_SUM_US,
        &BYE_RECEIVE_TO_200_MAX_US,
        &BYE_RECEIVE_TO_200_OVER_500MS,
    ]
}

fn transaction_latency_metrics() -> [&'static LatencyMetric; 10] {
    [
        &TRANSACTION_DISPATCH_QUEUE,
        &TRANSACTION_HANDLER_TOTAL,
        &TRANSACTION_HANDLER_INVITE,
        &TRANSACTION_HANDLER_ACK,
        &TRANSACTION_HANDLER_BYE,
        &TRANSACTION_HANDLER_CANCEL,
        &TRANSACTION_HANDLER_OTHER,
        &SERVER_TRANSACTION_CREATE,
        &EXISTING_TRANSACTION_DISPATCH,
        &TRANSACTION_EVENT_BROADCAST,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retransmission_diagnostics_format_counts() {
        set_enabled_for_tests(false);
        set_transaction_timing_enabled_for_tests(false);
        reset();
        record_duplicate_invite_existing_transaction();
        record_udp_receive_to_incoming_call_emit(Duration::from_micros(125));
        record_transaction_dispatch_queue_delay(Duration::from_micros(75));
        record_transaction_handler("invite", Duration::from_micros(100));
        record_server_transaction_create(Duration::from_micros(350));
        record_existing_transaction_dispatch(Duration::from_micros(400));
        record_transaction_event_broadcast(Duration::from_micros(450));
        let disabled = snapshot();
        assert_eq!(disabled.duplicate_invite_existing_transaction, 0);
        assert_eq!(disabled.udp_receive_to_incoming_call_emit.count, 0);
        assert_eq!(disabled.transaction_dispatch_queue.count, 0);
        assert_eq!(disabled.transaction_handler_total.count, 0);
        assert_eq!(disabled.server_transaction_create.count, 0);
        assert_eq!(disabled.existing_transaction_dispatch.count, 0);
        assert_eq!(disabled.transaction_event_broadcast.count, 0);

        set_enabled_for_tests(true);
        set_transaction_timing_enabled_for_tests(false);
        reset();

        record_transaction_dispatch_queue_delay(Duration::from_micros(75));
        record_transaction_handler("invite", Duration::from_micros(100));
        record_server_transaction_create(Duration::from_micros(350));
        record_existing_transaction_dispatch(Duration::from_micros(400));
        record_transaction_event_broadcast(Duration::from_micros(450));
        let transaction_disabled = snapshot();
        assert_eq!(transaction_disabled.transaction_dispatch_queue.count, 0);
        assert_eq!(transaction_disabled.transaction_handler_total.count, 0);
        assert_eq!(transaction_disabled.server_transaction_create.count, 0);
        assert_eq!(transaction_disabled.existing_transaction_dispatch.count, 0);
        assert_eq!(transaction_disabled.transaction_event_broadcast.count, 0);

        set_transaction_timing_enabled_for_tests(true);
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
        record_udp_receive_to_incoming_call_emit(Duration::from_micros(125));
        record_bye_receive_to_200(Duration::from_micros(250));
        record_transaction_dispatch_queue_delay(Duration::from_micros(75));
        record_transaction_handler("invite", Duration::from_micros(100));
        record_transaction_handler("ack", Duration::from_micros(150));
        record_transaction_handler("bye", Duration::from_micros(200));
        record_transaction_handler("cancel", Duration::from_micros(250));
        record_transaction_handler("other", Duration::from_micros(300));
        record_server_transaction_create(Duration::from_micros(350));
        record_existing_transaction_dispatch(Duration::from_micros(400));
        record_transaction_event_broadcast(Duration::from_micros(450));

        let snapshot = snapshot();
        assert_eq!(snapshot.duplicate_invite_existing_transaction, 1);
        assert_eq!(snapshot.duplicate_invite_cache_hit, 1);
        assert_eq!(snapshot.duplicate_bye_tombstone_hit, 1);
        assert_eq!(snapshot.udp_receive_to_incoming_call_emit.count, 1);
        assert_eq!(snapshot.bye_receive_to_200.count, 1);
        assert_eq!(snapshot.transaction_dispatch_queue.count, 1);
        assert_eq!(snapshot.transaction_handler_total.count, 5);
        assert_eq!(snapshot.transaction_handler_invite.count, 1);
        assert_eq!(snapshot.transaction_handler_ack.count, 1);
        assert_eq!(snapshot.transaction_handler_bye.count, 1);
        assert_eq!(snapshot.transaction_handler_cancel.count, 1);
        assert_eq!(snapshot.transaction_handler_other.count, 1);
        assert_eq!(snapshot.server_transaction_create.count, 1);
        assert_eq!(snapshot.existing_transaction_dispatch.count, 1);
        assert_eq!(snapshot.transaction_event_broadcast.count, 1);
        let summary = format_summary(&snapshot);
        assert!(summary.contains("dup_invite_cache_hit=1"));
        assert!(summary.contains("dup_bye_tombstone_hit=1"));
        assert!(summary.contains("udp_receive_to_incoming_call_emit=[count=1"));
        assert!(summary.contains("bye_receive_to_200=[count=1"));
        assert!(summary.contains("transaction_dispatch_queue=[count=1"));
        assert!(summary.contains("transaction_handler=[total=[count=5"));
    }
}

//! SIP transaction/dialog diagnostics for duplicate recovery under UDP load.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

pub(crate) mod safe_log;

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
    10, 25, 50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 25_000, 50_000, 100_000, 250_000,
    500_000, 1_000_000, 2_500_000, 5_000_000,
];
const MAX_TRANSACTION_DISPATCH_DIAG_WORKERS: usize = 64;
const MAX_CALL_TIMING_TRACE_ENTRIES: usize = 20_000;

static ENABLED_OVERRIDE: AtomicU8 = AtomicU8::new(0);
static TRANSACTION_TIMING_ENABLED: AtomicU8 = AtomicU8::new(0);
static DIALOG_TIMING_ENABLED: AtomicU8 = AtomicU8::new(0);
static CALL_TIMING_TRACE_OVERFLOW: AtomicU64 = AtomicU64::new(0);
static CALL_TIMING_TRACES: LazyLock<Mutex<HashMap<String, CallTimingTraceCounts>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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

static OK_200_INVITE_FIRST: AtomicU64 = AtomicU64::new(0);
static OK_200_INVITE_DUPLICATE_CACHE: AtomicU64 = AtomicU64::new(0);
static OK_200_INVITE_PROACTIVE_RETRANSMIT: AtomicU64 = AtomicU64::new(0);
static OK_200_BYE_FRESH: AtomicU64 = AtomicU64::new(0);
static OK_200_BYE_TOMBSTONE: AtomicU64 = AtomicU64::new(0);
static OK_200_BYE_DUPLICATE_TERMINATED: AtomicU64 = AtomicU64::new(0);
static OK_200_OTHER: AtomicU64 = AtomicU64::new(0);

static UAC_INVITE_2XX_RESPONSE: AtomicU64 = AtomicU64::new(0);
static UAC_INVITE_2XX_ACK_ATTEMPT: AtomicU64 = AtomicU64::new(0);
static UAC_INVITE_2XX_ACK_SUCCESS: AtomicU64 = AtomicU64::new(0);
static UAC_INVITE_2XX_ACK_FAILURE: AtomicU64 = AtomicU64::new(0);
static UAC_INVITE_2XX_CALL_ANSWERED_EMIT: AtomicU64 = AtomicU64::new(0);
static HUB_RESPONSE_INVITE_2XX: AtomicU64 = AtomicU64::new(0);
static HUB_RESPONSE_INVITE_2XX_SESSION_FOUND: AtomicU64 = AtomicU64::new(0);
static HUB_RESPONSE_INVITE_2XX_SESSION_MISSING: AtomicU64 = AtomicU64::new(0);
static HUB_CALL_ANSWERED: AtomicU64 = AtomicU64::new(0);
static HUB_CALL_ANSWERED_SESSION_FOUND: AtomicU64 = AtomicU64::new(0);
static HUB_CALL_ANSWERED_SESSION_MISSING: AtomicU64 = AtomicU64::new(0);
static HUB_ACK_SENT: AtomicU64 = AtomicU64::new(0);
static HUB_ACK_SENT_SESSION_FOUND: AtomicU64 = AtomicU64::new(0);
static HUB_ACK_SENT_SESSION_MISSING: AtomicU64 = AtomicU64::new(0);

static DIALOG_ROUTE_REQUEST: AtomicU64 = AtomicU64::new(0);
static DIALOG_ROUTE_STORED: AtomicU64 = AtomicU64::new(0);
static DIALOG_ROUTE_TRANSACTION_KEY: AtomicU64 = AtomicU64::new(0);
static DIALOG_ROUTE_FALLBACK: AtomicU64 = AtomicU64::new(0);
static DIALOG_ROUTE_WORKER_MISMATCH: AtomicU64 = AtomicU64::new(0);
static DIALOG_ROUTE_INVITE: AtomicU64 = AtomicU64::new(0);
static DIALOG_ROUTE_ACK: AtomicU64 = AtomicU64::new(0);
static DIALOG_ROUTE_BYE: AtomicU64 = AtomicU64::new(0);
static DIALOG_ROUTE_CANCEL: AtomicU64 = AtomicU64::new(0);
static DIALOG_ROUTE_LIFECYCLE: AtomicU64 = AtomicU64::new(0);
static DIALOG_ROUTE_OTHER: AtomicU64 = AtomicU64::new(0);

static TERMINATION_CLEANUP_ENQUEUED: AtomicU64 = AtomicU64::new(0);
static TERMINATION_CLEANUP_QUEUE_FULL: AtomicU64 = AtomicU64::new(0);
static TERMINATION_CLEANUP_WORKER_SPAWNED: AtomicU64 = AtomicU64::new(0);
static TERMINATION_CLEANUP_IN_FLIGHT: AtomicU64 = AtomicU64::new(0);
static TERMINATION_CLEANUP_MAX_IN_FLIGHT: AtomicU64 = AtomicU64::new(0);
static TERMINATION_CLEANUP_POLL_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static TERMINATION_CLEANUP_REMOVED: AtomicU64 = AtomicU64::new(0);
static TERMINATION_CLEANUP_BATCHES: AtomicU64 = AtomicU64::new(0);
static TERMINATION_CLEANUP_BATCH_TOTAL: AtomicU64 = AtomicU64::new(0);
static TERMINATION_CLEANUP_BATCH_MAX: AtomicU64 = AtomicU64::new(0);
static TERMINATION_CLEANUP_INDEXED_SCAN_KEYS: AtomicU64 = AtomicU64::new(0);
static TERMINATION_CLEANUP_FULL_SCAN_CLIENT_KEYS: AtomicU64 = AtomicU64::new(0);
static TERMINATION_CLEANUP_FULL_SCAN_SERVER_KEYS: AtomicU64 = AtomicU64::new(0);

static INVITE_2XX_MAINTENANCE_TICKS: AtomicU64 = AtomicU64::new(0);
static INVITE_2XX_MAINTENANCE_CACHE_LEN_TOTAL: AtomicU64 = AtomicU64::new(0);
static INVITE_2XX_MAINTENANCE_CACHE_LEN_MAX: AtomicU64 = AtomicU64::new(0);
static INVITE_2XX_MAINTENANCE_DUE_QUEUE_LEN_TOTAL: AtomicU64 = AtomicU64::new(0);
static INVITE_2XX_MAINTENANCE_DUE_QUEUE_LEN_MAX: AtomicU64 = AtomicU64::new(0);
static INVITE_2XX_MAINTENANCE_SCANNED: AtomicU64 = AtomicU64::new(0);
static INVITE_2XX_MAINTENANCE_DUE: AtomicU64 = AtomicU64::new(0);
static INVITE_2XX_MAINTENANCE_EXPIRED: AtomicU64 = AtomicU64::new(0);
static INVITE_2XX_MAINTENANCE_CAPPED_TICKS: AtomicU64 = AtomicU64::new(0);

static GLOBAL_PUBLISH_COUNT: AtomicU64 = AtomicU64::new(0);
static GLOBAL_PUBLISH_HANDLER_COUNT_TOTAL: AtomicU64 = AtomicU64::new(0);
static GLOBAL_PUBLISH_HANDLER_COUNT_MAX: AtomicU64 = AtomicU64::new(0);
static GLOBAL_PUBLISH_INCOMING_CALL: AtomicU64 = AtomicU64::new(0);
static GLOBAL_PUBLISH_ACK: AtomicU64 = AtomicU64::new(0);
static GLOBAL_PUBLISH_BYE: AtomicU64 = AtomicU64::new(0);
static GLOBAL_PUBLISH_OTHER: AtomicU64 = AtomicU64::new(0);

static TRANSACTION_DISPATCH_QUEUE_DEPTH_TOTAL: AtomicU64 = AtomicU64::new(0);
static TRANSACTION_DISPATCH_QUEUE_DEPTH_MAX: AtomicU64 = AtomicU64::new(0);
static TRANSACTION_RUNNER_STARTED: AtomicU64 = AtomicU64::new(0);
static TRANSACTION_RUNNER_EXITED: AtomicU64 = AtomicU64::new(0);
static TRANSACTION_RUNNER_ACTIVE: AtomicU64 = AtomicU64::new(0);
static TRANSACTION_RUNNER_ACTIVE_MAX: AtomicU64 = AtomicU64::new(0);
static TRANSACTION_RUNNER_DESTROY_WAKE_SENT: AtomicU64 = AtomicU64::new(0);
static TRANSACTION_RUNNER_DESTROY_WAKE_FAILED: AtomicU64 = AtomicU64::new(0);

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

static BYE_TOMBSTONE_TABLE_SIZE_MAX: AtomicU64 = AtomicU64::new(0);

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

struct WorkerDispatchMetric {
    queue: LatencyMetric,
    depth_max: AtomicU64,
}

impl WorkerDispatchMetric {
    fn new() -> Self {
        Self {
            queue: LatencyMetric::new(),
            depth_max: AtomicU64::new(0),
        }
    }

    fn reset(&self) {
        self.queue.reset();
        self.depth_max.store(0, Ordering::Relaxed);
    }

    fn snapshot(&self, worker_id: usize) -> TransactionDispatchWorkerSnapshot {
        TransactionDispatchWorkerSnapshot {
            worker_id,
            queue: self.queue.snapshot(),
            depth_max: self.depth_max.load(Ordering::Relaxed),
        }
    }
}

static TRANSACTION_DISPATCH_QUEUE: LatencyMetric = LatencyMetric::new();
static TRANSACTION_DISPATCH_QUEUE_INVITE: LatencyMetric = LatencyMetric::new();
static TRANSACTION_DISPATCH_QUEUE_ACK: LatencyMetric = LatencyMetric::new();
static TRANSACTION_DISPATCH_QUEUE_BYE: LatencyMetric = LatencyMetric::new();
static TRANSACTION_DISPATCH_QUEUE_CANCEL: LatencyMetric = LatencyMetric::new();
static TRANSACTION_DISPATCH_QUEUE_OTHER: LatencyMetric = LatencyMetric::new();
static TRANSACTION_DISPATCH_QUEUE_WORKERS: OnceLock<Vec<WorkerDispatchMetric>> = OnceLock::new();
static TRANSACTION_HANDLER_TOTAL: LatencyMetric = LatencyMetric::new();
static TRANSACTION_HANDLER_INVITE: LatencyMetric = LatencyMetric::new();
static TRANSACTION_HANDLER_ACK: LatencyMetric = LatencyMetric::new();
static TRANSACTION_HANDLER_BYE: LatencyMetric = LatencyMetric::new();
static TRANSACTION_HANDLER_CANCEL: LatencyMetric = LatencyMetric::new();
static TRANSACTION_HANDLER_OTHER: LatencyMetric = LatencyMetric::new();
static SERVER_TRANSACTION_CREATE: LatencyMetric = LatencyMetric::new();
static EXISTING_TRANSACTION_DISPATCH: LatencyMetric = LatencyMetric::new();
static TRANSACTION_EVENT_BROADCAST: LatencyMetric = LatencyMetric::new();
static TRANSACTION_DISPATCH_BACKPRESSURE: LatencyMetric = LatencyMetric::new();
static UDP_RECEIVE_TO_INVITE_200: LatencyMetric = LatencyMetric::new();
static DIALOG_EVENT_DISPATCH_QUEUE: LatencyMetric = LatencyMetric::new();
static DIALOG_EVENT_DISPATCH_BACKPRESSURE: LatencyMetric = LatencyMetric::new();
static DIALOG_EVENT_HANDLER_TOTAL: LatencyMetric = LatencyMetric::new();
static DIALOG_EVENT_HANDLER_INVITE: LatencyMetric = LatencyMetric::new();
static DIALOG_EVENT_HANDLER_ACK: LatencyMetric = LatencyMetric::new();
static DIALOG_EVENT_HANDLER_BYE: LatencyMetric = LatencyMetric::new();
static DIALOG_EVENT_HANDLER_CANCEL: LatencyMetric = LatencyMetric::new();
static DIALOG_EVENT_HANDLER_OTHER: LatencyMetric = LatencyMetric::new();
static DIALOG_SESSION_PUBLISH_TOTAL: LatencyMetric = LatencyMetric::new();
static DIALOG_SESSION_PUBLISH_INCOMING_CALL: LatencyMetric = LatencyMetric::new();
static DIALOG_SESSION_PUBLISH_ACK: LatencyMetric = LatencyMetric::new();
static DIALOG_SESSION_PUBLISH_BYE: LatencyMetric = LatencyMetric::new();
static DIALOG_SESSION_PUBLISH_OTHER: LatencyMetric = LatencyMetric::new();
static DIALOG_LOOKUP: LatencyMetric = LatencyMetric::new();
static DIALOG_INITIAL_INVITE_SETUP: LatencyMetric = LatencyMetric::new();
static BYE_PATH_UDP_TO_HANDLER: LatencyMetric = LatencyMetric::new();
static BYE_PATH_TX_RECEIVED_TO_HANDLER: LatencyMetric = LatencyMetric::new();
static BYE_PATH_HANDLER_TO_SEND_START: LatencyMetric = LatencyMetric::new();
static BYE_PATH_SEND_RESPONSE: LatencyMetric = LatencyMetric::new();
static BYE_PATH_RELEASE_TX: LatencyMetric = LatencyMetric::new();
static BYE_PATH_CLEANUP_EMIT: LatencyMetric = LatencyMetric::new();
static BYE_PATH_CLEANUP_REMOVE_STORAGE: LatencyMetric = LatencyMetric::new();
static BYE_TOMBSTONE_LOOKUP: LatencyMetric = LatencyMetric::new();
static BYE_TOMBSTONE_PRUNE: LatencyMetric = LatencyMetric::new();
static TERMINATION_CLEANUP_INDEXED_SCAN: LatencyMetric = LatencyMetric::new();
static TERMINATION_CLEANUP_FULL_SCAN: LatencyMetric = LatencyMetric::new();
static TERMINATION_CLEANUP_TIMER_UNREGISTER: LatencyMetric = LatencyMetric::new();
static INVITE_2XX_MAINTENANCE: LatencyMetric = LatencyMetric::new();
static INVITE_2XX_PROACTIVE_SEND: LatencyMetric = LatencyMetric::new();
static GLOBAL_PUBLISH_TOTAL: LatencyMetric = LatencyMetric::new();

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TransactionDispatchWorkerSnapshot {
    pub worker_id: usize,
    pub queue: LatencySnapshot,
    pub depth_max: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CallTimingTraceSnapshot {
    pub call_id: String,
    pub first_uac_invite_2xx_response_epoch_us: Option<u64>,
    pub last_uac_invite_2xx_response_epoch_us: Option<u64>,
    pub first_uac_ack_attempt_epoch_us: Option<u64>,
    pub last_uac_ack_attempt_epoch_us: Option<u64>,
    pub first_uac_ack_success_epoch_us: Option<u64>,
    pub last_uac_ack_success_epoch_us: Option<u64>,
    pub first_uac_ack_failure_epoch_us: Option<u64>,
    pub last_uac_ack_failure_epoch_us: Option<u64>,
    pub first_uac_call_answered_emit_epoch_us: Option<u64>,
    pub last_uac_call_answered_emit_epoch_us: Option<u64>,
    pub first_hub_response_invite_2xx_epoch_us: Option<u64>,
    pub last_hub_response_invite_2xx_epoch_us: Option<u64>,
    pub first_hub_call_answered_epoch_us: Option<u64>,
    pub last_hub_call_answered_epoch_us: Option<u64>,
    pub first_hub_ack_sent_epoch_us: Option<u64>,
    pub last_hub_ack_sent_epoch_us: Option<u64>,
    pub first_uas_ack_received_epoch_us: Option<u64>,
    pub last_uas_ack_received_epoch_us: Option<u64>,
    pub first_lifecycle_call_answered_epoch_us: Option<u64>,
    pub last_lifecycle_call_answered_epoch_us: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
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
    pub ok_200_invite_first: u64,
    pub ok_200_invite_duplicate_cache: u64,
    pub ok_200_invite_proactive_retransmit: u64,
    pub ok_200_bye_fresh: u64,
    pub ok_200_bye_tombstone: u64,
    pub ok_200_bye_duplicate_terminated: u64,
    pub ok_200_other: u64,
    pub uac_invite_2xx_response: u64,
    pub uac_invite_2xx_ack_attempt: u64,
    pub uac_invite_2xx_ack_success: u64,
    pub uac_invite_2xx_ack_failure: u64,
    pub uac_invite_2xx_call_answered_emit: u64,
    pub hub_response_invite_2xx: u64,
    pub hub_response_invite_2xx_session_found: u64,
    pub hub_response_invite_2xx_session_missing: u64,
    pub hub_call_answered: u64,
    pub hub_call_answered_session_found: u64,
    pub hub_call_answered_session_missing: u64,
    pub hub_ack_sent: u64,
    pub hub_ack_sent_session_found: u64,
    pub hub_ack_sent_session_missing: u64,
    pub call_timing_trace_overflow: u64,
    pub call_timing_traces: Vec<CallTimingTraceSnapshot>,
    pub dialog_route_request: u64,
    pub dialog_route_stored: u64,
    pub dialog_route_transaction_key: u64,
    pub dialog_route_fallback: u64,
    pub dialog_route_worker_mismatch: u64,
    pub dialog_route_invite: u64,
    pub dialog_route_ack: u64,
    pub dialog_route_bye: u64,
    pub dialog_route_cancel: u64,
    pub dialog_route_lifecycle: u64,
    pub dialog_route_other: u64,
    pub termination_cleanup_enqueued: u64,
    pub termination_cleanup_queue_full: u64,
    pub termination_cleanup_worker_spawned: u64,
    pub termination_cleanup_in_flight: u64,
    pub termination_cleanup_max_in_flight: u64,
    pub termination_cleanup_poll_attempts: u64,
    pub termination_cleanup_removed: u64,
    pub termination_cleanup_batches: u64,
    pub termination_cleanup_batch_total: u64,
    pub termination_cleanup_batch_max: u64,
    pub termination_cleanup_indexed_scan_keys: u64,
    pub termination_cleanup_full_scan_client_keys: u64,
    pub termination_cleanup_full_scan_server_keys: u64,
    pub invite_2xx_maintenance_ticks: u64,
    pub invite_2xx_maintenance_cache_len_total: u64,
    pub invite_2xx_maintenance_cache_len_max: u64,
    pub invite_2xx_maintenance_due_queue_len_total: u64,
    pub invite_2xx_maintenance_due_queue_len_max: u64,
    pub invite_2xx_maintenance_scanned: u64,
    pub invite_2xx_maintenance_due: u64,
    pub invite_2xx_maintenance_expired: u64,
    pub invite_2xx_maintenance_capped_ticks: u64,
    pub global_publish_count: u64,
    pub global_publish_handler_count_total: u64,
    pub global_publish_handler_count_max: u64,
    pub global_publish_incoming_call: u64,
    pub global_publish_ack: u64,
    pub global_publish_bye: u64,
    pub global_publish_other: u64,
    pub transaction_dispatch_queue_depth_total: u64,
    pub transaction_dispatch_queue_depth_max: u64,
    pub transaction_runner_started: u64,
    pub transaction_runner_exited: u64,
    pub transaction_runner_active: u64,
    pub transaction_runner_active_max: u64,
    pub transaction_runner_destroy_wake_sent: u64,
    pub transaction_runner_destroy_wake_failed: u64,
    pub bye_tombstone_table_size_max: u64,
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
    pub bye_path_udp_to_handler: LatencySnapshot,
    pub bye_path_tx_received_to_handler: LatencySnapshot,
    pub bye_path_handler_to_send_start: LatencySnapshot,
    pub bye_path_send_response: LatencySnapshot,
    pub bye_path_release_tx: LatencySnapshot,
    pub bye_path_cleanup_emit: LatencySnapshot,
    pub bye_path_cleanup_remove_storage: LatencySnapshot,
    pub bye_tombstone_lookup: LatencySnapshot,
    pub bye_tombstone_prune: LatencySnapshot,
    pub transaction_dispatch_queue: LatencySnapshot,
    pub transaction_dispatch_queue_invite: LatencySnapshot,
    pub transaction_dispatch_queue_ack: LatencySnapshot,
    pub transaction_dispatch_queue_bye: LatencySnapshot,
    pub transaction_dispatch_queue_cancel: LatencySnapshot,
    pub transaction_dispatch_queue_other: LatencySnapshot,
    pub transaction_dispatch_queue_by_worker: Vec<TransactionDispatchWorkerSnapshot>,
    pub transaction_handler_total: LatencySnapshot,
    pub transaction_handler_invite: LatencySnapshot,
    pub transaction_handler_ack: LatencySnapshot,
    pub transaction_handler_bye: LatencySnapshot,
    pub transaction_handler_cancel: LatencySnapshot,
    pub transaction_handler_other: LatencySnapshot,
    pub server_transaction_create: LatencySnapshot,
    pub existing_transaction_dispatch: LatencySnapshot,
    pub transaction_event_broadcast: LatencySnapshot,
    pub transaction_dispatch_backpressure: LatencySnapshot,
    pub udp_receive_to_invite_200: LatencySnapshot,
    pub dialog_event_dispatch_queue: LatencySnapshot,
    pub dialog_event_dispatch_backpressure: LatencySnapshot,
    pub dialog_event_handler_total: LatencySnapshot,
    pub dialog_event_handler_invite: LatencySnapshot,
    pub dialog_event_handler_ack: LatencySnapshot,
    pub dialog_event_handler_bye: LatencySnapshot,
    pub dialog_event_handler_cancel: LatencySnapshot,
    pub dialog_event_handler_other: LatencySnapshot,
    pub dialog_session_publish_total: LatencySnapshot,
    pub dialog_session_publish_incoming_call: LatencySnapshot,
    pub dialog_session_publish_ack: LatencySnapshot,
    pub dialog_session_publish_bye: LatencySnapshot,
    pub dialog_session_publish_other: LatencySnapshot,
    pub dialog_lookup: LatencySnapshot,
    pub dialog_initial_invite_setup: LatencySnapshot,
    pub termination_cleanup_indexed_scan: LatencySnapshot,
    pub termination_cleanup_full_scan: LatencySnapshot,
    pub termination_cleanup_timer_unregister: LatencySnapshot,
    pub invite_2xx_maintenance: LatencySnapshot,
    pub invite_2xx_proactive_send: LatencySnapshot,
    pub global_publish_total: LatencySnapshot,
}

#[derive(Debug, Clone, Default)]
struct CallTimingTraceCounts {
    first_uac_invite_2xx_response_epoch_us: Option<u64>,
    last_uac_invite_2xx_response_epoch_us: Option<u64>,
    first_uac_ack_attempt_epoch_us: Option<u64>,
    last_uac_ack_attempt_epoch_us: Option<u64>,
    first_uac_ack_success_epoch_us: Option<u64>,
    last_uac_ack_success_epoch_us: Option<u64>,
    first_uac_ack_failure_epoch_us: Option<u64>,
    last_uac_ack_failure_epoch_us: Option<u64>,
    first_uac_call_answered_emit_epoch_us: Option<u64>,
    last_uac_call_answered_emit_epoch_us: Option<u64>,
    first_hub_response_invite_2xx_epoch_us: Option<u64>,
    last_hub_response_invite_2xx_epoch_us: Option<u64>,
    first_hub_call_answered_epoch_us: Option<u64>,
    last_hub_call_answered_epoch_us: Option<u64>,
    first_hub_ack_sent_epoch_us: Option<u64>,
    last_hub_ack_sent_epoch_us: Option<u64>,
    first_uas_ack_received_epoch_us: Option<u64>,
    last_uas_ack_received_epoch_us: Option<u64>,
    first_lifecycle_call_answered_epoch_us: Option<u64>,
    last_lifecycle_call_answered_epoch_us: Option<u64>,
}

impl CallTimingTraceCounts {
    fn record_uac_invite_2xx_response(&mut self, now_us: u64) {
        set_first_last_u64(
            &mut self.first_uac_invite_2xx_response_epoch_us,
            &mut self.last_uac_invite_2xx_response_epoch_us,
            now_us,
        );
    }

    fn record_uac_ack_attempt(&mut self, now_us: u64) {
        set_first_last_u64(
            &mut self.first_uac_ack_attempt_epoch_us,
            &mut self.last_uac_ack_attempt_epoch_us,
            now_us,
        );
    }

    fn record_uac_ack_success(&mut self, now_us: u64) {
        set_first_last_u64(
            &mut self.first_uac_ack_success_epoch_us,
            &mut self.last_uac_ack_success_epoch_us,
            now_us,
        );
    }

    fn record_uac_ack_failure(&mut self, now_us: u64) {
        set_first_last_u64(
            &mut self.first_uac_ack_failure_epoch_us,
            &mut self.last_uac_ack_failure_epoch_us,
            now_us,
        );
    }

    fn record_uac_call_answered_emit(&mut self, now_us: u64) {
        set_first_last_u64(
            &mut self.first_uac_call_answered_emit_epoch_us,
            &mut self.last_uac_call_answered_emit_epoch_us,
            now_us,
        );
    }

    fn record_hub_response_invite_2xx(&mut self, now_us: u64) {
        set_first_last_u64(
            &mut self.first_hub_response_invite_2xx_epoch_us,
            &mut self.last_hub_response_invite_2xx_epoch_us,
            now_us,
        );
    }

    fn record_hub_call_answered(&mut self, now_us: u64) {
        set_first_last_u64(
            &mut self.first_hub_call_answered_epoch_us,
            &mut self.last_hub_call_answered_epoch_us,
            now_us,
        );
    }

    fn record_hub_ack_sent(&mut self, now_us: u64) {
        set_first_last_u64(
            &mut self.first_hub_ack_sent_epoch_us,
            &mut self.last_hub_ack_sent_epoch_us,
            now_us,
        );
    }

    fn record_uas_ack_received(&mut self, now_us: u64) {
        set_first_last_u64(
            &mut self.first_uas_ack_received_epoch_us,
            &mut self.last_uas_ack_received_epoch_us,
            now_us,
        );
    }

    fn record_lifecycle_call_answered(&mut self, now_us: u64) {
        set_first_last_u64(
            &mut self.first_lifecycle_call_answered_epoch_us,
            &mut self.last_lifecycle_call_answered_epoch_us,
            now_us,
        );
    }

    fn snapshot(&self, call_id: String) -> CallTimingTraceSnapshot {
        CallTimingTraceSnapshot {
            call_id,
            first_uac_invite_2xx_response_epoch_us: self.first_uac_invite_2xx_response_epoch_us,
            last_uac_invite_2xx_response_epoch_us: self.last_uac_invite_2xx_response_epoch_us,
            first_uac_ack_attempt_epoch_us: self.first_uac_ack_attempt_epoch_us,
            last_uac_ack_attempt_epoch_us: self.last_uac_ack_attempt_epoch_us,
            first_uac_ack_success_epoch_us: self.first_uac_ack_success_epoch_us,
            last_uac_ack_success_epoch_us: self.last_uac_ack_success_epoch_us,
            first_uac_ack_failure_epoch_us: self.first_uac_ack_failure_epoch_us,
            last_uac_ack_failure_epoch_us: self.last_uac_ack_failure_epoch_us,
            first_uac_call_answered_emit_epoch_us: self.first_uac_call_answered_emit_epoch_us,
            last_uac_call_answered_emit_epoch_us: self.last_uac_call_answered_emit_epoch_us,
            first_hub_response_invite_2xx_epoch_us: self.first_hub_response_invite_2xx_epoch_us,
            last_hub_response_invite_2xx_epoch_us: self.last_hub_response_invite_2xx_epoch_us,
            first_hub_call_answered_epoch_us: self.first_hub_call_answered_epoch_us,
            last_hub_call_answered_epoch_us: self.last_hub_call_answered_epoch_us,
            first_hub_ack_sent_epoch_us: self.first_hub_ack_sent_epoch_us,
            last_hub_ack_sent_epoch_us: self.last_hub_ack_sent_epoch_us,
            first_uas_ack_received_epoch_us: self.first_uas_ack_received_epoch_us,
            last_uas_ack_received_epoch_us: self.last_uas_ack_received_epoch_us,
            first_lifecycle_call_answered_epoch_us: self.first_lifecycle_call_answered_epoch_us,
            last_lifecycle_call_answered_epoch_us: self.last_lifecycle_call_answered_epoch_us,
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

pub fn dialog_timing_enabled() -> bool {
    enabled()
        && match DIALOG_TIMING_ENABLED.load(Ordering::Relaxed) {
            2 => true,
            _ => false,
        }
}

pub fn set_dialog_timing_enabled(enabled: bool) {
    DIALOG_TIMING_ENABLED.store(if enabled { 2 } else { 1 }, Ordering::Relaxed);
}

#[cfg(test)]
fn set_enabled_for_tests(enabled: bool) {
    set_enabled(enabled);
}

#[cfg(test)]
fn set_transaction_timing_enabled_for_tests(enabled: bool) {
    set_transaction_timing_enabled(enabled);
}

#[cfg(test)]
fn set_dialog_timing_enabled_for_tests(enabled: bool) {
    set_dialog_timing_enabled(enabled);
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
    for metric in transaction_dispatch_worker_metrics() {
        metric.reset();
    }
    for metric in dialog_latency_metrics() {
        metric.reset();
    }
    if let Ok(mut traces) = CALL_TIMING_TRACES.lock() {
        traces.clear();
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
        ok_200_invite_first: OK_200_INVITE_FIRST.load(Ordering::Relaxed),
        ok_200_invite_duplicate_cache: OK_200_INVITE_DUPLICATE_CACHE.load(Ordering::Relaxed),
        ok_200_invite_proactive_retransmit: OK_200_INVITE_PROACTIVE_RETRANSMIT
            .load(Ordering::Relaxed),
        ok_200_bye_fresh: OK_200_BYE_FRESH.load(Ordering::Relaxed),
        ok_200_bye_tombstone: OK_200_BYE_TOMBSTONE.load(Ordering::Relaxed),
        ok_200_bye_duplicate_terminated: OK_200_BYE_DUPLICATE_TERMINATED.load(Ordering::Relaxed),
        ok_200_other: OK_200_OTHER.load(Ordering::Relaxed),
        uac_invite_2xx_response: UAC_INVITE_2XX_RESPONSE.load(Ordering::Relaxed),
        uac_invite_2xx_ack_attempt: UAC_INVITE_2XX_ACK_ATTEMPT.load(Ordering::Relaxed),
        uac_invite_2xx_ack_success: UAC_INVITE_2XX_ACK_SUCCESS.load(Ordering::Relaxed),
        uac_invite_2xx_ack_failure: UAC_INVITE_2XX_ACK_FAILURE.load(Ordering::Relaxed),
        uac_invite_2xx_call_answered_emit: UAC_INVITE_2XX_CALL_ANSWERED_EMIT
            .load(Ordering::Relaxed),
        hub_response_invite_2xx: HUB_RESPONSE_INVITE_2XX.load(Ordering::Relaxed),
        hub_response_invite_2xx_session_found: HUB_RESPONSE_INVITE_2XX_SESSION_FOUND
            .load(Ordering::Relaxed),
        hub_response_invite_2xx_session_missing: HUB_RESPONSE_INVITE_2XX_SESSION_MISSING
            .load(Ordering::Relaxed),
        hub_call_answered: HUB_CALL_ANSWERED.load(Ordering::Relaxed),
        hub_call_answered_session_found: HUB_CALL_ANSWERED_SESSION_FOUND.load(Ordering::Relaxed),
        hub_call_answered_session_missing: HUB_CALL_ANSWERED_SESSION_MISSING
            .load(Ordering::Relaxed),
        hub_ack_sent: HUB_ACK_SENT.load(Ordering::Relaxed),
        hub_ack_sent_session_found: HUB_ACK_SENT_SESSION_FOUND.load(Ordering::Relaxed),
        hub_ack_sent_session_missing: HUB_ACK_SENT_SESSION_MISSING.load(Ordering::Relaxed),
        call_timing_trace_overflow: CALL_TIMING_TRACE_OVERFLOW.load(Ordering::Relaxed),
        call_timing_traces: call_timing_trace_snapshots(),
        dialog_route_request: DIALOG_ROUTE_REQUEST.load(Ordering::Relaxed),
        dialog_route_stored: DIALOG_ROUTE_STORED.load(Ordering::Relaxed),
        dialog_route_transaction_key: DIALOG_ROUTE_TRANSACTION_KEY.load(Ordering::Relaxed),
        dialog_route_fallback: DIALOG_ROUTE_FALLBACK.load(Ordering::Relaxed),
        dialog_route_worker_mismatch: DIALOG_ROUTE_WORKER_MISMATCH.load(Ordering::Relaxed),
        dialog_route_invite: DIALOG_ROUTE_INVITE.load(Ordering::Relaxed),
        dialog_route_ack: DIALOG_ROUTE_ACK.load(Ordering::Relaxed),
        dialog_route_bye: DIALOG_ROUTE_BYE.load(Ordering::Relaxed),
        dialog_route_cancel: DIALOG_ROUTE_CANCEL.load(Ordering::Relaxed),
        dialog_route_lifecycle: DIALOG_ROUTE_LIFECYCLE.load(Ordering::Relaxed),
        dialog_route_other: DIALOG_ROUTE_OTHER.load(Ordering::Relaxed),
        termination_cleanup_enqueued: TERMINATION_CLEANUP_ENQUEUED.load(Ordering::Relaxed),
        termination_cleanup_queue_full: TERMINATION_CLEANUP_QUEUE_FULL.load(Ordering::Relaxed),
        termination_cleanup_worker_spawned: TERMINATION_CLEANUP_WORKER_SPAWNED
            .load(Ordering::Relaxed),
        termination_cleanup_in_flight: TERMINATION_CLEANUP_IN_FLIGHT.load(Ordering::Relaxed),
        termination_cleanup_max_in_flight: TERMINATION_CLEANUP_MAX_IN_FLIGHT
            .load(Ordering::Relaxed),
        termination_cleanup_poll_attempts: TERMINATION_CLEANUP_POLL_ATTEMPTS
            .load(Ordering::Relaxed),
        termination_cleanup_removed: TERMINATION_CLEANUP_REMOVED.load(Ordering::Relaxed),
        termination_cleanup_batches: TERMINATION_CLEANUP_BATCHES.load(Ordering::Relaxed),
        termination_cleanup_batch_total: TERMINATION_CLEANUP_BATCH_TOTAL.load(Ordering::Relaxed),
        termination_cleanup_batch_max: TERMINATION_CLEANUP_BATCH_MAX.load(Ordering::Relaxed),
        termination_cleanup_indexed_scan_keys: TERMINATION_CLEANUP_INDEXED_SCAN_KEYS
            .load(Ordering::Relaxed),
        termination_cleanup_full_scan_client_keys: TERMINATION_CLEANUP_FULL_SCAN_CLIENT_KEYS
            .load(Ordering::Relaxed),
        termination_cleanup_full_scan_server_keys: TERMINATION_CLEANUP_FULL_SCAN_SERVER_KEYS
            .load(Ordering::Relaxed),
        invite_2xx_maintenance_ticks: INVITE_2XX_MAINTENANCE_TICKS.load(Ordering::Relaxed),
        invite_2xx_maintenance_cache_len_total: INVITE_2XX_MAINTENANCE_CACHE_LEN_TOTAL
            .load(Ordering::Relaxed),
        invite_2xx_maintenance_cache_len_max: INVITE_2XX_MAINTENANCE_CACHE_LEN_MAX
            .load(Ordering::Relaxed),
        invite_2xx_maintenance_due_queue_len_total: INVITE_2XX_MAINTENANCE_DUE_QUEUE_LEN_TOTAL
            .load(Ordering::Relaxed),
        invite_2xx_maintenance_due_queue_len_max: INVITE_2XX_MAINTENANCE_DUE_QUEUE_LEN_MAX
            .load(Ordering::Relaxed),
        invite_2xx_maintenance_scanned: INVITE_2XX_MAINTENANCE_SCANNED.load(Ordering::Relaxed),
        invite_2xx_maintenance_due: INVITE_2XX_MAINTENANCE_DUE.load(Ordering::Relaxed),
        invite_2xx_maintenance_expired: INVITE_2XX_MAINTENANCE_EXPIRED.load(Ordering::Relaxed),
        invite_2xx_maintenance_capped_ticks: INVITE_2XX_MAINTENANCE_CAPPED_TICKS
            .load(Ordering::Relaxed),
        global_publish_count: GLOBAL_PUBLISH_COUNT.load(Ordering::Relaxed),
        global_publish_handler_count_total: GLOBAL_PUBLISH_HANDLER_COUNT_TOTAL
            .load(Ordering::Relaxed),
        global_publish_handler_count_max: GLOBAL_PUBLISH_HANDLER_COUNT_MAX.load(Ordering::Relaxed),
        global_publish_incoming_call: GLOBAL_PUBLISH_INCOMING_CALL.load(Ordering::Relaxed),
        global_publish_ack: GLOBAL_PUBLISH_ACK.load(Ordering::Relaxed),
        global_publish_bye: GLOBAL_PUBLISH_BYE.load(Ordering::Relaxed),
        global_publish_other: GLOBAL_PUBLISH_OTHER.load(Ordering::Relaxed),
        transaction_dispatch_queue_depth_total: TRANSACTION_DISPATCH_QUEUE_DEPTH_TOTAL
            .load(Ordering::Relaxed),
        transaction_dispatch_queue_depth_max: TRANSACTION_DISPATCH_QUEUE_DEPTH_MAX
            .load(Ordering::Relaxed),
        transaction_runner_started: TRANSACTION_RUNNER_STARTED.load(Ordering::Relaxed),
        transaction_runner_exited: TRANSACTION_RUNNER_EXITED.load(Ordering::Relaxed),
        transaction_runner_active: TRANSACTION_RUNNER_ACTIVE.load(Ordering::Relaxed),
        transaction_runner_active_max: TRANSACTION_RUNNER_ACTIVE_MAX.load(Ordering::Relaxed),
        transaction_runner_destroy_wake_sent: TRANSACTION_RUNNER_DESTROY_WAKE_SENT
            .load(Ordering::Relaxed),
        transaction_runner_destroy_wake_failed: TRANSACTION_RUNNER_DESTROY_WAKE_FAILED
            .load(Ordering::Relaxed),
        bye_tombstone_table_size_max: BYE_TOMBSTONE_TABLE_SIZE_MAX.load(Ordering::Relaxed),
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
        bye_path_udp_to_handler: BYE_PATH_UDP_TO_HANDLER.snapshot(),
        bye_path_tx_received_to_handler: BYE_PATH_TX_RECEIVED_TO_HANDLER.snapshot(),
        bye_path_handler_to_send_start: BYE_PATH_HANDLER_TO_SEND_START.snapshot(),
        bye_path_send_response: BYE_PATH_SEND_RESPONSE.snapshot(),
        bye_path_release_tx: BYE_PATH_RELEASE_TX.snapshot(),
        bye_path_cleanup_emit: BYE_PATH_CLEANUP_EMIT.snapshot(),
        bye_path_cleanup_remove_storage: BYE_PATH_CLEANUP_REMOVE_STORAGE.snapshot(),
        bye_tombstone_lookup: BYE_TOMBSTONE_LOOKUP.snapshot(),
        bye_tombstone_prune: BYE_TOMBSTONE_PRUNE.snapshot(),
        transaction_dispatch_queue: TRANSACTION_DISPATCH_QUEUE.snapshot(),
        transaction_dispatch_queue_invite: TRANSACTION_DISPATCH_QUEUE_INVITE.snapshot(),
        transaction_dispatch_queue_ack: TRANSACTION_DISPATCH_QUEUE_ACK.snapshot(),
        transaction_dispatch_queue_bye: TRANSACTION_DISPATCH_QUEUE_BYE.snapshot(),
        transaction_dispatch_queue_cancel: TRANSACTION_DISPATCH_QUEUE_CANCEL.snapshot(),
        transaction_dispatch_queue_other: TRANSACTION_DISPATCH_QUEUE_OTHER.snapshot(),
        transaction_dispatch_queue_by_worker: transaction_dispatch_worker_metrics()
            .iter()
            .enumerate()
            .map(|(idx, metric)| metric.snapshot(idx))
            .collect(),
        transaction_handler_total: TRANSACTION_HANDLER_TOTAL.snapshot(),
        transaction_handler_invite: TRANSACTION_HANDLER_INVITE.snapshot(),
        transaction_handler_ack: TRANSACTION_HANDLER_ACK.snapshot(),
        transaction_handler_bye: TRANSACTION_HANDLER_BYE.snapshot(),
        transaction_handler_cancel: TRANSACTION_HANDLER_CANCEL.snapshot(),
        transaction_handler_other: TRANSACTION_HANDLER_OTHER.snapshot(),
        server_transaction_create: SERVER_TRANSACTION_CREATE.snapshot(),
        existing_transaction_dispatch: EXISTING_TRANSACTION_DISPATCH.snapshot(),
        transaction_event_broadcast: TRANSACTION_EVENT_BROADCAST.snapshot(),
        transaction_dispatch_backpressure: TRANSACTION_DISPATCH_BACKPRESSURE.snapshot(),
        udp_receive_to_invite_200: UDP_RECEIVE_TO_INVITE_200.snapshot(),
        dialog_event_dispatch_queue: DIALOG_EVENT_DISPATCH_QUEUE.snapshot(),
        dialog_event_dispatch_backpressure: DIALOG_EVENT_DISPATCH_BACKPRESSURE.snapshot(),
        dialog_event_handler_total: DIALOG_EVENT_HANDLER_TOTAL.snapshot(),
        dialog_event_handler_invite: DIALOG_EVENT_HANDLER_INVITE.snapshot(),
        dialog_event_handler_ack: DIALOG_EVENT_HANDLER_ACK.snapshot(),
        dialog_event_handler_bye: DIALOG_EVENT_HANDLER_BYE.snapshot(),
        dialog_event_handler_cancel: DIALOG_EVENT_HANDLER_CANCEL.snapshot(),
        dialog_event_handler_other: DIALOG_EVENT_HANDLER_OTHER.snapshot(),
        dialog_session_publish_total: DIALOG_SESSION_PUBLISH_TOTAL.snapshot(),
        dialog_session_publish_incoming_call: DIALOG_SESSION_PUBLISH_INCOMING_CALL.snapshot(),
        dialog_session_publish_ack: DIALOG_SESSION_PUBLISH_ACK.snapshot(),
        dialog_session_publish_bye: DIALOG_SESSION_PUBLISH_BYE.snapshot(),
        dialog_session_publish_other: DIALOG_SESSION_PUBLISH_OTHER.snapshot(),
        dialog_lookup: DIALOG_LOOKUP.snapshot(),
        dialog_initial_invite_setup: DIALOG_INITIAL_INVITE_SETUP.snapshot(),
        termination_cleanup_indexed_scan: TERMINATION_CLEANUP_INDEXED_SCAN.snapshot(),
        termination_cleanup_full_scan: TERMINATION_CLEANUP_FULL_SCAN.snapshot(),
        termination_cleanup_timer_unregister: TERMINATION_CLEANUP_TIMER_UNREGISTER.snapshot(),
        invite_2xx_maintenance: INVITE_2XX_MAINTENANCE.snapshot(),
        invite_2xx_proactive_send: INVITE_2XX_PROACTIVE_SEND.snapshot(),
        global_publish_total: GLOBAL_PUBLISH_TOTAL.snapshot(),
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
    let transaction_dispatch_queue_by_worker =
        format_transaction_dispatch_workers(&snapshot.transaction_dispatch_queue_by_worker);
    format!(
        "[sip_retrans_diag] dup_invite_existing_tx={} dup_invite_cache_hit={} \
         dup_invite_cache_miss={} invite_2xx_cache_insert={} invite_2xx_cache_expired={} \
         invite_2xx_proactive_retx={} invite_2xx_ack_removed={} invite_2xx_ack_avg_ms={:.3} \
         dup_bye_existing_tx={} dup_bye_tombstone_hit={} dup_bye_tombstone_miss={} \
         dup_bye_terminated_dialog={} ack_matched={} ack_unmatched={} \
         ack_delivered={} bye_200_sent={} bye_cleanup_emitted={} bye_cleanup_delivered={} \
         bye_cleanup_missing={} ok_200_source=[invite_first={} invite_duplicate_cache={} \
         invite_proactive_retransmit={} bye_fresh={} bye_tombstone={} \
         bye_duplicate_terminated={} other={}] dialog_route=[request={} stored={} transaction_key={} \
         fallback={} worker_mismatch={} invite={} ack={} bye={} cancel={} lifecycle={} \
         other={}] termination_cleanup=[enqueued={} queue_full={} worker_spawned={} \
         in_flight={} max_in_flight={} poll_attempts={} removed={} batches={} \
         batch_total={} batch_max={} indexed_scan_keys={} full_scan_client_keys={} \
         full_scan_server_keys={}] invite_2xx_maintenance=[ticks={} cache_len_total={} \
         cache_len_max={} due_queue_len_total={} due_queue_len_max={} scanned={} due={} \
         expired={} capped_ticks={}] global_publish=[count={} \
         handler_count_total={} handler_count_max={} incoming_call={} ack={} bye={} other={}] \
         transaction_dispatch_queue_depth=[total={} max={}] \
         first_invite_to_200=[count={} avg_us={} p50_us={} \
         p95_us={} p99_us={} p999_us={} max_us={} over_500ms={}] \
         dialog_to_session_queue=[count={} avg_us={} p50_us={} p95_us={} p99_us={} \
         p999_us={} max_us={} over_500ms={} incoming_call={} ack_received={} \
         bye_received={} terminal={} other={}] \
         udp_receive_to_incoming_call_emit=[{}] bye_receive_to_200=[{}] \
         bye_path=[udp_to_handler=[{}] tx_received_to_handler=[{}] \
         handler_to_send_start=[{}] send_response=[{}] release_tx=[{}] \
         cleanup_emit=[{}] cleanup_remove_storage=[{}]] \
         bye_tombstone=[hit={} miss={} table_size_max={} lookup=[{}] prune=[{}]] \
         transaction_dispatch_queue=[{}] transaction_dispatch_queue_by_kind=[invite=[{}] \
         ack=[{}] bye=[{}] cancel=[{}] other=[{}]] \
         transaction_dispatch_queue_by_worker=[{}] transaction_handler=[total=[{}] invite=[{}] \
         ack=[{}] bye=[{}] cancel=[{}] other=[{}]] server_transaction_create=[{}] \
         existing_transaction_dispatch=[{}] transaction_event_broadcast=[{}] \
         transaction_dispatch_backpressure=[{}] \
         udp_receive_to_invite_200=[{}] dialog_event_dispatch_queue=[{}] \
         dialog_event_dispatch_backpressure=[{}] dialog_event_handler=[total=[{}] \
         invite=[{}] ack=[{}] bye=[{}] cancel=[{}] other=[{}]] \
         dialog_session_publish=[total=[{}] incoming_call=[{}] ack=[{}] bye=[{}] \
         other=[{}]] dialog_lookup=[{}] dialog_initial_invite_setup=[{}] \
         termination_cleanup_indexed_scan=[{}] termination_cleanup_full_scan=[{}] \
         termination_cleanup_timer_unregister=[{}] invite_2xx_maintenance_time=[{}] \
         invite_2xx_proactive_send=[{}] global_publish_total=[{}]",
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
        snapshot.ok_200_invite_first,
        snapshot.ok_200_invite_duplicate_cache,
        snapshot.ok_200_invite_proactive_retransmit,
        snapshot.ok_200_bye_fresh,
        snapshot.ok_200_bye_tombstone,
        snapshot.ok_200_bye_duplicate_terminated,
        snapshot.ok_200_other,
        snapshot.dialog_route_request,
        snapshot.dialog_route_stored,
        snapshot.dialog_route_transaction_key,
        snapshot.dialog_route_fallback,
        snapshot.dialog_route_worker_mismatch,
        snapshot.dialog_route_invite,
        snapshot.dialog_route_ack,
        snapshot.dialog_route_bye,
        snapshot.dialog_route_cancel,
        snapshot.dialog_route_lifecycle,
        snapshot.dialog_route_other,
        snapshot.termination_cleanup_enqueued,
        snapshot.termination_cleanup_queue_full,
        snapshot.termination_cleanup_worker_spawned,
        snapshot.termination_cleanup_in_flight,
        snapshot.termination_cleanup_max_in_flight,
        snapshot.termination_cleanup_poll_attempts,
        snapshot.termination_cleanup_removed,
        snapshot.termination_cleanup_batches,
        snapshot.termination_cleanup_batch_total,
        snapshot.termination_cleanup_batch_max,
        snapshot.termination_cleanup_indexed_scan_keys,
        snapshot.termination_cleanup_full_scan_client_keys,
        snapshot.termination_cleanup_full_scan_server_keys,
        snapshot.invite_2xx_maintenance_ticks,
        snapshot.invite_2xx_maintenance_cache_len_total,
        snapshot.invite_2xx_maintenance_cache_len_max,
        snapshot.invite_2xx_maintenance_due_queue_len_total,
        snapshot.invite_2xx_maintenance_due_queue_len_max,
        snapshot.invite_2xx_maintenance_scanned,
        snapshot.invite_2xx_maintenance_due,
        snapshot.invite_2xx_maintenance_expired,
        snapshot.invite_2xx_maintenance_capped_ticks,
        snapshot.global_publish_count,
        snapshot.global_publish_handler_count_total,
        snapshot.global_publish_handler_count_max,
        snapshot.global_publish_incoming_call,
        snapshot.global_publish_ack,
        snapshot.global_publish_bye,
        snapshot.global_publish_other,
        snapshot.transaction_dispatch_queue_depth_total,
        snapshot.transaction_dispatch_queue_depth_max,
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
        format_latency(&snapshot.bye_path_udp_to_handler),
        format_latency(&snapshot.bye_path_tx_received_to_handler),
        format_latency(&snapshot.bye_path_handler_to_send_start),
        format_latency(&snapshot.bye_path_send_response),
        format_latency(&snapshot.bye_path_release_tx),
        format_latency(&snapshot.bye_path_cleanup_emit),
        format_latency(&snapshot.bye_path_cleanup_remove_storage),
        snapshot.duplicate_bye_tombstone_hit,
        snapshot.duplicate_bye_tombstone_miss,
        snapshot.bye_tombstone_table_size_max,
        format_latency(&snapshot.bye_tombstone_lookup),
        format_latency(&snapshot.bye_tombstone_prune),
        format_latency(&snapshot.transaction_dispatch_queue),
        format_latency(&snapshot.transaction_dispatch_queue_invite),
        format_latency(&snapshot.transaction_dispatch_queue_ack),
        format_latency(&snapshot.transaction_dispatch_queue_bye),
        format_latency(&snapshot.transaction_dispatch_queue_cancel),
        format_latency(&snapshot.transaction_dispatch_queue_other),
        transaction_dispatch_queue_by_worker,
        format_latency(&snapshot.transaction_handler_total),
        format_latency(&snapshot.transaction_handler_invite),
        format_latency(&snapshot.transaction_handler_ack),
        format_latency(&snapshot.transaction_handler_bye),
        format_latency(&snapshot.transaction_handler_cancel),
        format_latency(&snapshot.transaction_handler_other),
        format_latency(&snapshot.server_transaction_create),
        format_latency(&snapshot.existing_transaction_dispatch),
        format_latency(&snapshot.transaction_event_broadcast),
        format_latency(&snapshot.transaction_dispatch_backpressure),
        format_latency(&snapshot.udp_receive_to_invite_200),
        format_latency(&snapshot.dialog_event_dispatch_queue),
        format_latency(&snapshot.dialog_event_dispatch_backpressure),
        format_latency(&snapshot.dialog_event_handler_total),
        format_latency(&snapshot.dialog_event_handler_invite),
        format_latency(&snapshot.dialog_event_handler_ack),
        format_latency(&snapshot.dialog_event_handler_bye),
        format_latency(&snapshot.dialog_event_handler_cancel),
        format_latency(&snapshot.dialog_event_handler_other),
        format_latency(&snapshot.dialog_session_publish_total),
        format_latency(&snapshot.dialog_session_publish_incoming_call),
        format_latency(&snapshot.dialog_session_publish_ack),
        format_latency(&snapshot.dialog_session_publish_bye),
        format_latency(&snapshot.dialog_session_publish_other),
        format_latency(&snapshot.dialog_lookup),
        format_latency(&snapshot.dialog_initial_invite_setup),
        format_latency(&snapshot.termination_cleanup_indexed_scan),
        format_latency(&snapshot.termination_cleanup_full_scan),
        format_latency(&snapshot.termination_cleanup_timer_unregister),
        format_latency(&snapshot.invite_2xx_maintenance),
        format_latency(&snapshot.invite_2xx_proactive_send),
        format_latency(&snapshot.global_publish_total),
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

pub fn record_200_ok_invite_first() {
    increment(&OK_200_INVITE_FIRST);
}

pub(crate) fn record_200_ok_invite_duplicate_cache() {
    increment(&OK_200_INVITE_DUPLICATE_CACHE);
}

pub(crate) fn record_200_ok_invite_proactive_retransmit() {
    increment(&OK_200_INVITE_PROACTIVE_RETRANSMIT);
}

pub(crate) fn record_200_ok_bye_fresh() {
    increment(&OK_200_BYE_FRESH);
}

pub(crate) fn record_200_ok_bye_tombstone() {
    increment(&OK_200_BYE_TOMBSTONE);
}

pub(crate) fn record_200_ok_bye_duplicate_terminated() {
    increment(&OK_200_BYE_DUPLICATE_TERMINATED);
}

pub(crate) fn record_200_ok_other() {
    increment(&OK_200_OTHER);
}

pub(crate) fn record_uac_invite_2xx_response() {
    increment(&UAC_INVITE_2XX_RESPONSE);
}

pub(crate) fn record_uac_invite_2xx_ack_attempt() {
    increment(&UAC_INVITE_2XX_ACK_ATTEMPT);
}

pub(crate) fn record_uac_invite_2xx_ack_success() {
    increment(&UAC_INVITE_2XX_ACK_SUCCESS);
}

pub(crate) fn record_uac_invite_2xx_ack_failure() {
    increment(&UAC_INVITE_2XX_ACK_FAILURE);
}

pub(crate) fn record_uac_invite_2xx_call_answered_emit() {
    increment(&UAC_INVITE_2XX_CALL_ANSWERED_EMIT);
}

pub(crate) fn record_hub_response_invite_2xx_session(found: bool) {
    increment(&HUB_RESPONSE_INVITE_2XX);
    if found {
        increment(&HUB_RESPONSE_INVITE_2XX_SESSION_FOUND);
    } else {
        increment(&HUB_RESPONSE_INVITE_2XX_SESSION_MISSING);
    }
}

pub(crate) fn record_hub_call_answered_session(found: bool) {
    increment(&HUB_CALL_ANSWERED);
    if found {
        increment(&HUB_CALL_ANSWERED_SESSION_FOUND);
    } else {
        increment(&HUB_CALL_ANSWERED_SESSION_MISSING);
    }
}

pub(crate) fn record_hub_ack_sent_session(found: bool) {
    increment(&HUB_ACK_SENT);
    if found {
        increment(&HUB_ACK_SENT_SESSION_FOUND);
    } else {
        increment(&HUB_ACK_SENT_SESSION_MISSING);
    }
}

pub fn record_call_timing_uac_invite_2xx_response(call_id: &str) {
    record_call_timing_trace(call_id, |trace, now_us| {
        trace.record_uac_invite_2xx_response(now_us);
    });
}

pub fn record_call_timing_uac_ack_attempt(call_id: &str) {
    record_call_timing_trace(call_id, |trace, now_us| {
        trace.record_uac_ack_attempt(now_us);
    });
}

pub fn record_call_timing_uac_ack_success(call_id: &str) {
    record_call_timing_trace(call_id, |trace, now_us| {
        trace.record_uac_ack_success(now_us);
    });
}

pub fn record_call_timing_uac_ack_failure(call_id: &str) {
    record_call_timing_trace(call_id, |trace, now_us| {
        trace.record_uac_ack_failure(now_us);
    });
}

pub fn record_call_timing_uac_call_answered_emit(call_id: &str) {
    record_call_timing_trace(call_id, |trace, now_us| {
        trace.record_uac_call_answered_emit(now_us);
    });
}

pub fn record_call_timing_hub_response_invite_2xx(call_id: &str) {
    record_call_timing_trace(call_id, |trace, now_us| {
        trace.record_hub_response_invite_2xx(now_us);
    });
}

pub fn record_call_timing_hub_call_answered(call_id: &str) {
    record_call_timing_trace(call_id, |trace, now_us| {
        trace.record_hub_call_answered(now_us);
    });
}

pub fn record_call_timing_hub_ack_sent(call_id: &str) {
    record_call_timing_trace(call_id, |trace, now_us| {
        trace.record_hub_ack_sent(now_us);
    });
}

pub fn record_call_timing_uas_ack_received(call_id: &str) {
    record_call_timing_trace(call_id, |trace, now_us| {
        trace.record_uas_ack_received(now_us);
    });
}

pub fn record_call_timing_lifecycle_call_answered(call_id: &str) {
    record_call_timing_trace(call_id, |trace, now_us| {
        trace.record_lifecycle_call_answered(now_us);
    });
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

pub(crate) fn record_dialog_route(source: &str, kind: &str, mismatch: bool) {
    if !dialog_timing_enabled() {
        return;
    }

    match source {
        "request" => {
            DIALOG_ROUTE_REQUEST.fetch_add(1, Ordering::Relaxed);
        }
        "stored" => {
            DIALOG_ROUTE_STORED.fetch_add(1, Ordering::Relaxed);
        }
        "transaction_key" => {
            DIALOG_ROUTE_TRANSACTION_KEY.fetch_add(1, Ordering::Relaxed);
        }
        _ => {
            DIALOG_ROUTE_FALLBACK.fetch_add(1, Ordering::Relaxed);
        }
    };

    match kind {
        "invite" => {
            DIALOG_ROUTE_INVITE.fetch_add(1, Ordering::Relaxed);
        }
        "ack" => {
            DIALOG_ROUTE_ACK.fetch_add(1, Ordering::Relaxed);
        }
        "bye" => {
            DIALOG_ROUTE_BYE.fetch_add(1, Ordering::Relaxed);
        }
        "cancel" => {
            DIALOG_ROUTE_CANCEL.fetch_add(1, Ordering::Relaxed);
        }
        "lifecycle" => {
            DIALOG_ROUTE_LIFECYCLE.fetch_add(1, Ordering::Relaxed);
        }
        _ => {
            DIALOG_ROUTE_OTHER.fetch_add(1, Ordering::Relaxed);
        }
    };

    if mismatch {
        DIALOG_ROUTE_WORKER_MISMATCH.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn record_termination_cleanup_enqueued() {
    if transaction_timing_enabled() {
        TERMINATION_CLEANUP_ENQUEUED.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn record_termination_cleanup_queue_full() {
    if transaction_timing_enabled() {
        TERMINATION_CLEANUP_QUEUE_FULL.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn record_termination_cleanup_worker_spawned() {
    if transaction_timing_enabled() {
        TERMINATION_CLEANUP_WORKER_SPAWNED.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn record_termination_cleanup_batch(size: usize) {
    if !transaction_timing_enabled() {
        return;
    }
    let size = size as u64;
    TERMINATION_CLEANUP_BATCHES.fetch_add(1, Ordering::Relaxed);
    TERMINATION_CLEANUP_BATCH_TOTAL.fetch_add(size, Ordering::Relaxed);
    update_max(&TERMINATION_CLEANUP_BATCH_MAX, size);
}

pub(crate) fn record_termination_cleanup_in_flight(delta: i64) {
    if !transaction_timing_enabled() {
        return;
    }
    let current = if delta >= 0 {
        TERMINATION_CLEANUP_IN_FLIGHT.fetch_add(delta as u64, Ordering::Relaxed) + delta as u64
    } else {
        TERMINATION_CLEANUP_IN_FLIGHT
            .fetch_sub((-delta) as u64, Ordering::Relaxed)
            .saturating_sub((-delta) as u64)
    };
    update_max(&TERMINATION_CLEANUP_MAX_IN_FLIGHT, current);
}

pub(crate) fn record_termination_cleanup_poll_attempts(attempts: u64) {
    if transaction_timing_enabled() {
        TERMINATION_CLEANUP_POLL_ATTEMPTS.fetch_add(attempts, Ordering::Relaxed);
    }
}

pub(crate) fn record_termination_cleanup_removed() {
    if transaction_timing_enabled() {
        TERMINATION_CLEANUP_REMOVED.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn record_termination_cleanup_indexed_scan(scanned_keys: usize, elapsed: Duration) {
    if !transaction_timing_enabled() {
        return;
    }
    TERMINATION_CLEANUP_INDEXED_SCAN_KEYS.fetch_add(scanned_keys as u64, Ordering::Relaxed);
    TERMINATION_CLEANUP_INDEXED_SCAN.record(elapsed);
}

pub(crate) fn record_termination_cleanup_full_scan(
    client_keys: usize,
    server_keys: usize,
    elapsed: Duration,
) {
    if !transaction_timing_enabled() {
        return;
    }
    TERMINATION_CLEANUP_FULL_SCAN_CLIENT_KEYS.fetch_add(client_keys as u64, Ordering::Relaxed);
    TERMINATION_CLEANUP_FULL_SCAN_SERVER_KEYS.fetch_add(server_keys as u64, Ordering::Relaxed);
    TERMINATION_CLEANUP_FULL_SCAN.record(elapsed);
}

pub(crate) fn record_termination_cleanup_timer_unregister(elapsed: Duration) {
    if transaction_timing_enabled() {
        TERMINATION_CLEANUP_TIMER_UNREGISTER.record(elapsed);
    }
}

pub(crate) fn record_invite_2xx_maintenance(
    cache_len: usize,
    due_queue_len: usize,
    scanned: usize,
    due: usize,
    expired: usize,
    capped: bool,
    elapsed: Duration,
) {
    if !transaction_timing_enabled() {
        return;
    }
    INVITE_2XX_MAINTENANCE_TICKS.fetch_add(1, Ordering::Relaxed);
    INVITE_2XX_MAINTENANCE_CACHE_LEN_TOTAL.fetch_add(cache_len as u64, Ordering::Relaxed);
    update_max(&INVITE_2XX_MAINTENANCE_CACHE_LEN_MAX, cache_len as u64);
    INVITE_2XX_MAINTENANCE_DUE_QUEUE_LEN_TOTAL.fetch_add(due_queue_len as u64, Ordering::Relaxed);
    update_max(
        &INVITE_2XX_MAINTENANCE_DUE_QUEUE_LEN_MAX,
        due_queue_len as u64,
    );
    INVITE_2XX_MAINTENANCE_SCANNED.fetch_add(scanned as u64, Ordering::Relaxed);
    INVITE_2XX_MAINTENANCE_DUE.fetch_add(due as u64, Ordering::Relaxed);
    INVITE_2XX_MAINTENANCE_EXPIRED.fetch_add(expired as u64, Ordering::Relaxed);
    if capped {
        INVITE_2XX_MAINTENANCE_CAPPED_TICKS.fetch_add(1, Ordering::Relaxed);
    }
    INVITE_2XX_MAINTENANCE.record(elapsed);
}

pub(crate) fn record_invite_2xx_proactive_send(elapsed: Duration) {
    if transaction_timing_enabled() {
        INVITE_2XX_PROACTIVE_SEND.record(elapsed);
    }
}

pub(crate) fn record_global_publish(kind: &str, handler_count: usize, elapsed: Duration) {
    if !dialog_timing_enabled() {
        return;
    }
    GLOBAL_PUBLISH_COUNT.fetch_add(1, Ordering::Relaxed);
    GLOBAL_PUBLISH_HANDLER_COUNT_TOTAL.fetch_add(handler_count as u64, Ordering::Relaxed);
    update_max(&GLOBAL_PUBLISH_HANDLER_COUNT_MAX, handler_count as u64);
    match kind {
        "incoming_call" => {
            GLOBAL_PUBLISH_INCOMING_CALL.fetch_add(1, Ordering::Relaxed);
        }
        "ack_received" => {
            GLOBAL_PUBLISH_ACK.fetch_add(1, Ordering::Relaxed);
        }
        "bye_received" => {
            GLOBAL_PUBLISH_BYE.fetch_add(1, Ordering::Relaxed);
        }
        _ => {
            GLOBAL_PUBLISH_OTHER.fetch_add(1, Ordering::Relaxed);
        }
    };
    GLOBAL_PUBLISH_TOTAL.record(elapsed);
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
    FIRST_INVITE_TO_200_BUCKETS[latency_bucket_index(elapsed_us)].fetch_add(1, Ordering::Relaxed);
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

pub(crate) fn record_bye_path_udp_to_handler(elapsed: Duration) {
    if dialog_timing_enabled() {
        BYE_PATH_UDP_TO_HANDLER.record(elapsed);
    }
}

pub(crate) fn record_bye_path_tx_received_to_handler(elapsed: Duration) {
    if dialog_timing_enabled() {
        BYE_PATH_TX_RECEIVED_TO_HANDLER.record(elapsed);
    }
}

pub(crate) fn record_bye_path_handler_to_send_start(elapsed: Duration) {
    if dialog_timing_enabled() {
        BYE_PATH_HANDLER_TO_SEND_START.record(elapsed);
    }
}

pub(crate) fn record_bye_path_send_response(elapsed: Duration) {
    if dialog_timing_enabled() {
        BYE_PATH_SEND_RESPONSE.record(elapsed);
    }
}

pub(crate) fn record_bye_path_release_tx(elapsed: Duration) {
    if dialog_timing_enabled() {
        BYE_PATH_RELEASE_TX.record(elapsed);
    }
}

pub(crate) fn record_bye_path_cleanup_emit(elapsed: Duration) {
    if dialog_timing_enabled() {
        BYE_PATH_CLEANUP_EMIT.record(elapsed);
    }
}

pub(crate) fn record_bye_path_cleanup_remove_storage(elapsed: Duration) {
    if dialog_timing_enabled() {
        BYE_PATH_CLEANUP_REMOVE_STORAGE.record(elapsed);
    }
}

pub(crate) fn record_bye_tombstone_lookup(elapsed: Duration) {
    if dialog_timing_enabled() {
        BYE_TOMBSTONE_LOOKUP.record(elapsed);
    }
}

pub(crate) fn record_bye_tombstone_prune(elapsed: Duration) {
    if dialog_timing_enabled() {
        BYE_TOMBSTONE_PRUNE.record(elapsed);
    }
}

pub(crate) fn record_bye_tombstone_observed_size(size: usize) {
    if dialog_timing_enabled() {
        update_max(&BYE_TOMBSTONE_TABLE_SIZE_MAX, size as u64);
    }
}

pub fn record_udp_receive_to_invite_200(elapsed: Duration) {
    if enabled() {
        UDP_RECEIVE_TO_INVITE_200.record(elapsed);
    }
}

#[allow(dead_code)]
pub(crate) fn record_transaction_dispatch_queue_delay(elapsed: Duration) {
    if transaction_timing_enabled() {
        TRANSACTION_DISPATCH_QUEUE.record(elapsed);
    }
}

#[allow(dead_code)]
pub(crate) fn record_transaction_dispatch_queue_depth(depth: usize) {
    if !transaction_timing_enabled() {
        return;
    }
    let depth = depth as u64;
    TRANSACTION_DISPATCH_QUEUE_DEPTH_TOTAL.fetch_add(depth, Ordering::Relaxed);
    update_max(&TRANSACTION_DISPATCH_QUEUE_DEPTH_MAX, depth);
}

pub(crate) fn record_transaction_runner_started() {
    TRANSACTION_RUNNER_STARTED.fetch_add(1, Ordering::Relaxed);
    let active = TRANSACTION_RUNNER_ACTIVE.fetch_add(1, Ordering::Relaxed) + 1;
    update_max(&TRANSACTION_RUNNER_ACTIVE_MAX, active);
}

pub(crate) fn record_transaction_runner_exited() {
    TRANSACTION_RUNNER_EXITED.fetch_add(1, Ordering::Relaxed);
    let _ =
        TRANSACTION_RUNNER_ACTIVE.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_sub(1)
        });
}

pub(crate) fn record_transaction_runner_destroy_wake_sent() {
    TRANSACTION_RUNNER_DESTROY_WAKE_SENT.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_transaction_runner_destroy_wake_failed() {
    TRANSACTION_RUNNER_DESTROY_WAKE_FAILED.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_transaction_dispatch_queue_by_worker_and_kind(
    worker_id: usize,
    kind: &str,
    elapsed: Duration,
    depth: usize,
) {
    if !transaction_timing_enabled() {
        return;
    }

    TRANSACTION_DISPATCH_QUEUE.record(elapsed);
    match kind {
        "invite" => TRANSACTION_DISPATCH_QUEUE_INVITE.record(elapsed),
        "ack" => TRANSACTION_DISPATCH_QUEUE_ACK.record(elapsed),
        "bye" => TRANSACTION_DISPATCH_QUEUE_BYE.record(elapsed),
        "cancel" => TRANSACTION_DISPATCH_QUEUE_CANCEL.record(elapsed),
        _ => TRANSACTION_DISPATCH_QUEUE_OTHER.record(elapsed),
    }

    let depth = depth as u64;
    TRANSACTION_DISPATCH_QUEUE_DEPTH_TOTAL.fetch_add(depth, Ordering::Relaxed);
    update_max(&TRANSACTION_DISPATCH_QUEUE_DEPTH_MAX, depth);

    if let Some(worker) = transaction_dispatch_worker_metrics().get(worker_id) {
        worker.queue.record(elapsed);
        update_max(&worker.depth_max, depth);
    }
}

pub(crate) fn record_transaction_dispatch_backpressure(elapsed: Duration) {
    if transaction_timing_enabled() {
        TRANSACTION_DISPATCH_BACKPRESSURE.record(elapsed);
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

pub(crate) fn record_dialog_event_dispatch_queue_delay(elapsed: Duration) {
    if dialog_timing_enabled() {
        DIALOG_EVENT_DISPATCH_QUEUE.record(elapsed);
    }
}

pub(crate) fn record_dialog_event_dispatch_backpressure(elapsed: Duration) {
    if dialog_timing_enabled() {
        DIALOG_EVENT_DISPATCH_BACKPRESSURE.record(elapsed);
    }
}

pub(crate) fn record_dialog_event_handler(kind: &str, elapsed: Duration) {
    if !dialog_timing_enabled() {
        return;
    }
    DIALOG_EVENT_HANDLER_TOTAL.record(elapsed);
    match kind {
        "invite" => DIALOG_EVENT_HANDLER_INVITE.record(elapsed),
        "ack" => DIALOG_EVENT_HANDLER_ACK.record(elapsed),
        "bye" => DIALOG_EVENT_HANDLER_BYE.record(elapsed),
        "cancel" => DIALOG_EVENT_HANDLER_CANCEL.record(elapsed),
        _ => DIALOG_EVENT_HANDLER_OTHER.record(elapsed),
    }
}

pub(crate) fn record_dialog_session_publish(kind: &str, elapsed: Duration) {
    if !dialog_timing_enabled() {
        return;
    }
    DIALOG_SESSION_PUBLISH_TOTAL.record(elapsed);
    match kind {
        "incoming_call" => DIALOG_SESSION_PUBLISH_INCOMING_CALL.record(elapsed),
        "ack_received" => DIALOG_SESSION_PUBLISH_ACK.record(elapsed),
        "bye_received" => DIALOG_SESSION_PUBLISH_BYE.record(elapsed),
        _ => DIALOG_SESSION_PUBLISH_OTHER.record(elapsed),
    }
}

pub(crate) fn record_dialog_lookup(elapsed: Duration) {
    if dialog_timing_enabled() {
        DIALOG_LOOKUP.record(elapsed);
    }
}

pub(crate) fn record_dialog_initial_invite_setup(elapsed: Duration) {
    if dialog_timing_enabled() {
        DIALOG_INITIAL_INVITE_SETUP.record(elapsed);
    }
}

fn record_call_timing_trace(call_id: &str, update: impl FnOnce(&mut CallTimingTraceCounts, u64)) {
    if !enabled() || call_id.is_empty() {
        return;
    }

    let Ok(mut traces) = CALL_TIMING_TRACES.lock() else {
        return;
    };
    if !traces.contains_key(call_id) && traces.len() >= MAX_CALL_TIMING_TRACE_ENTRIES {
        CALL_TIMING_TRACE_OVERFLOW.fetch_add(1, Ordering::Relaxed);
        return;
    }

    let now_us = epoch_us();
    let trace = traces.entry(call_id.to_string()).or_default();
    update(trace, now_us);
}

fn call_timing_trace_snapshots() -> Vec<CallTimingTraceSnapshot> {
    let Ok(traces) = CALL_TIMING_TRACES.lock() else {
        return Vec::new();
    };

    let mut rows = traces
        .iter()
        .map(|(call_id, trace)| trace.snapshot(call_id.clone()))
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.call_id.cmp(&right.call_id));
    rows
}

fn set_first_last_u64(first: &mut Option<u64>, last: &mut Option<u64>, value: u64) {
    if first.is_none() {
        *first = Some(value);
    }
    *last = Some(value);
}

fn increment(counter: &AtomicU64) {
    if enabled() {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

fn latency_bucket_index(elapsed_us: u64) -> usize {
    let bucketed_us = elapsed_us.min(*LATENCY_BUCKET_UPPER_US.last().unwrap());
    LATENCY_BUCKET_UPPER_US
        .iter()
        .position(|upper| bucketed_us <= *upper)
        .unwrap_or(LATENCY_BUCKET_UPPER_US.len() - 1)
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
    DIALOG_TO_SESSION_QUEUE_BUCKETS[latency_bucket_index(elapsed_us)]
        .fetch_add(1, Ordering::Relaxed);

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
    buckets[latency_bucket_index(elapsed_us)].fetch_add(1, Ordering::Relaxed);
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

fn format_transaction_dispatch_workers(workers: &[TransactionDispatchWorkerSnapshot]) -> String {
    let mut parts = Vec::new();
    for worker in workers {
        if worker.queue.count == 0 && worker.depth_max == 0 {
            continue;
        }
        parts.push(format!(
            "w{}=[{} depth_max={}]",
            worker.worker_id,
            format_latency(&worker.queue),
            worker.depth_max
        ));
    }
    if parts.is_empty() {
        "none".to_string()
    } else {
        parts.join(" ")
    }
}

fn transaction_dispatch_worker_metrics() -> &'static [WorkerDispatchMetric] {
    TRANSACTION_DISPATCH_QUEUE_WORKERS
        .get_or_init(|| {
            (0..MAX_TRANSACTION_DISPATCH_DIAG_WORKERS)
                .map(|_| WorkerDispatchMetric::new())
                .collect()
        })
        .as_slice()
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

fn epoch_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
        .min(u128::from(u64::MAX)) as u64
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
    *LATENCY_BUCKET_UPPER_US.last().unwrap()
}

fn all_counters() -> Vec<&'static AtomicU64> {
    vec![
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
        &OK_200_INVITE_FIRST,
        &OK_200_INVITE_DUPLICATE_CACHE,
        &OK_200_INVITE_PROACTIVE_RETRANSMIT,
        &OK_200_BYE_FRESH,
        &OK_200_BYE_TOMBSTONE,
        &OK_200_BYE_DUPLICATE_TERMINATED,
        &OK_200_OTHER,
        &UAC_INVITE_2XX_RESPONSE,
        &UAC_INVITE_2XX_ACK_ATTEMPT,
        &UAC_INVITE_2XX_ACK_SUCCESS,
        &UAC_INVITE_2XX_ACK_FAILURE,
        &UAC_INVITE_2XX_CALL_ANSWERED_EMIT,
        &HUB_RESPONSE_INVITE_2XX,
        &HUB_RESPONSE_INVITE_2XX_SESSION_FOUND,
        &HUB_RESPONSE_INVITE_2XX_SESSION_MISSING,
        &HUB_CALL_ANSWERED,
        &HUB_CALL_ANSWERED_SESSION_FOUND,
        &HUB_CALL_ANSWERED_SESSION_MISSING,
        &HUB_ACK_SENT,
        &HUB_ACK_SENT_SESSION_FOUND,
        &HUB_ACK_SENT_SESSION_MISSING,
        &CALL_TIMING_TRACE_OVERFLOW,
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
        &DIALOG_ROUTE_REQUEST,
        &DIALOG_ROUTE_STORED,
        &DIALOG_ROUTE_TRANSACTION_KEY,
        &DIALOG_ROUTE_FALLBACK,
        &DIALOG_ROUTE_WORKER_MISMATCH,
        &DIALOG_ROUTE_INVITE,
        &DIALOG_ROUTE_ACK,
        &DIALOG_ROUTE_BYE,
        &DIALOG_ROUTE_CANCEL,
        &DIALOG_ROUTE_LIFECYCLE,
        &DIALOG_ROUTE_OTHER,
        &TERMINATION_CLEANUP_ENQUEUED,
        &TERMINATION_CLEANUP_QUEUE_FULL,
        &TERMINATION_CLEANUP_WORKER_SPAWNED,
        &TERMINATION_CLEANUP_IN_FLIGHT,
        &TERMINATION_CLEANUP_MAX_IN_FLIGHT,
        &TERMINATION_CLEANUP_POLL_ATTEMPTS,
        &TERMINATION_CLEANUP_REMOVED,
        &TERMINATION_CLEANUP_BATCHES,
        &TERMINATION_CLEANUP_BATCH_TOTAL,
        &TERMINATION_CLEANUP_BATCH_MAX,
        &TERMINATION_CLEANUP_INDEXED_SCAN_KEYS,
        &TERMINATION_CLEANUP_FULL_SCAN_CLIENT_KEYS,
        &TERMINATION_CLEANUP_FULL_SCAN_SERVER_KEYS,
        &INVITE_2XX_MAINTENANCE_TICKS,
        &INVITE_2XX_MAINTENANCE_CACHE_LEN_TOTAL,
        &INVITE_2XX_MAINTENANCE_CACHE_LEN_MAX,
        &INVITE_2XX_MAINTENANCE_DUE_QUEUE_LEN_TOTAL,
        &INVITE_2XX_MAINTENANCE_DUE_QUEUE_LEN_MAX,
        &INVITE_2XX_MAINTENANCE_SCANNED,
        &INVITE_2XX_MAINTENANCE_DUE,
        &INVITE_2XX_MAINTENANCE_EXPIRED,
        &INVITE_2XX_MAINTENANCE_CAPPED_TICKS,
        &GLOBAL_PUBLISH_COUNT,
        &GLOBAL_PUBLISH_HANDLER_COUNT_TOTAL,
        &GLOBAL_PUBLISH_HANDLER_COUNT_MAX,
        &GLOBAL_PUBLISH_INCOMING_CALL,
        &GLOBAL_PUBLISH_ACK,
        &GLOBAL_PUBLISH_BYE,
        &GLOBAL_PUBLISH_OTHER,
        &TRANSACTION_DISPATCH_QUEUE_DEPTH_TOTAL,
        &TRANSACTION_DISPATCH_QUEUE_DEPTH_MAX,
        &TRANSACTION_RUNNER_STARTED,
        &TRANSACTION_RUNNER_EXITED,
        &TRANSACTION_RUNNER_ACTIVE,
        &TRANSACTION_RUNNER_ACTIVE_MAX,
        &TRANSACTION_RUNNER_DESTROY_WAKE_SENT,
        &TRANSACTION_RUNNER_DESTROY_WAKE_FAILED,
        &BYE_TOMBSTONE_TABLE_SIZE_MAX,
    ]
}

fn transaction_latency_metrics() -> [&'static LatencyMetric; 21] {
    [
        &TRANSACTION_DISPATCH_QUEUE,
        &TRANSACTION_DISPATCH_QUEUE_INVITE,
        &TRANSACTION_DISPATCH_QUEUE_ACK,
        &TRANSACTION_DISPATCH_QUEUE_BYE,
        &TRANSACTION_DISPATCH_QUEUE_CANCEL,
        &TRANSACTION_DISPATCH_QUEUE_OTHER,
        &TRANSACTION_HANDLER_TOTAL,
        &TRANSACTION_HANDLER_INVITE,
        &TRANSACTION_HANDLER_ACK,
        &TRANSACTION_HANDLER_BYE,
        &TRANSACTION_HANDLER_CANCEL,
        &TRANSACTION_HANDLER_OTHER,
        &SERVER_TRANSACTION_CREATE,
        &EXISTING_TRANSACTION_DISPATCH,
        &TRANSACTION_EVENT_BROADCAST,
        &TRANSACTION_DISPATCH_BACKPRESSURE,
        &TERMINATION_CLEANUP_INDEXED_SCAN,
        &TERMINATION_CLEANUP_FULL_SCAN,
        &TERMINATION_CLEANUP_TIMER_UNREGISTER,
        &INVITE_2XX_MAINTENANCE,
        &INVITE_2XX_PROACTIVE_SEND,
    ]
}

fn dialog_latency_metrics() -> [&'static LatencyMetric; 26] {
    [
        &UDP_RECEIVE_TO_INVITE_200,
        &DIALOG_EVENT_DISPATCH_QUEUE,
        &DIALOG_EVENT_DISPATCH_BACKPRESSURE,
        &DIALOG_EVENT_HANDLER_TOTAL,
        &DIALOG_EVENT_HANDLER_INVITE,
        &DIALOG_EVENT_HANDLER_ACK,
        &DIALOG_EVENT_HANDLER_BYE,
        &DIALOG_EVENT_HANDLER_CANCEL,
        &DIALOG_EVENT_HANDLER_OTHER,
        &DIALOG_SESSION_PUBLISH_TOTAL,
        &DIALOG_SESSION_PUBLISH_INCOMING_CALL,
        &DIALOG_SESSION_PUBLISH_ACK,
        &DIALOG_SESSION_PUBLISH_BYE,
        &DIALOG_SESSION_PUBLISH_OTHER,
        &DIALOG_LOOKUP,
        &DIALOG_INITIAL_INVITE_SETUP,
        &BYE_PATH_UDP_TO_HANDLER,
        &BYE_PATH_TX_RECEIVED_TO_HANDLER,
        &BYE_PATH_HANDLER_TO_SEND_START,
        &BYE_PATH_SEND_RESPONSE,
        &BYE_PATH_RELEASE_TX,
        &BYE_PATH_CLEANUP_EMIT,
        &BYE_PATH_CLEANUP_REMOVE_STORAGE,
        &BYE_TOMBSTONE_LOOKUP,
        &BYE_TOMBSTONE_PRUNE,
        &GLOBAL_PUBLISH_TOTAL,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_latency_bucket_reports_finite_upper_bound() {
        let buckets = latency_buckets!();
        buckets[LATENCY_BUCKET_UPPER_US.len() - 1].store(1, Ordering::Relaxed);

        let p999 = percentile_per_mille_us(&buckets, 1, 999);

        assert_eq!(p999, 5_000_000);
        assert_ne!(p999, u64::MAX);
    }

    #[test]
    fn call_timing_traces_record_and_reset() {
        set_enabled_for_tests(true);
        reset();

        record_call_timing_uac_invite_2xx_response("call-a");
        record_call_timing_uac_ack_attempt("call-a");
        record_call_timing_uac_ack_success("call-a");
        record_call_timing_uac_call_answered_emit("call-a");
        record_call_timing_hub_response_invite_2xx("call-a");
        record_call_timing_hub_call_answered("call-a");
        record_call_timing_hub_ack_sent("call-a");
        record_call_timing_uas_ack_received("call-a");
        record_call_timing_lifecycle_call_answered("call-a");

        let first_snapshot = snapshot();
        assert_eq!(first_snapshot.call_timing_trace_overflow, 0);
        assert_eq!(first_snapshot.call_timing_traces.len(), 1);
        let trace = &first_snapshot.call_timing_traces[0];
        assert_eq!(trace.call_id, "call-a");
        assert!(trace.first_uac_invite_2xx_response_epoch_us.is_some());
        assert!(
            trace.last_uac_invite_2xx_response_epoch_us
                >= trace.first_uac_invite_2xx_response_epoch_us
        );
        assert!(trace.first_uac_ack_attempt_epoch_us.is_some());
        assert!(trace.first_uac_ack_success_epoch_us.is_some());
        assert!(trace.first_uac_call_answered_emit_epoch_us.is_some());
        assert!(trace.first_hub_response_invite_2xx_epoch_us.is_some());
        assert!(trace.first_hub_call_answered_epoch_us.is_some());
        assert!(trace.first_hub_ack_sent_epoch_us.is_some());
        assert!(trace.first_uas_ack_received_epoch_us.is_some());
        assert!(trace.first_lifecycle_call_answered_epoch_us.is_some());

        reset();
        assert!(snapshot().call_timing_traces.is_empty());
    }

    #[test]
    fn retransmission_diagnostics_format_counts() {
        set_enabled_for_tests(false);
        set_transaction_timing_enabled_for_tests(false);
        set_dialog_timing_enabled_for_tests(false);
        reset();
        record_duplicate_invite_existing_transaction();
        record_udp_receive_to_incoming_call_emit(Duration::from_micros(125));
        record_udp_receive_to_invite_200(Duration::from_micros(225));
        record_transaction_dispatch_queue_delay(Duration::from_micros(75));
        record_transaction_handler("invite", Duration::from_micros(100));
        record_server_transaction_create(Duration::from_micros(350));
        record_existing_transaction_dispatch(Duration::from_micros(400));
        record_transaction_event_broadcast(Duration::from_micros(450));
        record_transaction_dispatch_backpressure(Duration::from_micros(475));
        record_transaction_dispatch_queue_depth(13);
        record_transaction_dispatch_queue_by_worker_and_kind(
            2,
            "bye",
            Duration::from_micros(500),
            17,
        );
        record_bye_path_send_response(Duration::from_micros(525));
        record_bye_tombstone_lookup(Duration::from_micros(550));
        record_bye_tombstone_observed_size(19);
        record_dialog_event_dispatch_queue_delay(Duration::from_micros(80));
        record_dialog_event_handler("invite", Duration::from_micros(90));
        record_dialog_session_publish("incoming_call", Duration::from_micros(110));
        record_200_ok_invite_first();
        let disabled = snapshot();
        assert_eq!(disabled.duplicate_invite_existing_transaction, 0);
        assert_eq!(disabled.ok_200_invite_first, 0);
        assert_eq!(disabled.udp_receive_to_incoming_call_emit.count, 0);
        assert_eq!(disabled.udp_receive_to_invite_200.count, 0);
        assert_eq!(disabled.transaction_dispatch_queue.count, 0);
        assert_eq!(disabled.transaction_dispatch_backpressure.count, 0);
        assert_eq!(disabled.transaction_dispatch_queue_depth_max, 0);
        assert_eq!(
            disabled.transaction_dispatch_queue_by_worker[2].depth_max,
            0
        );
        assert_eq!(disabled.bye_path_send_response.count, 0);
        assert_eq!(disabled.bye_tombstone_lookup.count, 0);
        assert_eq!(disabled.bye_tombstone_table_size_max, 0);
        assert_eq!(disabled.transaction_handler_total.count, 0);
        assert_eq!(disabled.server_transaction_create.count, 0);
        assert_eq!(disabled.existing_transaction_dispatch.count, 0);
        assert_eq!(disabled.transaction_event_broadcast.count, 0);
        assert_eq!(disabled.dialog_event_dispatch_queue.count, 0);
        assert_eq!(disabled.dialog_event_handler_total.count, 0);
        assert_eq!(disabled.dialog_session_publish_total.count, 0);

        set_enabled_for_tests(true);
        set_transaction_timing_enabled_for_tests(false);
        set_dialog_timing_enabled_for_tests(false);
        reset();

        record_transaction_dispatch_queue_delay(Duration::from_micros(75));
        record_transaction_handler("invite", Duration::from_micros(100));
        record_server_transaction_create(Duration::from_micros(350));
        record_existing_transaction_dispatch(Duration::from_micros(400));
        record_transaction_event_broadcast(Duration::from_micros(450));
        record_transaction_dispatch_backpressure(Duration::from_micros(475));
        record_transaction_dispatch_queue_depth(13);
        record_transaction_dispatch_queue_by_worker_and_kind(
            2,
            "bye",
            Duration::from_micros(500),
            17,
        );
        record_bye_path_send_response(Duration::from_micros(525));
        record_bye_tombstone_lookup(Duration::from_micros(550));
        record_bye_tombstone_observed_size(19);
        record_dialog_event_dispatch_queue_delay(Duration::from_micros(80));
        record_dialog_event_handler("invite", Duration::from_micros(90));
        record_dialog_session_publish("incoming_call", Duration::from_micros(110));
        let transaction_disabled = snapshot();
        assert_eq!(transaction_disabled.transaction_dispatch_queue.count, 0);
        assert_eq!(
            transaction_disabled.transaction_dispatch_backpressure.count,
            0
        );
        assert_eq!(transaction_disabled.transaction_dispatch_queue_depth_max, 0);
        assert_eq!(
            transaction_disabled.transaction_dispatch_queue_by_worker[2].depth_max,
            0
        );
        assert_eq!(transaction_disabled.bye_path_send_response.count, 0);
        assert_eq!(transaction_disabled.bye_tombstone_lookup.count, 0);
        assert_eq!(transaction_disabled.bye_tombstone_table_size_max, 0);
        assert_eq!(transaction_disabled.transaction_handler_total.count, 0);
        assert_eq!(transaction_disabled.server_transaction_create.count, 0);
        assert_eq!(transaction_disabled.existing_transaction_dispatch.count, 0);
        assert_eq!(transaction_disabled.transaction_event_broadcast.count, 0);
        assert_eq!(transaction_disabled.dialog_event_dispatch_queue.count, 0);
        assert_eq!(transaction_disabled.dialog_event_handler_total.count, 0);
        assert_eq!(transaction_disabled.dialog_session_publish_total.count, 0);

        set_transaction_timing_enabled_for_tests(true);
        set_dialog_timing_enabled_for_tests(true);
        reset();

        record_duplicate_invite_existing_transaction();
        record_duplicate_invite_cache_hit();
        record_duplicate_invite_cache_miss();
        record_invite_2xx_cache_insert();
        record_invite_2xx_cache_expired();
        record_invite_2xx_proactive_retransmit();
        record_invite_2xx_ack_removed(Duration::from_millis(5));
        record_200_ok_invite_first();
        record_200_ok_invite_duplicate_cache();
        record_200_ok_invite_proactive_retransmit();
        record_200_ok_bye_fresh();
        record_200_ok_bye_tombstone();
        record_200_ok_bye_duplicate_terminated();
        record_200_ok_other();
        record_duplicate_bye_existing_transaction();
        record_duplicate_bye_tombstone_hit();
        record_duplicate_bye_tombstone_miss();
        record_duplicate_bye_terminated_dialog();
        record_udp_receive_to_incoming_call_emit(Duration::from_micros(125));
        record_bye_receive_to_200(Duration::from_micros(250));
        record_udp_receive_to_invite_200(Duration::from_micros(275));
        record_transaction_dispatch_queue_delay(Duration::from_micros(75));
        record_transaction_handler("invite", Duration::from_micros(100));
        record_transaction_handler("ack", Duration::from_micros(150));
        record_transaction_handler("bye", Duration::from_micros(200));
        record_transaction_handler("cancel", Duration::from_micros(250));
        record_transaction_handler("other", Duration::from_micros(300));
        record_server_transaction_create(Duration::from_micros(350));
        record_existing_transaction_dispatch(Duration::from_micros(400));
        record_transaction_event_broadcast(Duration::from_micros(450));
        record_transaction_dispatch_backpressure(Duration::from_micros(475));
        record_transaction_dispatch_queue_depth(9);
        record_transaction_dispatch_queue_by_worker_and_kind(
            2,
            "bye",
            Duration::from_micros(500),
            11,
        );
        record_dialog_event_dispatch_queue_delay(Duration::from_micros(75));
        record_dialog_event_dispatch_backpressure(Duration::from_micros(80));
        record_dialog_event_handler("invite", Duration::from_micros(100));
        record_dialog_event_handler("ack", Duration::from_micros(150));
        record_dialog_event_handler("bye", Duration::from_micros(200));
        record_dialog_event_handler("cancel", Duration::from_micros(250));
        record_dialog_event_handler("other", Duration::from_micros(300));
        record_dialog_session_publish("incoming_call", Duration::from_micros(125));
        record_dialog_session_publish("ack_received", Duration::from_micros(175));
        record_dialog_session_publish("bye_received", Duration::from_micros(225));
        record_dialog_session_publish("other", Duration::from_micros(275));
        record_dialog_lookup(Duration::from_micros(325));
        record_dialog_initial_invite_setup(Duration::from_micros(375));
        record_bye_path_udp_to_handler(Duration::from_micros(385));
        record_bye_path_tx_received_to_handler(Duration::from_micros(395));
        record_bye_path_handler_to_send_start(Duration::from_micros(405));
        record_bye_path_send_response(Duration::from_micros(415));
        record_bye_path_release_tx(Duration::from_micros(425));
        record_bye_path_cleanup_emit(Duration::from_micros(435));
        record_bye_path_cleanup_remove_storage(Duration::from_micros(445));
        record_bye_tombstone_lookup(Duration::from_micros(455));
        record_bye_tombstone_prune(Duration::from_micros(465));
        record_bye_tombstone_observed_size(17);
        record_dialog_route("request", "invite", true);
        record_dialog_route("stored", "lifecycle", false);
        record_termination_cleanup_enqueued();
        record_termination_cleanup_queue_full();
        record_termination_cleanup_worker_spawned();
        record_termination_cleanup_batch(3);
        record_termination_cleanup_in_flight(1);
        record_termination_cleanup_in_flight(-1);
        record_termination_cleanup_poll_attempts(2);
        record_termination_cleanup_removed();
        record_termination_cleanup_indexed_scan(5, Duration::from_micros(425));
        record_termination_cleanup_full_scan(7, 11, Duration::from_micros(475));
        record_termination_cleanup_timer_unregister(Duration::from_micros(525));
        record_invite_2xx_maintenance(13, 31, 17, 19, 23, true, Duration::from_micros(575));
        record_invite_2xx_proactive_send(Duration::from_micros(625));
        record_global_publish("incoming_call", 29, Duration::from_micros(675));

        let snapshot = snapshot();
        assert_eq!(snapshot.duplicate_invite_existing_transaction, 1);
        assert_eq!(snapshot.duplicate_invite_cache_hit, 1);
        assert_eq!(snapshot.ok_200_invite_first, 1);
        assert_eq!(snapshot.ok_200_invite_duplicate_cache, 1);
        assert_eq!(snapshot.ok_200_invite_proactive_retransmit, 1);
        assert_eq!(snapshot.ok_200_bye_fresh, 1);
        assert_eq!(snapshot.ok_200_bye_tombstone, 1);
        assert_eq!(snapshot.ok_200_bye_duplicate_terminated, 1);
        assert_eq!(snapshot.ok_200_other, 1);
        assert_eq!(snapshot.duplicate_bye_tombstone_hit, 1);
        assert_eq!(snapshot.udp_receive_to_incoming_call_emit.count, 1);
        assert_eq!(snapshot.bye_receive_to_200.count, 1);
        assert_eq!(snapshot.udp_receive_to_invite_200.count, 1);
        assert_eq!(snapshot.transaction_dispatch_queue.count, 2);
        assert_eq!(snapshot.transaction_dispatch_queue_bye.count, 1);
        assert_eq!(
            snapshot.transaction_dispatch_queue_by_worker[2].queue.count,
            1
        );
        assert_eq!(
            snapshot.transaction_dispatch_queue_by_worker[2].depth_max,
            11
        );
        assert_eq!(snapshot.transaction_handler_total.count, 5);
        assert_eq!(snapshot.transaction_handler_invite.count, 1);
        assert_eq!(snapshot.transaction_handler_ack.count, 1);
        assert_eq!(snapshot.transaction_handler_bye.count, 1);
        assert_eq!(snapshot.transaction_handler_cancel.count, 1);
        assert_eq!(snapshot.transaction_handler_other.count, 1);
        assert_eq!(snapshot.server_transaction_create.count, 1);
        assert_eq!(snapshot.existing_transaction_dispatch.count, 1);
        assert_eq!(snapshot.transaction_event_broadcast.count, 1);
        assert_eq!(snapshot.transaction_dispatch_backpressure.count, 1);
        assert_eq!(snapshot.transaction_dispatch_queue_depth_total, 20);
        assert_eq!(snapshot.transaction_dispatch_queue_depth_max, 11);
        assert_eq!(snapshot.dialog_event_dispatch_queue.count, 1);
        assert_eq!(snapshot.dialog_event_dispatch_backpressure.count, 1);
        assert_eq!(snapshot.dialog_event_handler_total.count, 5);
        assert_eq!(snapshot.dialog_event_handler_invite.count, 1);
        assert_eq!(snapshot.dialog_event_handler_ack.count, 1);
        assert_eq!(snapshot.dialog_event_handler_bye.count, 1);
        assert_eq!(snapshot.dialog_event_handler_cancel.count, 1);
        assert_eq!(snapshot.dialog_event_handler_other.count, 1);
        assert_eq!(snapshot.dialog_session_publish_total.count, 4);
        assert_eq!(snapshot.dialog_session_publish_incoming_call.count, 1);
        assert_eq!(snapshot.dialog_session_publish_ack.count, 1);
        assert_eq!(snapshot.dialog_session_publish_bye.count, 1);
        assert_eq!(snapshot.dialog_session_publish_other.count, 1);
        assert_eq!(snapshot.dialog_lookup.count, 1);
        assert_eq!(snapshot.dialog_initial_invite_setup.count, 1);
        assert_eq!(snapshot.dialog_route_request, 1);
        assert_eq!(snapshot.dialog_route_stored, 1);
        assert_eq!(snapshot.dialog_route_worker_mismatch, 1);
        assert_eq!(snapshot.dialog_route_lifecycle, 1);
        assert_eq!(snapshot.bye_path_udp_to_handler.count, 1);
        assert_eq!(snapshot.bye_path_send_response.count, 1);
        assert_eq!(snapshot.bye_path_cleanup_remove_storage.count, 1);
        assert_eq!(snapshot.bye_tombstone_lookup.count, 1);
        assert_eq!(snapshot.bye_tombstone_prune.count, 1);
        assert_eq!(snapshot.bye_tombstone_table_size_max, 17);
        assert_eq!(snapshot.termination_cleanup_enqueued, 1);
        assert_eq!(snapshot.termination_cleanup_queue_full, 1);
        assert_eq!(snapshot.termination_cleanup_worker_spawned, 1);
        assert_eq!(snapshot.termination_cleanup_batches, 1);
        assert_eq!(snapshot.termination_cleanup_batch_total, 3);
        assert_eq!(snapshot.termination_cleanup_max_in_flight, 1);
        assert_eq!(snapshot.termination_cleanup_poll_attempts, 2);
        assert_eq!(snapshot.termination_cleanup_removed, 1);
        assert_eq!(snapshot.termination_cleanup_indexed_scan_keys, 5);
        assert_eq!(snapshot.termination_cleanup_full_scan_client_keys, 7);
        assert_eq!(snapshot.termination_cleanup_full_scan_server_keys, 11);
        assert_eq!(snapshot.invite_2xx_maintenance_ticks, 1);
        assert_eq!(snapshot.invite_2xx_maintenance_cache_len_max, 13);
        assert_eq!(snapshot.invite_2xx_maintenance_due_queue_len_max, 31);
        assert_eq!(snapshot.invite_2xx_maintenance_scanned, 17);
        assert_eq!(snapshot.invite_2xx_maintenance_due, 19);
        assert_eq!(snapshot.invite_2xx_maintenance_expired, 23);
        assert_eq!(snapshot.invite_2xx_maintenance_capped_ticks, 1);
        assert_eq!(snapshot.global_publish_count, 1);
        assert_eq!(snapshot.global_publish_handler_count_max, 29);
        let summary = format_summary(&snapshot);
        assert!(summary.contains("dup_invite_cache_hit=1"));
        assert!(summary.contains("ok_200_source=[invite_first=1"));
        assert!(summary.contains("dup_bye_tombstone_hit=1"));
        assert!(summary.contains("udp_receive_to_incoming_call_emit=[count=1"));
        assert!(summary.contains("bye_receive_to_200=[count=1"));
        assert!(summary.contains("transaction_dispatch_queue=[count=2"));
        assert!(summary.contains("bye_path=[udp_to_handler=[count=1"));
        assert!(summary.contains("bye_tombstone=[hit=1 miss=1 table_size_max=17"));
        assert!(summary.contains("transaction_dispatch_queue_depth=[total=20 max=11]"));
        assert!(summary.contains("transaction_dispatch_queue_by_kind=[invite=[count=0"));
        assert!(summary.contains("bye=[count=1"));
        assert!(summary.contains("transaction_dispatch_queue_by_worker=[w2=[count=1"));
        assert!(summary.contains("transaction_dispatch_backpressure=[count=1"));
        assert!(summary.contains("transaction_handler=[total=[count=5"));
        assert!(summary.contains("udp_receive_to_invite_200=[count=1"));
        assert!(summary.contains("dialog_event_dispatch_queue=[count=1"));
        assert!(summary.contains("dialog_event_handler=[total=[count=5"));
        assert!(summary.contains("dialog_session_publish=[total=[count=4"));
        assert!(summary.contains("dialog_route=[request=1 stored=1"));
        assert!(summary.contains("termination_cleanup=[enqueued=1"));
        assert!(summary.contains("invite_2xx_maintenance=[ticks=1"));
        assert!(summary.contains("global_publish=[count=1"));
    }
}

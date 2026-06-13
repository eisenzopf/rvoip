//! Lightweight UDP/SIP transport diagnostics for high-rate benchmarks.
//!
//! Counters are only updated when enabled through `Config::sip_udp_diagnostics`
//! so the normal transport hot path does not pay an atomic cost during regular
//! use.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rvoip_sip_core::{Message, Method};

const SEND_BUCKET_LABELS: [&str; 6] = ["<100us", "<500us", "<1ms", "<5ms", "<10ms", ">=10ms"];
const MAX_ENDPOINT_SNAPSHOTS: usize = 64;
const MAX_CALL_TRACE_ENTRIES: usize = 20_000;
const LATENCY_BUCKET_UPPER_US: [u64; 18] = [
    10, 25, 50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 25_000, 50_000, 100_000, 250_000,
    500_000, 1_000_000, 2_500_000, 5_000_000,
];

static ENABLED_OVERRIDE: AtomicU8 = AtomicU8::new(0);

static UDP_DATAGRAMS_RECEIVED: AtomicU64 = AtomicU64::new(0);
static UDP_WORKER_QUEUE_ENQUEUED: AtomicU64 = AtomicU64::new(0);
static UDP_WORKER_QUEUE_FULL: AtomicU64 = AtomicU64::new(0);
static UDP_PARSE_OK: AtomicU64 = AtomicU64::new(0);
static UDP_PARSE_FAILED: AtomicU64 = AtomicU64::new(0);

static INBOUND_INVITE: AtomicU64 = AtomicU64::new(0);
static INBOUND_ACK: AtomicU64 = AtomicU64::new(0);
static INBOUND_BYE: AtomicU64 = AtomicU64::new(0);
static INBOUND_OTHER_REQUEST: AtomicU64 = AtomicU64::new(0);
static INBOUND_1XX: AtomicU64 = AtomicU64::new(0);
static INBOUND_INVITE_2XX: AtomicU64 = AtomicU64::new(0);
static INBOUND_2XX_OTHER: AtomicU64 = AtomicU64::new(0);
static INBOUND_3XX_6XX: AtomicU64 = AtomicU64::new(0);
static INBOUND_OTHER_RESPONSE: AtomicU64 = AtomicU64::new(0);
static CALL_TRACE_OVERFLOW: AtomicU64 = AtomicU64::new(0);

static TRANSPORT_CHANNEL_BACKPRESSURE_EVENTS: AtomicU64 = AtomicU64::new(0);
static TRANSPORT_CHANNEL_BACKPRESSURE_NS: AtomicU64 = AtomicU64::new(0);
static MANAGER_CHANNEL_BACKPRESSURE_EVENTS: AtomicU64 = AtomicU64::new(0);
static MANAGER_CHANNEL_BACKPRESSURE_NS: AtomicU64 = AtomicU64::new(0);

static OUTBOUND_SENDS: AtomicU64 = AtomicU64::new(0);
static OUTBOUND_SEND_ERRORS: AtomicU64 = AtomicU64::new(0);
static OUTBOUND_RAW_SENDS: AtomicU64 = AtomicU64::new(0);
static OUTBOUND_INVITE: AtomicU64 = AtomicU64::new(0);
static OUTBOUND_ACK: AtomicU64 = AtomicU64::new(0);
static OUTBOUND_BYE: AtomicU64 = AtomicU64::new(0);
static OUTBOUND_OTHER_REQUEST: AtomicU64 = AtomicU64::new(0);
static OUTBOUND_1XX: AtomicU64 = AtomicU64::new(0);
static OUTBOUND_2XX: AtomicU64 = AtomicU64::new(0);
static OUTBOUND_3XX_6XX: AtomicU64 = AtomicU64::new(0);
static OUTBOUND_OTHER_RESPONSE: AtomicU64 = AtomicU64::new(0);

static SEND_LT_100US: AtomicU64 = AtomicU64::new(0);
static SEND_LT_500US: AtomicU64 = AtomicU64::new(0);
static SEND_LT_1MS: AtomicU64 = AtomicU64::new(0);
static SEND_LT_5MS: AtomicU64 = AtomicU64::new(0);
static SEND_LT_10MS: AtomicU64 = AtomicU64::new(0);
static SEND_GE_10MS: AtomicU64 = AtomicU64::new(0);

static UDP_READ_TO_WORKER_QUEUE_COUNT: AtomicU64 = AtomicU64::new(0);
static UDP_READ_TO_WORKER_QUEUE_SUM_US: AtomicU64 = AtomicU64::new(0);
static UDP_READ_TO_WORKER_QUEUE_MAX_US: AtomicU64 = AtomicU64::new(0);
static UDP_READ_TO_WORKER_QUEUE_OVER_500MS: AtomicU64 = AtomicU64::new(0);
static UDP_READ_TO_WORKER_QUEUE_BUCKETS: [AtomicU64; 18] = [
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

static UDP_PARSE_COUNT: AtomicU64 = AtomicU64::new(0);
static UDP_PARSE_SUM_US: AtomicU64 = AtomicU64::new(0);
static UDP_PARSE_MAX_US: AtomicU64 = AtomicU64::new(0);
static UDP_PARSE_OVER_500MS: AtomicU64 = AtomicU64::new(0);
static UDP_PARSE_BUCKETS: [AtomicU64; 18] = [
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

static PARSE_TO_TRANSPORT_MANAGER_COUNT: AtomicU64 = AtomicU64::new(0);
static PARSE_TO_TRANSPORT_MANAGER_SUM_US: AtomicU64 = AtomicU64::new(0);
static PARSE_TO_TRANSPORT_MANAGER_MAX_US: AtomicU64 = AtomicU64::new(0);
static PARSE_TO_TRANSPORT_MANAGER_OVER_500MS: AtomicU64 = AtomicU64::new(0);
static PARSE_TO_TRANSPORT_MANAGER_BUCKETS: [AtomicU64; 18] = [
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

static TRANSPORT_MANAGER_TO_TRANSACTION_COUNT: AtomicU64 = AtomicU64::new(0);
static TRANSPORT_MANAGER_TO_TRANSACTION_SUM_US: AtomicU64 = AtomicU64::new(0);
static TRANSPORT_MANAGER_TO_TRANSACTION_MAX_US: AtomicU64 = AtomicU64::new(0);
static TRANSPORT_MANAGER_TO_TRANSACTION_OVER_500MS: AtomicU64 = AtomicU64::new(0);
static TRANSPORT_MANAGER_TO_TRANSACTION_BUCKETS: [AtomicU64; 18] = [
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

static UDP_RECEIVE_POLL_COUNT: AtomicU64 = AtomicU64::new(0);
static UDP_RECEIVE_POLL_SUM_US: AtomicU64 = AtomicU64::new(0);
static UDP_RECEIVE_POLL_MAX_US: AtomicU64 = AtomicU64::new(0);
static UDP_RECEIVE_POLL_OVER_500MS: AtomicU64 = AtomicU64::new(0);
static UDP_RECEIVE_POLL_BUCKETS: [AtomicU64; 18] = [
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

static UDP_RECEIVE_LOOP_GAP_COUNT: AtomicU64 = AtomicU64::new(0);
static UDP_RECEIVE_LOOP_GAP_SUM_US: AtomicU64 = AtomicU64::new(0);
static UDP_RECEIVE_LOOP_GAP_MAX_US: AtomicU64 = AtomicU64::new(0);
static UDP_RECEIVE_LOOP_GAP_OVER_500MS: AtomicU64 = AtomicU64::new(0);
static UDP_RECEIVE_LOOP_GAP_BUCKETS: [AtomicU64; 18] = [
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

static INBOUND_BY_SOURCE: LazyLock<Mutex<HashMap<SocketAddr, EndpointMethodCounts>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static INBOUND_BY_LOCAL: LazyLock<Mutex<HashMap<SocketAddr, EndpointMethodCounts>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static RECEIVE_LOOP_BY_LOCAL: LazyLock<Mutex<HashMap<SocketAddr, ReceiveLoopEndpointCounts>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static CALL_TRACES: LazyLock<Mutex<HashMap<String, CallTraceCounts>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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
pub struct EndpointMethodSnapshot {
    pub endpoint: String,
    pub total: u64,
    pub invite: u64,
    pub ack: u64,
    pub bye: u64,
    pub other_request: u64,
    pub response_1xx: u64,
    pub invite_2xx: u64,
    pub response_2xx_other: u64,
    pub response_3xx_6xx: u64,
    pub response_other: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReceiveLoopEndpointSnapshot {
    pub endpoint: String,
    pub datagrams: u64,
    pub max_gap_us: u64,
    pub over_500ms_gaps: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallTraceSnapshot {
    pub call_id: String,
    pub inbound_invite: u64,
    pub inbound_ack: u64,
    pub inbound_bye: u64,
    pub inbound_invite_2xx: u64,
    pub outbound_invite: u64,
    pub outbound_ack: u64,
    pub outbound_bye: u64,
    pub outbound_invite_2xx: u64,
    pub outbound_raw_invite_2xx: u64,
    pub outbound_target_send_errors: u64,
    pub first_inbound_invite_epoch_us: Option<u64>,
    pub last_inbound_invite_epoch_us: Option<u64>,
    pub first_inbound_ack_epoch_us: Option<u64>,
    pub last_inbound_ack_epoch_us: Option<u64>,
    pub first_inbound_bye_epoch_us: Option<u64>,
    pub last_inbound_bye_epoch_us: Option<u64>,
    pub first_inbound_invite_2xx_epoch_us: Option<u64>,
    pub last_inbound_invite_2xx_epoch_us: Option<u64>,
    pub first_outbound_invite_epoch_us: Option<u64>,
    pub last_outbound_invite_epoch_us: Option<u64>,
    pub first_outbound_ack_epoch_us: Option<u64>,
    pub last_outbound_ack_epoch_us: Option<u64>,
    pub first_outbound_bye_epoch_us: Option<u64>,
    pub last_outbound_bye_epoch_us: Option<u64>,
    pub first_outbound_invite_2xx_epoch_us: Option<u64>,
    pub last_outbound_invite_2xx_epoch_us: Option<u64>,
    pub first_outbound_raw_invite_2xx_epoch_us: Option<u64>,
    pub last_outbound_raw_invite_2xx_epoch_us: Option<u64>,
    pub first_inbound_source: Option<String>,
    pub last_inbound_source: Option<String>,
    pub first_inbound_local: Option<String>,
    pub last_inbound_local: Option<String>,
    pub first_outbound_local: Option<String>,
    pub last_outbound_local: Option<String>,
    pub first_outbound_destination: Option<String>,
    pub last_outbound_destination: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub udp_datagrams_received: u64,
    pub udp_worker_queue_enqueued: u64,
    pub udp_worker_queue_full: u64,
    pub udp_parse_ok: u64,
    pub udp_parse_failed: u64,
    pub inbound_invite: u64,
    pub inbound_ack: u64,
    pub inbound_bye: u64,
    pub inbound_other_request: u64,
    pub inbound_1xx: u64,
    pub inbound_invite_2xx: u64,
    pub inbound_2xx_other: u64,
    pub inbound_3xx_6xx: u64,
    pub inbound_other_response: u64,
    pub transport_channel_backpressure_events: u64,
    pub transport_channel_backpressure_ns: u64,
    pub manager_channel_backpressure_events: u64,
    pub manager_channel_backpressure_ns: u64,
    pub outbound_sends: u64,
    pub outbound_send_errors: u64,
    pub outbound_raw_sends: u64,
    pub outbound_invite: u64,
    pub outbound_ack: u64,
    pub outbound_bye: u64,
    pub outbound_other_request: u64,
    pub outbound_1xx: u64,
    pub outbound_2xx: u64,
    pub outbound_3xx_6xx: u64,
    pub outbound_other_response: u64,
    pub send_latency_buckets: [u64; 6],
    pub udp_read_to_worker_queue: LatencySnapshot,
    pub udp_receive_poll: LatencySnapshot,
    pub udp_receive_loop_gap: LatencySnapshot,
    pub udp_parse: LatencySnapshot,
    pub parse_to_transport_manager: LatencySnapshot,
    pub transport_manager_to_transaction: LatencySnapshot,
    pub inbound_by_source: Vec<EndpointMethodSnapshot>,
    pub inbound_by_local: Vec<EndpointMethodSnapshot>,
    pub receive_loop_by_local: Vec<ReceiveLoopEndpointSnapshot>,
    pub call_trace_overflow: u64,
    pub call_traces: Vec<CallTraceSnapshot>,
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
    for bucket in &UDP_READ_TO_WORKER_QUEUE_BUCKETS {
        bucket.store(0, Ordering::Relaxed);
    }
    for bucket in &UDP_PARSE_BUCKETS {
        bucket.store(0, Ordering::Relaxed);
    }
    for bucket in &PARSE_TO_TRANSPORT_MANAGER_BUCKETS {
        bucket.store(0, Ordering::Relaxed);
    }
    for bucket in &TRANSPORT_MANAGER_TO_TRANSACTION_BUCKETS {
        bucket.store(0, Ordering::Relaxed);
    }
    for bucket in &UDP_RECEIVE_POLL_BUCKETS {
        bucket.store(0, Ordering::Relaxed);
    }
    for bucket in &UDP_RECEIVE_LOOP_GAP_BUCKETS {
        bucket.store(0, Ordering::Relaxed);
    }
    if let Ok(mut by_source) = INBOUND_BY_SOURCE.lock() {
        by_source.clear();
    }
    if let Ok(mut by_local) = INBOUND_BY_LOCAL.lock() {
        by_local.clear();
    }
    if let Ok(mut by_local) = RECEIVE_LOOP_BY_LOCAL.lock() {
        by_local.clear();
    }
    if let Ok(mut call_traces) = CALL_TRACES.lock() {
        call_traces.clear();
    }
}

pub fn snapshot() -> Snapshot {
    Snapshot {
        udp_datagrams_received: UDP_DATAGRAMS_RECEIVED.load(Ordering::Relaxed),
        udp_worker_queue_enqueued: UDP_WORKER_QUEUE_ENQUEUED.load(Ordering::Relaxed),
        udp_worker_queue_full: UDP_WORKER_QUEUE_FULL.load(Ordering::Relaxed),
        udp_parse_ok: UDP_PARSE_OK.load(Ordering::Relaxed),
        udp_parse_failed: UDP_PARSE_FAILED.load(Ordering::Relaxed),
        inbound_invite: INBOUND_INVITE.load(Ordering::Relaxed),
        inbound_ack: INBOUND_ACK.load(Ordering::Relaxed),
        inbound_bye: INBOUND_BYE.load(Ordering::Relaxed),
        inbound_other_request: INBOUND_OTHER_REQUEST.load(Ordering::Relaxed),
        inbound_1xx: INBOUND_1XX.load(Ordering::Relaxed),
        inbound_invite_2xx: INBOUND_INVITE_2XX.load(Ordering::Relaxed),
        inbound_2xx_other: INBOUND_2XX_OTHER.load(Ordering::Relaxed),
        inbound_3xx_6xx: INBOUND_3XX_6XX.load(Ordering::Relaxed),
        inbound_other_response: INBOUND_OTHER_RESPONSE.load(Ordering::Relaxed),
        transport_channel_backpressure_events: TRANSPORT_CHANNEL_BACKPRESSURE_EVENTS
            .load(Ordering::Relaxed),
        transport_channel_backpressure_ns: TRANSPORT_CHANNEL_BACKPRESSURE_NS
            .load(Ordering::Relaxed),
        manager_channel_backpressure_events: MANAGER_CHANNEL_BACKPRESSURE_EVENTS
            .load(Ordering::Relaxed),
        manager_channel_backpressure_ns: MANAGER_CHANNEL_BACKPRESSURE_NS.load(Ordering::Relaxed),
        outbound_sends: OUTBOUND_SENDS.load(Ordering::Relaxed),
        outbound_send_errors: OUTBOUND_SEND_ERRORS.load(Ordering::Relaxed),
        outbound_raw_sends: OUTBOUND_RAW_SENDS.load(Ordering::Relaxed),
        outbound_invite: OUTBOUND_INVITE.load(Ordering::Relaxed),
        outbound_ack: OUTBOUND_ACK.load(Ordering::Relaxed),
        outbound_bye: OUTBOUND_BYE.load(Ordering::Relaxed),
        outbound_other_request: OUTBOUND_OTHER_REQUEST.load(Ordering::Relaxed),
        outbound_1xx: OUTBOUND_1XX.load(Ordering::Relaxed),
        outbound_2xx: OUTBOUND_2XX.load(Ordering::Relaxed),
        outbound_3xx_6xx: OUTBOUND_3XX_6XX.load(Ordering::Relaxed),
        outbound_other_response: OUTBOUND_OTHER_RESPONSE.load(Ordering::Relaxed),
        send_latency_buckets: [
            SEND_LT_100US.load(Ordering::Relaxed),
            SEND_LT_500US.load(Ordering::Relaxed),
            SEND_LT_1MS.load(Ordering::Relaxed),
            SEND_LT_5MS.load(Ordering::Relaxed),
            SEND_LT_10MS.load(Ordering::Relaxed),
            SEND_GE_10MS.load(Ordering::Relaxed),
        ],
        udp_read_to_worker_queue: latency_snapshot(
            &UDP_READ_TO_WORKER_QUEUE_BUCKETS,
            &UDP_READ_TO_WORKER_QUEUE_COUNT,
            &UDP_READ_TO_WORKER_QUEUE_SUM_US,
            &UDP_READ_TO_WORKER_QUEUE_MAX_US,
            &UDP_READ_TO_WORKER_QUEUE_OVER_500MS,
        ),
        udp_receive_poll: latency_snapshot(
            &UDP_RECEIVE_POLL_BUCKETS,
            &UDP_RECEIVE_POLL_COUNT,
            &UDP_RECEIVE_POLL_SUM_US,
            &UDP_RECEIVE_POLL_MAX_US,
            &UDP_RECEIVE_POLL_OVER_500MS,
        ),
        udp_receive_loop_gap: latency_snapshot(
            &UDP_RECEIVE_LOOP_GAP_BUCKETS,
            &UDP_RECEIVE_LOOP_GAP_COUNT,
            &UDP_RECEIVE_LOOP_GAP_SUM_US,
            &UDP_RECEIVE_LOOP_GAP_MAX_US,
            &UDP_RECEIVE_LOOP_GAP_OVER_500MS,
        ),
        udp_parse: latency_snapshot(
            &UDP_PARSE_BUCKETS,
            &UDP_PARSE_COUNT,
            &UDP_PARSE_SUM_US,
            &UDP_PARSE_MAX_US,
            &UDP_PARSE_OVER_500MS,
        ),
        parse_to_transport_manager: latency_snapshot(
            &PARSE_TO_TRANSPORT_MANAGER_BUCKETS,
            &PARSE_TO_TRANSPORT_MANAGER_COUNT,
            &PARSE_TO_TRANSPORT_MANAGER_SUM_US,
            &PARSE_TO_TRANSPORT_MANAGER_MAX_US,
            &PARSE_TO_TRANSPORT_MANAGER_OVER_500MS,
        ),
        transport_manager_to_transaction: latency_snapshot(
            &TRANSPORT_MANAGER_TO_TRANSACTION_BUCKETS,
            &TRANSPORT_MANAGER_TO_TRANSACTION_COUNT,
            &TRANSPORT_MANAGER_TO_TRANSACTION_SUM_US,
            &TRANSPORT_MANAGER_TO_TRANSACTION_MAX_US,
            &TRANSPORT_MANAGER_TO_TRANSACTION_OVER_500MS,
        ),
        inbound_by_source: endpoint_snapshot(&INBOUND_BY_SOURCE),
        inbound_by_local: endpoint_snapshot(&INBOUND_BY_LOCAL),
        receive_loop_by_local: receive_loop_endpoint_snapshot(),
        call_trace_overflow: CALL_TRACE_OVERFLOW.load(Ordering::Relaxed),
        call_traces: call_trace_snapshot(),
    }
}

pub fn format_summary(snapshot: &Snapshot) -> String {
    let transport_wait_ms = snapshot.transport_channel_backpressure_ns as f64 / 1_000_000.0;
    let manager_wait_ms = snapshot.manager_channel_backpressure_ns as f64 / 1_000_000.0;
    let buckets = SEND_BUCKET_LABELS
        .iter()
        .zip(snapshot.send_latency_buckets)
        .map(|(label, count)| format!("{label}={count}"))
        .collect::<Vec<_>>()
        .join(",");

    format!(
        "[sip_udp_diag] recv={} queued={} queue_full={} parse_ok={} parse_err={} \
         transport_backpressure_events={} transport_backpressure_ms={:.3} \
         manager_backpressure_events={} manager_backpressure_ms={:.3} \
         in_req_invite={} in_req_ack={} in_req_bye={} in_req_other={} \
         in_resp_1xx={} in_resp_invite_2xx={} in_resp_2xx_other={} in_resp_3xx_6xx={} in_resp_other={} \
         sends={} send_errors={} raw_sends={} req_invite={} req_ack={} req_bye={} req_other={} \
         resp_1xx={} resp_2xx={} resp_3xx_6xx={} resp_other={} send_latency=[{}] \
         udp_read_to_worker_queue=[{}] udp_receive_poll=[{}] udp_receive_loop_gap=[{}] \
         udp_parse=[{}] parse_to_transport_manager=[{}] \
         transport_manager_to_transaction=[{}]",
        snapshot.udp_datagrams_received,
        snapshot.udp_worker_queue_enqueued,
        snapshot.udp_worker_queue_full,
        snapshot.udp_parse_ok,
        snapshot.udp_parse_failed,
        snapshot.transport_channel_backpressure_events,
        transport_wait_ms,
        snapshot.manager_channel_backpressure_events,
        manager_wait_ms,
        snapshot.inbound_invite,
        snapshot.inbound_ack,
        snapshot.inbound_bye,
        snapshot.inbound_other_request,
        snapshot.inbound_1xx,
        snapshot.inbound_invite_2xx,
        snapshot.inbound_2xx_other,
        snapshot.inbound_3xx_6xx,
        snapshot.inbound_other_response,
        snapshot.outbound_sends,
        snapshot.outbound_send_errors,
        snapshot.outbound_raw_sends,
        snapshot.outbound_invite,
        snapshot.outbound_ack,
        snapshot.outbound_bye,
        snapshot.outbound_other_request,
        snapshot.outbound_1xx,
        snapshot.outbound_2xx,
        snapshot.outbound_3xx_6xx,
        snapshot.outbound_other_response,
        buckets,
        format_latency(&snapshot.udp_read_to_worker_queue),
        format_latency(&snapshot.udp_receive_poll),
        format_latency(&snapshot.udp_receive_loop_gap),
        format_latency(&snapshot.udp_parse),
        format_latency(&snapshot.parse_to_transport_manager),
        format_latency(&snapshot.transport_manager_to_transaction),
    )
}

pub(crate) fn record_udp_datagram_received() {
    increment(&UDP_DATAGRAMS_RECEIVED);
}

pub(crate) fn record_udp_worker_queue_enqueued() {
    increment(&UDP_WORKER_QUEUE_ENQUEUED);
}

pub(crate) fn record_udp_worker_queue_full() {
    increment(&UDP_WORKER_QUEUE_FULL);
}

pub(crate) fn record_udp_parse_ok() {
    increment(&UDP_PARSE_OK);
}

pub(crate) fn record_udp_parse_failed() {
    increment(&UDP_PARSE_FAILED);
}

pub(crate) fn record_inbound_message(
    message: &Message,
    source: SocketAddr,
    local_addr: SocketAddr,
) {
    if !enabled() {
        return;
    }

    let kind = WireKind::from_message(message);
    record_inbound_kind(kind);
    record_endpoint_kind(&INBOUND_BY_SOURCE, source, kind);
    record_endpoint_kind(&INBOUND_BY_LOCAL, local_addr, kind);

    if let Some(call_id) = message_call_id(message) {
        record_call_trace(&call_id, |trace| {
            trace.record_inbound(kind, source, local_addr);
        });
    }
}

pub fn record_udp_read_to_worker_queue(elapsed: Duration) {
    record_latency(
        elapsed,
        &UDP_READ_TO_WORKER_QUEUE_COUNT,
        &UDP_READ_TO_WORKER_QUEUE_SUM_US,
        &UDP_READ_TO_WORKER_QUEUE_MAX_US,
        &UDP_READ_TO_WORKER_QUEUE_OVER_500MS,
        &UDP_READ_TO_WORKER_QUEUE_BUCKETS,
    );
}

pub fn record_udp_receive_poll(elapsed: Duration) {
    record_latency(
        elapsed,
        &UDP_RECEIVE_POLL_COUNT,
        &UDP_RECEIVE_POLL_SUM_US,
        &UDP_RECEIVE_POLL_MAX_US,
        &UDP_RECEIVE_POLL_OVER_500MS,
        &UDP_RECEIVE_POLL_BUCKETS,
    );
}

pub fn record_udp_receive_loop_gap(local_addr: SocketAddr, elapsed: Duration) {
    record_latency(
        elapsed,
        &UDP_RECEIVE_LOOP_GAP_COUNT,
        &UDP_RECEIVE_LOOP_GAP_SUM_US,
        &UDP_RECEIVE_LOOP_GAP_MAX_US,
        &UDP_RECEIVE_LOOP_GAP_OVER_500MS,
        &UDP_RECEIVE_LOOP_GAP_BUCKETS,
    );

    if !enabled() {
        return;
    }
    let elapsed_us = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
    if let Ok(mut by_local) = RECEIVE_LOOP_BY_LOCAL.lock() {
        by_local
            .entry(local_addr)
            .or_default()
            .record_gap(elapsed_us);
    }
}

pub fn record_udp_parse(elapsed: Duration) {
    record_latency(
        elapsed,
        &UDP_PARSE_COUNT,
        &UDP_PARSE_SUM_US,
        &UDP_PARSE_MAX_US,
        &UDP_PARSE_OVER_500MS,
        &UDP_PARSE_BUCKETS,
    );
}

pub fn record_parse_to_transport_manager(elapsed: Duration) {
    record_latency(
        elapsed,
        &PARSE_TO_TRANSPORT_MANAGER_COUNT,
        &PARSE_TO_TRANSPORT_MANAGER_SUM_US,
        &PARSE_TO_TRANSPORT_MANAGER_MAX_US,
        &PARSE_TO_TRANSPORT_MANAGER_OVER_500MS,
        &PARSE_TO_TRANSPORT_MANAGER_BUCKETS,
    );
}

pub fn record_transport_manager_to_transaction(elapsed: Duration) {
    record_latency(
        elapsed,
        &TRANSPORT_MANAGER_TO_TRANSACTION_COUNT,
        &TRANSPORT_MANAGER_TO_TRANSACTION_SUM_US,
        &TRANSPORT_MANAGER_TO_TRANSACTION_MAX_US,
        &TRANSPORT_MANAGER_TO_TRANSACTION_OVER_500MS,
        &TRANSPORT_MANAGER_TO_TRANSACTION_BUCKETS,
    );
}

pub fn record_manager_channel_backpressure(wait: Duration) {
    if enabled() {
        MANAGER_CHANNEL_BACKPRESSURE_EVENTS.fetch_add(1, Ordering::Relaxed);
        MANAGER_CHANNEL_BACKPRESSURE_NS.fetch_add(ns(wait), Ordering::Relaxed);
    }
}

pub(crate) fn record_transport_channel_backpressure(wait: Duration) {
    if enabled() {
        TRANSPORT_CHANNEL_BACKPRESSURE_EVENTS.fetch_add(1, Ordering::Relaxed);
        TRANSPORT_CHANNEL_BACKPRESSURE_NS.fetch_add(ns(wait), Ordering::Relaxed);
    }
}

pub(crate) fn record_outbound_message(
    message: &Message,
    local_addr: SocketAddr,
    destination: SocketAddr,
    elapsed: Duration,
    failed: bool,
) {
    if !enabled() {
        return;
    }
    OUTBOUND_SENDS.fetch_add(1, Ordering::Relaxed);
    if failed {
        OUTBOUND_SEND_ERRORS.fetch_add(1, Ordering::Relaxed);
    }
    record_send_latency(elapsed);
    match message {
        Message::Request(request) => match request.method() {
            Method::Invite => increment_always(&OUTBOUND_INVITE),
            Method::Ack => increment_always(&OUTBOUND_ACK),
            Method::Bye => increment_always(&OUTBOUND_BYE),
            _ => increment_always(&OUTBOUND_OTHER_REQUEST),
        },
        Message::Response(response) => {
            let code = response.status_code();
            match code {
                100..=199 => increment_always(&OUTBOUND_1XX),
                200..=299 => increment_always(&OUTBOUND_2XX),
                300..=699 => increment_always(&OUTBOUND_3XX_6XX),
                _ => increment_always(&OUTBOUND_OTHER_RESPONSE),
            }
        }
    }
    if let Some(call_id) = message_call_id(message) {
        let kind = WireKind::from_message(message);
        record_call_trace(&call_id, |trace| {
            trace.record_outbound(kind, local_addr, destination, false, failed);
        });
    }
}

pub(crate) fn record_outbound_raw(
    bytes: &[u8],
    local_addr: SocketAddr,
    destination: SocketAddr,
    elapsed: Duration,
    failed: bool,
) {
    if !enabled() {
        return;
    }
    OUTBOUND_SENDS.fetch_add(1, Ordering::Relaxed);
    OUTBOUND_RAW_SENDS.fetch_add(1, Ordering::Relaxed);
    if failed {
        OUTBOUND_SEND_ERRORS.fetch_add(1, Ordering::Relaxed);
    }
    record_send_latency(elapsed);
    if let Ok(message) = rvoip_sip_core::parse_message(bytes) {
        if let Some(call_id) = message_call_id(&message) {
            let kind = WireKind::from_message(&message);
            record_call_trace(&call_id, |trace| {
                trace.record_outbound(kind, local_addr, destination, true, failed);
            });
        }
    }
}

fn record_inbound_kind(kind: WireKind) {
    match kind {
        WireKind::Invite => increment_always(&INBOUND_INVITE),
        WireKind::Ack => increment_always(&INBOUND_ACK),
        WireKind::Bye => increment_always(&INBOUND_BYE),
        WireKind::OtherRequest => increment_always(&INBOUND_OTHER_REQUEST),
        WireKind::Response1xx => increment_always(&INBOUND_1XX),
        WireKind::Invite2xx => increment_always(&INBOUND_INVITE_2XX),
        WireKind::Response2xxOther => increment_always(&INBOUND_2XX_OTHER),
        WireKind::Response3xx6xx => increment_always(&INBOUND_3XX_6XX),
        WireKind::ResponseOther => increment_always(&INBOUND_OTHER_RESPONSE),
    }
}

fn record_endpoint_kind(
    map: &LazyLock<Mutex<HashMap<SocketAddr, EndpointMethodCounts>>>,
    endpoint: SocketAddr,
    kind: WireKind,
) {
    if let Ok(mut map) = map.lock() {
        map.entry(endpoint).or_default().record(kind);
    }
}

fn record_call_trace(call_id: &str, update: impl FnOnce(&mut CallTraceCounts)) {
    let Ok(mut traces) = CALL_TRACES.lock() else {
        return;
    };
    if !traces.contains_key(call_id) && traces.len() >= MAX_CALL_TRACE_ENTRIES {
        CALL_TRACE_OVERFLOW.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let trace = traces.entry(call_id.to_string()).or_default();
    update(trace);
}

fn message_call_id(message: &Message) -> Option<String> {
    match message {
        Message::Request(request) => request.call_id().map(|call_id| call_id.value().to_string()),
        Message::Response(response) => response
            .call_id()
            .map(|call_id| call_id.value().to_string()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WireKind {
    Invite,
    Ack,
    Bye,
    OtherRequest,
    Response1xx,
    Invite2xx,
    Response2xxOther,
    Response3xx6xx,
    ResponseOther,
}

impl WireKind {
    fn from_message(message: &Message) -> Self {
        match message {
            Message::Request(request) => match request.method() {
                Method::Invite => Self::Invite,
                Method::Ack => Self::Ack,
                Method::Bye => Self::Bye,
                _ => Self::OtherRequest,
            },
            Message::Response(response) => {
                let code = response.status_code();
                match code {
                    100..=199 => Self::Response1xx,
                    200..=299 => {
                        if response
                            .cseq()
                            .is_some_and(|cseq| cseq.method == Method::Invite)
                        {
                            Self::Invite2xx
                        } else {
                            Self::Response2xxOther
                        }
                    }
                    300..=699 => Self::Response3xx6xx,
                    _ => Self::ResponseOther,
                }
            }
        }
    }

    fn is_call_trace_target(self) -> bool {
        matches!(self, Self::Invite | Self::Ack | Self::Bye | Self::Invite2xx)
    }
}

#[derive(Debug, Clone, Default)]
struct EndpointMethodCounts {
    total: u64,
    invite: u64,
    ack: u64,
    bye: u64,
    other_request: u64,
    response_1xx: u64,
    invite_2xx: u64,
    response_2xx_other: u64,
    response_3xx_6xx: u64,
    response_other: u64,
}

impl EndpointMethodCounts {
    fn record(&mut self, kind: WireKind) {
        self.total = self.total.saturating_add(1);
        match kind {
            WireKind::Invite => self.invite = self.invite.saturating_add(1),
            WireKind::Ack => self.ack = self.ack.saturating_add(1),
            WireKind::Bye => self.bye = self.bye.saturating_add(1),
            WireKind::OtherRequest => {
                self.other_request = self.other_request.saturating_add(1);
            }
            WireKind::Response1xx => self.response_1xx = self.response_1xx.saturating_add(1),
            WireKind::Invite2xx => self.invite_2xx = self.invite_2xx.saturating_add(1),
            WireKind::Response2xxOther => {
                self.response_2xx_other = self.response_2xx_other.saturating_add(1);
            }
            WireKind::Response3xx6xx => {
                self.response_3xx_6xx = self.response_3xx_6xx.saturating_add(1);
            }
            WireKind::ResponseOther => self.response_other = self.response_other.saturating_add(1),
        }
    }

    fn snapshot(&self, endpoint: SocketAddr) -> EndpointMethodSnapshot {
        EndpointMethodSnapshot {
            endpoint: endpoint.to_string(),
            total: self.total,
            invite: self.invite,
            ack: self.ack,
            bye: self.bye,
            other_request: self.other_request,
            response_1xx: self.response_1xx,
            invite_2xx: self.invite_2xx,
            response_2xx_other: self.response_2xx_other,
            response_3xx_6xx: self.response_3xx_6xx,
            response_other: self.response_other,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ReceiveLoopEndpointCounts {
    datagrams: u64,
    max_gap_us: u64,
    over_500ms_gaps: u64,
}

impl ReceiveLoopEndpointCounts {
    fn record_gap(&mut self, elapsed_us: u64) {
        self.datagrams = self.datagrams.saturating_add(1);
        self.max_gap_us = self.max_gap_us.max(elapsed_us);
        if elapsed_us > 500_000 {
            self.over_500ms_gaps = self.over_500ms_gaps.saturating_add(1);
        }
    }

    fn snapshot(&self, endpoint: SocketAddr) -> ReceiveLoopEndpointSnapshot {
        ReceiveLoopEndpointSnapshot {
            endpoint: endpoint.to_string(),
            datagrams: self.datagrams,
            max_gap_us: self.max_gap_us,
            over_500ms_gaps: self.over_500ms_gaps,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct CallTraceCounts {
    inbound_invite: u64,
    inbound_ack: u64,
    inbound_bye: u64,
    inbound_invite_2xx: u64,
    outbound_invite: u64,
    outbound_ack: u64,
    outbound_bye: u64,
    outbound_invite_2xx: u64,
    outbound_raw_invite_2xx: u64,
    outbound_target_send_errors: u64,
    first_inbound_invite_epoch_us: Option<u64>,
    last_inbound_invite_epoch_us: Option<u64>,
    first_inbound_ack_epoch_us: Option<u64>,
    last_inbound_ack_epoch_us: Option<u64>,
    first_inbound_bye_epoch_us: Option<u64>,
    last_inbound_bye_epoch_us: Option<u64>,
    first_inbound_invite_2xx_epoch_us: Option<u64>,
    last_inbound_invite_2xx_epoch_us: Option<u64>,
    first_outbound_invite_epoch_us: Option<u64>,
    last_outbound_invite_epoch_us: Option<u64>,
    first_outbound_ack_epoch_us: Option<u64>,
    last_outbound_ack_epoch_us: Option<u64>,
    first_outbound_bye_epoch_us: Option<u64>,
    last_outbound_bye_epoch_us: Option<u64>,
    first_outbound_invite_2xx_epoch_us: Option<u64>,
    last_outbound_invite_2xx_epoch_us: Option<u64>,
    first_outbound_raw_invite_2xx_epoch_us: Option<u64>,
    last_outbound_raw_invite_2xx_epoch_us: Option<u64>,
    first_inbound_source: Option<String>,
    last_inbound_source: Option<String>,
    first_inbound_local: Option<String>,
    last_inbound_local: Option<String>,
    first_outbound_local: Option<String>,
    last_outbound_local: Option<String>,
    first_outbound_destination: Option<String>,
    last_outbound_destination: Option<String>,
}

impl CallTraceCounts {
    fn record_inbound(&mut self, kind: WireKind, source: SocketAddr, local_addr: SocketAddr) {
        if !kind.is_call_trace_target() {
            return;
        }
        let now_us = epoch_us();
        match kind {
            WireKind::Invite => {
                self.inbound_invite = self.inbound_invite.saturating_add(1);
                set_first_last_u64(
                    &mut self.first_inbound_invite_epoch_us,
                    &mut self.last_inbound_invite_epoch_us,
                    now_us,
                );
            }
            WireKind::Ack => {
                self.inbound_ack = self.inbound_ack.saturating_add(1);
                set_first_last_u64(
                    &mut self.first_inbound_ack_epoch_us,
                    &mut self.last_inbound_ack_epoch_us,
                    now_us,
                );
            }
            WireKind::Bye => {
                self.inbound_bye = self.inbound_bye.saturating_add(1);
                set_first_last_u64(
                    &mut self.first_inbound_bye_epoch_us,
                    &mut self.last_inbound_bye_epoch_us,
                    now_us,
                );
            }
            WireKind::Invite2xx => {
                self.inbound_invite_2xx = self.inbound_invite_2xx.saturating_add(1);
                set_first_last_u64(
                    &mut self.first_inbound_invite_2xx_epoch_us,
                    &mut self.last_inbound_invite_2xx_epoch_us,
                    now_us,
                );
            }
            _ => {}
        }
        set_first_last(
            &mut self.first_inbound_source,
            &mut self.last_inbound_source,
            source.to_string(),
        );
        set_first_last(
            &mut self.first_inbound_local,
            &mut self.last_inbound_local,
            local_addr.to_string(),
        );
    }

    fn record_outbound(
        &mut self,
        kind: WireKind,
        local_addr: SocketAddr,
        destination: SocketAddr,
        raw: bool,
        failed: bool,
    ) {
        if !kind.is_call_trace_target() {
            return;
        }
        let now_us = epoch_us();
        match kind {
            WireKind::Invite => {
                self.outbound_invite = self.outbound_invite.saturating_add(1);
                set_first_last_u64(
                    &mut self.first_outbound_invite_epoch_us,
                    &mut self.last_outbound_invite_epoch_us,
                    now_us,
                );
            }
            WireKind::Ack => {
                self.outbound_ack = self.outbound_ack.saturating_add(1);
                set_first_last_u64(
                    &mut self.first_outbound_ack_epoch_us,
                    &mut self.last_outbound_ack_epoch_us,
                    now_us,
                );
            }
            WireKind::Bye => {
                self.outbound_bye = self.outbound_bye.saturating_add(1);
                set_first_last_u64(
                    &mut self.first_outbound_bye_epoch_us,
                    &mut self.last_outbound_bye_epoch_us,
                    now_us,
                );
            }
            WireKind::Invite2xx => {
                self.outbound_invite_2xx = self.outbound_invite_2xx.saturating_add(1);
                set_first_last_u64(
                    &mut self.first_outbound_invite_2xx_epoch_us,
                    &mut self.last_outbound_invite_2xx_epoch_us,
                    now_us,
                );
                if raw {
                    self.outbound_raw_invite_2xx = self.outbound_raw_invite_2xx.saturating_add(1);
                    set_first_last_u64(
                        &mut self.first_outbound_raw_invite_2xx_epoch_us,
                        &mut self.last_outbound_raw_invite_2xx_epoch_us,
                        now_us,
                    );
                }
            }
            _ => {}
        }
        if failed {
            self.outbound_target_send_errors = self.outbound_target_send_errors.saturating_add(1);
        }
        set_first_last(
            &mut self.first_outbound_local,
            &mut self.last_outbound_local,
            local_addr.to_string(),
        );
        set_first_last(
            &mut self.first_outbound_destination,
            &mut self.last_outbound_destination,
            destination.to_string(),
        );
    }

    fn snapshot(&self, call_id: String) -> CallTraceSnapshot {
        CallTraceSnapshot {
            call_id,
            inbound_invite: self.inbound_invite,
            inbound_ack: self.inbound_ack,
            inbound_bye: self.inbound_bye,
            inbound_invite_2xx: self.inbound_invite_2xx,
            outbound_invite: self.outbound_invite,
            outbound_ack: self.outbound_ack,
            outbound_bye: self.outbound_bye,
            outbound_invite_2xx: self.outbound_invite_2xx,
            outbound_raw_invite_2xx: self.outbound_raw_invite_2xx,
            outbound_target_send_errors: self.outbound_target_send_errors,
            first_inbound_invite_epoch_us: self.first_inbound_invite_epoch_us,
            last_inbound_invite_epoch_us: self.last_inbound_invite_epoch_us,
            first_inbound_ack_epoch_us: self.first_inbound_ack_epoch_us,
            last_inbound_ack_epoch_us: self.last_inbound_ack_epoch_us,
            first_inbound_bye_epoch_us: self.first_inbound_bye_epoch_us,
            last_inbound_bye_epoch_us: self.last_inbound_bye_epoch_us,
            first_inbound_invite_2xx_epoch_us: self.first_inbound_invite_2xx_epoch_us,
            last_inbound_invite_2xx_epoch_us: self.last_inbound_invite_2xx_epoch_us,
            first_outbound_invite_epoch_us: self.first_outbound_invite_epoch_us,
            last_outbound_invite_epoch_us: self.last_outbound_invite_epoch_us,
            first_outbound_ack_epoch_us: self.first_outbound_ack_epoch_us,
            last_outbound_ack_epoch_us: self.last_outbound_ack_epoch_us,
            first_outbound_bye_epoch_us: self.first_outbound_bye_epoch_us,
            last_outbound_bye_epoch_us: self.last_outbound_bye_epoch_us,
            first_outbound_invite_2xx_epoch_us: self.first_outbound_invite_2xx_epoch_us,
            last_outbound_invite_2xx_epoch_us: self.last_outbound_invite_2xx_epoch_us,
            first_outbound_raw_invite_2xx_epoch_us: self.first_outbound_raw_invite_2xx_epoch_us,
            last_outbound_raw_invite_2xx_epoch_us: self.last_outbound_raw_invite_2xx_epoch_us,
            first_inbound_source: self.first_inbound_source.clone(),
            last_inbound_source: self.last_inbound_source.clone(),
            first_inbound_local: self.first_inbound_local.clone(),
            last_inbound_local: self.last_inbound_local.clone(),
            first_outbound_local: self.first_outbound_local.clone(),
            last_outbound_local: self.last_outbound_local.clone(),
            first_outbound_destination: self.first_outbound_destination.clone(),
            last_outbound_destination: self.last_outbound_destination.clone(),
        }
    }
}

fn set_first_last(first: &mut Option<String>, last: &mut Option<String>, value: String) {
    if first.is_none() {
        *first = Some(value.clone());
    }
    *last = Some(value);
}

fn set_first_last_u64(first: &mut Option<u64>, last: &mut Option<u64>, value: u64) {
    if first.is_none() {
        *first = Some(value);
    }
    *last = Some(value);
}

fn endpoint_snapshot(
    map: &LazyLock<Mutex<HashMap<SocketAddr, EndpointMethodCounts>>>,
) -> Vec<EndpointMethodSnapshot> {
    let Ok(map) = map.lock() else {
        return Vec::new();
    };
    let mut rows = map
        .iter()
        .map(|(endpoint, counts)| counts.snapshot(*endpoint))
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        b.total
            .cmp(&a.total)
            .then_with(|| a.endpoint.cmp(&b.endpoint))
    });
    rows.truncate(MAX_ENDPOINT_SNAPSHOTS);
    rows
}

fn receive_loop_endpoint_snapshot() -> Vec<ReceiveLoopEndpointSnapshot> {
    let Ok(map) = RECEIVE_LOOP_BY_LOCAL.lock() else {
        return Vec::new();
    };
    let mut rows = map
        .iter()
        .map(|(endpoint, counts)| counts.snapshot(*endpoint))
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        b.max_gap_us
            .cmp(&a.max_gap_us)
            .then_with(|| b.datagrams.cmp(&a.datagrams))
            .then_with(|| a.endpoint.cmp(&b.endpoint))
    });
    rows.truncate(MAX_ENDPOINT_SNAPSHOTS);
    rows
}

fn call_trace_snapshot() -> Vec<CallTraceSnapshot> {
    let Ok(traces) = CALL_TRACES.lock() else {
        return Vec::new();
    };
    let mut rows = traces
        .iter()
        .map(|(call_id, trace)| trace.snapshot(call_id.clone()))
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| a.call_id.cmp(&b.call_id));
    rows
}

fn record_send_latency(elapsed: Duration) {
    let us = elapsed.as_micros();
    let bucket = if us < 100 {
        &SEND_LT_100US
    } else if us < 500 {
        &SEND_LT_500US
    } else if us < 1_000 {
        &SEND_LT_1MS
    } else if us < 5_000 {
        &SEND_LT_5MS
    } else if us < 10_000 {
        &SEND_LT_10MS
    } else {
        &SEND_GE_10MS
    };
    increment_always(bucket);
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

fn latency_bucket_index(elapsed_us: u64) -> usize {
    let bucketed_us = elapsed_us.min(*LATENCY_BUCKET_UPPER_US.last().unwrap());
    LATENCY_BUCKET_UPPER_US
        .iter()
        .position(|upper| bucketed_us <= *upper)
        .unwrap_or(LATENCY_BUCKET_UPPER_US.len() - 1)
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

fn increment(counter: &AtomicU64) {
    if enabled() {
        increment_always(counter);
    }
}

fn increment_always(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
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

fn all_counters() -> [&'static AtomicU64; 60] {
    [
        &UDP_DATAGRAMS_RECEIVED,
        &UDP_WORKER_QUEUE_ENQUEUED,
        &UDP_WORKER_QUEUE_FULL,
        &UDP_PARSE_OK,
        &UDP_PARSE_FAILED,
        &INBOUND_INVITE,
        &INBOUND_ACK,
        &INBOUND_BYE,
        &INBOUND_OTHER_REQUEST,
        &INBOUND_1XX,
        &INBOUND_INVITE_2XX,
        &INBOUND_2XX_OTHER,
        &INBOUND_3XX_6XX,
        &INBOUND_OTHER_RESPONSE,
        &CALL_TRACE_OVERFLOW,
        &TRANSPORT_CHANNEL_BACKPRESSURE_EVENTS,
        &TRANSPORT_CHANNEL_BACKPRESSURE_NS,
        &MANAGER_CHANNEL_BACKPRESSURE_EVENTS,
        &MANAGER_CHANNEL_BACKPRESSURE_NS,
        &OUTBOUND_SENDS,
        &OUTBOUND_SEND_ERRORS,
        &OUTBOUND_RAW_SENDS,
        &OUTBOUND_INVITE,
        &OUTBOUND_ACK,
        &OUTBOUND_BYE,
        &OUTBOUND_OTHER_REQUEST,
        &OUTBOUND_1XX,
        &OUTBOUND_2XX,
        &OUTBOUND_3XX_6XX,
        &OUTBOUND_OTHER_RESPONSE,
        &SEND_LT_100US,
        &SEND_LT_500US,
        &SEND_LT_1MS,
        &SEND_LT_5MS,
        &SEND_LT_10MS,
        &SEND_GE_10MS,
        &UDP_READ_TO_WORKER_QUEUE_COUNT,
        &UDP_READ_TO_WORKER_QUEUE_SUM_US,
        &UDP_READ_TO_WORKER_QUEUE_MAX_US,
        &UDP_READ_TO_WORKER_QUEUE_OVER_500MS,
        &UDP_PARSE_COUNT,
        &UDP_PARSE_SUM_US,
        &UDP_PARSE_MAX_US,
        &UDP_PARSE_OVER_500MS,
        &PARSE_TO_TRANSPORT_MANAGER_COUNT,
        &PARSE_TO_TRANSPORT_MANAGER_SUM_US,
        &PARSE_TO_TRANSPORT_MANAGER_MAX_US,
        &PARSE_TO_TRANSPORT_MANAGER_OVER_500MS,
        &TRANSPORT_MANAGER_TO_TRANSACTION_COUNT,
        &TRANSPORT_MANAGER_TO_TRANSACTION_SUM_US,
        &TRANSPORT_MANAGER_TO_TRANSACTION_MAX_US,
        &TRANSPORT_MANAGER_TO_TRANSACTION_OVER_500MS,
        &UDP_RECEIVE_POLL_COUNT,
        &UDP_RECEIVE_POLL_SUM_US,
        &UDP_RECEIVE_POLL_MAX_US,
        &UDP_RECEIVE_POLL_OVER_500MS,
        &UDP_RECEIVE_LOOP_GAP_COUNT,
        &UDP_RECEIVE_LOOP_GAP_SUM_US,
        &UDP_RECEIVE_LOOP_GAP_MAX_US,
        &UDP_RECEIVE_LOOP_GAP_OVER_500MS,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};

    #[test]
    fn diagnostics_summary_includes_counter_values() {
        set_enabled_for_tests(false);
        reset();
        record_udp_datagram_received();
        record_udp_read_to_worker_queue(Duration::from_micros(25));
        record_udp_receive_poll(Duration::from_micros(30));
        let disabled = snapshot();
        assert_eq!(disabled.udp_datagrams_received, 0);
        assert_eq!(disabled.udp_read_to_worker_queue.count, 0);
        assert_eq!(disabled.udp_receive_poll.count, 0);

        set_enabled_for_tests(true);
        reset();

        record_udp_datagram_received();
        record_udp_worker_queue_enqueued();
        record_udp_worker_queue_full();
        record_udp_parse_ok();
        record_udp_parse_failed();
        record_udp_read_to_worker_queue(Duration::from_micros(25));
        record_udp_receive_poll(Duration::from_micros(30));
        record_udp_parse(Duration::from_micros(50));
        record_parse_to_transport_manager(Duration::from_micros(75));
        record_transport_manager_to_transaction(Duration::from_micros(100));
        record_transport_channel_backpressure(Duration::from_millis(2));
        record_manager_channel_backpressure(Duration::from_millis(3));

        let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("a"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("diag-test")
            .cseq(1)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-diag"))
            .build();
        let invite_200 = SimpleResponseBuilder::response_from_request(
            &request,
            rvoip_sip_core::StatusCode::Ok,
            Some("OK"),
        )
        .build();
        let local_addr = "127.0.0.1:5060".parse().unwrap();
        let peer_addr = "127.0.0.1:5070".parse().unwrap();
        record_udp_receive_loop_gap(local_addr, Duration::from_millis(750));
        record_inbound_message(&Message::Request(request.clone()), peer_addr, local_addr);
        record_inbound_message(
            &Message::Response(invite_200.clone()),
            peer_addr,
            local_addr,
        );
        record_outbound_message(
            &Message::Request(request),
            local_addr,
            peer_addr,
            Duration::from_micros(50),
            false,
        );
        record_outbound_raw(
            &Message::Response(invite_200).to_bytes(),
            local_addr,
            peer_addr,
            Duration::from_millis(11),
            true,
        );

        let snapshot = snapshot();
        assert_eq!(snapshot.udp_datagrams_received, 1);
        assert_eq!(snapshot.udp_worker_queue_enqueued, 1);
        assert_eq!(snapshot.udp_worker_queue_full, 1);
        assert_eq!(snapshot.udp_parse_ok, 1);
        assert_eq!(snapshot.udp_parse_failed, 1);
        assert_eq!(snapshot.udp_read_to_worker_queue.count, 1);
        assert_eq!(snapshot.udp_receive_poll.count, 1);
        assert_eq!(snapshot.udp_receive_loop_gap.count, 1);
        assert_eq!(snapshot.udp_receive_loop_gap.over_500ms, 1);
        assert_eq!(snapshot.udp_parse.count, 1);
        assert_eq!(snapshot.parse_to_transport_manager.count, 1);
        assert_eq!(snapshot.transport_manager_to_transaction.count, 1);
        assert_eq!(snapshot.transport_channel_backpressure_events, 1);
        assert_eq!(snapshot.manager_channel_backpressure_events, 1);
        assert_eq!(snapshot.outbound_sends, 2);
        assert_eq!(snapshot.outbound_send_errors, 1);
        assert_eq!(snapshot.outbound_invite, 1);
        assert_eq!(snapshot.inbound_invite, 1);
        assert_eq!(snapshot.inbound_invite_2xx, 1);
        assert_eq!(snapshot.inbound_by_source.len(), 1);
        assert_eq!(snapshot.receive_loop_by_local.len(), 1);
        assert_eq!(snapshot.receive_loop_by_local[0].max_gap_us, 750_000);
        assert_eq!(snapshot.receive_loop_by_local[0].over_500ms_gaps, 1);
        assert_eq!(snapshot.call_traces.len(), 1);
        let trace = &snapshot.call_traces[0];
        assert_eq!(trace.inbound_invite, 1);
        assert_eq!(trace.inbound_invite_2xx, 1);
        assert_eq!(trace.outbound_invite, 1);
        assert_eq!(trace.outbound_raw_invite_2xx, 1);
        assert!(trace.first_inbound_invite_epoch_us.is_some());
        assert!(trace.last_inbound_invite_epoch_us >= trace.first_inbound_invite_epoch_us);
        assert!(trace.first_inbound_invite_2xx_epoch_us.is_some());
        assert!(trace.first_outbound_invite_epoch_us.is_some());
        assert!(trace.first_outbound_raw_invite_2xx_epoch_us.is_some());
        assert!(format_summary(&snapshot).contains("recv=1"));
        assert!(format_summary(&snapshot).contains("in_req_invite=1"));
        assert!(format_summary(&snapshot).contains("udp_read_to_worker_queue=[count=1"));
        assert!(format_summary(&snapshot).contains("udp_receive_loop_gap=[count=1"));
    }

    #[test]
    fn overflow_latency_bucket_reports_finite_upper_bound() {
        let buckets: [AtomicU64; 18] = std::array::from_fn(|_| AtomicU64::new(0));
        buckets[LATENCY_BUCKET_UPPER_US.len() - 1].store(1, Ordering::Relaxed);

        let p999 = percentile_per_mille_us(&buckets, 1, 999);

        assert_eq!(p999, 5_000_000);
        assert_ne!(p999, u64::MAX);
    }
}

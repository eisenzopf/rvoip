//! Lightweight UDP/SIP transport diagnostics for high-rate benchmarks.
//!
//! Counters are only updated when enabled through `Config::sip_udp_diagnostics`
//! so the normal transport hot path does not pay an atomic cost during regular
//! use.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::time::Duration;

use rvoip_sip_core::{Message, Method};

const SEND_BUCKET_LABELS: [&str; 6] = ["<100us", "<500us", "<1ms", "<5ms", "<10ms", ">=10ms"];

static ENABLED_OVERRIDE: AtomicU8 = AtomicU8::new(0);

static UDP_DATAGRAMS_RECEIVED: AtomicU64 = AtomicU64::new(0);
static UDP_WORKER_QUEUE_ENQUEUED: AtomicU64 = AtomicU64::new(0);
static UDP_WORKER_QUEUE_FULL: AtomicU64 = AtomicU64::new(0);
static UDP_PARSE_OK: AtomicU64 = AtomicU64::new(0);
static UDP_PARSE_FAILED: AtomicU64 = AtomicU64::new(0);

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub udp_datagrams_received: u64,
    pub udp_worker_queue_enqueued: u64,
    pub udp_worker_queue_full: u64,
    pub udp_parse_ok: u64,
    pub udp_parse_failed: u64,
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
        udp_datagrams_received: UDP_DATAGRAMS_RECEIVED.load(Ordering::Relaxed),
        udp_worker_queue_enqueued: UDP_WORKER_QUEUE_ENQUEUED.load(Ordering::Relaxed),
        udp_worker_queue_full: UDP_WORKER_QUEUE_FULL.load(Ordering::Relaxed),
        udp_parse_ok: UDP_PARSE_OK.load(Ordering::Relaxed),
        udp_parse_failed: UDP_PARSE_FAILED.load(Ordering::Relaxed),
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
         sends={} send_errors={} raw_sends={} req_invite={} req_ack={} req_bye={} req_other={} \
         resp_1xx={} resp_2xx={} resp_3xx_6xx={} resp_other={} send_latency=[{}]",
        snapshot.udp_datagrams_received,
        snapshot.udp_worker_queue_enqueued,
        snapshot.udp_worker_queue_full,
        snapshot.udp_parse_ok,
        snapshot.udp_parse_failed,
        snapshot.transport_channel_backpressure_events,
        transport_wait_ms,
        snapshot.manager_channel_backpressure_events,
        manager_wait_ms,
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

pub(crate) fn record_outbound_message(message: &Message, elapsed: Duration, failed: bool) {
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
}

pub(crate) fn record_outbound_raw(elapsed: Duration, failed: bool) {
    if !enabled() {
        return;
    }
    OUTBOUND_SENDS.fetch_add(1, Ordering::Relaxed);
    OUTBOUND_RAW_SENDS.fetch_add(1, Ordering::Relaxed);
    if failed {
        OUTBOUND_SEND_ERRORS.fetch_add(1, Ordering::Relaxed);
    }
    record_send_latency(elapsed);
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

fn all_counters() -> [&'static AtomicU64; 26] {
    [
        &UDP_DATAGRAMS_RECEIVED,
        &UDP_WORKER_QUEUE_ENQUEUED,
        &UDP_WORKER_QUEUE_FULL,
        &UDP_PARSE_OK,
        &UDP_PARSE_FAILED,
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
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::builder::SimpleRequestBuilder;

    #[test]
    fn diagnostics_summary_includes_counter_values() {
        set_enabled_for_tests(true);
        reset();

        record_udp_datagram_received();
        record_udp_worker_queue_enqueued();
        record_udp_worker_queue_full();
        record_udp_parse_ok();
        record_udp_parse_failed();
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
        record_outbound_message(&Message::Request(request), Duration::from_micros(50), false);
        record_outbound_raw(Duration::from_millis(11), true);

        let snapshot = snapshot();
        assert_eq!(snapshot.udp_datagrams_received, 1);
        assert_eq!(snapshot.udp_worker_queue_enqueued, 1);
        assert_eq!(snapshot.udp_worker_queue_full, 1);
        assert_eq!(snapshot.udp_parse_ok, 1);
        assert_eq!(snapshot.udp_parse_failed, 1);
        assert_eq!(snapshot.transport_channel_backpressure_events, 1);
        assert_eq!(snapshot.manager_channel_backpressure_events, 1);
        assert_eq!(snapshot.outbound_sends, 2);
        assert_eq!(snapshot.outbound_send_errors, 1);
        assert_eq!(snapshot.outbound_invite, 1);
        assert!(format_summary(&snapshot).contains("recv=1"));
    }
}

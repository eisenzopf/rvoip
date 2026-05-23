//! SIP transaction/dialog diagnostics for duplicate recovery under UDP load.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

static ENABLED: OnceLock<bool> = OnceLock::new();
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
}

pub fn enabled() -> bool {
    match ENABLED_OVERRIDE.load(Ordering::Relaxed) {
        1 => return false,
        2 => return true,
        _ => {}
    }
    *ENABLED.get_or_init(|| {
        matches!(
            std::env::var("RVOIP_SIP_UDP_DIAGNOSTICS").ok().as_deref(),
            Some("1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
        )
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
         dup_bye_terminated_dialog={}",
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

fn increment(counter: &AtomicU64) {
    if enabled() {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

fn ns(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

fn all_counters() -> [&'static AtomicU64; 12] {
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

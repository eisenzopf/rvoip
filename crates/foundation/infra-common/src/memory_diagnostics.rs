//! Perf-only memory retention diagnostics.
//!
//! This module is intentionally feature-gated behind `memory-diagnostics`.
//! Production builds do not compile the counters, allocator snapshots, or
//! diagnostic task wrappers.

use std::backtrace::Backtrace;
use std::collections::VecDeque;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::task::JoinHandle;

const BACKTRACE_SAMPLE_LIMIT: usize = 32;

static REGISTRY: OnceLock<Registry> = OnceLock::new();

fn registry() -> &'static Registry {
    REGISTRY.get_or_init(Registry::default)
}

#[derive(Default)]
struct Registry {
    kinds: DashMap<&'static str, Arc<ObjectCounters>>,
    backtraces: Mutex<VecDeque<BacktraceSample>>,
}

#[derive(Default)]
struct ObjectCounters {
    created: AtomicU64,
    dropped: AtomicU64,
    bytes_created: AtomicU64,
    bytes_dropped: AtomicU64,
    live: AtomicU64,
    peak_live: AtomicU64,
    bytes_live: AtomicU64,
    peak_bytes: AtomicU64,
    checkouts: AtomicU64,
    returns: AtomicU64,
    dropped_full: AtomicU64,
}

#[derive(Debug, Clone, Serialize)]
struct BacktraceSample {
    kind: &'static str,
    bytes: u64,
    timestamp_millis: u128,
    backtrace: String,
}

#[derive(Debug)]
pub struct ObjectGuard {
    kind: &'static str,
    bytes: u64,
}

impl ObjectGuard {
    pub fn new(kind: &'static str, bytes: impl TryInto<u64>) -> Self {
        let bytes = bytes.try_into().unwrap_or(0);
        record_created(kind, bytes);
        Self { kind, bytes }
    }
}

impl Drop for ObjectGuard {
    fn drop(&mut self) {
        record_dropped(self.kind, self.bytes);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryDiagnosticSummary {
    pub kind: &'static str,
    pub created: u64,
    pub dropped: u64,
    pub bytes_created: u64,
    pub bytes_dropped: u64,
    pub live: u64,
    pub peak_live: u64,
    pub bytes_live: u64,
    pub peak_bytes: u64,
    pub checkouts: u64,
    pub returns: u64,
    pub dropped_full: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AllocatorProcessInfo {
    pub elapsed_msecs: usize,
    pub user_msecs: usize,
    pub system_msecs: usize,
    pub current_rss_bytes: usize,
    pub peak_rss_bytes: usize,
    pub current_commit_bytes: usize,
    pub peak_commit_bytes: usize,
    pub page_faults: usize,
}

pub fn record_created(kind: &'static str, bytes: impl TryInto<u64>) {
    let bytes = bytes.try_into().unwrap_or(0);
    let counters = counters(kind);
    counters.created.fetch_add(1, Ordering::Relaxed);
    if bytes > 0 {
        counters.bytes_created.fetch_add(bytes, Ordering::Relaxed);
    }
    let live = counters.live.fetch_add(1, Ordering::Relaxed) + 1;
    update_max(&counters.peak_live, live);
    if bytes > 0 {
        let bytes_live = counters.bytes_live.fetch_add(bytes, Ordering::Relaxed) + bytes;
        update_max(&counters.peak_bytes, bytes_live);
    }
    maybe_record_backtrace(kind, bytes);
}

pub fn record_dropped(kind: &'static str, bytes: impl TryInto<u64>) {
    let bytes = bytes.try_into().unwrap_or(0);
    let counters = counters(kind);
    counters.dropped.fetch_add(1, Ordering::Relaxed);
    if bytes > 0 {
        counters.bytes_dropped.fetch_add(bytes, Ordering::Relaxed);
    }
    counters
        .live
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            Some(value.saturating_sub(1))
        })
        .ok();
    if bytes > 0 {
        counters
            .bytes_live
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_sub(bytes))
            })
            .ok();
    }
}

pub fn record_checkout(kind: &'static str, bytes: impl TryInto<u64>) {
    let bytes = bytes.try_into().unwrap_or(0);
    let counters = counters(kind);
    counters.checkouts.fetch_add(1, Ordering::Relaxed);
    if bytes > 0 {
        let bytes_live = counters.bytes_live.fetch_add(bytes, Ordering::Relaxed) + bytes;
        update_max(&counters.peak_bytes, bytes_live);
    }
}

pub fn record_return(kind: &'static str, bytes: impl TryInto<u64>) {
    let bytes = bytes.try_into().unwrap_or(0);
    let counters = counters(kind);
    counters.returns.fetch_add(1, Ordering::Relaxed);
    if bytes > 0 {
        counters
            .bytes_live
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_sub(bytes))
            })
            .ok();
    }
}

pub fn record_dropped_full(kind: &'static str, _bytes: impl TryInto<u64>) {
    let counters = counters(kind);
    counters.dropped_full.fetch_add(1, Ordering::Relaxed);
}

pub fn record_transient_allocation(kind: &'static str, bytes: impl TryInto<u64>) {
    let bytes = bytes.try_into().unwrap_or(0);
    record_created(kind, bytes);
    record_dropped(kind, bytes);
}

pub fn spawn_tracked<F>(kind: &'static str, future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let guard = ObjectGuard::new(kind, 0);
    tokio::spawn(async move {
        let _guard = guard;
        future.await
    })
}

pub fn snapshot() -> Value {
    let mut kinds = registry()
        .kinds
        .iter()
        .map(|entry| snapshot_kind(*entry.key(), entry.value()))
        .collect::<Vec<_>>();
    kinds.sort_by(|a, b| a.kind.cmp(b.kind));
    json!({
        "enabled": true,
        "kinds": kinds,
        "backtrace_samples": backtrace_samples(),
    })
}

pub fn allocator_snapshot() -> Value {
    #[cfg(feature = "no-global-allocator")]
    {
        json!({
            "enabled": true,
            "active_allocator": active_allocator(),
            "process": null,
            "stats": null,
            "unsupported_reason": "mimalloc global allocator disabled; use OS heap snapshots for this run",
        })
    }
    #[cfg(not(feature = "no-global-allocator"))]
    {
        let process = allocator_process_info();
        let stats_json = mimalloc::MiMalloc::stats_json()
            .ok()
            .and_then(|stats| stats.to_str().ok().map(str::to_owned))
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
        json!({
            "enabled": true,
            "active_allocator": active_allocator(),
            "process": process,
            "stats": stats_json,
        })
    }
}

pub fn active_allocator() -> &'static str {
    if cfg!(feature = "no-global-allocator") {
        "system"
    } else {
        "mimalloc"
    }
}

pub fn allocator_process_info() -> AllocatorProcessInfo {
    #[cfg(feature = "no-global-allocator")]
    {
        AllocatorProcessInfo {
            elapsed_msecs: 0,
            user_msecs: 0,
            system_msecs: 0,
            current_rss_bytes: 0,
            peak_rss_bytes: 0,
            current_commit_bytes: 0,
            peak_commit_bytes: 0,
            page_faults: 0,
        }
    }
    #[cfg(not(feature = "no-global-allocator"))]
    {
        let mut elapsed_msecs = 0usize;
        let mut user_msecs = 0usize;
        let mut system_msecs = 0usize;
        let mut current_rss_bytes = 0usize;
        let mut peak_rss_bytes = 0usize;
        let mut current_commit_bytes = 0usize;
        let mut peak_commit_bytes = 0usize;
        let mut page_faults = 0usize;

        unsafe {
            libmimalloc_sys::mi_process_info(
                &mut elapsed_msecs,
                &mut user_msecs,
                &mut system_msecs,
                &mut current_rss_bytes,
                &mut peak_rss_bytes,
                &mut current_commit_bytes,
                &mut peak_commit_bytes,
                &mut page_faults,
            );
        }

        AllocatorProcessInfo {
            elapsed_msecs,
            user_msecs,
            system_msecs,
            current_rss_bytes,
            peak_rss_bytes,
            current_commit_bytes,
            peak_commit_bytes,
            page_faults,
        }
    }
}

pub fn collect_allocator(force: bool) {
    #[cfg(feature = "no-global-allocator")]
    {
        let _ = force;
    }
    #[cfg(not(feature = "no-global-allocator"))]
    {
        unsafe {
            libmimalloc_sys::mi_collect(force);
        }
    }
}

pub fn reset() {
    registry().kinds.clear();
    if let Ok(mut backtraces) = registry().backtraces.lock() {
        backtraces.clear();
    }
}

fn counters(kind: &'static str) -> Arc<ObjectCounters> {
    registry()
        .kinds
        .entry(kind)
        .or_insert_with(|| Arc::new(ObjectCounters::default()))
        .clone()
}

fn snapshot_kind(kind: &'static str, counters: &ObjectCounters) -> MemoryDiagnosticSummary {
    let created = counters.created.load(Ordering::Relaxed);
    let dropped = counters.dropped.load(Ordering::Relaxed);
    let live = if created > 0 || dropped > 0 {
        created.saturating_sub(dropped)
    } else {
        counters.live.load(Ordering::Relaxed)
    };
    let bytes_live = counters.bytes_live.load(Ordering::Relaxed);
    let bytes_live = if live == 0 && created == dropped {
        0
    } else {
        bytes_live
    };

    MemoryDiagnosticSummary {
        kind,
        created,
        dropped,
        bytes_created: counters.bytes_created.load(Ordering::Relaxed),
        bytes_dropped: counters.bytes_dropped.load(Ordering::Relaxed),
        live,
        peak_live: counters.peak_live.load(Ordering::Relaxed),
        bytes_live,
        peak_bytes: counters.peak_bytes.load(Ordering::Relaxed),
        checkouts: counters.checkouts.load(Ordering::Relaxed),
        returns: counters.returns.load(Ordering::Relaxed),
        dropped_full: counters.dropped_full.load(Ordering::Relaxed),
    }
}

fn update_max(target: &AtomicU64, value: u64) {
    let mut current = target.load(Ordering::Relaxed);
    while value > current {
        match target.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(next) => current = next,
        }
    }
}

fn maybe_record_backtrace(kind: &'static str, bytes: u64) {
    if !backtraces_enabled() {
        return;
    }
    let Ok(mut backtraces) = registry().backtraces.lock() else {
        return;
    };
    if backtraces.len() >= BACKTRACE_SAMPLE_LIMIT {
        return;
    }
    let timestamp_millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    backtraces.push_back(BacktraceSample {
        kind,
        bytes,
        timestamp_millis,
        backtrace: format!("{:?}", Backtrace::force_capture()),
    });
}

fn backtrace_samples() -> Vec<BacktraceSample> {
    registry()
        .backtraces
        .lock()
        .map(|samples| samples.iter().cloned().collect())
        .unwrap_or_default()
}

fn backtraces_enabled() -> bool {
    matches!(
        std::env::var("RVOIP_PERF_MEMORY_BACKTRACES").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_tracks_live_bytes() {
        reset();
        {
            let _guard = ObjectGuard::new("test.guard", 128_u64);
            let snapshot = snapshot();
            let kind = snapshot["kinds"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["kind"] == "test.guard")
                .unwrap();
            assert_eq!(kind["live"], 1);
            assert_eq!(kind["bytes_live"], 128);
            assert_eq!(kind["bytes_created"], 128);
        }
        let snapshot = snapshot();
        let kind = snapshot["kinds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["kind"] == "test.guard")
            .unwrap();
        assert_eq!(kind["live"], 0);
        assert_eq!(kind["bytes_live"], 0);
        assert_eq!(kind["created"], 1);
        assert_eq!(kind["dropped"], 1);
        assert_eq!(kind["bytes_dropped"], 128);
    }

    #[test]
    fn pool_events_track_checkout_return() {
        reset();
        record_checkout("test.pool", 64_u64);
        record_return("test.pool", 64_u64);
        record_dropped_full("test.pool", 0_u64);
        let snapshot = snapshot();
        let kind = snapshot["kinds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["kind"] == "test.pool")
            .unwrap();
        assert_eq!(kind["checkouts"], 1);
        assert_eq!(kind["returns"], 1);
        assert_eq!(kind["dropped_full"], 1);
        assert_eq!(kind["bytes_live"], 0);
    }

    #[test]
    fn allocator_snapshot_serializes() {
        let snapshot = allocator_snapshot();
        assert_eq!(snapshot["enabled"], true);
        assert!(snapshot["process"]["current_rss_bytes"].as_u64().is_some());
    }

    #[test]
    fn snapshot_reconciles_concurrent_object_event_race() {
        reset();
        let counters = counters("test.concurrent");
        counters.created.store(10, Ordering::Relaxed);
        counters.dropped.store(10, Ordering::Relaxed);
        counters.live.store(1, Ordering::Relaxed);
        counters.bytes_live.store(24, Ordering::Relaxed);

        let snapshot = snapshot();
        let kind = snapshot["kinds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["kind"] == "test.concurrent")
            .unwrap();
        assert_eq!(kind["created"], 10);
        assert_eq!(kind["dropped"], 10);
        assert_eq!(kind["live"], 0);
        assert_eq!(kind["bytes_live"], 0);
    }

    #[test]
    fn transient_allocation_tracks_churn_without_live_bytes() {
        reset();
        record_transient_allocation("test.transient", 64_u64);
        let snapshot = snapshot();
        let kind = snapshot["kinds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["kind"] == "test.transient")
            .unwrap();
        assert_eq!(kind["created"], 1);
        assert_eq!(kind["dropped"], 1);
        assert_eq!(kind["live"], 0);
        assert_eq!(kind["bytes_live"], 0);
        assert_eq!(kind["bytes_created"], 64);
        assert_eq!(kind["bytes_dropped"], 64);
    }
}

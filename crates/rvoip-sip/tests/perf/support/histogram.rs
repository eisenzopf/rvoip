//! Thin wrapper around `hdrhistogram::Histogram` so scenarios can
//! `record(nanos)` and emit a JSON-serialisable percentile snapshot.
//!
//! HDR Histogram is the gold standard for latency-distribution capture
//! in network workloads — it keeps the high-cardinality tail (p99 /
//! p99.9 / max) accurately without per-sample overhead.

use hdrhistogram::Histogram;
use serde::Serialize;
use std::sync::Mutex;

/// Single latency bucket recorded in nanoseconds. The internal
/// histogram covers the range 1 ns … 60 s with 3 significant digits of
/// precision (the hdrhistogram default for that range), which is plenty
/// for VoIP latency reporting.
pub struct LatencyHistogram {
    inner: Mutex<Histogram<u64>>,
    label: String,
}

impl LatencyHistogram {
    /// `label` shows up in the JSON output and stdout summary
    /// (e.g. `"post_dial_delay"`, `"full_cycle"`).
    pub fn new(label: impl Into<String>) -> Self {
        let inner = Histogram::new_with_bounds(1, 60_000_000_000, 3).expect("hdrhistogram bounds");
        Self {
            inner: Mutex::new(inner),
            label: label.into(),
        }
    }

    /// Record a single latency sample, in nanoseconds. Values above the
    /// 60 s ceiling are clamped (an out-of-range sample at this scale
    /// would already be a hard timeout, not a latency to measure).
    pub fn record_nanos(&self, nanos: u64) {
        let mut h = self.inner.lock().expect("histogram lock");
        let clamped = nanos.min(60_000_000_000);
        let _ = h.record(clamped);
    }

    /// Snapshot the canonical percentile set used by VoIP reporting.
    pub fn snapshot(&self) -> LatencySnapshot {
        let h = self.inner.lock().expect("histogram lock");
        LatencySnapshot {
            label: self.label.clone(),
            count: h.len(),
            min: h.min(),
            max: h.max(),
            mean_ns: h.mean(),
            p50: h.value_at_quantile(0.50),
            p95: h.value_at_quantile(0.95),
            p99: h.value_at_quantile(0.99),
            p99_9: h.value_at_quantile(0.999),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LatencySnapshot {
    pub label: String,
    pub count: u64,
    pub min: u64,
    pub max: u64,
    pub mean_ns: f64,
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub p99_9: u64,
}

impl LatencySnapshot {
    /// Format a row for the stdout summary table:
    /// `  post_dial_delay  count=11990  p50=1.2ms  p95=4.1ms  p99=9.8ms  p99.9=22.0ms  max=47.0ms`
    pub fn format_row(&self) -> String {
        format!(
            "  {:<22}  count={:>7}  p50={:>8}  p95={:>8}  p99={:>8}  p99.9={:>8}  max={:>8}",
            self.label,
            self.count,
            fmt_ns(self.p50),
            fmt_ns(self.p95),
            fmt_ns(self.p99),
            fmt_ns(self.p99_9),
            fmt_ns(self.max),
        )
    }
}

fn fmt_ns(ns: u64) -> String {
    if ns >= 1_000_000_000 {
        format!("{:.2}s", ns as f64 / 1_000_000_000.0)
    } else if ns >= 1_000_000 {
        format!("{:.1}ms", ns as f64 / 1_000_000.0)
    } else if ns >= 1_000 {
        format!("{:.1}µs", ns as f64 / 1_000.0)
    } else {
        format!("{}ns", ns)
    }
}

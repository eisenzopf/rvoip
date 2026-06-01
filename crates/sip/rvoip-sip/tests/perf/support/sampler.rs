//! Combined CPU% + RSS resource sampler.
//!
//! Background task samples `sys.process(pid)` on a fixed cadence
//! during the steady window. On stop, returns a [`ResourceSummary`]
//! with:
//!
//! - `baseline_rss_mb` (first sample),
//! - `peak_rss_mb` (max across samples),
//! - `rss_growth_mb_per_min` (linear-regression slope across samples —
//!   the leak indicator that backs the "no leaks" Rust pitch),
//! - `rss_tail_growth_mb_per_min` (same slope over the final tail window,
//!   used as the sustained-growth release gate),
//! - `rss_samples` (the raw time series, suitable for plotting),
//! - `avg_cpu_pct` (mean process-level CPU across samples).
//!
//! Modelled on what real VoIP perf reports include (rtpengine,
//! OpenSIPS): not just a peak number but the curve so a reader can
//! see whether peak was a spike or a plateau.

#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Serialize)]
pub struct ResourceSample {
    /// Seconds since the sampler started.
    pub t_secs: f64,
    pub rss_mb: f64,
    pub cpu_pct: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResourceSummary {
    pub baseline_rss_mb: f64,
    pub peak_rss_mb: f64,
    pub rss_growth_mb_per_min: f64,
    pub rss_tail_growth_mb_per_min: f64,
    pub rss_tail_window_secs: f64,
    pub avg_cpu_pct: f64,
    pub samples: Vec<ResourceSample>,
}

pub struct ResourceSampler {
    samples: Arc<Mutex<Vec<ResourceSample>>>,
    stop: Arc<AtomicBool>,
    task: Option<JoinHandle<()>>,
    started: Instant,
}

impl ResourceSampler {
    /// Start sampling. The first sample is taken immediately so
    /// `baseline_rss_mb` reflects the state before the load began.
    pub fn start(interval: Duration) -> Self {
        let samples: Arc<Mutex<Vec<ResourceSample>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let samples_task = Arc::clone(&samples);
        let stop_task = Arc::clone(&stop);
        let started = Instant::now();
        let pid = Pid::from_u32(std::process::id());

        let task = tokio::spawn(async move {
            let mut sys = System::new();
            loop {
                // Refresh CPU + memory for our PID. sysinfo's cpu_usage
                // is "delta since last refresh of this process", so the
                // first reading after `new()` is essentially 0; we
                // still record it (it gets averaged out by subsequent
                // samples).
                sys.refresh_processes_specifics(
                    ProcessesToUpdate::Some(&[pid]),
                    ProcessRefreshKind::new().with_memory().with_cpu(),
                );
                if let Some(proc_) = sys.process(pid) {
                    let rss_mb = proc_.memory() as f64 / (1024.0 * 1024.0);
                    let cpu_pct = proc_.cpu_usage();
                    let t_secs = started.elapsed().as_secs_f64();
                    samples_task
                        .lock()
                        .expect("sampler lock")
                        .push(ResourceSample {
                            t_secs,
                            rss_mb,
                            cpu_pct,
                        });
                }
                if stop_task.load(Ordering::Relaxed) {
                    break;
                }
                tokio::time::sleep(interval).await;
            }
        });

        Self {
            samples,
            stop,
            task: Some(task),
            started,
        }
    }

    /// Stop sampling and compute the summary. Drops the first CPU
    /// sample from the average (it's always 0 — see the comment above
    /// about sysinfo's `cpu_usage` semantics).
    pub async fn stop(mut self) -> ResourceSummary {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.task.take() {
            let _ = t.await;
        }
        let samples = std::mem::take(&mut *self.samples.lock().expect("sampler lock"));

        let baseline_rss_mb = samples.first().map(|s| s.rss_mb).unwrap_or(0.0);
        let peak_rss_mb = samples.iter().map(|s| s.rss_mb).fold(0.0f64, f64::max);

        // Linear-regression slope of RSS over time, normalised to MB/min.
        let rss_growth_mb_per_min = linear_slope_mb_per_sec(&samples) * 60.0;
        let tail_window_secs = rss_tail_window_secs();
        let tail_samples = tail_samples(&samples, tail_window_secs);
        let rss_tail_growth_mb_per_min = linear_slope_mb_per_sec(tail_samples) * 60.0;

        // CPU average: drop the first sample (always 0, sysinfo
        // semantics) and average the rest. Falls back to 0 if there
        // aren't at least 2 samples.
        let avg_cpu_pct = if samples.len() > 1 {
            let sum: f64 = samples.iter().skip(1).map(|s| s.cpu_pct as f64).sum();
            sum / (samples.len() - 1) as f64
        } else {
            0.0
        };

        ResourceSummary {
            baseline_rss_mb,
            peak_rss_mb,
            rss_growth_mb_per_min,
            rss_tail_growth_mb_per_min,
            rss_tail_window_secs: tail_window_secs,
            avg_cpu_pct,
            samples,
        }
    }
}

fn tail_samples(samples: &[ResourceSample], tail_window_secs: f64) -> &[ResourceSample] {
    let Some(last) = samples.last() else {
        return samples;
    };
    let min_t = (last.t_secs - tail_window_secs).max(0.0);
    let start = samples
        .iter()
        .position(|sample| sample.t_secs >= min_t)
        .unwrap_or(0);
    &samples[start..]
}

fn rss_tail_window_secs() -> f64 {
    const DEFAULT_TAIL_WINDOW_SECS: f64 = 60.0;
    match std::env::var("RVOIP_PERF_RSS_TAIL_WINDOW_SECS") {
        Ok(raw) => {
            let value: f64 = raw.parse().unwrap_or_else(|_| {
                panic!("RVOIP_PERF_RSS_TAIL_WINDOW_SECS must be finite and greater than 0")
            });
            assert!(
                value.is_finite() && value > 0.0,
                "RVOIP_PERF_RSS_TAIL_WINDOW_SECS must be finite and greater than 0"
            );
            value
        }
        Err(std::env::VarError::NotPresent) => DEFAULT_TAIL_WINDOW_SECS,
        Err(err) => panic!("RVOIP_PERF_RSS_TAIL_WINDOW_SECS could not be read: {err}"),
    }
}

/// Least-squares slope of `rss_mb` against `t_secs`, in MB per second.
/// Returns 0.0 if fewer than 2 samples (no slope is defined).
fn linear_slope_mb_per_sec(samples: &[ResourceSample]) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let n = samples.len() as f64;
    let sum_x: f64 = samples.iter().map(|s| s.t_secs).sum();
    let sum_y: f64 = samples.iter().map(|s| s.rss_mb).sum();
    let sum_xy: f64 = samples.iter().map(|s| s.t_secs * s.rss_mb).sum();
    let sum_xx: f64 = samples.iter().map(|s| s.t_secs * s.t_secs).sum();
    let denom = n * sum_xx - sum_x * sum_x;
    if denom.abs() < f64::EPSILON {
        return 0.0;
    }
    (n * sum_xy - sum_x * sum_y) / denom
}

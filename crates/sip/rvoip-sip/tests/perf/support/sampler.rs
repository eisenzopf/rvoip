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
//! - `rss_tail_growth_mb_per_min` (same slope over the final *available*
//!   portion of the requested tail window),
//! - requested and actual tail coverage (a short run must never claim that a
//!   60-second window was measured),
//! - optional phase-selected slopes such as the canonical active-load and
//!   post-drain cleanup windows,
//! - `rss_samples` (the raw time series, suitable for plotting),
//! - `avg_cpu_pct` (mean process-level CPU across samples).
//!
//! Modelled on what real VoIP perf reports include (rtpengine,
//! OpenSIPS): not just a peak number but the curve so a reader can
//! see whether peak was a spike or a plateau.

#![allow(dead_code)]

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
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

/// A caller-defined resource-measurement window. Times are relative to the
/// sampler start, not wall clock, so profiler and scheduler delays cannot move
/// a sample into a different phase accidentally.
#[derive(Debug, Clone)]
pub struct ResourceWindowSpec {
    pub name: String,
    pub start_phase: String,
    pub end_phase: String,
    pub start: Duration,
    pub end: Duration,
    pub requested_coverage: Duration,
}

impl ResourceWindowSpec {
    pub fn new(
        name: impl Into<String>,
        start_phase: impl Into<String>,
        end_phase: impl Into<String>,
        start: Duration,
        end: Duration,
    ) -> Self {
        assert!(end >= start, "resource window end precedes its start");
        Self {
            name: name.into(),
            start_phase: start_phase.into(),
            end_phase: end_phase.into(),
            start,
            end,
            requested_coverage: end - start,
        }
    }

    /// Define a window whose configured duration is distinct from the exact
    /// instant at which sampling happened to stop. This prevents scheduler
    /// overshoot from changing the declared measurement contract.
    pub fn with_requested_coverage(
        name: impl Into<String>,
        start_phase: impl Into<String>,
        end_phase: impl Into<String>,
        start: Duration,
        end: Duration,
        requested_coverage: Duration,
    ) -> Self {
        assert!(end >= start, "resource window end precedes its start");
        Self {
            name: name.into(),
            start_phase: start_phase.into(),
            end_phase: end_phase.into(),
            start,
            end,
            requested_coverage,
        }
    }
}

/// Coverage and slope for one explicitly selected phase window.
#[derive(Debug, Clone, Serialize)]
pub struct ResourceWindowSummary {
    pub name: String,
    pub start_phase: String,
    pub end_phase: String,
    pub requested_start_secs: f64,
    pub requested_end_secs: f64,
    pub requested_coverage_secs: f64,
    pub first_sample_secs: Option<f64>,
    pub last_sample_secs: Option<f64>,
    pub actual_coverage_secs: f64,
    pub sample_count: usize,
    pub boundary_tolerance_secs: f64,
    pub complete: bool,
    pub rss_growth_mb_per_min: f64,
    /// Median RSS in the first robust endpoint band.
    pub rss_start_median_mb: Option<f64>,
    /// Median RSS in the last robust endpoint band.
    pub rss_end_median_mb: Option<f64>,
    /// Signed end-minus-start median RSS. Positive values are retained growth.
    pub rss_retained_growth_mb: Option<f64>,
    /// Median timestamp of the samples contributing to the starting RSS band.
    pub rss_start_representative_secs: Option<f64>,
    /// Median timestamp of the samples contributing to the ending RSS band.
    pub rss_end_representative_secs: Option<f64>,
    /// Separation between the two representative timestamps. This, rather
    /// than the requested outer window, is the time basis for the endpoint
    /// growth rate.
    pub rss_endpoint_separation_secs: Option<f64>,
    /// Endpoint-median retained growth normalized by the representative
    /// timestamp separation.
    pub rss_endpoint_growth_mb_per_hour: Option<f64>,
    /// Target duration of each endpoint band. Bands use the first and last
    /// sixth of the observed window, capped at 15 seconds.
    pub rss_endpoint_band_secs: f64,
    pub rss_start_sample_count: usize,
    pub rss_end_sample_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResourceSummary {
    pub baseline_rss_mb: f64,
    pub peak_rss_mb: f64,
    pub rss_growth_mb_per_min: f64,
    pub rss_tail_growth_mb_per_min: f64,
    /// Actual observed coverage, retained under the legacy field name.
    pub rss_tail_window_secs: f64,
    pub rss_tail_window_requested_secs: f64,
    pub rss_tail_window_complete: bool,
    pub rss_tail_sample_count: usize,
    pub sample_interval_estimate_secs: f64,
    pub windows: Vec<ResourceWindowSummary>,
    pub avg_cpu_pct: f64,
    pub sample_count: usize,
    pub samples_path: Option<PathBuf>,
    pub samples: Vec<ResourceSample>,
}

impl ResourceSummary {
    pub fn empty() -> Self {
        Self {
            baseline_rss_mb: 0.0,
            peak_rss_mb: 0.0,
            rss_growth_mb_per_min: 0.0,
            rss_tail_growth_mb_per_min: 0.0,
            rss_tail_window_secs: 0.0,
            rss_tail_window_requested_secs: rss_tail_window_secs(),
            rss_tail_window_complete: false,
            rss_tail_sample_count: 0,
            sample_interval_estimate_secs: 0.0,
            windows: Vec::new(),
            avg_cpu_pct: 0.0,
            sample_count: 0,
            samples_path: None,
            samples: Vec::new(),
        }
    }
}

pub struct ResourceSampler {
    samples: Arc<Mutex<Vec<ResourceSample>>>,
    stop: Arc<AtomicBool>,
    task: Option<JoinHandle<()>>,
    samples_path: Option<PathBuf>,
    started: Instant,
}

impl ResourceSampler {
    /// Start sampling. The first sample is taken immediately so
    /// `baseline_rss_mb` reflects the state before the load began.
    pub fn start(interval: Duration) -> Self {
        Self::start_inner(interval, None)
    }

    /// Start sampling and append each sample to `path` as JSONL while the
    /// test is running. The in-memory series remains available for summary
    /// math, but callers can clear it before writing the final report.
    pub fn start_with_output(interval: Duration, path: PathBuf) -> Self {
        Self::start_inner(interval, Some(path))
    }

    fn start_inner(interval: Duration, samples_path: Option<PathBuf>) -> Self {
        let samples: Arc<Mutex<Vec<ResourceSample>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let samples_task = Arc::clone(&samples);
        let stop_task = Arc::clone(&stop);
        let started = Instant::now();
        let pid = Pid::from_u32(std::process::id());
        let task_samples_path = samples_path.clone();

        let task = tokio::spawn(async move {
            let mut sys = System::new();
            let mut writer = task_samples_path.map(|path| {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).expect("create resource sample dir");
                }
                BufWriter::new(File::create(path).expect("create resource sample JSONL"))
            });
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
                    let sample = ResourceSample {
                        t_secs,
                        rss_mb,
                        cpu_pct,
                    };
                    samples_task
                        .lock()
                        .expect("sampler lock")
                        .push(sample.clone());
                    if let Some(writer) = writer.as_mut() {
                        serde_json::to_writer(&mut *writer, &sample)
                            .expect("write resource sample JSONL");
                        writer
                            .write_all(b"\n")
                            .expect("write resource sample newline");
                        writer.flush().expect("flush resource sample JSONL");
                    }
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
            samples_path,
            started,
        }
    }

    /// Elapsed time in the sampler's clock domain. Callers use this to mark
    /// exact phase boundaries for [`stop_with_windows`](Self::stop_with_windows).
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Stop sampling and compute the summary. Drops the first CPU
    /// sample from the average (it's always 0 — see the comment above
    /// about sysinfo's `cpu_usage` semantics).
    pub async fn stop(mut self) -> ResourceSummary {
        self.stop_with_windows_inner(Vec::new()).await
    }

    /// Stop sampling and also calculate slopes over named phase windows.
    pub async fn stop_with_windows(mut self, windows: Vec<ResourceWindowSpec>) -> ResourceSummary {
        self.stop_with_windows_inner(windows).await
    }

    async fn stop_with_windows_inner(
        &mut self,
        windows: Vec<ResourceWindowSpec>,
    ) -> ResourceSummary {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.task.take() {
            let _ = t.await;
        }
        let samples = std::mem::take(&mut *self.samples.lock().expect("sampler lock"));
        let sample_count = samples.len();

        let baseline_rss_mb = samples.first().map(|s| s.rss_mb).unwrap_or(0.0);
        let peak_rss_mb = samples.iter().map(|s| s.rss_mb).fold(0.0f64, f64::max);

        // Linear-regression slope of RSS over time, normalised to MB/min.
        let rss_growth_mb_per_min = linear_slope_mb_per_sec(&samples) * 60.0;
        let sample_interval_estimate_secs = sample_interval_estimate_secs(&samples);
        let tail_window_requested_secs = rss_tail_window_secs();
        let tail_samples = tail_samples(&samples, tail_window_requested_secs);
        let rss_tail_growth_mb_per_min = linear_slope_mb_per_sec(tail_samples) * 60.0;
        let tail_window_actual_secs = sample_coverage_secs(tail_samples);
        let rss_tail_window_complete = coverage_complete(
            tail_window_actual_secs,
            tail_window_requested_secs,
            sample_interval_estimate_secs,
        );
        let window_summaries = windows
            .into_iter()
            .map(|window| summarize_window(&samples, window, sample_interval_estimate_secs))
            .collect();

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
            rss_tail_window_secs: tail_window_actual_secs,
            rss_tail_window_requested_secs: tail_window_requested_secs,
            rss_tail_window_complete,
            rss_tail_sample_count: tail_samples.len(),
            sample_interval_estimate_secs,
            windows: window_summaries,
            avg_cpu_pct,
            sample_count,
            samples_path: self.samples_path.take(),
            samples,
        }
    }
}

fn summarize_window(
    samples: &[ResourceSample],
    window: ResourceWindowSpec,
    sample_interval_estimate_secs: f64,
) -> ResourceWindowSummary {
    let requested_start_secs = window.start.as_secs_f64();
    let requested_end_secs = window.end.as_secs_f64();
    let requested_coverage_secs = window.requested_coverage.as_secs_f64();
    let selected = samples
        .iter()
        .filter(|sample| {
            sample.t_secs >= requested_start_secs && sample.t_secs <= requested_end_secs
        })
        .cloned()
        .collect::<Vec<_>>();
    let actual_coverage_secs = sample_coverage_secs(&selected);
    let boundary_tolerance_secs = sample_interval_estimate_secs.max(0.001) * 1.5;
    let starts_in_time = selected
        .first()
        .is_some_and(|sample| sample.t_secs <= requested_start_secs + boundary_tolerance_secs);
    let ends_in_time = selected
        .last()
        .is_some_and(|sample| sample.t_secs + boundary_tolerance_secs >= requested_end_secs);
    let endpoint_summary = robust_endpoint_summary(&selected);
    let endpoint_separation_secs = endpoint_summary.as_ref().map(|summary| {
        (summary.end_representative_secs - summary.start_representative_secs).max(0.0)
    });
    let endpoint_growth_mb_per_hour = endpoint_summary.as_ref().and_then(|summary| {
        let separation_secs = summary.end_representative_secs - summary.start_representative_secs;
        (separation_secs > 0.0)
            .then(|| (summary.end_median_mb - summary.start_median_mb) * 3600.0 / separation_secs)
    });

    ResourceWindowSummary {
        name: window.name,
        start_phase: window.start_phase,
        end_phase: window.end_phase,
        requested_start_secs,
        requested_end_secs,
        requested_coverage_secs,
        first_sample_secs: selected.first().map(|sample| sample.t_secs),
        last_sample_secs: selected.last().map(|sample| sample.t_secs),
        actual_coverage_secs,
        sample_count: selected.len(),
        boundary_tolerance_secs,
        complete: selected.len() >= 2 && starts_in_time && ends_in_time,
        rss_growth_mb_per_min: linear_slope_mb_per_sec(&selected) * 60.0,
        rss_start_median_mb: endpoint_summary
            .as_ref()
            .map(|summary| summary.start_median_mb),
        rss_end_median_mb: endpoint_summary
            .as_ref()
            .map(|summary| summary.end_median_mb),
        rss_retained_growth_mb: endpoint_summary
            .as_ref()
            .map(|summary| summary.end_median_mb - summary.start_median_mb),
        rss_start_representative_secs: endpoint_summary
            .as_ref()
            .map(|summary| summary.start_representative_secs),
        rss_end_representative_secs: endpoint_summary
            .as_ref()
            .map(|summary| summary.end_representative_secs),
        rss_endpoint_separation_secs: endpoint_separation_secs,
        rss_endpoint_growth_mb_per_hour: endpoint_growth_mb_per_hour,
        rss_endpoint_band_secs: endpoint_summary
            .as_ref()
            .map_or(0.0, |summary| summary.band_secs),
        rss_start_sample_count: endpoint_summary
            .as_ref()
            .map_or(0, |summary| summary.start_sample_count),
        rss_end_sample_count: endpoint_summary
            .as_ref()
            .map_or(0, |summary| summary.end_sample_count),
    }
}

#[derive(Debug)]
struct RobustEndpointSummary {
    start_median_mb: f64,
    end_median_mb: f64,
    start_representative_secs: f64,
    end_representative_secs: f64,
    band_secs: f64,
    start_sample_count: usize,
    end_sample_count: usize,
}

/// Summarize retained RSS with robust endpoint medians instead of projecting a
/// short-window least-squares slope to an hour. The endpoint bands make the
/// signal insensitive to one-off allocator and sampler spikes while preserving
/// a signed, operationally meaningful absolute delta.
fn robust_endpoint_summary(samples: &[ResourceSample]) -> Option<RobustEndpointSummary> {
    const MIN_ENDPOINT_SAMPLES: usize = 3;
    const MAX_ENDPOINT_BAND_SECS: f64 = 15.0;

    if samples.len() < MIN_ENDPOINT_SAMPLES * 2 {
        return None;
    }
    let actual_coverage_secs = sample_coverage_secs(samples);
    if actual_coverage_secs <= 0.0 {
        return None;
    }
    let band_secs = (actual_coverage_secs / 6.0).min(MAX_ENDPOINT_BAND_SECS);
    let first_t = samples.first()?.t_secs;
    let last_t = samples.last()?.t_secs;
    let start_values = samples
        .iter()
        .take_while(|sample| sample.t_secs <= first_t + band_secs)
        .map(|sample| sample.rss_mb)
        .collect::<Vec<_>>();
    let start_times = samples
        .iter()
        .take_while(|sample| sample.t_secs <= first_t + band_secs)
        .map(|sample| sample.t_secs)
        .collect::<Vec<_>>();
    let end_values = samples
        .iter()
        .skip_while(|sample| sample.t_secs < last_t - band_secs)
        .map(|sample| sample.rss_mb)
        .collect::<Vec<_>>();
    let end_times = samples
        .iter()
        .skip_while(|sample| sample.t_secs < last_t - band_secs)
        .map(|sample| sample.t_secs)
        .collect::<Vec<_>>();
    if start_values.len() < MIN_ENDPOINT_SAMPLES || end_values.len() < MIN_ENDPOINT_SAMPLES {
        return None;
    }

    Some(RobustEndpointSummary {
        start_median_mb: median(start_values.clone()),
        end_median_mb: median(end_values.clone()),
        start_representative_secs: median(start_times),
        end_representative_secs: median(end_times),
        band_secs,
        start_sample_count: start_values.len(),
        end_sample_count: end_values.len(),
    })
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    let midpoint = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[midpoint - 1] + values[midpoint]) / 2.0
    } else {
        values[midpoint]
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

fn sample_coverage_secs(samples: &[ResourceSample]) -> f64 {
    match (samples.first(), samples.last()) {
        (Some(first), Some(last)) => (last.t_secs - first.t_secs).max(0.0),
        _ => 0.0,
    }
}

fn sample_interval_estimate_secs(samples: &[ResourceSample]) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let mut intervals = samples
        .windows(2)
        .map(|pair| (pair[1].t_secs - pair[0].t_secs).max(0.0))
        .collect::<Vec<_>>();
    intervals.sort_by(f64::total_cmp);
    intervals[intervals.len() / 2]
}

fn coverage_complete(actual_secs: f64, requested_secs: f64, interval_secs: f64) -> bool {
    actual_secs + interval_secs.max(0.001) * 1.5 >= requested_secs
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(t_secs: f64, rss_mb: f64) -> ResourceSample {
        ResourceSample {
            t_secs,
            rss_mb,
            cpu_pct: 0.0,
        }
    }

    #[test]
    fn short_tail_reports_actual_instead_of_requested_coverage() {
        let samples = (0..=80)
            .map(|index| sample(index as f64 * 0.5, index as f64))
            .collect::<Vec<_>>();
        let tail = tail_samples(&samples, 60.0);
        let actual = sample_coverage_secs(tail);
        let interval = sample_interval_estimate_secs(&samples);
        assert_eq!(actual, 40.0);
        assert!(!coverage_complete(actual, 60.0, interval));
    }

    #[test]
    fn phase_window_selects_only_the_named_interval() {
        let samples = (0..=260)
            .map(|index| sample(index as f64 * 0.5, index as f64 * 0.25))
            .collect::<Vec<_>>();
        let summary = summarize_window(
            &samples,
            ResourceWindowSpec::new(
                "active_load",
                "point_start",
                "calls_drained",
                Duration::ZERO,
                Duration::from_secs(35),
            ),
            0.5,
        );
        assert!(summary.complete);
        assert_eq!(summary.sample_count, 71);
        assert_eq!(summary.actual_coverage_secs, 35.0);
        assert!((summary.rss_growth_mb_per_min - 30.0).abs() < 0.001);
    }

    #[test]
    fn explicit_requested_coverage_ignores_delayed_sampler_stop() {
        let samples = (0..=190)
            .map(|index| sample(10.0 + index as f64 * 0.5, 4500.0))
            .collect::<Vec<_>>();
        let summary = summarize_window(
            &samples,
            ResourceWindowSpec::with_requested_coverage(
                "post_drain_cleanup",
                "calls_drained",
                "post_drain_cleanup_end",
                Duration::from_secs(10),
                Duration::from_secs_f64(105.001947),
                Duration::from_secs(95),
            ),
            0.5,
        );
        assert_eq!(summary.requested_coverage_secs, 95.0);
        assert_eq!(summary.actual_coverage_secs, 95.0);
        assert!(summary.complete);
    }

    #[test]
    fn robust_endpoint_delta_rejects_spikes_not_stable_growth() {
        let samples = (0..=190)
            .map(|index| {
                let t_secs = index as f64 * 0.5;
                let retained = if t_secs >= 80.0 { 1.0 } else { 0.0 };
                let spike = match index {
                    8 => 100.0,
                    184 => -100.0,
                    _ => 0.0,
                };
                sample(t_secs, 4500.0 + retained + spike)
            })
            .collect::<Vec<_>>();
        let summary = robust_endpoint_summary(&samples).expect("robust endpoint summary");
        assert_eq!(summary.band_secs, 15.0);
        assert!((summary.end_median_mb - summary.start_median_mb - 1.0).abs() < 0.001);
        assert!(
            (summary.end_representative_secs - summary.start_representative_secs - 80.0).abs()
                < 0.001
        );
        assert!(summary.start_sample_count >= 30);
        assert!(summary.end_sample_count >= 30);
    }

    #[test]
    fn robust_endpoint_delta_preserves_material_growth() {
        let samples = (0..=190)
            .map(|index| {
                let t_secs = index as f64 * 0.5;
                sample(t_secs, 4500.0 + 10.0 * t_secs / 95.0)
            })
            .collect::<Vec<_>>();
        let summary = robust_endpoint_summary(&samples).expect("robust endpoint summary");
        assert!(summary.end_median_mb - summary.start_median_mb > 8.0);
    }

    #[test]
    fn endpoint_rate_uses_representative_timestamp_separation() {
        let samples = (0..=190)
            .map(|index| {
                let t_secs = index as f64 * 0.5;
                sample(t_secs, 4500.0 + 10.5 * t_secs / 3600.0)
            })
            .collect::<Vec<_>>();
        let window = summarize_window(
            &samples,
            ResourceWindowSpec::new(
                "post_drain_cleanup",
                "calls_drained",
                "post_drain_cleanup_end",
                Duration::ZERO,
                Duration::from_secs(95),
            ),
            0.5,
        );

        assert!((window.rss_endpoint_separation_secs.unwrap() - 80.0).abs() < 0.001);
        assert!((window.rss_endpoint_growth_mb_per_hour.unwrap() - 10.5).abs() < 0.001);
    }
}

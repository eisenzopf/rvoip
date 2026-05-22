//! Load profile: ramp → steady → cooldown.
//!
//! `LoadProfile::run` dispatches one logical request per token, using a
//! sleep-until-next-tick pacer so offered load matches the target CPS
//! within the resolution of `tokio::time::sleep_until`. At the end of
//! the steady window the dispatcher stops issuing new work and returns;
//! the caller awaits its outstanding handles and then enters cooldown
//! sampling.

use serde::Serialize;
use std::future::Future;
use std::time::{Duration, Instant};
use tokio::time::sleep_until;

#[derive(Debug, Clone, Serialize)]
pub struct LoadProfile {
    pub target_cps: f64,
    pub ramp_secs: u64,
    pub steady_secs: u64,
    pub cooldown_secs: u64,
}

impl LoadProfile {
    pub fn from_env(default_cps: f64, default_steady: u64) -> Self {
        Self {
            target_cps: env_f64("RVOIP_PERF_TARGET_CPS", default_cps),
            ramp_secs: env_u64("RVOIP_PERF_RAMP_SECS", 5),
            steady_secs: env_u64("RVOIP_PERF_STEADY_SECS", default_steady),
            cooldown_secs: env_u64("RVOIP_PERF_COOLDOWN_SECS", 5),
        }
    }

    /// Build a LoadProfile for one point in a sweep. Ramp / steady /
    /// cooldown are env-overridable per the usual knobs (so a single
    /// `RVOIP_PERF_STEADY_SECS` change applies to every sweep point);
    /// only `target_cps` is parametric.
    pub fn for_point(point: f64, default_steady: u64) -> Self {
        Self {
            target_cps: point,
            ramp_secs: env_u64("RVOIP_PERF_RAMP_SECS", 5),
            steady_secs: env_u64("RVOIP_PERF_STEADY_SECS", default_steady),
            cooldown_secs: env_u64("RVOIP_PERF_COOLDOWN_SECS", 5),
        }
    }

    /// Total expected calls offered across the ramp + steady phases.
    pub fn total_calls(&self) -> u64 {
        // Triangular ramp: area = 0.5 * base * height = 0.5 * ramp_secs *
        // target_cps. Steady phase is a rectangle.
        let ramp = 0.5 * (self.ramp_secs as f64) * self.target_cps;
        let steady = (self.steady_secs as f64) * self.target_cps;
        (ramp + steady).round() as u64
    }

    /// Run `dispatch` once per pacer tick across ramp + steady. The
    /// closure receives a 0-indexed sequence number; its return value
    /// is not awaited (callers are responsible for tracking spawned
    /// task handles). Returns the wall-clock duration of the active
    /// phase (excluding cooldown — the caller drives cooldown after
    /// awaiting outstanding handles).
    pub async fn run<F>(&self, mut dispatch: F) -> Duration
    where
        F: FnMut(u64),
    {
        let start = Instant::now();
        let ramp = Duration::from_secs(self.ramp_secs);
        let steady = Duration::from_secs(self.steady_secs);
        let active = ramp + steady;
        let active_deadline = start + active;
        let mut seq: u64 = 0;

        loop {
            let now = Instant::now();
            if now >= active_deadline {
                break;
            }
            // Effective CPS at `now`: linearly ramped from 0 → target
            // during the ramp window, then pinned at target.
            let elapsed = now - start;
            let cps = if elapsed < ramp {
                self.target_cps * (elapsed.as_secs_f64() / self.ramp_secs as f64)
            } else {
                self.target_cps
            }
            .max(1.0); // avoid div-by-zero at t=0
            let tick = Duration::from_secs_f64(1.0 / cps);

            dispatch(seq);
            seq += 1;

            let next = tokio::time::Instant::now() + tick;
            sleep_until(next).await;
        }

        start.elapsed()
    }

    /// Convenience: spawn `count` calls back-to-back without pacing.
    /// Used by scenarios that measure a fixed batch (e.g. concurrent
    /// active calls held open) rather than a sustained rate.
    pub async fn burst<F, Fut>(count: u64, mut dispatch: F)
    where
        F: FnMut(u64) -> Fut,
        Fut: Future<Output = ()>,
    {
        for i in 0..count {
            dispatch(i).await;
        }
    }
}

fn env_f64(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

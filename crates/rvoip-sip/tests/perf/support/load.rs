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
use tokio::time::{sleep_until, Instant as TokioInstant};

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

    /// Run `dispatch` across ramp + steady using an absolute-time pacer.
    ///
    /// Instead of sleeping `1 / cps` after each dispatch, the scheduler wakes
    /// at a fixed tick and computes how many calls should have been offered by
    /// that wall-clock instant. If the runtime wakes late, it emits the backlog
    /// in one batch. This keeps high-CPS pressure honest and prevents sleep
    /// drift from silently lowering the offered load.
    pub async fn run<F>(&self, mut dispatch: F) -> Duration
    where
        F: FnMut(u64),
    {
        let start = Instant::now();
        let active = Duration::from_secs(self.ramp_secs + self.steady_secs);
        let active_deadline = start + active;
        let tick = Duration::from_millis(env_u64("RVOIP_PERF_SCHED_TICK_MS", 1).max(1));
        let total_calls = self.total_calls();
        let mut seq: u64 = 0;
        let mut next_tick = start;

        loop {
            let now = Instant::now();
            let elapsed = now.saturating_duration_since(start).min(active);
            let desired = self.expected_calls_at(elapsed).min(total_calls);

            while seq < desired {
                dispatch(seq);
                seq += 1;
            }

            if now >= active_deadline {
                break;
            }

            next_tick += tick;
            while next_tick <= now {
                next_tick += tick;
            }
            sleep_until(TokioInstant::from_std(next_tick)).await;
        }

        while seq < total_calls {
            dispatch(seq);
            seq += 1;
        }

        start.elapsed()
    }

    fn expected_calls_at(&self, elapsed: Duration) -> u64 {
        let elapsed_s = elapsed.as_secs_f64();
        let ramp_s = self.ramp_secs as f64;
        let steady_start_s = ramp_s;

        let ramp_calls = if ramp_s > 0.0 {
            let ramp_elapsed_s = elapsed_s.min(ramp_s);
            0.5 * self.target_cps * ramp_elapsed_s * ramp_elapsed_s / ramp_s
        } else {
            0.0
        };
        let steady_calls = self.target_cps * (elapsed_s - steady_start_s).max(0.0);

        (ramp_calls + steady_calls).floor() as u64
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

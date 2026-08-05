//! Adaptive jitter target.
//!
//! The drain converges to a target depth, and that target is a latency choice:
//! too shallow underruns on the next burst, too deep is delay the caller hears.
//! A constant cannot be right for both a LAN and a congested WAN, so measure it.
//!
//! Uses the RFC 3550 §6.4.1 interarrival jitter estimator, which is the same
//! smoothing every RTP stack applies, adapted to a source that delivers whole
//! frames in coalesced chunks rather than one packet per period.

use std::time::Instant;

/// Smoothing divisor from RFC 3550 §6.4.1: `J += (|D| - J) / 16`.
const RFC3550_GAIN: f32 = 16.0;

/// Multiple of measured jitter to hold. Three is the usual engineering choice:
/// it covers the tail of a roughly exponential deviation without paying for the
/// worst case observed once.
const TARGET_JITTER_MULTIPLE: f32 = 3.0;

/// Tracks arrival deviation and recommends a buffer depth.
#[derive(Debug, Default)]
pub(crate) struct JitterEstimator {
    last_arrival: Option<Instant>,
    /// RFC 3550 `J`, in milliseconds.
    jitter_ms: f32,
    observations: u64,
}

impl JitterEstimator {
    /// Record the arrival of a chunk carrying `frames` whole frames.
    ///
    /// `D` is the difference between how long the chunk *took* to arrive and
    /// how much audio it carries. A perfectly paced source has `D == 0`; a
    /// source that hands over 90 ms of audio in one instant registers that 90 ms
    /// as deviation, which is exactly the depth the buffer must cover.
    pub(crate) fn observe(&mut self, now: Instant, frames: usize, frame_ms: u32) {
        if frames == 0 {
            return;
        }
        if let Some(previous) = self.last_arrival {
            let actual_ms = now.saturating_duration_since(previous).as_secs_f32() * 1_000.0;
            let carried_ms = frames as f32 * frame_ms as f32;
            let deviation = (actual_ms - carried_ms).abs();
            self.jitter_ms += (deviation - self.jitter_ms) / RFC3550_GAIN;
            self.observations = self.observations.saturating_add(1);
        }
        self.last_arrival = Some(now);
    }

    pub(crate) fn jitter_ms(&self) -> f32 {
        self.jitter_ms
    }

    /// Depth to converge to, in frames, clamped to `[floor_ms, ceiling_ms]`.
    ///
    /// Falls back to the floor until there is enough history to trust: an
    /// estimate from two packets is noise, and starting shallow on a bursty
    /// source causes audible underrun.
    pub(crate) fn target_frames(
        &self,
        frame_ms: u32,
        floor_ms: u32,
        ceiling_ms: u32,
    ) -> usize {
        let frame_ms = frame_ms.max(1);
        let floor_frames = floor_ms.div_ceil(frame_ms) as usize;
        if self.observations < 16 {
            return floor_frames;
        }
        let wanted_ms = (self.jitter_ms * TARGET_JITTER_MULTIPLE)
            .clamp(floor_ms as f32, ceiling_ms as f32);
        let frames = (wanted_ms / frame_ms as f32).ceil() as usize;
        frames.max(floor_frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn steady(estimator: &mut JitterEstimator, count: usize, gap_ms: u64, frames: usize) {
        let mut now = Instant::now();
        for _ in 0..count {
            now += Duration::from_millis(gap_ms);
            estimator.observe(now, frames, 20);
        }
    }

    #[test]
    fn a_perfectly_paced_source_measures_no_jitter() {
        let mut estimator = JitterEstimator::default();
        // One 20 ms frame every 20 ms.
        steady(&mut estimator, 100, 20, 1);
        assert!(
            estimator.jitter_ms() < 1.0,
            "paced source should measure ~0, got {}",
            estimator.jitter_ms()
        );
        // With no jitter the target collapses to the floor.
        assert_eq!(estimator.target_frames(20, 60, 500), 3);
    }

    /// The measured live Vapi shape: pairs of ~46 ms chunks arriving together,
    /// then a ~50 ms gap. The estimator should land near the burst size.
    #[test]
    fn a_coalescing_source_measures_its_burst() {
        let mut estimator = JitterEstimator::default();
        let mut now = Instant::now();
        for _ in 0..200 {
            // Two chunks back to back, each carrying ~46 ms of audio.
            estimator.observe(now, 2, 20);
            estimator.observe(now, 2, 20);
            now += Duration::from_millis(50);
        }
        let jitter = estimator.jitter_ms();
        assert!(
            jitter > 5.0,
            "a coalescing source must register deviation, got {jitter}"
        );
        let target = estimator.target_frames(20, 60, 500);
        // Comfortably above the floor, far below the ceiling.
        assert!(
            (3..=25).contains(&target),
            "target {target} frames is outside a sane range for this source"
        );
    }

    #[test]
    fn the_target_is_clamped_at_both_ends() {
        let mut estimator = JitterEstimator::default();
        // Wildly erratic arrivals.
        let mut now = Instant::now();
        for i in 0..100 {
            now += Duration::from_millis(if i % 2 == 0 { 400 } else { 1 });
            estimator.observe(now, 1, 20);
        }
        // However bad it gets, never exceed the ceiling.
        assert!(estimator.target_frames(20, 60, 500) <= 25);
        // And never drop below the floor.
        assert!(estimator.target_frames(20, 200, 500) >= 10);
    }

    #[test]
    fn an_untrusted_estimate_falls_back_to_the_floor() {
        let mut estimator = JitterEstimator::default();
        let now = Instant::now();
        estimator.observe(now, 1, 20);
        estimator.observe(now + Duration::from_millis(500), 1, 20);
        // Two observations is noise, not a measurement.
        assert_eq!(estimator.target_frames(20, 60, 500), 3);
    }

    #[test]
    fn a_zero_frame_chunk_is_ignored() {
        let mut estimator = JitterEstimator::default();
        let now = Instant::now();
        estimator.observe(now, 0, 20);
        assert_eq!(estimator.jitter_ms(), 0.0);
    }
}

//! Per-call media health.
//!
//! A bridge between an unpaced TCP source and a paced RTP sink fails in ways
//! that are invisible from the outside: a queue that fills stays filled and
//! becomes permanent delay, a route that loses its sink keeps consuming frames
//! and discarding them, a burst larger than the buffer silently truncates an
//! utterance. Every one of those reports success.
//!
//! Nothing in the comparable field exposes this. `mod_audio_fork`,
//! `mod_audio_stream`, and the VoiSmart fork each call FreeSWITCH's
//! `media_bug_inuse` probe zero times. This is the counter set we wanted while
//! debugging and did not have.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Live counters for one Vapi media session. Cheap enough to update per frame.
#[derive(Debug, Default)]
pub(crate) struct MediaHealthState {
    inbound_queue_frames: AtomicUsize,
    inbound_queue_high_water: AtomicUsize,
    outbound_queue_frames: AtomicUsize,
    outbound_queue_high_water: AtomicUsize,
    inbound_dropped: AtomicU64,
    outbound_dropped: AtomicU64,
    barge_in_dropped: AtomicU64,
    underrun_ticks: AtomicU64,
    catchup_ticks: AtomicU64,
    catchup_blocked_ticks: AtomicU64,
    media_write_timeouts: AtomicU64,
    /// Largest single WebSocket binary message observed. The burst envelope:
    /// if this ever approaches the jitter buffer, drop-oldest is discarding
    /// most of an utterance rather than trimming latency.
    largest_inbound_chunk_bytes: AtomicUsize,
    /// Measured RFC 3550 jitter in tenths of a millisecond, so the adaptive
    /// target can be checked against reality rather than trusted.
    jitter_tenths_ms: AtomicU64,
    jitter_target_frames: AtomicUsize,
    inbound_frames: AtomicU64,
    outbound_frames: AtomicU64,
}

impl MediaHealthState {
    pub(crate) fn record_inbound_chunk(&self, bytes: usize) {
        self.largest_inbound_chunk_bytes
            .fetch_max(bytes, Ordering::Relaxed);
    }

    pub(crate) fn set_inbound_depth(&self, frames: usize) {
        self.inbound_queue_frames.store(frames, Ordering::Relaxed);
        self.inbound_queue_high_water
            .fetch_max(frames, Ordering::Relaxed);
    }

    pub(crate) fn set_outbound_depth(&self, frames: usize) {
        self.outbound_queue_frames.store(frames, Ordering::Relaxed);
        self.outbound_queue_high_water
            .fetch_max(frames, Ordering::Relaxed);
    }

    pub(crate) fn add_inbound_dropped(&self, n: u64) {
        self.inbound_dropped.fetch_add(n, Ordering::Relaxed);
    }

    pub(crate) fn add_outbound_dropped(&self, n: u64) {
        self.outbound_dropped.fetch_add(n, Ordering::Relaxed);
    }

    pub(crate) fn add_barge_in_dropped(&self, n: u64) {
        self.barge_in_dropped.fetch_add(n, Ordering::Relaxed);
    }

    pub(crate) fn tick_underrun(&self) {
        self.underrun_ticks.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn tick_catchup(&self) {
        self.catchup_ticks.fetch_add(1, Ordering::Relaxed);
    }

    /// The drain was above target but could not release an extra frame,
    /// because the downstream consumer is itself rate-paced.
    pub(crate) fn tick_catchup_blocked(&self) {
        self.catchup_blocked_ticks.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn tick_write_timeout(&self) {
        self.media_write_timeouts.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn set_jitter(&self, jitter_ms: f32, target_frames: usize) {
        self.jitter_tenths_ms
            .store((jitter_ms * 10.0).round().max(0.0) as u64, Ordering::Relaxed);
        self.jitter_target_frames
            .store(target_frames, Ordering::Relaxed);
    }

    pub(crate) fn add_inbound_frames(&self, n: u64) {
        self.inbound_frames.fetch_add(n, Ordering::Relaxed);
    }

    pub(crate) fn add_outbound_frames(&self, n: u64) {
        self.outbound_frames.fetch_add(n, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self, frame_ms: u64) -> VapiMediaHealth {
        let depth = |v: &AtomicUsize| v.load(Ordering::Relaxed);
        let count = |v: &AtomicU64| v.load(Ordering::Relaxed);
        VapiMediaHealth {
            inbound_queue_ms: depth(&self.inbound_queue_frames) as u64 * frame_ms,
            inbound_queue_high_water_ms: depth(&self.inbound_queue_high_water) as u64 * frame_ms,
            outbound_queue_ms: depth(&self.outbound_queue_frames) as u64 * frame_ms,
            outbound_queue_high_water_ms: depth(&self.outbound_queue_high_water) as u64 * frame_ms,
            inbound_frames: count(&self.inbound_frames),
            outbound_frames: count(&self.outbound_frames),
            inbound_dropped: count(&self.inbound_dropped),
            outbound_dropped: count(&self.outbound_dropped),
            barge_in_dropped: count(&self.barge_in_dropped),
            underrun_ticks: count(&self.underrun_ticks),
            catchup_ticks: count(&self.catchup_ticks),
            catchup_blocked_ticks: count(&self.catchup_blocked_ticks),
            media_write_timeouts: count(&self.media_write_timeouts),
            largest_inbound_chunk_bytes: depth(&self.largest_inbound_chunk_bytes),
            measured_jitter_ms: count(&self.jitter_tenths_ms) as f32 / 10.0,
            jitter_target_ms: depth(&self.jitter_target_frames) as u64 * frame_ms,
        }
    }
}

/// A point-in-time view of one call's media health.
///
/// Durations are in milliseconds because that is what the numbers *mean*:
/// queue depth on a fixed-rate drain is added one-way delay, not a count.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct VapiMediaHealth {
    /// Current inbound jitter-buffer depth, i.e. added delay toward the caller.
    /// This is the steady-state figure and the one to alert on.
    pub inbound_queue_ms: u64,
    /// Peak depth over the whole call. Interpret with care: call
    /// establishment reliably produces a one-off spike (measured 340-500 ms
    /// outbound while steady state was 0 ms), so a high value here does NOT by
    /// itself mean the call carried that much delay.
    pub inbound_queue_high_water_ms: u64,
    /// Current outbound depth, i.e. added delay toward the agent.
    pub outbound_queue_ms: u64,
    pub outbound_queue_high_water_ms: u64,
    pub inbound_frames: u64,
    pub outbound_frames: u64,
    pub inbound_dropped: u64,
    pub outbound_dropped: u64,
    /// Frames discarded because the caller interrupted. Expected on a healthy
    /// conversation; a zero here on a chatty call means barge-in is not firing.
    pub barge_in_dropped: u64,
    /// Ticks with no audio available. The agent is silent most of a call, so
    /// this is normally the majority.
    pub underrun_ticks: u64,
    /// Ticks that actually released extra frames to re-converge latency.
    pub catchup_ticks: u64,
    /// Ticks that were above target but could not accelerate, because the
    /// downstream consumer is itself paced at real time. A high value here
    /// against a low `catchup_ticks` means the valve cannot do its job in this
    /// topology and the queue cap is the only real latency control.
    pub catchup_blocked_ticks: u64,
    pub media_write_timeouts: u64,
    /// Largest single inbound WebSocket message seen. Compare against the
    /// jitter buffer: if it approaches it, a burst is being truncated rather
    /// than merely delayed.
    pub largest_inbound_chunk_bytes: usize,
    /// RFC 3550 interarrival jitter measured over frame arrivals.
    pub measured_jitter_ms: f32,
    /// Depth the drain is currently converging to, derived from the above.
    pub jitter_target_ms: u64,
}

impl VapiMediaHealth {
    /// Whether the buffers are sitting deep rather than transiting.
    ///
    /// Deliberately reads *current* depth, not high-water: establishment
    /// produces a one-off spike on every healthy call, so alerting on the peak
    /// would fire constantly and mean nothing.
    pub fn is_latency_degraded(&self, budget_ms: u64) -> bool {
        self.inbound_queue_ms > budget_ms || self.outbound_queue_ms > budget_ms
    }

    /// Whether a single burst has approached the buffer, which means
    /// drop-oldest is discarding speech rather than trimming delay.
    pub fn is_burst_envelope_exceeded(&self, buffer_bytes: usize) -> bool {
        buffer_bytes > 0 && self.largest_inbound_chunk_bytes * 2 >= buffer_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_is_reported_as_delay_not_a_count() {
        let state = MediaHealthState::default();
        state.set_inbound_depth(25);
        state.set_inbound_depth(3);
        let snapshot = state.snapshot(20);
        // 25 frames was the peak: half a second of added one-way delay.
        assert_eq!(snapshot.inbound_queue_high_water_ms, 500);
        assert_eq!(snapshot.inbound_queue_ms, 60);
    }

    #[test]
    fn latency_degradation_is_judged_against_a_budget() {
        let state = MediaHealthState::default();
        state.set_inbound_depth(30);
        let snapshot = state.snapshot(20);
        assert!(snapshot.is_latency_degraded(500));
        assert!(!snapshot.is_latency_degraded(1_000));
    }

    /// Every healthy call spikes during establishment and then settles. Judging
    /// on the peak would flag all of them; judging on current depth flags none.
    /// Measured live: outbound peaked at 500 ms and ended at 0 ms.
    #[test]
    fn an_establishment_spike_is_not_reported_as_degraded() {
        let state = MediaHealthState::default();
        state.set_inbound_depth(25);
        state.set_inbound_depth(0);
        let snapshot = state.snapshot(20);
        assert_eq!(snapshot.inbound_queue_high_water_ms, 500);
        assert_eq!(snapshot.inbound_queue_ms, 0);
        assert!(
            !snapshot.is_latency_degraded(400),
            "a settled call must not be flagged on its startup peak"
        );
    }

    /// The risk this exists to catch: a burst larger than the buffer means
    /// drop-oldest discards most of an utterance and the caller hears the tail.
    #[test]
    fn burst_envelope_alarms_before_the_buffer_is_exceeded() {
        let state = MediaHealthState::default();
        // Measured live Vapi maximum: 743 bytes against a 25-frame (4000-byte)
        // buffer. Comfortable.
        state.record_inbound_chunk(743);
        assert!(!state.snapshot(20).is_burst_envelope_exceeded(4_000));
        // An utterance-sized burst is not.
        state.record_inbound_chunk(2_400);
        assert!(state.snapshot(20).is_burst_envelope_exceeded(4_000));
    }

    #[test]
    fn largest_chunk_is_a_maximum_not_a_last_value() {
        let state = MediaHealthState::default();
        state.record_inbound_chunk(743);
        state.record_inbound_chunk(160);
        assert_eq!(state.snapshot(20).largest_inbound_chunk_bytes, 743);
    }
}

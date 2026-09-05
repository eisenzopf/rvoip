//! Playout smoothing and packet-loss concealment for a decoded audio stream.
//!
//! Packet-level reordering belongs below this, in `rtp-core`'s
//! [`rvoip_rtp_core::buffer::AdaptiveJitterBuffer`]. What remains once frames
//! are decoded is the part
//! a listener actually hears: audio arriving in bursts rather than evenly,
//! and gaps where a packet never arrived at all.
//!
//! This buffer holds a short backlog so that late frames have somewhere to
//! land, emits frames in timestamp order, and — when the next frame in the
//! sequence is missing and waiting longer would cost more than it saves —
//! synthesizes one rather than emitting nothing.
//!
//! **Concealment is repeat-with-fade**, not pitch-synchronous extrapolation.
//! The previous frame is replayed with a decaying gain, and consecutive
//! losses decay to silence over a few frames. That is deliberately the cheap
//! technique: it removes the click of an abrupt gap, which is the dominant
//! artifact, without claiming the quality of G.711 Appendix I or Opus's
//! built-in PLC. A long burst of loss will sound like a fade to silence,
//! which is honest and far better than a train of clicks.
//!
//! Time is supplied by the caller rather than read from a clock, so playout
//! behaviour is exactly reproducible in tests.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::types::AudioFrame;

/// How the buffer trades latency against loss.
#[derive(Clone, Copy, Debug)]
pub struct PlayoutConfig {
    /// Depth the buffer fills to before it starts emitting.
    ///
    /// This is added latency in exchange for absorbing jitter. Two frames
    /// (~40 ms at the usual 20 ms packetization) is the customary floor for
    /// telephony; a route with visible jitter wants more.
    pub target_depth_frames: usize,
    /// Ceiling the adaptive depth may grow to.
    pub max_depth_frames: usize,
    /// How many consecutive frames may be concealed before the buffer stops
    /// synthesizing and emits silence.
    ///
    /// Repeating a frame indefinitely turns a dropped call into a buzzing
    /// loop, which sounds like a fault in Thelve rather than in the network.
    pub max_consecutive_concealed: usize,
    /// Whether the depth tracks observed jitter.
    pub adaptive: bool,
}

impl Default for PlayoutConfig {
    fn default() -> Self {
        Self {
            target_depth_frames: 2,
            max_depth_frames: 10,
            max_consecutive_concealed: 5,
            adaptive: true,
        }
    }
}

/// What the buffer did, for quality reporting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayoutStats {
    pub frames_emitted: u64,
    /// Frames that arrived after their playout point had passed. They are
    /// dropped: inserting them would play audio out of order.
    pub frames_late: u64,
    /// Frames synthesized because the real one never arrived.
    pub frames_concealed: u64,
    /// Frames arriving with a timestamp already seen.
    pub frames_duplicate: u64,
    /// Frames emitted ahead of the next ordinary tick to drain excess depth.
    ///
    /// This is the latency-reconvergence valve: a faster sender clock or an
    /// arrival burst cannot grow the queue without bound.
    pub frames_catch_up: u64,
    /// Estimated remote media-clock skew in parts per million.
    pub clock_skew_ppm: i32,
    /// Current depth in frames.
    pub depth: usize,
}

/// Reorders, paces, and conceals a decoded audio stream.
pub struct PlayoutBuffer {
    config: PlayoutConfig,
    /// Pending frames keyed by RTP timestamp, which is the media clock and
    /// therefore the correct playout order even when arrival order differs.
    pending: BTreeMap<u32, AudioFrame>,
    /// Timestamp of the first arrival, used as the wrap-safe ordering origin
    /// until the first frame is emitted.
    origin_timestamp: Option<u32>,
    /// The timestamp the next emitted frame should carry, once known.
    next_timestamp: Option<u32>,
    /// Samples per frame, learned from the first frame so a gap can be
    /// concealed at the right length before any second frame arrives.
    samples_per_frame: Option<u32>,
    last_emitted: Option<AudioFrame>,
    consecutive_concealed: usize,
    primed: bool,
    /// Local media-clock deadline for the next ordinary emission.
    next_playout_at: Option<Instant>,
    /// Inter-arrival jitter estimate, in frames, smoothed like RFC 3550's.
    jitter_frames: f64,
    last_arrival: Option<Instant>,
    /// Long-baseline arrival/timestamp origin used to distinguish oscillator
    /// drift from short-lived network jitter.
    clock_origin: Option<(Instant, u32)>,
    clock_skew_ppm: f64,
    stats: PlayoutStats,
}

impl std::fmt::Debug for PlayoutBuffer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlayoutBuffer")
            .field("pending", &self.pending.len())
            .field("primed", &self.primed)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl PlayoutBuffer {
    #[must_use]
    pub fn new(config: PlayoutConfig) -> Self {
        Self {
            config,
            pending: BTreeMap::new(),
            origin_timestamp: None,
            next_timestamp: None,
            samples_per_frame: None,
            last_emitted: None,
            consecutive_concealed: 0,
            primed: false,
            next_playout_at: None,
            jitter_frames: 0.0,
            last_arrival: None,
            clock_origin: None,
            clock_skew_ppm: 0.0,
            stats: PlayoutStats::default(),
        }
    }

    /// Accept one decoded frame.
    ///
    /// `arrived_at` drives the jitter estimate only; ordering comes from the
    /// frame's own RTP timestamp.
    pub fn push(&mut self, frame: AudioFrame, arrived_at: Instant) {
        self.origin_timestamp.get_or_insert(frame.timestamp);
        self.update_clock_skew(frame.timestamp, frame.sample_rate, arrived_at);
        let samples = u32::try_from(frame.samples.len()).unwrap_or(0);
        if samples > 0 {
            // The first frame teaches the buffer its frame length, so a gap
            // can be concealed at the right size even early in the stream.
            self.samples_per_frame.get_or_insert(samples);
        }

        let previous_arrival = self.last_arrival.replace(arrived_at);
        if self.config.adaptive {
            if let (Some(previous), Some(spacing)) = (previous_arrival, self.frame_duration()) {
                // Deviation from even spacing, smoothed by 1/16 as RFC 3550
                // smooths its own jitter estimate.
                let gap = arrived_at.saturating_duration_since(previous);
                let deviation = gap.as_secs_f64() - spacing.as_secs_f64();
                self.jitter_frames += (deviation.abs() / spacing.as_secs_f64().max(f64::EPSILON)
                    - self.jitter_frames)
                    / 16.0;
            }
        }

        if let Some(next) = self.next_timestamp {
            if wrapped_before(frame.timestamp, next) {
                // Its playout moment has passed; inserting it now would play
                // audio out of order.
                self.stats.frames_late += 1;
                return;
            }
        }
        if self.pending.contains_key(&frame.timestamp) {
            self.stats.frames_duplicate += 1;
            return;
        }
        self.pending.insert(frame.timestamp, frame);

        // A buffer past its ceiling is a route that is not going to recover
        // by waiting; release the oldest rather than growing latency.
        while self.pending.len() > self.config.max_depth_frames {
            if let Some(oldest) = self.oldest_pending_timestamp() {
                self.pending.remove(&oldest);
                self.stats.frames_late += 1;
            }
        }

        if !self.primed && self.pending.len() >= self.target_depth().max(1) {
            self.primed = true;
            // Preserve the configured backlog instead of consuming one frame
            // at the same instant it finishes filling. For two 20 ms frames,
            // the first sample is played 40 ms after the first arrival.
            let initial_wait = self.frame_duration().unwrap_or(Duration::from_millis(20));
            self.next_playout_at = Some(arrived_at + initial_wait);
        }
    }

    /// Produce the next frame to play, if one is due.
    ///
    /// Returns `None` while the buffer is still filling to its depth, which
    /// is the only time it deliberately produces nothing.
    pub fn pop(&mut self) -> Option<AudioFrame> {
        if !self.primed {
            if self.pending.len() < self.target_depth().max(1) {
                return None;
            }
            self.primed = true;
        }

        self.pop_next()
    }

    /// Local media-clock deadline for the next playout decision.
    ///
    /// `None` means the buffer is still filling. Once primed, the caller
    /// should wake at this deadline even if no packet arrives: a missing RTP
    /// packet must become PLC on time rather than an audible hole.
    #[must_use]
    pub const fn next_deadline(&self) -> Option<Instant> {
        self.next_playout_at
    }

    /// Emit one frame only when its media-clock deadline is due, or when the
    /// queue is above its current target and must drain to reconverge latency.
    ///
    /// The ordinary path advances from the previous deadline (not from
    /// `now`), so task scheduling jitter cannot permanently skew the playout
    /// clock. The early-drain path deliberately leaves the deadline unchanged:
    /// it is the equivalent of a jitter buffer's skip-timer valve.
    pub fn pop_due(&mut self, now: Instant) -> Option<AudioFrame> {
        if !self.primed {
            return None;
        }

        let deadline = self.next_playout_at?;
        let due = now >= deadline;
        let drain_down = self.pending.len() > self.target_depth().max(1);
        if !due && !drain_down {
            return None;
        }

        let frame = self.pop_next()?;
        if due {
            let nominal = nonzero_frame_duration(&frame)
                .or_else(|| self.frame_duration())
                .unwrap_or(Duration::from_millis(20));
            let duration = scale_duration(nominal, self.clock_skew_ppm);
            self.next_playout_at = Some(deadline + duration);
        } else {
            self.stats.frames_catch_up += 1;
        }
        Some(frame)
    }

    fn pop_next(&mut self) -> Option<AudioFrame> {
        let expected = match self.next_timestamp {
            Some(timestamp) => timestamp,
            None => self.oldest_pending_timestamp()?,
        };

        if let Some(frame) = self.pending.remove(&expected) {
            self.consecutive_concealed = 0;
            self.advance(&frame);
            self.stats.frames_emitted += 1;
            self.last_emitted = Some(frame.clone());
            return Some(frame);
        }

        // The expected frame is absent, and a playout clock that has already
        // started cannot wait: the concealment *is* the wait. Bounded by
        // `max_consecutive_concealed`, past which it becomes silence, so a
        // far end that has simply gone quiet does not buzz forever.
        self.conceal(expected)
    }

    fn oldest_pending_timestamp(&self) -> Option<u32> {
        let origin = self.origin_timestamp?;
        self.pending
            .keys()
            .copied()
            // A signed wrapping delta gives the correct order for the
            // practical RTP window on either side of 2^32 wrap.
            .min_by_key(|timestamp| timestamp.wrapping_sub(origin) as i32)
    }

    /// Synthesize the frame that should have arrived.
    fn conceal(&mut self, expected: u32) -> Option<AudioFrame> {
        let template = self.last_emitted.clone()?;
        if self.consecutive_concealed >= self.config.max_consecutive_concealed {
            // Past this, the far end is gone rather than jittery. Emitting
            // silence is honest; repeating is a buzz that sounds like us.
            let mut silence = template.clone();
            silence.samples.iter_mut().for_each(|sample| *sample = 0);
            silence.timestamp = expected;
            self.consecutive_concealed += 1;
            self.stats.frames_concealed += 1;
            self.stats.frames_emitted += 1;
            self.advance(&silence);
            self.last_emitted = Some(silence.clone());
            return Some(silence);
        }

        // Repeat with a decaying gain: each successive loss is quieter, so a
        // burst fades out instead of stuttering the same 20 ms forever.
        let step = self.consecutive_concealed + 1;
        let gain = 1.0_f32 / (1.0 + step as f32);
        let mut concealed = template;
        for sample in &mut concealed.samples {
            *sample = (f32::from(*sample) * gain) as i16;
        }
        concealed.timestamp = expected;
        self.consecutive_concealed += 1;
        self.stats.frames_concealed += 1;
        self.stats.frames_emitted += 1;
        self.advance(&concealed);
        self.last_emitted = Some(concealed.clone());
        Some(concealed)
    }

    /// Move the expected timestamp on by one frame.
    fn advance(&mut self, emitted: &AudioFrame) {
        let samples = u32::try_from(emitted.samples.len())
            .ok()
            .filter(|count| *count > 0)
            .or(self.samples_per_frame)
            .unwrap_or(0);
        self.next_timestamp = Some(emitted.timestamp.wrapping_add(samples));
    }

    /// Depth to fill to, grown by observed jitter when adaptive.
    fn target_depth(&self) -> usize {
        if !self.config.adaptive {
            return self.config.target_depth_frames;
        }
        let extra = self.jitter_frames.ceil().max(0.0) as usize;
        self.config
            .target_depth_frames
            .saturating_add(extra)
            .min(self.config.max_depth_frames)
    }

    fn frame_duration(&self) -> Option<Duration> {
        self.last_emitted
            .as_ref()
            .map(|frame| frame.duration)
            .filter(|duration| !duration.is_zero())
            .or_else(|| {
                self.pending
                    .values()
                    .next()
                    .map(|frame| frame.duration)
                    .filter(|duration| !duration.is_zero())
            })
    }

    fn update_clock_skew(&mut self, timestamp: u32, sample_rate: u32, arrived_at: Instant) {
        let Some((origin_arrival, origin_timestamp)) = self.clock_origin else {
            self.clock_origin = Some((arrived_at, timestamp));
            return;
        };
        if sample_rate == 0 {
            return;
        }

        let timestamp_delta = timestamp.wrapping_sub(origin_timestamp);
        // A signed-negative wrapping delta is an out-of-order packet, not a
        // clock observation. Wait for one second of media before estimating;
        // over that baseline ordinary packet jitter contributes little.
        if timestamp_delta as i32 <= 0 || timestamp_delta < sample_rate {
            return;
        }
        let nominal = f64::from(timestamp_delta) / f64::from(sample_rate);
        let actual = arrived_at
            .saturating_duration_since(origin_arrival)
            .as_secs_f64();
        let observed_ppm = ((actual / nominal) - 1.0) * 1_000_000.0;
        if observed_ppm.is_finite() {
            // Commodity oscillators are normally well inside ±100 ppm. The
            // wider bound tolerates imperfect clocks while preventing a route
            // delay step from turning into an unsafe playout-rate change.
            self.clock_skew_ppm = observed_ppm.clamp(-1_000.0, 1_000.0);
            self.stats.clock_skew_ppm = self.clock_skew_ppm.round() as i32;
        }
    }

    #[must_use]
    pub fn stats(&self) -> PlayoutStats {
        PlayoutStats {
            depth: self.pending.len(),
            ..self.stats
        }
    }
}

fn nonzero_frame_duration(frame: &AudioFrame) -> Option<Duration> {
    (!frame.duration.is_zero()).then_some(frame.duration)
}

fn scale_duration(duration: Duration, skew_ppm: f64) -> Duration {
    Duration::from_secs_f64(duration.as_secs_f64() * (1.0 + skew_ppm / 1_000_000.0))
}

/// Whether `a` precedes `b` in RTP timestamp space, which wraps at 2^32.
///
/// A plain `<` would treat a wrap as a 4-billion-sample jump backwards and
/// discard every frame after it.
fn wrapped_before(a: u32, b: u32) -> bool {
    a != b && b.wrapping_sub(a) < u32::MAX / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLES: usize = 160;

    fn frame(timestamp: u32, level: i16) -> AudioFrame {
        AudioFrame {
            samples: vec![level; SAMPLES],
            sample_rate: 8_000,
            channels: 1,
            duration: Duration::from_millis(20),
            timestamp,
        }
    }

    fn buffer() -> (PlayoutBuffer, Instant) {
        (
            PlayoutBuffer::new(PlayoutConfig {
                adaptive: false,
                ..PlayoutConfig::default()
            }),
            Instant::now(),
        )
    }

    #[test]
    fn it_fills_before_it_plays() {
        let (mut playout, now) = buffer();
        playout.push(frame(0, 100), now);
        assert!(
            playout.pop().is_none(),
            "playing the first frame immediately leaves nothing to absorb jitter with"
        );
        playout.push(frame(160, 200), now);
        assert_eq!(playout.pop().expect("primed").timestamp, 0);
    }

    #[test]
    fn it_reorders_by_timestamp_rather_than_arrival() {
        let (mut playout, now) = buffer();
        // Arrives second, first, third.
        playout.push(frame(160, 2), now);
        playout.push(frame(0, 1), now);
        playout.push(frame(320, 3), now);

        let order: Vec<u32> = (0..3)
            .filter_map(|_| playout.pop().map(|frame| frame.timestamp))
            .collect();
        assert_eq!(order, vec![0, 160, 320]);
    }

    #[test]
    fn scheduled_playout_waits_for_the_media_clock_not_packet_arrival() {
        let mut playout = PlayoutBuffer::new(PlayoutConfig {
            target_depth_frames: 3,
            adaptive: false,
            ..PlayoutConfig::default()
        });
        let now = Instant::now();
        playout.push(frame(0, 1), now);
        playout.push(frame(160, 2), now);

        assert!(playout.pop_due(now).is_none());
        playout.push(frame(320, 3), now + Duration::from_millis(1));
        assert!(
            playout.pop_due(now + Duration::from_millis(1)).is_none(),
            "a network arrival must not advance the playout clock"
        );
        assert!(playout.pop_due(now + Duration::from_millis(20)).is_none());
        assert_eq!(
            playout
                .pop_due(now + Duration::from_millis(21))
                .expect("first tick")
                .timestamp,
            0
        );
        assert!(playout.pop_due(now + Duration::from_millis(22)).is_none());
    }

    #[test]
    fn scheduled_playout_conceals_a_missing_packet_on_its_deadline() {
        let (mut playout, now) = buffer();
        playout.push(frame(0, 1_000), now);
        playout.push(frame(320, 1_000), now);

        assert!(playout.pop_due(now).is_none());
        assert_eq!(
            playout
                .pop_due(now + Duration::from_millis(20))
                .expect("first tick")
                .timestamp,
            0
        );
        let concealed = playout
            .pop_due(now + Duration::from_millis(40))
            .expect("loss deadline");
        assert_eq!(concealed.timestamp, 160);
        assert_eq!(playout.stats().frames_concealed, 1);
        assert_eq!(
            playout
                .pop_due(now + Duration::from_millis(60))
                .expect("real media resumes")
                .timestamp,
            320
        );
    }

    #[test]
    fn a_gap_is_concealed_rather_than_left_silent() {
        let (mut playout, now) = buffer();
        playout.push(frame(0, 1_000), now);
        playout.push(frame(320, 1_000), now); // 160 never arrives
        playout.push(frame(480, 1_000), now);

        assert_eq!(playout.pop().expect("first").timestamp, 0);
        let concealed = playout.pop().expect("the gap produces a frame");
        assert_eq!(concealed.timestamp, 160);
        assert!(
            concealed.samples.iter().any(|sample| *sample != 0),
            "concealment repeats audio at reduced gain; silence would click"
        );
        assert!(
            concealed.samples[0].abs() < 1_000,
            "the repeat is attenuated rather than played at full level"
        );
        assert_eq!(playout.stats().frames_concealed, 1);
        assert_eq!(playout.pop().expect("resumes").timestamp, 320);
    }

    #[test]
    fn a_long_loss_fades_to_silence_instead_of_buzzing() {
        let mut playout = PlayoutBuffer::new(PlayoutConfig {
            adaptive: false,
            max_consecutive_concealed: 2,
            ..PlayoutConfig::default()
        });
        let now = Instant::now();
        playout.push(frame(0, 4_000), now);
        playout.push(frame(160, 4_000), now);
        playout.pop().expect("first");
        playout.pop().expect("second");

        // Nothing more arrives: successive concealments must decay.
        let first = playout.pop().expect("conceal 1");
        let second = playout.pop().expect("conceal 2");
        let third = playout.pop().expect("conceal 3");
        assert!(first.samples[0].abs() > second.samples[0].abs());
        assert_eq!(
            third.samples[0], 0,
            "past the ceiling the far end is gone, so silence is the honest output"
        );
    }

    #[test]
    fn a_frame_that_arrives_after_its_moment_is_dropped() {
        let (mut playout, now) = buffer();
        playout.push(frame(0, 1), now);
        playout.push(frame(160, 2), now);
        playout.pop().expect("first");
        playout.pop().expect("second");

        // 0 already played; replaying it would play audio out of order.
        playout.push(frame(0, 9), now);
        assert_eq!(playout.stats().frames_late, 1);
        assert!(playout.pop().is_none() || playout.stats().frames_concealed > 0);
    }

    #[test]
    fn a_repeated_timestamp_is_counted_once() {
        let (mut playout, now) = buffer();
        playout.push(frame(0, 1), now);
        playout.push(frame(0, 1), now);
        assert_eq!(playout.stats().frames_duplicate, 1);
        assert_eq!(playout.stats().depth, 1);
    }

    #[test]
    fn the_buffer_does_not_grow_without_bound() {
        let mut playout = PlayoutBuffer::new(PlayoutConfig {
            adaptive: false,
            max_depth_frames: 3,
            ..PlayoutConfig::default()
        });
        let now = Instant::now();
        for index in 0..10 {
            playout.push(frame(index * 160, 1), now);
        }
        assert!(
            playout.stats().depth <= 3,
            "a route that is not recovering must not accumulate latency"
        );
    }

    #[test]
    fn a_fast_remote_clock_drains_down_instead_of_accumulating_latency() {
        let mut playout = PlayoutBuffer::new(PlayoutConfig {
            adaptive: false,
            ..PlayoutConfig::default()
        });
        let start = Instant::now();
        let remote_period = Duration::from_nanos(19_999_000); // 50 ppm fast

        for index in 0_u32..180_000 {
            let arrival = start + remote_period * index;
            playout.push(frame(index.wrapping_mul(160), 1), arrival);
            while playout
                .next_deadline()
                .is_some_and(|deadline| deadline <= arrival)
            {
                playout.pop_due(arrival).expect("due frame");
            }
            while playout.pop_due(arrival).is_some() {}
        }

        let stats = playout.stats();
        assert!(stats.frames_catch_up > 0, "the drain valve never opened");
        assert!(
            stats.depth <= PlayoutConfig::default().target_depth_frames,
            "a one-hour 50 ppm skew accumulated {} frames",
            stats.depth
        );
    }

    #[test]
    fn a_slow_remote_clock_uses_bounded_plc_instead_of_drifting() {
        let mut playout = PlayoutBuffer::new(PlayoutConfig {
            adaptive: false,
            ..PlayoutConfig::default()
        });
        let start = Instant::now();
        let remote_period = Duration::from_nanos(20_001_000); // 50 ppm slow

        for index in 0_u32..180_000 {
            let arrival = start + remote_period * index;
            while playout
                .next_deadline()
                .is_some_and(|deadline| deadline <= arrival)
            {
                playout.pop_due(arrival).expect("due frame or PLC");
            }
            playout.push(frame(index.wrapping_mul(160), 1), arrival);
        }

        let stats = playout.stats();
        assert!(
            (stats.clock_skew_ppm - 50).abs() <= 1,
            "remote clock estimate was {} ppm instead of 50 ppm",
            stats.clock_skew_ppm
        );
        assert!(
            stats.frames_concealed <= 1,
            "clock tracking should avoid frame-sized PLC corrections, got {}",
            stats.frames_concealed
        );
        assert!(stats.depth <= PlayoutConfig::default().target_depth_frames);
    }

    #[test]
    fn timestamp_wrap_is_not_mistaken_for_a_leap_backwards() {
        assert!(wrapped_before(u32::MAX - 160, u32::MAX));
        // Across the wrap, the earlier timestamp is the larger number.
        assert!(wrapped_before(u32::MAX, 160));
        assert!(!wrapped_before(160, u32::MAX));
    }

    #[test]
    fn startup_order_crosses_timestamp_wrap_correctly() {
        let (mut playout, now) = buffer();
        playout.push(frame(u32::MAX - 159, 1), now);
        playout.push(frame(0, 2), now);

        assert!(playout.pop_due(now).is_none());
        assert_eq!(
            playout
                .pop_due(now + Duration::from_millis(20))
                .expect("pre-wrap frame")
                .timestamp,
            u32::MAX - 159
        );
        assert_eq!(
            playout
                .pop_due(now + Duration::from_millis(40))
                .expect("post-wrap frame")
                .timestamp,
            0
        );
    }
}

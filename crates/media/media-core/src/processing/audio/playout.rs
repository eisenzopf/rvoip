//! Playout smoothing and packet-loss concealment for a decoded audio stream.
//!
//! Packet-level reordering belongs below this, in `rtp-core`'s
//! [`AdaptiveJitterBuffer`]. What remains once frames are decoded is the part
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
    /// Current depth in frames.
    pub depth: usize,
}

/// Reorders, paces, and conceals a decoded audio stream.
pub struct PlayoutBuffer {
    config: PlayoutConfig,
    /// Pending frames keyed by RTP timestamp, which is the media clock and
    /// therefore the correct playout order even when arrival order differs.
    pending: BTreeMap<u32, AudioFrame>,
    /// The timestamp the next emitted frame should carry, once known.
    next_timestamp: Option<u32>,
    /// Samples per frame, learned from the first frame so a gap can be
    /// concealed at the right length before any second frame arrives.
    samples_per_frame: Option<u32>,
    last_emitted: Option<AudioFrame>,
    consecutive_concealed: usize,
    primed: bool,
    /// Inter-arrival jitter estimate, in frames, smoothed like RFC 3550's.
    jitter_frames: f64,
    last_arrival: Option<Instant>,
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
            next_timestamp: None,
            samples_per_frame: None,
            last_emitted: None,
            consecutive_concealed: 0,
            primed: false,
            jitter_frames: 0.0,
            last_arrival: None,
            stats: PlayoutStats::default(),
        }
    }

    /// Accept one decoded frame.
    ///
    /// `arrived_at` drives the jitter estimate only; ordering comes from the
    /// frame's own RTP timestamp.
    pub fn push(&mut self, frame: AudioFrame, arrived_at: Instant) {
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
            if let Some((&oldest, _)) = self.pending.iter().next() {
                self.pending.remove(&oldest);
                self.stats.frames_late += 1;
            }
        }
    }

    /// Produce the next frame to play, if one is due.
    ///
    /// Returns `None` while the buffer is still filling to its depth, which
    /// is the only time it deliberately produces nothing.
    pub fn pop(&mut self) -> Option<AudioFrame> {
        let depth = self.target_depth();
        if !self.primed {
            if self.pending.len() < depth {
                return None;
            }
            self.primed = true;
        }

        let expected = match self.next_timestamp {
            Some(timestamp) => timestamp,
            None => {
                let first = *self.pending.keys().next()?;
                first
            }
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
        let _ = depth;
        self.conceal(expected)
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

    #[must_use]
    pub fn stats(&self) -> PlayoutStats {
        PlayoutStats {
            depth: self.pending.len(),
            ..self.stats
        }
    }
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
    fn timestamp_wrap_is_not_mistaken_for_a_leap_backwards() {
        assert!(wrapped_before(u32::MAX - 160, u32::MAX));
        // Across the wrap, the earlier timestamp is the larger number.
        assert!(wrapped_before(u32::MAX, 160));
        assert!(!wrapped_before(160, u32::MAX));
    }
}

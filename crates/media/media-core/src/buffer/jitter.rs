//! Adaptive Jitter Buffer
//!
//! This module implements an adaptive jitter buffer for VoIP that handles
//! packet reordering, network jitter compensation, and smooth audio playback.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;
use tracing::{debug, trace, warn};

use crate::error::Result;
use crate::types::{AudioFrame, MediaPacket};

/// Jitter buffer configuration
#[derive(Debug, Clone)]
pub struct JitterBufferConfig {
    /// Initial buffer depth in frames
    pub initial_depth: usize,
    /// Minimum buffer depth in frames
    pub min_depth: usize,
    /// Maximum buffer depth in frames
    pub max_depth: usize,
    /// Frame duration in milliseconds
    pub frame_duration_ms: u32,
    /// Maximum age for late packets (ms)
    pub max_late_packet_age_ms: u32,
    /// Adaptation strategy
    pub adaptation_strategy: AdaptationStrategy,
    /// Enable statistics collection
    pub enable_statistics: bool,
}

impl Default for JitterBufferConfig {
    fn default() -> Self {
        Self {
            initial_depth: 4,      // 80ms at 20ms frames
            min_depth: 2,          // 40ms minimum
            max_depth: 20,         // 400ms maximum
            frame_duration_ms: 20, // Standard 20ms frames
            max_late_packet_age_ms: 100,
            adaptation_strategy: AdaptationStrategy::Conservative,
            enable_statistics: true,
        }
    }
}

/// Buffer adaptation strategies
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdaptationStrategy {
    /// Conservative: slow to adapt, stable
    Conservative,
    /// Balanced: moderate adaptation speed
    Balanced,
    /// Aggressive: fast adaptation, responsive
    Aggressive,
}

/// Buffered frame with metadata. The arrival-time / RTP-timestamp /
/// sequence-number fields are captured on insert for the late-arrival
/// statistics path that's wired in but not yet read on every pull.
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct BufferedFrame {
    /// Audio frame data
    frame: AudioFrame,
    /// Arrival timestamp
    arrival_time: Instant,
    /// Original RTP timestamp
    rtp_timestamp: u32,
    /// Sequence number
    sequence_number: u16,
}

/// Jitter buffer statistics
#[derive(Debug, Clone, Default)]
pub struct JitterBufferStats {
    /// Frames received
    pub frames_received: u64,
    /// Frames played
    pub frames_played: u64,
    /// Frames dropped (late)
    pub frames_dropped_late: u64,
    /// Frames dropped (overflow)
    pub frames_dropped_overflow: u64,
    /// Current buffer depth
    pub current_depth: usize,
    /// Average jitter (ms)
    pub average_jitter_ms: f32,
    /// Maximum jitter seen (ms)
    pub max_jitter_ms: f32,
    /// Adaptation count
    pub adaptation_count: u64,
    /// Underrun count
    pub underrun_count: u64,
}

/// Adaptive jitter buffer for smooth audio playback.
///
/// Per Phase C6 the per-frame hot path collapses six separate
/// `tokio::sync::RwLock.write().await` acquisitions (one each for
/// buffer / stats / next_sequence / jitter_state / last_playout_time /
/// target_depth) into a single `parking_lot::Mutex<Inner>` plus a
/// handful of atomic stat counters. The mutex is never held across
/// `.await`, so we drop the tokio scheduler dip entirely.
pub struct JitterBuffer {
    /// Configuration
    config: JitterBufferConfig,
    /// Tightly-coupled per-frame state: the BTreeMap, next-expected
    /// sequence cursor, RFC 3550 jitter accumulator, and last-playout
    /// timestamp. Lives behind one `parking_lot::Mutex` because every
    /// mutation hits two or more of these fields together.
    state: parking_lot::Mutex<JitterBufferInner>,
    /// Current target buffer depth. Atomic so `get_target_depth` and
    /// the playout-readiness check don't need to acquire the inner
    /// lock.
    target_depth: AtomicUsize,
    /// Statistics counters. Atomic loads/stores so concurrent
    /// `get_statistics()` snapshots never block the recv path. The
    /// non-counter fields of `JitterBufferStats` (current_depth,
    /// average_jitter_ms, max_jitter_ms) are synthesised at snapshot
    /// time from the mutex-protected state and atomic counters.
    frames_received: AtomicU64,
    frames_played: AtomicU64,
    frames_dropped_late: AtomicU64,
    frames_dropped_overflow: AtomicU64,
    underrun_count: AtomicU64,
    adaptation_count: AtomicU64,
}

/// Tightly-coupled per-frame state held under the single `state`
/// mutex.
struct JitterBufferInner {
    /// Buffered frames (indexed by sequence number)
    buffer: BTreeMap<u16, BufferedFrame>,
    /// Next expected sequence number
    next_sequence: Option<u16>,
    /// Jitter calculation state
    jitter_state: JitterState,
    /// Last playout timestamp
    last_playout_time: Option<Instant>,
}

/// Jitter calculation state
#[derive(Debug, Default)]
struct JitterState {
    /// Previous packet arrival time
    prev_arrival: Option<Instant>,
    /// Previous RTP timestamp
    prev_timestamp: Option<u32>,
    /// Running jitter estimate (RFC 3550)
    jitter: f32,
    /// Maximum jitter ever observed (ms), tracked here so `get_statistics`
    /// can synthesise it without needing a separate atomic with f32 bits.
    max_jitter_ms: f32,
}

impl JitterBuffer {
    /// Create a new jitter buffer
    pub fn new(config: JitterBufferConfig) -> Self {
        debug!("Creating JitterBuffer with config: {:?}", config);

        Self {
            target_depth: AtomicUsize::new(config.initial_depth),
            state: parking_lot::Mutex::new(JitterBufferInner {
                buffer: BTreeMap::new(),
                next_sequence: None,
                jitter_state: JitterState::default(),
                last_playout_time: None,
            }),
            config,
            frames_received: AtomicU64::new(0),
            frames_played: AtomicU64::new(0),
            frames_dropped_late: AtomicU64::new(0),
            frames_dropped_overflow: AtomicU64::new(0),
            underrun_count: AtomicU64::new(0),
            adaptation_count: AtomicU64::new(0),
        }
    }

    /// Add a frame to the buffer.
    ///
    /// Single mutex acquisition guards the jitter calc, lateness
    /// check, buffer insert, and overflow eviction; the counter
    /// updates are then a few relaxed atomic stores outside the
    /// critical section.
    pub async fn add_frame(&self, packet: MediaPacket, frame: AudioFrame) -> Result<()> {
        let arrival_time = Instant::now();
        let seq = packet.sequence_number;

        // Snapshot the depth and `Outcome` we observed inside the
        // single mutex section so we can update atomic counters and
        // log outside the lock.
        enum Outcome {
            Inserted { depth: usize },
            DroppedLate,
            InsertedAfterOverflow { depth: usize, evicted_seq: u16 },
        }

        let max_late_ms = self.config.max_late_packet_age_ms as u128;
        let max_depth = self.config.max_depth;

        let outcome = {
            let mut state = self.state.lock();

            // Update jitter calculation (RFC 3550) using the prev
            // arrival/timestamp snapshot in state.
            if let (Some(prev_arrival), Some(prev_timestamp)) = (
                state.jitter_state.prev_arrival,
                state.jitter_state.prev_timestamp,
            ) {
                let arrival_diff = arrival_time.duration_since(prev_arrival).as_millis() as i32;
                let timestamp_diff = packet.timestamp.wrapping_sub(prev_timestamp) as i32;
                let d = (arrival_diff - timestamp_diff).abs() as f32;
                state.jitter_state.jitter += (d - state.jitter_state.jitter) / 16.0;
                if state.jitter_state.jitter > state.jitter_state.max_jitter_ms {
                    state.jitter_state.max_jitter_ms = state.jitter_state.jitter;
                }
            }
            state.jitter_state.prev_arrival = Some(arrival_time);
            state.jitter_state.prev_timestamp = Some(packet.timestamp);

            // Lateness check: sequence-distance wrap-around + time-based.
            let too_late = {
                let seq_late = state
                    .next_sequence
                    .map(|expected| seq.wrapping_sub(expected) > 32768)
                    .unwrap_or(false);
                let time_late = state
                    .last_playout_time
                    .map(|last| arrival_time.duration_since(last).as_millis() > max_late_ms)
                    .unwrap_or(false);
                seq_late || time_late
            };
            if too_late {
                Outcome::DroppedLate
            } else {
                let buffered_frame = BufferedFrame {
                    frame,
                    arrival_time,
                    rtp_timestamp: packet.timestamp,
                    sequence_number: seq,
                };

                let mut evicted = None;
                if state.buffer.len() >= max_depth {
                    if let Some((&oldest, _)) = state.buffer.iter().next() {
                        state.buffer.remove(&oldest);
                        evicted = Some(oldest);
                    }
                }
                state.buffer.insert(seq, buffered_frame);
                let depth = state.buffer.len();
                match evicted {
                    Some(evicted_seq) => Outcome::InsertedAfterOverflow { depth, evicted_seq },
                    None => Outcome::Inserted { depth },
                }
            }
        };

        // Counters + tracing outside the lock.
        match outcome {
            Outcome::DroppedLate => {
                self.frames_dropped_late.fetch_add(1, Ordering::Relaxed);
                trace!("Dropped late packet: seq={}", seq);
            }
            Outcome::Inserted { depth } => {
                self.frames_received.fetch_add(1, Ordering::Relaxed);
                trace!(
                    "Added frame to jitter buffer: seq={}, buffer_depth={}",
                    seq,
                    depth
                );
            }
            Outcome::InsertedAfterOverflow { depth, evicted_seq } => {
                self.frames_received.fetch_add(1, Ordering::Relaxed);
                self.frames_dropped_overflow.fetch_add(1, Ordering::Relaxed);
                warn!("Buffer overflow, dropped frame: seq={}", evicted_seq);
                trace!(
                    "Added frame to jitter buffer: seq={}, buffer_depth={}",
                    seq,
                    depth
                );
            }
        }

        Ok(())
    }

    /// Get the next frame for playout.
    ///
    /// Single mutex acquisition covers the readiness check, gap
    /// handling, frame removal, next-sequence cursor advance, and
    /// last-playout-time update. Adaptation (target_depth recompute)
    /// runs outside the lock against atomics.
    pub async fn get_next_frame(&self) -> Result<Option<AudioFrame>> {
        let target_depth = self.target_depth.load(Ordering::Relaxed);

        #[allow(dead_code)]
        enum Pulled {
            Empty,
            NotReady,
            Frame(AudioFrame),
            Underrun,
        }

        let pulled = {
            let mut state = self.state.lock();

            // Lazy init: first call adopts the oldest buffered seq as
            // the playout cursor.
            if state.next_sequence.is_none() {
                if let Some((&first_seq, _)) = state.buffer.iter().next() {
                    state.next_sequence = Some(first_seq);
                } else {
                    return Ok(None); // Buffer empty
                }
            }
            let target_seq = state.next_sequence.expect("set above");

            // Readiness gate: target_depth frames must be buffered.
            if state.buffer.len() < target_depth {
                Pulled::NotReady
            } else if let Some(buffered_frame) = state.buffer.remove(&target_seq) {
                state.next_sequence = Some(target_seq.wrapping_add(1));
                state.last_playout_time = Some(Instant::now());
                Pulled::Frame(buffered_frame.frame)
            } else {
                // Gap: try the next available frame, or report underrun.
                let next_available = state
                    .buffer
                    .range(target_seq.wrapping_add(1)..)
                    .next()
                    .map(|(&s, f)| (s, f.clone()));
                if let Some((next_seq, buffered_frame)) = next_available {
                    state.buffer.remove(&next_seq);
                    state.next_sequence = Some(target_seq.wrapping_add(1));
                    state.last_playout_time = Some(Instant::now());
                    warn!(
                        "Missing frame seq={}, using seq={} instead",
                        target_seq, next_seq
                    );
                    Pulled::Frame(buffered_frame.frame)
                } else {
                    state.next_sequence = Some(target_seq.wrapping_add(1));
                    Pulled::Underrun
                }
            }
        };

        let frame = match pulled {
            Pulled::Empty => return Ok(None),
            Pulled::NotReady => return Ok(None),
            Pulled::Underrun => {
                self.underrun_count.fetch_add(1, Ordering::Relaxed);
                None
            }
            Pulled::Frame(f) => {
                self.frames_played.fetch_add(1, Ordering::Relaxed);
                Some(f)
            }
        };

        // Adapt buffer depth based on observed jitter. The mutex is
        // acquired only to read the latest jitter value, then dropped
        // before we CAS target_depth.
        self.adapt_buffer_depth();

        Ok(frame)
    }

    /// Adapt buffer depth based on network conditions. Synchronous —
    /// no `.await`. Takes a brief read on the inner mutex for the
    /// current jitter, then uses a CAS-style store on the
    /// `target_depth` atomic.
    fn adapt_buffer_depth(&self) {
        let jitter = self.state.lock().jitter_state.jitter;
        let current_target = self.target_depth.load(Ordering::Relaxed);
        let frame_duration = self.config.frame_duration_ms as f32;
        let jitter_frames = (jitter / frame_duration).ceil() as usize;
        let desired_depth = match self.config.adaptation_strategy {
            AdaptationStrategy::Conservative => {
                (current_target + jitter_frames * 2).max(self.config.min_depth)
            }
            AdaptationStrategy::Balanced => (jitter_frames * 3 + 2).max(self.config.min_depth),
            AdaptationStrategy::Aggressive => (jitter_frames * 2 + 1).max(self.config.min_depth),
        }
        .min(self.config.max_depth);

        if desired_depth != current_target
            && (desired_depth as i32 - current_target as i32).abs() > 1
        {
            self.target_depth.store(desired_depth, Ordering::Relaxed);
            self.adaptation_count.fetch_add(1, Ordering::Relaxed);
            debug!(
                "Adapted jitter buffer depth: {} -> {} (jitter: {:.1}ms)",
                current_target, desired_depth, jitter
            );
        }
    }

    /// Get current buffer depth
    pub async fn get_current_depth(&self) -> usize {
        self.state.lock().buffer.len()
    }

    /// Get target buffer depth
    pub async fn get_target_depth(&self) -> usize {
        self.target_depth.load(Ordering::Relaxed)
    }

    /// Get buffer statistics. Aggregates atomic counters with a brief
    /// inner-mutex snapshot for the jitter/depth fields.
    pub async fn get_statistics(&self) -> JitterBufferStats {
        let (current_depth, average_jitter_ms, max_jitter_ms) = {
            let state = self.state.lock();
            (
                state.buffer.len(),
                state.jitter_state.jitter,
                state.jitter_state.max_jitter_ms,
            )
        };
        JitterBufferStats {
            frames_received: self.frames_received.load(Ordering::Relaxed),
            frames_played: self.frames_played.load(Ordering::Relaxed),
            frames_dropped_late: self.frames_dropped_late.load(Ordering::Relaxed),
            frames_dropped_overflow: self.frames_dropped_overflow.load(Ordering::Relaxed),
            current_depth,
            average_jitter_ms,
            max_jitter_ms,
            adaptation_count: self.adaptation_count.load(Ordering::Relaxed),
            underrun_count: self.underrun_count.load(Ordering::Relaxed),
        }
    }

    /// Reset the buffer
    pub async fn reset(&self) {
        {
            let mut state = self.state.lock();
            state.buffer.clear();
            state.next_sequence = None;
            state.jitter_state = JitterState::default();
            state.last_playout_time = None;
        }
        self.target_depth
            .store(self.config.initial_depth, Ordering::Relaxed);
        debug!("Jitter buffer reset");
    }

    /// Check if buffer is empty
    pub async fn is_empty(&self) -> bool {
        self.state.lock().buffer.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_jitter_buffer_creation() {
        let config = JitterBufferConfig::default();
        let buffer = JitterBuffer::new(config);

        assert_eq!(buffer.get_current_depth().await, 0);
        assert_eq!(buffer.get_target_depth().await, 4);
        assert!(buffer.is_empty().await);
    }

    #[tokio::test]
    async fn test_frame_addition_and_retrieval() {
        let config = JitterBufferConfig {
            initial_depth: 2,
            min_depth: 1,
            max_depth: 10,
            ..Default::default()
        };
        let buffer = JitterBuffer::new(config);

        // Create test frames
        let frame1 = AudioFrame::new(vec![100; 160], 8000, 1, 1000);
        let frame2 = AudioFrame::new(vec![200; 160], 8000, 1, 1001);
        let frame3 = AudioFrame::new(vec![300; 160], 8000, 1, 1002);

        let packet1 = MediaPacket {
            payload: vec![1, 2, 3].into(),
            payload_type: 0,
            sequence_number: 1000,
            timestamp: 8000,
            ssrc: 12345,
            received_at: std::time::Instant::now(),
        };

        let packet2 = MediaPacket {
            sequence_number: 1001,
            timestamp: 8160,
            ..packet1.clone()
        };
        let packet3 = MediaPacket {
            sequence_number: 1002,
            timestamp: 8320,
            ..packet1.clone()
        };

        // Add first frame
        buffer.add_frame(packet1, frame1).await.unwrap();

        // Should not be ready for playout yet (need target_depth frames)
        assert!(buffer.get_next_frame().await.unwrap().is_none());

        // Add second frame - now should reach target depth
        buffer.add_frame(packet2, frame2).await.unwrap();

        // Should be able to get frames now (reached target depth of 2)
        let retrieved = buffer.get_next_frame().await.unwrap();
        assert!(retrieved.is_some());

        // Add third frame
        buffer.add_frame(packet3, frame3).await.unwrap();

        let stats = buffer.get_statistics().await;
        assert_eq!(stats.frames_received, 3);
        assert_eq!(stats.frames_played, 1);
    }

    #[tokio::test]
    async fn test_buffer_reset() {
        let buffer = JitterBuffer::new(JitterBufferConfig::default());

        let frame = AudioFrame::new(vec![100; 160], 8000, 1, 1000);
        let packet = MediaPacket {
            payload: vec![1, 2, 3].into(),
            payload_type: 0,
            sequence_number: 1000,
            timestamp: 8000,
            ssrc: 12345,
            received_at: std::time::Instant::now(),
        };

        buffer.add_frame(packet, frame).await.unwrap();
        assert!(!buffer.is_empty().await);

        buffer.reset().await;
        assert!(buffer.is_empty().await);
        assert_eq!(buffer.get_target_depth().await, 4); // Reset to initial
    }
}

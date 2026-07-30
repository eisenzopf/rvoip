#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

use crate::{RtpSequenceNumber, RtpTimestamp};

/// Jitter estimator implementing RFC 3550 jitter calculation algorithm
#[derive(Debug, Clone)]
pub struct JitterEstimator {
    /// Current jitter value (RFC 3550 interarrival jitter)
    jitter: f64,

    /// Last packet arrival time
    last_arrival: Option<Instant>,

    /// Last RTP timestamp
    last_timestamp: Option<RtpTimestamp>,

    /// Highest extended sequence number used as a timing reference.
    last_sequence: Option<u64>,

    /// Clock rate for timestamp conversion
    clock_rate: u32,

    /// Maximum jitter seen
    max_jitter: f64,

    /// Minimum jitter seen
    min_jitter: f64,

    /// Number of samples in the jitter calculation
    samples: u64,

    /// Average jitter (accumulated)
    avg_jitter: f64,
}

impl JitterEstimator {
    /// Create a new jitter estimator
    pub fn new(clock_rate: u32) -> Self {
        Self {
            jitter: 0.0,
            last_arrival: None,
            last_timestamp: None,
            last_sequence: None,
            clock_rate,
            max_jitter: 0.0,
            min_jitter: f64::MAX,
            samples: 0,
            avg_jitter: 0.0,
        }
    }

    /// Update the jitter estimate with a new packet
    pub fn update(&mut self, timestamp: RtpTimestamp, arrival: Instant) -> f64 {
        if let (Some(last_arrival), Some(last_timestamp)) = (self.last_arrival, self.last_timestamp)
        {
            // Calculate transit time difference as described in RFC 3550
            // D(i,j) = (Rj - Ri) - (Sj - Si) = (Rj - Sj) - (Ri - Si)

            // Convert arrival timestamps to seconds
            let arrival_delta = arrival.duration_since(last_arrival).as_secs_f64();

            // Convert RTP timestamps to seconds
            let ts_delta = timestamp_delta(last_timestamp, timestamp, self.clock_rate);

            // Difference between arrival and timestamp deltas
            let transit_delta = arrival_delta - ts_delta;

            // RFC 3550 jitter calculation:
            // J(i) = J(i-1) + (|D(i-1,i)| - J(i-1))/16
            self.jitter += (transit_delta.abs() - self.jitter) / 16.0;

            // Update stats
            self.max_jitter = self.max_jitter.max(self.jitter);
            self.min_jitter = self.min_jitter.min(self.jitter);
            self.samples += 1;
            self.avg_jitter += (self.jitter - self.avg_jitter) / (self.samples as f64);
        }

        // Update for next calculation
        self.last_arrival = Some(arrival);
        self.last_timestamp = Some(timestamp);

        self.jitter
    }

    /// Update the jitter estimate using an RTP sequence number.
    ///
    /// Duplicated, late, and reordered packets do not advance the timing
    /// reference. This prevents their later arrival time from corrupting the
    /// RFC 3550 transit-time calculation for the next in-order packet.
    pub fn update_with_sequence(
        &mut self,
        sequence: RtpSequenceNumber,
        timestamp: RtpTimestamp,
        arrival: Instant,
    ) -> f64 {
        if let Some(last_sequence) = self.last_sequence {
            let Some(extended_sequence) = extend_sequence_near(sequence, last_sequence) else {
                return self.jitter;
            };
            if extended_sequence <= last_sequence {
                return self.jitter;
            }
            self.last_sequence = Some(extended_sequence);
        } else {
            self.last_sequence = Some(sequence as u64);
        }

        self.update(timestamp, arrival)
    }

    /// Get the current jitter estimate in seconds
    pub fn get_jitter(&self) -> f64 {
        self.jitter
    }

    /// Get the current jitter estimate in milliseconds
    pub fn get_jitter_ms(&self) -> f64 {
        self.jitter * 1000.0
    }

    /// Get the maximum jitter seen in milliseconds
    pub fn get_max_jitter_ms(&self) -> f64 {
        self.max_jitter * 1000.0
    }

    /// Get the minimum jitter seen in milliseconds
    pub fn get_min_jitter_ms(&self) -> f64 {
        self.min_jitter * 1000.0
    }

    /// Get the average jitter in milliseconds
    pub fn get_avg_jitter_ms(&self) -> f64 {
        self.avg_jitter * 1000.0
    }

    /// Reset the jitter estimator
    pub fn reset(&mut self) {
        self.jitter = 0.0;
        self.last_arrival = None;
        self.last_timestamp = None;
        self.last_sequence = None;
        self.max_jitter = 0.0;
        self.min_jitter = f64::MAX;
        self.samples = 0;
        self.avg_jitter = 0.0;
    }
}

fn extend_sequence_near(sequence: RtpSequenceNumber, reference: u64) -> Option<u64> {
    let reference_low = (reference & 0xffff) as i32;
    let difference = sequence as i32 - reference_low;
    let cycle_base = reference & !0xffff;

    if difference < -0x8000 {
        Some(cycle_base + 0x1_0000 + sequence as u64)
    } else if difference > 0x8000 {
        cycle_base
            .checked_sub(0x1_0000)
            .map(|previous_cycle| previous_cycle + sequence as u64)
    } else {
        Some(cycle_base + sequence as u64)
    }
}

/// Calculate the difference between two RTP timestamps in seconds
fn timestamp_delta(ts1: RtpTimestamp, ts2: RtpTimestamp, clock_rate: u32) -> f64 {
    if clock_rate == 0 {
        return 0.0;
    }

    // Handle RTP timestamp wraparound
    let delta = if ts2 >= ts1 {
        ts2 - ts1
    } else {
        // Wraparound occurred
        (u32::MAX - ts1) + ts2 + 1
    };

    // Convert to seconds
    (delta as f64) / (clock_rate as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_timestamp_delta() {
        // Normal case
        let delta = timestamp_delta(1000, 2000, 8000);
        assert!((delta - 0.125).abs() < 0.000001); // 1000 samples at 8kHz = 125ms

        // Wraparound case
        let delta_wraparound = timestamp_delta(4294967295, 1000, 8000);
        assert!((delta_wraparound - 0.125125).abs() < 0.000001); // 1001 samples at 8kHz with wraparound

        // Zero clock rate
        assert_eq!(timestamp_delta(1000, 2000, 0), 0.0);
    }

    #[test]
    fn test_jitter_estimation() {
        let mut estimator = JitterEstimator::new(8000);

        // First packet - no jitter calculated yet
        let now = Instant::now();
        estimator.update(0, now);
        assert_eq!(estimator.get_jitter(), 0.0);

        // Second packet - perfect timing (no jitter)
        sleep(Duration::from_millis(20));
        let packet2_time = Instant::now();
        estimator.update(160, packet2_time); // 20ms = 160 samples at 8kHz
        assert!(estimator.get_jitter() < 0.001); // Very small jitter

        // Third packet - arriving much too early (introducing large jitter)
        sleep(Duration::from_millis(1)); // Only 1ms instead of 20ms
        let packet3_time = Instant::now();
        estimator.update(320, packet3_time); // 20ms = 160 samples at 8kHz

        // Fourth packet - arriving very late (large jitter)
        sleep(Duration::from_millis(60)); // 60ms instead of 20ms
        let packet4_time = Instant::now();
        estimator.update(480, packet4_time);

        // Fifth packet - arriving early again
        sleep(Duration::from_millis(1));
        let packet5_time = Instant::now();
        estimator.update(640, packet5_time);

        // With these extreme jitter patterns, the value should definitely be above 0.001
        assert!(
            estimator.get_jitter() > 0.001,
            "Jitter value is {} which is too small",
            estimator.get_jitter()
        );

        // Check stats
        assert!(estimator.get_max_jitter_ms() >= estimator.get_jitter_ms());
        assert!(estimator.get_min_jitter_ms() <= estimator.get_jitter_ms());
    }

    #[test]
    fn reordered_and_duplicate_packets_do_not_change_jitter_reference() {
        let mut estimator = JitterEstimator::new(8000);
        let start = Instant::now();

        estimator.update_with_sequence(10, 0, start);
        estimator.update_with_sequence(12, 320, start + Duration::from_millis(40));
        let before_late_packet = estimator.get_jitter();

        // This missing packet arrives much later. Including it would create a
        // multi-second jitter sample and poison the next calculation.
        estimator.update_with_sequence(11, 160, start + Duration::from_secs(5));
        estimator.update_with_sequence(12, 320, start + Duration::from_secs(6));
        assert_eq!(estimator.get_jitter(), before_late_packet);

        estimator.update_with_sequence(13, 480, start + Duration::from_millis(60));
        assert!(estimator.get_jitter() < 0.000_001);
    }

    #[test]
    fn sequence_aware_jitter_handles_wraparound() {
        let mut estimator = JitterEstimator::new(8000);
        let start = Instant::now();

        estimator.update_with_sequence(65535, 0, start);
        estimator.update_with_sequence(0, 160, start + Duration::from_millis(20));

        assert!(estimator.get_jitter() < 0.000_001);
        assert_eq!(estimator.last_sequence, Some(0x1_0000));
    }

    #[test]
    fn late_packet_before_cycle_zero_does_not_change_jitter() {
        let mut estimator = JitterEstimator::new(8000);
        let start = Instant::now();
        estimator.update_with_sequence(1, 160, start);

        let before = estimator.get_jitter();
        estimator.update_with_sequence(65535, 0, start + Duration::from_secs(5));
        assert_eq!(estimator.get_jitter(), before);
        assert_eq!(estimator.last_sequence, Some(1));
    }
}

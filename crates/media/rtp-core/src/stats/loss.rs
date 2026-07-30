use std::collections::HashSet;

use crate::RtpSequenceNumber;

/// Number of recent extended sequence numbers retained for duplicate and
/// reordering detection.
const RECEIVED_WINDOW_SIZE: i64 = 2048;

/// Packet loss tracker for RTP streams
#[derive(Debug, Clone)]
pub struct PacketLossTracker {
    /// Base sequence number (first received)
    base_seq: Option<RtpSequenceNumber>,

    /// Extended sequence number of the first packet.
    base_extended: Option<i64>,

    /// Highest extended sequence number received.
    highest_seq: i64,

    /// Number of packets actually received
    received: u64,

    /// Number of distinct packets received in the tracked sequence range.
    unique_received: u64,

    /// Expected-packet total captured when the preceding RTCP report was built.
    expected_prior: u64,

    /// Unique-received total captured when the preceding RTCP report was built.
    unique_received_prior: u64,

    /// Number of packets lost
    lost: u64,

    /// Number of duplicate packets
    duplicates: u64,

    /// Number of reordered packets
    reordered: u64,

    /// Sequence number cycle count of the highest packet.
    cycles: u32,

    /// Recently received extended sequence numbers.
    received_window: HashSet<i64>,

    /// Recent loss history (1=received, 0=lost) for burst detection
    loss_history: Vec<bool>,

    /// Size of the loss history window
    history_size: usize,

    /// Number of loss bursts detected
    burst_count: u64,

    /// Maximum burst length
    max_burst_length: u64,

    /// Current burst length
    current_burst_length: u64,
}

impl PacketLossTracker {
    /// Create a new packet loss tracker
    pub fn new() -> Self {
        Self {
            base_seq: None,
            base_extended: None,
            highest_seq: 0,
            received: 0,
            unique_received: 0,
            expected_prior: 0,
            unique_received_prior: 0,
            lost: 0,
            duplicates: 0,
            reordered: 0,
            cycles: 0,
            received_window: HashSet::with_capacity(RECEIVED_WINDOW_SIZE as usize),
            loss_history: Vec::with_capacity(64),
            history_size: 64,
            burst_count: 0,
            max_burst_length: 0,
            current_burst_length: 0,
        }
    }

    /// Process a packet with the given sequence number
    pub fn process(&mut self, seq: RtpSequenceNumber) -> PacketLossResult {
        self.received += 1;

        // Initialize if this is the first packet
        if self.base_seq.is_none() {
            self.base_seq = Some(seq);
            self.base_extended = Some(seq as i64);
            self.highest_seq = seq as i64;
            self.unique_received = 1;
            self.received_window.insert(seq as i64);
            self.loss_history.push(true); // First packet is received
            return PacketLossResult::FirstPacket { seq };
        }

        let extended_seq = self.extend_sequence(seq);
        if self.received_window.contains(&extended_seq) {
            self.duplicates += 1;
            return PacketLossResult::Duplicate { seq };
        }

        let highest_seq = self.highest_seq;
        if extended_seq < highest_seq {
            self.reordered += 1;

            // Sequence numbers older than the bounded window cannot safely be
            // distinguished from duplicates. Count them as reordered, but do
            // not change the established loss total.
            let oldest_tracked = highest_seq - (RECEIVED_WINDOW_SIZE - 1);
            if extended_seq >= oldest_tracked {
                self.received_window.insert(extended_seq);
                if extended_seq >= self.base_extended.expect("tracker is initialized") {
                    self.unique_received += 1;
                    self.lost = self.lost.saturating_sub(1);
                }
                self.add_to_history(true);
            }

            return PacketLossResult::Reordered {
                seq,
                expected: (highest_seq & 0xFFFF) as u16,
            };
        }

        if extended_seq > highest_seq {
            let gap = (extended_seq - highest_seq) as u64;
            self.unique_received += 1;
            self.received_window.insert(extended_seq);

            if gap > 1 {
                let lost_packets = gap - 1;
                self.lost += lost_packets;

                self.update_burst_stats(lost_packets);

                // The history is bounded; avoid work proportional to a
                // maliciously large sequence jump.
                for _ in 0..lost_packets.min(self.history_size as u64) {
                    self.add_to_history(false);
                }

                self.add_to_history(true);
                self.advance_highest(extended_seq);

                return PacketLossResult::Gap {
                    seq,
                    expected: (highest_seq + 1) as u16,
                    lost: lost_packets.min(u16::MAX as u64) as u16,
                };
            } else {
                self.add_to_history(true);
                self.advance_highest(extended_seq);

                return PacketLossResult::Sequential { seq };
            }
        }

        // Equality is handled by the received window above.
        PacketLossResult::Unknown
    }

    /// Return the extended highest sequence number used in RTCP reports.
    pub fn highest_extended_sequence(&self) -> u32 {
        self.highest_seq.clamp(0, u32::MAX as i64) as u32
    }

    /// Calculate the total number of expected packets
    pub fn calculate_expected(&self) -> u64 {
        if let Some(base_extended) = self.base_extended {
            (self.highest_seq - base_extended + 1).max(0) as u64
        } else {
            0
        }
    }

    /// Get the fraction of packets lost (0-255 scale)
    pub fn get_fraction_lost(&self) -> u8 {
        let expected = self.calculate_expected();
        if expected == 0 {
            return 0;
        }

        let lost = expected.saturating_sub(self.unique_received);

        let fraction = (lost as f64 / expected as f64) * 256.0;
        fraction.min(255.0) as u8
    }

    /// Return the packet-loss fraction for the interval since the preceding
    /// RTCP report and advance the report snapshot.
    ///
    /// RFC 3550 defines the fraction field over the reporting interval, not
    /// over the lifetime of the source. Late packets can make the interval's
    /// received delta exceed its expected delta; that recovery is encoded as
    /// zero rather than wrapping into a large loss fraction.
    pub fn take_interval_fraction_lost(&mut self) -> u8 {
        let expected = self.calculate_expected();
        let expected_interval = expected.saturating_sub(self.expected_prior);
        let received_interval = self
            .unique_received
            .saturating_sub(self.unique_received_prior);

        self.expected_prior = expected;
        self.unique_received_prior = self.unique_received;

        if expected_interval == 0 || received_interval >= expected_interval {
            return 0;
        }

        let lost_interval = expected_interval - received_interval;
        ((lost_interval * 256) / expected_interval).min(255) as u8
    }

    /// Calculate the cumulative number of packets lost
    pub fn get_cumulative_lost(&self) -> u32 {
        let expected = self.calculate_expected();
        let calculated = expected.saturating_sub(self.unique_received);
        debug_assert_eq!(self.lost, calculated);

        calculated.min(0x00ff_ffff) as u32
    }

    /// Get packet loss statistics
    pub fn get_stats(&self) -> PacketLossStats {
        let expected = self.calculate_expected();

        PacketLossStats {
            packets_received: self.received,
            packets_lost: self.get_cumulative_lost() as u64,
            packets_expected: expected,
            duplicates: self.duplicates,
            reordered: self.reordered,
            fraction_lost: self.get_fraction_lost(),
            burst_count: self.burst_count,
            max_burst_length: self.max_burst_length,
        }
    }

    /// Reset the tracker
    pub fn reset(&mut self) {
        self.base_seq = None;
        self.base_extended = None;
        self.highest_seq = 0;
        self.received = 0;
        self.unique_received = 0;
        self.expected_prior = 0;
        self.unique_received_prior = 0;
        self.lost = 0;
        self.duplicates = 0;
        self.reordered = 0;
        self.cycles = 0;
        self.received_window.clear();
        self.loss_history.clear();
        self.burst_count = 0;
        self.max_burst_length = 0;
        self.current_burst_length = 0;
    }

    // Internal helper methods

    /// Map a 16-bit sequence number to the cycle nearest the current highest
    /// sequence number. This permits late packets from the preceding cycle
    /// without mistaking them for a new wrap.
    fn extend_sequence(&self, seq: RtpSequenceNumber) -> i64 {
        let highest_low = (self.highest_seq & 0xffff) as i32;
        let difference = seq as i32 - highest_low;
        let cycle_base = self.highest_seq & !0xffff;

        if difference < -0x8000 {
            cycle_base + 0x1_0000 + seq as i64
        } else if difference > 0x8000 {
            cycle_base - 0x1_0000 + seq as i64
        } else {
            cycle_base + seq as i64
        }
    }

    fn advance_highest(&mut self, extended_seq: i64) {
        self.highest_seq = extended_seq;
        self.cycles = (extended_seq >> 16).clamp(0, u32::MAX as i64) as u32;

        let oldest_tracked = extended_seq - (RECEIVED_WINDOW_SIZE - 1);
        self.received_window
            .retain(|received| *received >= oldest_tracked);
    }

    /// Add a packet status to the loss history
    fn add_to_history(&mut self, received: bool) {
        if self.loss_history.len() >= self.history_size {
            self.loss_history.remove(0);
        }
        self.loss_history.push(received);
    }

    /// Update burst statistics when packets are lost
    fn update_burst_stats(&mut self, lost_count: u64) {
        if lost_count == 0 {
            // Reset current burst if any
            if self.current_burst_length > 0 {
                self.current_burst_length = 0;
            }
            return;
        }

        // Each newly observed gap counts as one burst.
        self.burst_count += 1;
        self.current_burst_length = lost_count;

        // Update max burst length
        if self.current_burst_length > self.max_burst_length {
            self.max_burst_length = self.current_burst_length;
        }
    }
}

/// Result of processing a packet
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PacketLossResult {
    /// First packet in the stream
    FirstPacket { seq: RtpSequenceNumber },

    /// Packet arrived in sequence
    Sequential { seq: RtpSequenceNumber },

    /// Gap in sequence numbers (packet loss)
    Gap {
        seq: RtpSequenceNumber,
        expected: RtpSequenceNumber,
        lost: u16,
    },

    /// Duplicate packet
    Duplicate { seq: RtpSequenceNumber },

    /// Reordered packet (arrived after a higher sequence number)
    Reordered {
        seq: RtpSequenceNumber,
        expected: RtpSequenceNumber,
    },

    /// Unknown situation
    Unknown,
}

/// Statistics about packet loss
#[derive(Debug, Clone)]
pub struct PacketLossStats {
    /// Number of packets received
    pub packets_received: u64,

    /// Number of packets lost
    pub packets_lost: u64,

    /// Number of packets expected
    pub packets_expected: u64,

    /// Number of duplicate packets
    pub duplicates: u64,

    /// Number of reordered packets
    pub reordered: u64,

    /// Fraction of packets lost (0-255 scale)
    pub fraction_lost: u8,

    /// Number of loss bursts
    pub burst_count: u64,

    /// Maximum burst length
    pub max_burst_length: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sequential_packets() {
        let mut tracker = PacketLossTracker::new();

        // Process sequential packets
        assert_eq!(
            tracker.process(1000),
            PacketLossResult::FirstPacket { seq: 1000 }
        );
        assert_eq!(
            tracker.process(1001),
            PacketLossResult::Sequential { seq: 1001 }
        );
        assert_eq!(
            tracker.process(1002),
            PacketLossResult::Sequential { seq: 1002 }
        );

        // Check stats
        let stats = tracker.get_stats();
        assert_eq!(stats.packets_received, 3);
        assert_eq!(stats.packets_lost, 0);
        assert_eq!(stats.packets_expected, 3);
        assert_eq!(stats.duplicates, 0);
        assert_eq!(stats.fraction_lost, 0);
    }

    #[test]
    fn test_packet_loss() {
        let mut tracker = PacketLossTracker::new();

        // Process packets with gap
        assert_eq!(
            tracker.process(1000),
            PacketLossResult::FirstPacket { seq: 1000 }
        );
        assert_eq!(
            tracker.process(1001),
            PacketLossResult::Sequential { seq: 1001 }
        );

        // Gap of 2 packets (1002 and 1003 missing)
        assert_eq!(
            tracker.process(1004),
            PacketLossResult::Gap {
                seq: 1004,
                expected: 1002,
                lost: 2
            }
        );

        // Check stats
        let stats = tracker.get_stats();
        assert_eq!(stats.packets_received, 3);
        assert_eq!(stats.packets_lost, 2);
        assert_eq!(stats.packets_expected, 5);
        assert_eq!(stats.duplicates, 0);

        // Fraction lost should be about 40% (2/5 = 0.4 * 256 = ~102)
        assert!(stats.fraction_lost >= 100 && stats.fraction_lost <= 105);
    }

    #[test]
    fn test_duplicate_packets() {
        let mut tracker = PacketLossTracker::new();

        // Initialize the tracker with some packets
        assert_eq!(
            tracker.process(1000),
            PacketLossResult::FirstPacket { seq: 1000 }
        );
        assert_eq!(
            tracker.process(1001),
            PacketLossResult::Sequential { seq: 1001 }
        );

        // Every sequence number in the bounded received window is recognized
        // as a duplicate, not only the most recently received one.
        let result1 = tracker.process(1000);
        assert_eq!(
            result1,
            PacketLossResult::Duplicate { seq: 1000 },
            "Expected Duplicate but got {:?}",
            result1
        );

        let result2 = tracker.process(1001);
        assert_eq!(
            result2,
            PacketLossResult::Duplicate { seq: 1001 },
            "Expected Duplicate but got {:?}",
            result2
        );

        // Check stats
        let stats = tracker.get_stats();
        assert_eq!(stats.packets_received, 4); // 2 unique + 2 more
        assert_eq!(stats.duplicates, 2);
        assert_eq!(stats.reordered, 0);
        assert_eq!(stats.packets_expected, 2); // Only expect 2 unique packets
    }

    #[test]
    fn test_reordered_packets() {
        let mut tracker = PacketLossTracker::new();

        // Process packets with reordering
        assert_eq!(
            tracker.process(1000),
            PacketLossResult::FirstPacket { seq: 1000 }
        );
        assert_eq!(
            tracker.process(1002),
            PacketLossResult::Gap {
                seq: 1002,
                expected: 1001,
                lost: 1
            }
        );
        assert_eq!(
            tracker.process(1001),
            PacketLossResult::Reordered {
                seq: 1001,
                expected: 1002
            }
        );

        // Check stats
        let stats = tracker.get_stats();
        assert_eq!(stats.packets_received, 3);
        assert_eq!(stats.reordered, 1);
        assert_eq!(stats.packets_lost, 0);
        assert_eq!(tracker.get_cumulative_lost(), 0);
    }

    #[test]
    fn packet_before_base_does_not_hide_later_loss() {
        let mut tracker = PacketLossTracker::new();

        tracker.process(1000);
        assert!(matches!(
            tracker.process(999),
            PacketLossResult::Reordered { .. }
        ));
        assert_eq!(
            tracker.process(1002),
            PacketLossResult::Gap {
                seq: 1002,
                expected: 1001,
                lost: 1,
            }
        );
        assert_eq!(tracker.get_cumulative_lost(), 1);
    }

    #[test]
    fn high_late_packet_in_cycle_zero_is_not_a_future_jump() {
        let mut tracker = PacketLossTracker::new();
        tracker.process(1);

        assert!(matches!(
            tracker.process(65535),
            PacketLossResult::Reordered { .. }
        ));
        assert_eq!(tracker.highest_extended_sequence(), 1);
        assert_eq!(tracker.get_cumulative_lost(), 0);
    }

    #[test]
    fn late_packet_before_sequence_zero_is_then_a_duplicate() {
        let mut tracker = PacketLossTracker::new();
        tracker.process(0);

        assert_eq!(
            tracker.process(65535),
            PacketLossResult::Reordered {
                seq: 65535,
                expected: 0,
            }
        );
        assert_eq!(
            tracker.process(65535),
            PacketLossResult::Duplicate { seq: 65535 }
        );
        assert_eq!(tracker.highest_extended_sequence(), 0);
        assert_eq!(tracker.get_stats().duplicates, 1);
    }

    #[test]
    fn rtcp_fraction_lost_is_scoped_to_each_report_interval() {
        let mut tracker = PacketLossTracker::new();
        tracker.process(10);
        tracker.process(12);

        assert_eq!(tracker.take_interval_fraction_lost(), 85);
        assert_eq!(tracker.get_cumulative_lost(), 1);

        for sequence in 13..=20 {
            tracker.process(sequence);
        }

        assert_eq!(tracker.take_interval_fraction_lost(), 0);
        assert_eq!(tracker.get_cumulative_lost(), 1);
    }

    #[test]
    fn test_sequence_wraparound() {
        let mut tracker = PacketLossTracker::new();

        // Process packets with sequence number wraparound
        assert_eq!(
            tracker.process(65533),
            PacketLossResult::FirstPacket { seq: 65533 }
        );
        assert_eq!(
            tracker.process(65534),
            PacketLossResult::Sequential { seq: 65534 }
        );
        assert_eq!(
            tracker.process(65535),
            PacketLossResult::Sequential { seq: 65535 }
        );
        assert_eq!(tracker.process(0), PacketLossResult::Sequential { seq: 0 });
        assert_eq!(tracker.process(1), PacketLossResult::Sequential { seq: 1 });

        // Check stats
        let stats = tracker.get_stats();
        assert_eq!(stats.packets_received, 5);
        assert_eq!(stats.packets_expected, 5);
        assert_eq!(stats.packets_lost, 0);

        // Check cycle count
        assert_eq!(tracker.cycles, 1);
        assert_eq!(tracker.highest_extended_sequence(), 0x1_0001);
    }

    #[test]
    fn test_reordering_around_sequence_wrap_fills_loss() {
        let mut tracker = PacketLossTracker::new();

        assert_eq!(
            tracker.process(65534),
            PacketLossResult::FirstPacket { seq: 65534 }
        );
        assert_eq!(
            tracker.process(0),
            PacketLossResult::Gap {
                seq: 0,
                expected: 65535,
                lost: 1,
            }
        );
        assert_eq!(tracker.get_cumulative_lost(), 1);

        assert_eq!(
            tracker.process(65535),
            PacketLossResult::Reordered {
                seq: 65535,
                expected: 0,
            }
        );
        assert_eq!(tracker.get_cumulative_lost(), 0);
        assert_eq!(
            tracker.process(65535),
            PacketLossResult::Duplicate { seq: 65535 }
        );
    }

    #[test]
    fn test_burst_detection() {
        let mut tracker = PacketLossTracker::new();

        // Process with two bursts of losses
        tracker.process(1000);
        tracker.process(1001);
        // First burst (1002-1005 lost)
        tracker.process(1006);
        // Some good packets
        tracker.process(1007);
        tracker.process(1008);
        // Second burst (1009-1010 lost)
        tracker.process(1011);

        // Check stats
        let stats = tracker.get_stats();
        assert_eq!(stats.burst_count, 2);
        // The max burst length is from the first gap (4 packets)
        assert_eq!(stats.max_burst_length, 4);
    }
}

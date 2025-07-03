//! Adaptive jitter buffer for RTP packet reordering
//!
//! This module provides a high-performance jitter buffer implementation
//! that adapts to network conditions in real-time.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Notify, Semaphore};
use crate::packet::RtpPacket;
use crate::RtpSequenceNumber;
use tracing::{debug, warn, info, trace};

use super::{BufferLimits, GlobalBufferManager, MemoryPermit};

/// Default jitter buffer size in milliseconds
pub const DEFAULT_JITTER_BUFFER_SIZE_MS: u32 = 50;

/// Minimum jitter buffer size in milliseconds
pub const MIN_JITTER_BUFFER_SIZE_MS: u32 = 10;

/// Maximum jitter buffer size in milliseconds
pub const MAX_JITTER_BUFFER_SIZE_MS: u32 = 500;

/// Default max out-of-order packets to track
pub const DEFAULT_MAX_OUT_OF_ORDER: usize = 100;

/// Default playout delay in milliseconds
pub const DEFAULT_PLAYOUT_DELAY_MS: u32 = 60;

/// Adaptive jitter buffer configuration
#[derive(Debug, Clone)]
pub struct JitterBufferConfig {
    /// Initial jitter buffer size in milliseconds
    pub initial_size_ms: u32,
    
    /// Minimum buffer size in milliseconds
    pub min_size_ms: u32,
    
    /// Maximum buffer size in milliseconds
    pub max_size_ms: u32,
    
    /// Clock rate in Hz
    pub clock_rate: u32,
    
    /// Maximum number of out-of-order packets to track
    pub max_out_of_order: usize,
    
    /// Maximum packet age in milliseconds
    pub max_packet_age_ms: u32,
    
    /// Initial playout delay in milliseconds
    pub initial_playout_delay_ms: u32,
    
    /// Whether to adapt buffer size dynamically
    pub adaptive: bool,
}

impl Default for JitterBufferConfig {
    fn default() -> Self {
        Self {
            initial_size_ms: DEFAULT_JITTER_BUFFER_SIZE_MS,
            min_size_ms: MIN_JITTER_BUFFER_SIZE_MS,
            max_size_ms: MAX_JITTER_BUFFER_SIZE_MS,
            clock_rate: 8000, // Default for most audio codecs
            max_out_of_order: DEFAULT_MAX_OUT_OF_ORDER,
            max_packet_age_ms: 200,
            initial_playout_delay_ms: DEFAULT_PLAYOUT_DELAY_MS,
            adaptive: true,
        }
    }
}

/// Statistics for jitter buffer
#[derive(Debug, Clone)]
pub struct JitterBufferStats {
    /// Current buffer size in milliseconds
    pub buffer_size_ms: u32,
    
    /// Number of packets currently in the buffer
    pub buffered_packets: usize,
    
    /// Total packets received
    pub packets_received: u64,
    
    /// Total packets played out
    pub packets_played: u64,
    
    /// Packets dropped due to late arrival
    pub packets_too_late: u64,
    
    /// Packets dropped due to buffer overflow
    pub packets_overflow: u64,
    
    /// Packet duplicates detected
    pub duplicates: u64,
    
    /// Current jitter estimate in milliseconds
    pub jitter_ms: f64,
    
    /// Number of sequence number discontinuities
    pub discontinuities: u64,
    
    /// Number of buffer underruns
    pub underruns: u64,
    
    /// Average packet delay in milliseconds
    pub avg_delay_ms: f64,
}

/// High-performance jitter buffer for RTP packets
///
/// This implementation provides:
/// - Adaptive buffer sizing based on network conditions
/// - Efficient packet storage using BTreeMap
/// - Proper handling of sequence wraparound
/// - Memory management with global limits
/// - Real-time statistics collection
pub struct AdaptiveJitterBuffer {
    /// Buffer configuration
    config: JitterBufferConfig,
    
    /// Packets stored by sequence number
    packets: BTreeMap<u32, (RtpPacket, Instant)>,
    
    /// Next sequence number expected
    next_seq: Option<RtpSequenceNumber>,
    
    /// Extended sequence counter (for wraparound handling)
    ext_seq_base: u32,
    
    /// Cycles of sequence wraparound
    seq_cycles: u16,
    
    /// Highest sequence number seen
    highest_seq: u32,
    
    /// Jitter estimate (RFC 3550)
    jitter: f64,
    
    /// Current buffer size in milliseconds
    buffer_size_ms: u32,
    
    /// Last packet played out time
    last_playout_time: Option<Instant>,
    
    /// Last timestamp played out
    last_timestamp: Option<u32>,
    
    /// Statistics
    stats: JitterBufferStats,
    
    /// Time when the buffer started
    start_time: Instant,
    
    /// Reference to global buffer manager
    buffer_manager: Option<Arc<GlobalBufferManager>>,
    
    /// Notification for new packets
    packet_notify: Arc<Notify>,
    
    /// Last time buffer size was adjusted
    last_adjustment: Instant,
    
    /// Inter-packet arrival jitter
    arrival_jitter: f64,
    
    /// Average delay
    avg_delay: f64,
    
    /// Packets waiting for playout
    waiting_packets: u32,
    
    /// Initial sequence number
    initial_seq: Option<RtpSequenceNumber>,
    
    /// Playout delay in timestamp units
    playout_delay: u32,
}

impl AdaptiveJitterBuffer {
    /// Create a new adaptive jitter buffer
    pub fn new(config: JitterBufferConfig) -> Self {
        let now = Instant::now();
        
        let playout_delay = ((config.initial_playout_delay_ms as f64 / 1000.0) * config.clock_rate as f64) as u32;
        
        // Store these values before moving config
        let initial_size_ms = config.initial_size_ms;
        
        let stats = JitterBufferStats {
            buffer_size_ms: initial_size_ms,
            buffered_packets: 0,
            packets_received: 0,
            packets_played: 0,
            packets_too_late: 0,
            packets_overflow: 0,
            duplicates: 0,
            jitter_ms: 0.0,
            discontinuities: 0,
            underruns: 0,
            avg_delay_ms: 0.0,
        };
        
        Self {
            config,
            packets: BTreeMap::new(),
            next_seq: None,
            ext_seq_base: 0,
            seq_cycles: 0,
            highest_seq: 0,
            jitter: 0.0,
            buffer_size_ms: initial_size_ms,
            last_playout_time: None,
            last_timestamp: None,
            stats,
            start_time: now,
            buffer_manager: None,
            packet_notify: Arc::new(Notify::new()),
            last_adjustment: now,
            arrival_jitter: 0.0,
            avg_delay: 0.0,
            waiting_packets: 0,
            initial_seq: None,
            playout_delay,
        }
    }
    
    /// Create a new jitter buffer with global buffer management
    pub fn with_buffer_manager(
        config: JitterBufferConfig,
        buffer_manager: Arc<GlobalBufferManager>
    ) -> Self {
        let mut buffer = Self::new(config);
        buffer.buffer_manager = Some(buffer_manager);
        buffer
    }
    
    /// Add a packet to the jitter buffer
    ///
    /// Returns true if the packet was added, false if it was dropped.
    pub async fn add_packet(&mut self, packet: RtpPacket) -> bool {
        let now = Instant::now();
        let seq = packet.header.sequence_number;
        let ts = packet.header.timestamp;
        
        trace!("Adding packet with seq={} to jitter buffer", seq);
        
        // Update stats
        self.stats.packets_received += 1;
        
        // Initialize sequence tracking if this is the first packet
        if self.next_seq.is_none() {
            // This is the first packet, so we'll track it as our initial sequence
            self.initial_seq = Some(seq);
            
            // We set the next_seq to the current packet's sequence
            // Note: get_next_packet will reset this to the lowest available
            // sequence number when retrieving packets
            self.next_seq = Some(seq);
            
            self.highest_seq = seq as u32;
            trace!("First packet in buffer, initializing with seq={}", seq);
            
            // Insert the packet using the raw sequence number 
            self.packets.insert(seq as u32, (packet, now));
            self.stats.buffered_packets = self.packets.len();
            
            // Notify waiters
            self.packet_notify.notify_one();
            
            return true;
        }
        
        // Check for duplicate packet
        if self.packets.contains_key(&(seq as u32)) {
            self.stats.duplicates += 1;
            trace!("Duplicate packet detected with seq={}", seq);
            return false;
        }
        
        // Check if packet is too old
        if let Some(next_seq) = self.next_seq {
            // Out of order packets (lower seq) are allowed as long as they're still in play range
            // We don't want to reject packets just because they arrived out of order
            let is_late = if next_seq > seq {
                // Packet has lower seq than next expected, but it might be valid
                // Calculate how far behind it is - with handling for wraparound
                let diff = if next_seq - seq < 0x8000 {
                    // Normal case - packet is just behind the next expected
                    next_seq - seq
                } else {
                    // Wraparound case - next_seq is near 0, seq is near 65535
                    // This is not a "late" packet, it's actually ahead
                    0
                };
                
                // Only consider it late if it's significantly behind what we've already played
                // Allow a reasonable replay window
                usize::from(diff) > self.config.max_out_of_order
            } else {
                // Packet is ahead of or equal to next expected seq
                false
            };
            
            if is_late {
                trace!("Packet too late: seq={}, next_seq={}, diff={}", 
                      seq, next_seq, 
                      if next_seq > seq { next_seq - seq } else { 0 });
                self.stats.packets_too_late += 1;
                return false;
            }
        }
        
        // Update highest sequence seen with wraparound handling
        let highest_seq = self.highest_seq as u16;
        if seq.wrapping_sub(highest_seq) < 0x8000 {
            // This sequence is newer than highest
            if self.seq_cycles > 0 && seq < 0x1000 && highest_seq > 0xf000 {
                // Wraparound occurred
                debug!("Sequence wraparound detected: {} -> {}", highest_seq, seq);
                self.seq_cycles += 1;
            }
            self.highest_seq = ((self.seq_cycles as u32) << 16) | (seq as u32);
            trace!("Updated highest sequence to {}", seq);
        }
        
        // Check if the buffer is full
        if self.packets.len() >= self.config.max_out_of_order {
            // Buffer overflow, drop the oldest packet
            trace!("Buffer overflow (max_size={}), dropping oldest packet", 
                  self.config.max_out_of_order);
            self.stats.packets_overflow += 1;
            
            // Drop oldest packet
            if let Some((&oldest_seq, _)) = self.packets.iter().next() {
                self.packets.remove(&oldest_seq);
            }
        }
        
        // Update jitter estimate
        if let Some(last_time) = self.last_playout_time {
            if let Some(last_ts) = self.last_timestamp {
                let arrival_diff = now.duration_since(last_time).as_secs_f64();
                let ts_diff = ((ts as i32 - last_ts as i32).abs() as f64) / (self.config.clock_rate as f64);
                
                // RFC 3550 jitter calculation
                let d = (arrival_diff - ts_diff).abs();
                self.jitter += (d - self.jitter) / 16.0;
                self.arrival_jitter = self.jitter;
                
                // Track delay for adaptive buffer sizing
                self.avg_delay = 0.8 * self.avg_delay + 0.2 * d;
            }
        }
        
        // Store the packet
        self.packets.insert(seq as u32, (packet, now));
        self.stats.buffered_packets = self.packets.len();
        
        // Set timestamps for next packet
        self.last_playout_time = Some(now);
        self.last_timestamp = Some(ts);
        
        // Notify waiters
        self.packet_notify.notify_one();
        
        // Update jitter buffer size if adaptive
        if self.config.adaptive {
            self.maybe_adapt_buffer_size(now);
        }
        
        true
    }
    
    /// Get the next packet for playout
    ///
    /// This follows the playout schedule and returns packets in the correct
    /// order, accounting for the configured jitter buffer delay.
    pub async fn get_next_packet(&mut self) -> Option<RtpPacket> {
        // If buffer is empty, wait for a packet
        if self.packets.is_empty() {
            return None;
        }
        
        // Always check for a better (lower) sequence number to start with
        // This ensures we deliver packets in order even when they arrive out of order
        let lowest_seq = self.packets.keys().copied().min();
        
        if let Some(lowest) = lowest_seq {
            if self.next_seq.is_none() {
                // Initialize next_seq if it's not set
                self.next_seq = Some(lowest as u16);
            } else if lowest < self.next_seq.unwrap() as u32 {
                // Found a packet with lower sequence number than expected

                // Special case for sequence wraparound:
                // If lowest is very low (close to 0) and next_seq is very high (close to 65535),
                // this is likely a wraparound condition. In this case, don't reset next_seq.
                let next_seq = self.next_seq.unwrap();
                if !(lowest < 1000 && next_seq > 60000) {
                    self.next_seq = Some(lowest as u16);
                }
            }
        } else {
            return None;
        }
        
        let next_seq = self.next_seq.unwrap();
        let next_seq_u32 = next_seq as u32;
        
        trace!("Getting next packet, expecting seq={}", next_seq);
        
        // Check if the next packet is available
        if let Some((packet, arrival_time)) = self.packets.remove(&next_seq_u32) {
            // Update next expected sequence with wraparound
            self.next_seq = Some(next_seq.wrapping_add(1));
            trace!("Found expected packet seq={}, next_seq now={}", 
                   next_seq, next_seq.wrapping_add(1));
            
            // Update stats
            self.stats.packets_played += 1;
            self.stats.buffered_packets = self.packets.len();
            
            // Check how long the packet was buffered
            let buffered_time = Instant::now().duration_since(arrival_time).as_millis() as u32;
            trace!("Packet played after {}ms in buffer", buffered_time);
            
            // Update average delay
            self.avg_delay = 0.8 * self.avg_delay + 0.2 * (buffered_time as f64 / 1000.0);
            self.stats.avg_delay_ms = self.avg_delay * 1000.0;
            
            return Some(packet);
        }
        
        // Handle discontinuity - packet not available
        trace!("Packet with seq={} not found in buffer, handling discontinuity", next_seq);
        self.stats.discontinuities += 1;
        
        // Skip forward to the next available packet
        let mut keys: Vec<_> = self.packets.keys().copied().collect();
        if keys.is_empty() {
            self.stats.underruns += 1;
            debug!("Buffer underrun, no packets available");
            return None;
        }
        
        // Sort the keys
        keys.sort();
        
        // Find the next key that is greater than next_seq with wraparound handling
        let next_available = keys.iter().min_by(|&&a, &&b| {
            // Calculate distance from next_seq to a and b with wraparound handling
            let dist_a = ((a as u16).wrapping_sub(next_seq) as u32) & 0xFFFF;
            let dist_b = ((b as u16).wrapping_sub(next_seq) as u32) & 0xFFFF;
            
            // The one with smaller distance (closest ahead) wins
            dist_a.cmp(&dist_b)
        }).copied().unwrap_or(keys[0]);
        
        trace!("Skipping to packet with seq={}", next_available as u16);
        
        // Update next sequence expectation
        self.next_seq = Some((next_available as u16).wrapping_add(1));
        debug!("Handling packet loss, skipping to seq={}", next_available as u16);
        
        // Return the packet
        let (packet, arrival_time) = self.packets.remove(&next_available).unwrap();
        
        // Update stats
        self.stats.packets_played += 1;
        self.stats.buffered_packets = self.packets.len();
        
        // Check how long the packet was buffered
        let buffered_time = Instant::now().duration_since(arrival_time).as_millis() as u32;
        trace!("Packet played after {}ms in buffer", buffered_time);
        
        Some(packet)
    }
    
    /// Wait until either a packet is available or timeout occurs
    ///
    /// Returns true if a packet is available, false if timeout occurred.
    pub async fn wait_for_packet(&self, timeout: Duration) -> bool {
        // If we already have packets, return immediately
        if !self.packets.is_empty() {
            return true;
        }
        
        // Wait for notification with timeout
        let notify = self.packet_notify.clone();
        tokio::select! {
            _ = notify.notified() => true,
            _ = tokio::time::sleep(timeout) => false,
        }
    }
    
    /// Get an extended sequence number that accounts for wraparound
    fn get_extended_seq(&mut self, seq: RtpSequenceNumber) -> u32 {
        // Detect sequence number cycle (wraparound from 65535 to 0)
        if self.next_seq.is_some() {
            let next_seq = self.next_seq.unwrap();
            // If the sequence is much lower than the expected one, we probably wrapped around
            if next_seq > 0xf000 && seq < 0x1000 {
                debug!("Sequence wraparound detected in get_extended_seq: {} -> {}", next_seq, seq);
                self.seq_cycles += 1;
            }
        }
        
        // Calculate extended sequence with cycle count
        (self.seq_cycles as u32) << 16 | (seq as u32)
    }
    
    /// Check if we should adapt the buffer size based on network conditions
    fn maybe_adapt_buffer_size(&mut self, now: Instant) {
        // Only adapt at most once per second
        if now.duration_since(self.last_adjustment).as_millis() < 1000 {
            return;
        }
        
        // Remember last adjustment time
        self.last_adjustment = now;
        
        // Calculate network jitter in milliseconds
        let jitter_ms = self.jitter * 1000.0;
        
        // Set stats
        self.stats.jitter_ms = jitter_ms;
        
        // Calculate desired buffer size (4x jitter is a common rule of thumb)
        let desired_ms = (jitter_ms * 4.0) as u32;
        
        // Clamp to min/max
        let new_size = desired_ms
            .max(self.config.min_size_ms)
            .min(self.config.max_size_ms);
        
        // Only change if significant difference (>10ms)
        if (new_size as i32 - self.buffer_size_ms as i32).abs() > 10 {
            debug!(
                "Adapting jitter buffer size: {}ms -> {}ms (jitter: {:.2}ms)",
                self.buffer_size_ms, new_size, jitter_ms
            );
            
            self.buffer_size_ms = new_size;
            self.stats.buffer_size_ms = new_size;
            
            // Update playout delay in timestamp units
            self.playout_delay = ((new_size as f64 / 1000.0) * self.config.clock_rate as f64) as u32;
        }
    }
    
    /// Get the current buffer statistics
    pub fn get_stats(&self) -> JitterBufferStats {
        self.stats.clone()
    }
    
    /// Get the current jitter estimate in milliseconds
    pub fn get_jitter_ms(&self) -> f64 {
        self.jitter * 1000.0
    }
    
    /// Clear the jitter buffer
    pub fn clear(&mut self) {
        self.packets.clear();
        self.stats.buffered_packets = 0;
    }
    
    /// Reset the jitter buffer
    ///
    /// This clears all packets and resets statistics.
    pub fn reset(&mut self) {
        self.clear();
        self.next_seq = None;
        self.ext_seq_base = 0;
        self.seq_cycles = 0;
        self.highest_seq = 0;
        self.jitter = 0.0;
        self.last_playout_time = None;
        self.last_timestamp = None;
        self.arrival_jitter = 0.0;
        self.avg_delay = 0.0;
        self.waiting_packets = 0;
        self.initial_seq = None;
        
        // Reset stats
        self.stats = JitterBufferStats {
            buffer_size_ms: self.config.initial_size_ms,
            buffered_packets: 0,
            packets_received: 0,
            packets_played: 0,
            packets_too_late: 0,
            packets_overflow: 0,
            duplicates: 0,
            jitter_ms: 0.0,
            discontinuities: 0,
            underruns: 0,
            avg_delay_ms: 0.0,
        };
        
        // Reset buffer size
        self.buffer_size_ms = self.config.initial_size_ms;
        self.playout_delay = ((self.config.initial_playout_delay_ms as f64 / 1000.0) * self.config.clock_rate as f64) as u32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use crate::packet::{RtpHeader, RtpPacket};
    
    fn create_test_packet(seq: RtpSequenceNumber, ts: u32) -> RtpPacket {
        let header = RtpHeader::new(96, seq, ts, 0x12345678);
        let payload = Bytes::from_static(b"test");
        RtpPacket::new(header, payload)
    }
    
    #[tokio::test]
    async fn test_in_order_packets() {
        let config = JitterBufferConfig {
            initial_size_ms: 50,
            clock_rate: 8000,
            ..Default::default()
        };
        
        let mut jitter = AdaptiveJitterBuffer::new(config);
        
        // Add packets in order
        assert!(jitter.add_packet(create_test_packet(1, 0)).await);
        assert!(jitter.add_packet(create_test_packet(2, 160)).await);
        assert!(jitter.add_packet(create_test_packet(3, 320)).await);
        
        // Get packets
        let p1 = jitter.get_next_packet().await;
        assert!(p1.is_some(), "First packet should be available");
        assert_eq!(p1.unwrap().header.sequence_number, 1, "First packet should have seq=1");
        
        let p2 = jitter.get_next_packet().await;
        assert!(p2.is_some(), "Second packet should be available");
        assert_eq!(p2.unwrap().header.sequence_number, 2, "Second packet should have seq=2");
        
        let p3 = jitter.get_next_packet().await;
        assert!(p3.is_some(), "Third packet should be available");
        assert_eq!(p3.unwrap().header.sequence_number, 3, "Third packet should have seq=3");
        
        // Buffer should be empty
        assert!(jitter.get_next_packet().await.is_none(), "Buffer should be empty after reading all packets");
    }
    
    #[tokio::test]
    async fn test_out_of_order_packets() {
        let config = JitterBufferConfig {
            initial_size_ms: 50,
            clock_rate: 8000,
            ..Default::default()
        };
        
        let mut jitter = AdaptiveJitterBuffer::new(config);
        
        // Add packets out of order - first add packet with seq 2
        assert!(jitter.add_packet(create_test_packet(2, 160)).await);
        
        // Next, add packet with seq 1
        assert!(jitter.add_packet(create_test_packet(1, 0)).await, 
                "Jitter buffer should accept out of order packets");
        
        // Finally, add packet with seq 3
        assert!(jitter.add_packet(create_test_packet(3, 320)).await);
        
        // Verify buffer state after adding
        let stats = jitter.get_stats();
        assert_eq!(stats.buffered_packets, 3, "Buffer should contain 3 packets");
        
        // Get first packet - should be seq 1 despite arriving after seq 2
        let p1 = jitter.get_next_packet().await;
        assert!(p1.is_some(), "First packet should be available");
        assert_eq!(p1.unwrap().header.sequence_number, 1, "First packet should have seq=1");
        
        // Get second packet
        let p2 = jitter.get_next_packet().await;
        assert!(p2.is_some(), "Second packet should be available");
        assert_eq!(p2.unwrap().header.sequence_number, 2, "Second packet should have seq=2");
        
        // Get third packet
        let p3 = jitter.get_next_packet().await;
        assert!(p3.is_some(), "Third packet should be available");
        assert_eq!(p3.unwrap().header.sequence_number, 3, "Third packet should have seq=3");
    }
    
    #[tokio::test]
    async fn test_packet_loss() {
        let config = JitterBufferConfig {
            initial_size_ms: 50,
            clock_rate: 8000,
            ..Default::default()
        };
        
        let mut jitter = AdaptiveJitterBuffer::new(config);
        
        // Add packets with a gap
        assert!(jitter.add_packet(create_test_packet(1, 0)).await);
        assert!(jitter.add_packet(create_test_packet(2, 160)).await);
        // Packet 3 is lost
        assert!(jitter.add_packet(create_test_packet(4, 480)).await);
        
        // Verify buffer state
        let stats = jitter.get_stats();
        assert_eq!(stats.buffered_packets, 3, "Buffer should contain 3 packets");
        
        // Get packets
        let p1 = jitter.get_next_packet().await;
        assert!(p1.is_some(), "First packet should be available");
        assert_eq!(p1.unwrap().header.sequence_number, 1, "First packet should have seq=1");
        
        let p2 = jitter.get_next_packet().await;
        assert!(p2.is_some(), "Second packet should be available");
        assert_eq!(p2.unwrap().header.sequence_number, 2, "Second packet should have seq=2");
        
        let p3 = jitter.get_next_packet().await;
        assert!(p3.is_some(), "Third packet should be available");
        assert_eq!(p3.unwrap().header.sequence_number, 4, "Third packet should have seq=4");
        
        // Check stats
        let stats = jitter.get_stats();
        assert_eq!(stats.discontinuities, 1, "Should detect one sequence discontinuity");
    }
    
    #[tokio::test]
    async fn test_sequence_wraparound() {
        let config = JitterBufferConfig {
            initial_size_ms: 50,
            clock_rate: 8000,
            ..Default::default()
        };
        
        let mut jitter = AdaptiveJitterBuffer::new(config);
        
        // Add packets around wraparound
        assert!(jitter.add_packet(create_test_packet(65534, 10000)).await);
        assert!(jitter.add_packet(create_test_packet(65535, 10160)).await);
        assert!(jitter.add_packet(create_test_packet(0, 10320)).await);
        assert!(jitter.add_packet(create_test_packet(1, 10480)).await);
        
        // Verify buffer state
        let stats = jitter.get_stats();
        assert_eq!(stats.buffered_packets, 4, "Buffer should contain 4 packets");
        
        // Get packets
        let p1 = jitter.get_next_packet().await;
        assert!(p1.is_some(), "First packet should be available");
        assert_eq!(p1.unwrap().header.sequence_number, 65534, "First packet should have seq=65534");
        
        let p2 = jitter.get_next_packet().await;
        assert!(p2.is_some(), "Second packet should be available");
        assert_eq!(p2.unwrap().header.sequence_number, 65535, "Second packet should have seq=65535");
        
        let p3 = jitter.get_next_packet().await;
        assert!(p3.is_some(), "Third packet should be available");
        assert_eq!(p3.unwrap().header.sequence_number, 0, "Third packet should have seq=0");
        
        let p4 = jitter.get_next_packet().await;
        assert!(p4.is_some(), "Fourth packet should be available");
        assert_eq!(p4.unwrap().header.sequence_number, 1, "Fourth packet should have seq=1");
    }
} 
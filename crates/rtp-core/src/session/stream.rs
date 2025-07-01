use bytes::Bytes;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{debug, warn};

use crate::error::Error;
use crate::packet::RtpPacket;
use crate::{Result, RtpSequenceNumber, RtpSsrc, RtpTimestamp};

/// Represents an RTP stream with sequence tracking and statistics
pub struct RtpStream {
    /// SSRC of the stream
    pub ssrc: RtpSsrc,
    
    /// Latest sequence number received
    latest_seq: RtpSequenceNumber,
    
    /// Highest sequence number received
    highest_seq: u32,
    
    /// Base sequence number (first received)
    base_seq: RtpSequenceNumber,
    
    /// Whether the stream has been initialized
    initialized: bool,
    
    /// Last time a packet was received
    last_packet_time: Instant,
    
    /// Number of packets received
    packets_received: u64,
    
    /// Number of bytes received
    bytes_received: u64,
    
    /// Number of packets lost (based on sequence gaps)
    packets_lost: u64,
    
    /// Number of duplicate packets
    duplicates: u64,
    
    /// Interarrival jitter estimate (RFC 3550)
    jitter: f64,
    
    /// Last arrival time used for jitter calculation
    last_arrival: Option<Instant>,
    
    /// Last RTP timestamp used for jitter calculation
    last_timestamp: Option<RtpTimestamp>,
    
    /// Clock rate for timestamp calculations
    clock_rate: u32,
    
    /// Jitter buffer for reordering packets (optional)
    jitter_buffer: Option<Arc<Mutex<VecDeque<RtpPacket>>>>,
    
    /// Maximum jitter buffer size in packets
    max_jitter_size: usize,
    
    /// Maximum packet age in the jitter buffer
    max_packet_age: Duration,
    
    /// Sequence cycle count (for wraparound handling)
    seq_cycles: u16,
    
    /// RTCP SR timestamp (middle 32 bits of NTP timestamp)
    last_sr_timestamp: Option<u32>,
    
    /// Time when the last SR was received
    last_sr_time: Option<Instant>,
}

impl RtpStream {
    /// Create a new RTP stream
    pub fn new(ssrc: RtpSsrc, clock_rate: u32) -> Self {
        Self {
            ssrc,
            latest_seq: 0,
            highest_seq: 0,
            base_seq: 0,
            initialized: false,
            last_packet_time: Instant::now(),
            packets_received: 0,
            bytes_received: 0,
            packets_lost: 0,
            duplicates: 0,
            jitter: 0.0,
            last_arrival: None,
            last_timestamp: None,
            clock_rate,
            jitter_buffer: None,
            max_jitter_size: 50, // Default size
            max_packet_age: Duration::from_millis(200), // Default 200ms
            seq_cycles: 0,
            last_sr_timestamp: None,
            last_sr_time: None,
        }
    }
    
    /// Create a new RTP stream with jitter buffer
    pub fn with_jitter_buffer(
        ssrc: RtpSsrc, 
        clock_rate: u32, 
        buffer_size: usize, 
        max_age_ms: u64
    ) -> Self {
        let mut stream = Self::new(ssrc, clock_rate);
        stream.enable_jitter_buffer(buffer_size, max_age_ms);
        stream
    }
    
    /// Enable jitter buffer for this stream
    pub fn enable_jitter_buffer(&mut self, size: usize, max_age_ms: u64) {
        self.jitter_buffer = Some(Arc::new(Mutex::new(VecDeque::with_capacity(size))));
        self.max_jitter_size = size;
        self.max_packet_age = Duration::from_millis(max_age_ms);
    }
    
    /// Disable jitter buffer
    pub fn disable_jitter_buffer(&mut self) {
        self.jitter_buffer = None;
    }
    
    /// Process a received RTP packet
    /// Returns the packet if it should be processed immediately,
    /// or None if it was placed in the jitter buffer
    pub fn process_packet(&mut self, packet: RtpPacket) -> Option<RtpPacket> {
        let now = Instant::now();
        self.last_packet_time = now;
        
        let seq = packet.header.sequence_number;
        let timestamp = packet.header.timestamp;
        
        // Update basic stats
        self.packets_received += 1;
        self.bytes_received += packet.size() as u64;
        
        // Initialize sequence tracking if this is the first packet
        if !self.initialized {
            self.init_sequence(seq);
            self.last_timestamp = Some(timestamp);
            self.last_arrival = Some(now);
            self.initialized = true;
            return Some(packet);
        }
        
        // Update sequence tracking
        self.update_sequence(seq);
        
        // Update jitter estimate
        if let (Some(last_arrival), Some(last_ts)) = (self.last_arrival, self.last_timestamp) {
            let arrival_diff = now.duration_since(last_arrival).as_secs_f64();
            let ts_diff = ((timestamp as i32 - last_ts as i32).abs() as f64) / (self.clock_rate as f64);
            
            // RFC 3550 jitter calculation
            let d = arrival_diff - ts_diff;
            self.jitter += (d.abs() - self.jitter) / 16.0;
        }
        
        self.last_arrival = Some(now);
        self.last_timestamp = Some(timestamp);
        
        // If using jitter buffer, add to buffer and return ordered packets
        if let Some(buffer) = &self.jitter_buffer {
            self.add_to_jitter_buffer(packet, buffer.clone());
            self.get_next_from_jitter_buffer(buffer.clone())
        } else {
            // No jitter buffer, return packet immediately
            Some(packet)
        }
    }
    
    /// Initialize sequence tracking
    fn init_sequence(&mut self, seq: RtpSequenceNumber) {
        self.base_seq = seq;
        self.latest_seq = seq;
        self.highest_seq = seq as u32;
        debug!("Initialized RTP stream with seq={}", seq);
    }
    
    /// Update sequence tracking with a new sequence number
    fn update_sequence(&mut self, seq: RtpSequenceNumber) {
        // Detect sequence number cycle (wraparound from 65535 to 0)
        if seq < 0x1000 && self.latest_seq > 0xf000 {
            debug!("Detected sequence wraparound: {} -> {}", self.latest_seq, seq);
            self.seq_cycles += 1;
        }
        
        // Check for duplicate
        if seq == self.latest_seq {
            self.duplicates += 1;
            return;
        }
        
        // Calculate extended sequence (with cycle count)
        let extended_seq = (self.seq_cycles as u32) << 16 | (seq as u32);
        
        // Check if this is the highest sequence seen
        if extended_seq > self.highest_seq {
            // Calculate lost packets (gap in sequence)
            let expected_seq = (self.latest_seq as u32 + 1) & 0xFFFF;
            if seq != expected_seq as u16 {
                // There's a gap - calculate how many packets were lost
                let gap = if seq > expected_seq as u16 {
                    seq - expected_seq as u16
                } else {
                    // Handle sequence number wraparound
                    ((0xFFFF as u32 + 1) - expected_seq as u32) as u16 + seq
                };
                
                if gap > 0 {
                    self.packets_lost += gap as u64;
                    debug!("Detected sequence gap: expected={}, got={}, lost={}", 
                           expected_seq, seq, gap);
                }
            }
            
            self.highest_seq = extended_seq;
        } else {
            // Out of order packet (older than highest)
            debug!("Out of order packet: seq={}, highest={}", seq, self.highest_seq & 0xFFFF);
        }
        
        self.latest_seq = seq;
    }
    
    /// Add a packet to the jitter buffer
    fn add_to_jitter_buffer(&self, packet: RtpPacket, buffer: Arc<Mutex<VecDeque<RtpPacket>>>) {
        if let Ok(mut buffer_lock) = buffer.lock() {
            if buffer_lock.len() >= self.max_jitter_size {
                // Buffer is full, remove oldest packet
                buffer_lock.pop_front();
                warn!("Jitter buffer full, dropping oldest packet");
            }
            
            // Find the correct position to insert this packet (sorted by sequence number)
            let seq = packet.header.sequence_number;
            let pos = buffer_lock.iter().position(|p| {
                let p_seq = p.header.sequence_number;
                is_sequence_newer(seq, p_seq)
            });
            
            if let Some(pos) = pos {
                buffer_lock.insert(pos, packet);
            } else {
                // Add to end
                buffer_lock.push_back(packet);
            }
        }
    }
    
    /// Get the next packet from the jitter buffer if it's ready to be processed
    fn get_next_from_jitter_buffer(&self, buffer: Arc<Mutex<VecDeque<RtpPacket>>>) -> Option<RtpPacket> {
        if let Ok(mut buffer_lock) = buffer.lock() {
            if buffer_lock.is_empty() {
                return None;
            }
            
            // Check if the oldest packet is old enough to be released
            let first_packet = buffer_lock.front()?;
            let expected_seq = (self.latest_seq as u32 + 1) & 0xFFFF;
            
            // Release packet if it's the next expected one, or if it's old
            if first_packet.header.sequence_number == expected_seq as u16 {
                return buffer_lock.pop_front();
            }
            
            // If buffer has enough packets or packet is too old, release it
            if buffer_lock.len() > self.max_jitter_size / 2 {
                return buffer_lock.pop_front();
            }
        }
        
        None
    }
    
    /// Get the current jitter estimate in milliseconds
    pub fn get_jitter_ms(&self) -> f64 {
        self.jitter * 1000.0
    }
    
    /// Get statistics for this stream
    pub fn get_stats(&self) -> RtpStreamStats {
        RtpStreamStats {
            ssrc: self.ssrc,
            packets_received: self.packets_received,
            bytes_received: self.bytes_received,
            packets_lost: self.packets_lost,
            duplicates: self.duplicates,
            last_packet_time: Some(self.last_packet_time),
            jitter: self.jitter as u32,
            first_seq: self.base_seq as u32,
            highest_seq: self.highest_seq,
            received: self.packets_received as u32,
        }
    }
    
    /// Ensure the stream is initialized with the given sequence number
    /// This is useful when packets might be held in the jitter buffer
    /// or discarded, but we still want to track the stream.
    pub fn ensure_initialized(&mut self, seq: u16) {
        if !self.initialized {
            self.init_sequence(seq as RtpSequenceNumber);
            
            // Make sure all required state is properly initialized
            self.packets_received = 0;
            self.highest_seq = seq as u32;
            self.latest_seq = seq as RtpSequenceNumber;
            self.packets_lost = 0;
            self.duplicates = 0;
            self.jitter = 0.0;
            self.last_arrival = None;
            self.last_timestamp = None;
            
            self.initialized = true;
            debug!("Initialized RTP stream with seq={}", seq);
        }
    }
    
    /// Update the last SR information
    pub fn update_last_sr_info(&mut self, sr_timestamp: u32, time: Instant) {
        self.last_sr_timestamp = Some(sr_timestamp);
        self.last_sr_time = Some(time);
    }
    
    /// Get the last SR information
    pub fn get_last_sr_info(&self) -> (Option<u32>, Option<Instant>) {
        (self.last_sr_timestamp, self.last_sr_time)
    }
    
    /// Calculate the delay since last SR in 1/65536 seconds units
    pub fn calculate_delay_since_last_sr(&self) -> u32 {
        if let (Some(timestamp), Some(time)) = (self.last_sr_timestamp, self.last_sr_time) {
            // Calculate delay in seconds, then convert to 1/65536 seconds
            let delay_secs = Instant::now().duration_since(time).as_secs_f64();
            (delay_secs * 65536.0) as u32
        } else {
            0
        }
    }
}

/// Determines if sequence a is newer than sequence b, accounting for wraparound
fn is_sequence_newer(a: RtpSequenceNumber, b: RtpSequenceNumber) -> bool {
    let half_range = 0x8000;
    // If the difference is larger than half the range, it's due to wraparound
    if b < a {
        (a - b) <= half_range
    } else {
        (b - a) > half_range
    }
}

/// Stats for an RTP stream
#[derive(Debug, Clone, Default)]
pub struct RtpStreamStats {
    /// SSRC of the stream
    pub ssrc: RtpSsrc,
    
    /// Number of packets received
    pub packets_received: u64,
    
    /// Number of bytes received
    pub bytes_received: u64,
    
    /// Number of packets lost (based on sequence gaps)
    pub packets_lost: u64,
    
    /// Number of duplicate packets
    pub duplicates: u64,
    
    /// Last time a packet was received
    pub last_packet_time: Option<Instant>,
    
    /// Interarrival jitter (RFC 3550)
    pub jitter: u32,
    
    /// First sequence number received
    pub first_seq: u32,
    
    /// Highest sequence number received
    pub highest_seq: u32,
    
    /// Number of packets actually received (may differ from packets_received due to jitter buffer)
    pub received: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use crate::packet::RtpHeader;
    
    fn create_test_packet(seq: RtpSequenceNumber, ts: RtpTimestamp) -> RtpPacket {
        let header = RtpHeader::new(96, seq, ts, 0x12345678);
        let payload = Bytes::from_static(b"test");
        RtpPacket::new(header, payload)
    }
    
    #[test]
    fn test_sequence_tracking() {
        let mut stream = RtpStream::new(0x12345678, 8000);
        
        // Process first packet
        stream.process_packet(create_test_packet(1000, 80000));
        assert_eq!(stream.base_seq, 1000);
        assert_eq!(stream.highest_seq, 1000);
        assert_eq!(stream.packets_received, 1);
        assert_eq!(stream.packets_lost, 0);
        
        // Process next packet in sequence
        stream.process_packet(create_test_packet(1001, 80160));
        assert_eq!(stream.highest_seq, 1001);
        assert_eq!(stream.packets_received, 2);
        assert_eq!(stream.packets_lost, 0);
        
        // Process packet with gap
        stream.process_packet(create_test_packet(1005, 80800));
        assert_eq!(stream.highest_seq, 1005);
        assert_eq!(stream.packets_received, 3);
        assert_eq!(stream.packets_lost, 3); // Missing 1002, 1003, 1004
        
        // Process duplicate
        stream.process_packet(create_test_packet(1005, 80800));
        assert_eq!(stream.highest_seq, 1005);
        assert_eq!(stream.packets_received, 4);
        assert_eq!(stream.duplicates, 1);
        
        // Process out-of-order packet
        stream.process_packet(create_test_packet(1003, 80480));
        assert_eq!(stream.highest_seq, 1005); // Highest shouldn't change
        assert_eq!(stream.packets_received, 5);
        assert_eq!(stream.packets_lost, 3); // Still 3 packets lost
    }
    
    #[test]
    fn test_sequence_wraparound() {
        let mut stream = RtpStream::new(0x12345678, 8000);
        
        // Start with high sequence number
        stream.process_packet(create_test_packet(65530, 80000));
        assert_eq!(stream.base_seq, 65530);
        assert_eq!(stream.highest_seq, 65530);
        assert_eq!(stream.seq_cycles, 0);
        
        // Process packets up to wraparound
        stream.process_packet(create_test_packet(65531, 80160));
        stream.process_packet(create_test_packet(65532, 80320));
        stream.process_packet(create_test_packet(65533, 80480));
        stream.process_packet(create_test_packet(65534, 80640));
        stream.process_packet(create_test_packet(65535, 80800));
        assert_eq!(stream.highest_seq, 65535);
        assert_eq!(stream.seq_cycles, 0);
        
        // Process packet after wraparound
        stream.process_packet(create_test_packet(0, 80960));
        assert_eq!(stream.highest_seq, 65536); // 1 cycle + sequence 0
        assert_eq!(stream.seq_cycles, 1);
        
        // Process a few more packets
        stream.process_packet(create_test_packet(1, 81120));
        stream.process_packet(create_test_packet(2, 81280));
        assert_eq!(stream.highest_seq, 65538); // 1 cycle + sequence 2
        assert_eq!(stream.seq_cycles, 1);
    }
    
    #[test]
    fn test_is_sequence_newer() {
        // Normal cases
        assert!(is_sequence_newer(101, 100));
        assert!(!is_sequence_newer(100, 101));
        
        // Equal case
        assert!(!is_sequence_newer(100, 100));
        
        // Wraparound cases
        assert!(is_sequence_newer(0, 65535));
        assert!(!is_sequence_newer(65535, 0));
        
        // Edge cases around wraparound
        assert!(is_sequence_newer(1, 65000));
        assert!(!is_sequence_newer(65000, 1));
        
        // Edge cases within half range
        assert!(is_sequence_newer(32768, 0));
        assert!(!is_sequence_newer(0, 32768));
    }
} 
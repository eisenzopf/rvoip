use std::time::{Duration, Instant};
use std::cmp::Ordering;

use bytes::Bytes;

/// A packet in a buffer with timing and sequence information
#[derive(Debug, Clone)]
pub struct BufferedPacket {
    /// Packet data
    pub data: Bytes,
    /// Sequence number
    pub sequence: u16,
    /// Timestamp in media clock units
    pub timestamp: u32,
    /// Whether this is a marker packet (typically end of frame)
    pub marker: bool,
    /// When the packet was received (local clock)
    pub arrival_time: Instant,
    /// Whether this is a padding packet (inserted for lost packets)
    pub is_padding: bool,
}

impl BufferedPacket {
    /// Create a new buffered packet
    pub fn new(
        data: Bytes,
        sequence: u16,
        timestamp: u32,
        marker: bool,
    ) -> Self {
        Self {
            data,
            sequence,
            timestamp,
            marker,
            arrival_time: Instant::now(),
            is_padding: false,
        }
    }
    
    /// Create a padding packet for a lost packet
    pub fn padding(sequence: u16, timestamp: u32) -> Self {
        Self {
            data: Bytes::new(),
            sequence,
            timestamp,
            marker: false,
            arrival_time: Instant::now(),
            is_padding: true,
        }
    }
}

impl PartialEq for BufferedPacket {
    fn eq(&self, other: &Self) -> bool {
        self.sequence == other.sequence
    }
}

impl Eq for BufferedPacket {}

impl PartialOrd for BufferedPacket {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BufferedPacket {
    fn cmp(&self, other: &Self) -> Ordering {
        // Handle sequence number wraparound
        const WRAPAROUND_THRESHOLD: u16 = 32768; // Half of the sequence number space
        
        let diff = self.sequence as i32 - other.sequence as i32;
        if diff.abs() > WRAPAROUND_THRESHOLD as i32 {
            // Wraparound case
            if self.sequence < other.sequence {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        } else {
            // Normal case
            self.sequence.cmp(&other.sequence)
        }
    }
}

/// A frame of media data, potentially consisting of multiple packets
#[derive(Debug, Clone)]
pub struct BufferedFrame {
    /// Frame timestamp
    pub timestamp: u32,
    /// Packets in this frame
    pub packets: Vec<BufferedPacket>,
    /// Whether the frame is complete
    pub is_complete: bool,
}

impl BufferedFrame {
    /// Create a new buffered frame with an initial packet
    pub fn new(initial_packet: BufferedPacket) -> Self {
        Self {
            timestamp: initial_packet.timestamp,
            packets: vec![initial_packet],
            is_complete: false,
        }
    }
    
    /// Add a packet to this frame
    pub fn add_packet(&mut self, packet: BufferedPacket) -> bool {
        if packet.timestamp != self.timestamp {
            return false;
        }
        
        // Check if we already have this packet
        if self.packets.iter().any(|p| p.sequence == packet.sequence) {
            return false;
        }
        
        // Add the packet
        self.packets.push(packet);
        
        // Sort packets by sequence number
        self.packets.sort_by(|a, b| a.sequence.cmp(&b.sequence));
        
        // Update completeness based on marker bit
        self.is_complete = self.packets.iter().any(|p| p.marker);
        
        true
    }
    
    /// Get the combined data for this frame
    pub fn data(&self) -> Bytes {
        // If we only have one packet, return it directly
        if self.packets.len() == 1 {
            return self.packets[0].data.clone();
        }
        
        // Otherwise, combine all packets
        let total_len = self.packets.iter().map(|p| p.data.len()).sum();
        let mut result = Vec::with_capacity(total_len);
        
        for packet in &self.packets {
            if !packet.is_padding {
                result.extend_from_slice(&packet.data);
            }
        }
        
        Bytes::from(result)
    }
    
    /// Get the first sequence number in this frame
    pub fn first_sequence(&self) -> Option<u16> {
        self.packets.first().map(|p| p.sequence)
    }
    
    /// Get the last sequence number in this frame
    pub fn last_sequence(&self) -> Option<u16> {
        self.packets.last().map(|p| p.sequence)
    }
    
    /// Check if this frame contains a specific sequence number
    pub fn contains_sequence(&self, sequence: u16) -> bool {
        self.packets.iter().any(|p| p.sequence == sequence)
    }
    
    /// Check if the frame has any packets
    pub fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }
    
    /// Get the number of packets in this frame
    pub fn packet_count(&self) -> usize {
        self.packets.len()
    }
    
    /// Get the arrival time of the first packet
    pub fn first_arrival_time(&self) -> Option<Instant> {
        self.packets.first().map(|p| p.arrival_time)
    }
}

/// Jitter calculation helper
#[derive(Debug, Clone)]
pub struct JitterCalculator {
    /// Last transit time measured
    last_transit: Option<Duration>,
    /// Current jitter estimate (in milliseconds)
    jitter: f64,
}

impl JitterCalculator {
    /// Create a new jitter calculator
    pub fn new() -> Self {
        Self {
            last_transit: None,
            jitter: 0.0,
        }
    }
    
    /// Update jitter calculation with a new packet
    pub fn update(&mut self, timestamp: u32, arrival_time: Instant, clock_rate: u32) {
        // Calculate transit time (how long the packet took to arrive)
        let now = Instant::now();
        let transit = now.duration_since(arrival_time);
        
        // Convert RTP timestamp to duration
        let timestamp_duration = Duration::from_secs_f64(timestamp as f64 / clock_rate as f64);
        
        // Combine to get complete transit time
        let total_transit = transit + timestamp_duration;
        
        // Update jitter calculation per RFC 3550
        if let Some(last_transit) = self.last_transit {
            // Calculate the difference between this transit and the last
            let transit_delta = if total_transit > last_transit {
                total_transit - last_transit
            } else {
                last_transit - total_transit
            };
            
            // Convert to milliseconds (as float for precision)
            let delta_ms = transit_delta.as_secs_f64() * 1000.0;
            
            // Update jitter using standard RFC 3550 formula: J += (|D(i-1,i)| - J) / 16
            self.jitter += (delta_ms - self.jitter) / 16.0;
        }
        
        // Store this transit time for next calculation
        self.last_transit = Some(total_transit);
    }
    
    /// Get the current jitter estimate in milliseconds
    pub fn jitter_ms(&self) -> f64 {
        self.jitter
    }
    
    /// Reset the jitter calculator
    pub fn reset(&mut self) {
        self.last_transit = None;
        self.jitter = 0.0;
    }
}

impl Default for JitterCalculator {
    fn default() -> Self {
        Self::new()
    }
} 
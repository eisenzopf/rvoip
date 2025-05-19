use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};
use std::cmp::{min, max};

use bytes::Bytes;
use tracing::{debug, trace, warn};
use rvoip_rtp_core::packet::RtpPacket;

use crate::buffer::common::{BufferedPacket, BufferedFrame, JitterCalculator};
use crate::error::{Error, Result};

/// Jitter buffer operating mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitterBufferMode {
    /// Fixed delay (constant buffer size in ms)
    Fixed,
    /// Adaptive delay (changes based on network conditions)
    Adaptive,
}

impl Default for JitterBufferMode {
    fn default() -> Self {
        Self::Adaptive
    }
}

/// Configuration for the jitter buffer
#[derive(Debug, Clone)]
pub struct JitterBufferConfig {
    /// Initial buffer size in milliseconds
    pub initial_delay_ms: u32,
    /// Minimum buffer size in milliseconds
    pub min_delay_ms: u32,
    /// Maximum buffer size in milliseconds
    pub max_delay_ms: u32,
    /// Operating mode (fixed or adaptive)
    pub mode: JitterBufferMode,
    /// Maximum number of frames to store
    pub max_frames: usize,
    /// Whether to generate padding for missing packets
    pub generate_padding: bool,
    /// Maximum time to wait for packet reordering (ms)
    pub reordering_time_ms: u32,
    /// Clock rate in Hz (for timestamp calculations)
    pub clock_rate: u32,
    /// Number of consecutive packets needed to trigger an adaptation
    pub adaptation_trigger_count: usize,
    /// How quickly to adapt to jitter (higher = faster) [0.0-1.0]
    pub adaptation_rate: f32,
}

impl Default for JitterBufferConfig {
    fn default() -> Self {
        Self {
            initial_delay_ms: 50,
            min_delay_ms: 20,
            max_delay_ms: 200,
            mode: JitterBufferMode::default(),
            max_frames: 100,
            generate_padding: true,
            reordering_time_ms: 50,
            clock_rate: 8000, // Default to 8kHz (common for telephony)
            adaptation_trigger_count: 5,
            adaptation_rate: 0.2,
        }
    }
}

/// Statistics for the jitter buffer
#[derive(Debug, Clone, Default)]
pub struct JitterBufferStats {
    /// Number of packets received
    pub packets_received: u64,
    /// Number of packets played out
    pub packets_played: u64,
    /// Number of late packets (arrived after their playout time)
    pub late_packets: u64,
    /// Number of lost packets (never arrived)
    pub lost_packets: u64,
    /// Number of duplicate packets
    pub duplicate_packets: u64,
    /// Number of packets reordered
    pub reordered_packets: u64,
    /// Current jitter estimate in milliseconds
    pub jitter_ms: f64,
    /// Current buffer delay in milliseconds
    pub current_delay_ms: u32,
    /// Number of frames in the buffer
    pub buffer_size: usize,
}

/// Jitter buffer for handling variable network packet timing
pub struct JitterBuffer {
    /// Configuration
    config: JitterBufferConfig,
    /// Statistics
    stats: JitterBufferStats,
    /// Buffered frames, sorted by timestamp
    frames: BTreeMap<u32, BufferedFrame>,
    /// Order of frames for playout
    frame_queue: VecDeque<u32>,
    /// Next expected sequence number
    next_sequence: Option<u16>,
    /// Received sequence numbers in a window
    received_sequences: Vec<u16>,
    /// Last played out timestamp
    last_played_timestamp: Option<u32>,
    /// Timestamp offset for converting to playout time
    timestamp_offset: u32,
    /// First packet received (for initializing)
    first_packet_received: bool,
    /// Jitter calculator
    jitter_calculator: JitterCalculator,
    /// Current delay in timestamp units
    current_delay: u32,
    /// Adaptation counter for triggering adaptations
    adaptation_counter: usize,
    /// Start time
    start_time: Instant,
}

impl JitterBuffer {
    /// Create a new jitter buffer with the given configuration
    pub fn new(config: JitterBufferConfig) -> Self {
        let timestamp_units_per_ms = config.clock_rate / 1000;
        let initial_delay = config.initial_delay_ms * timestamp_units_per_ms;
        
        Self {
            config,
            stats: JitterBufferStats::default(),
            frames: BTreeMap::new(),
            frame_queue: VecDeque::new(),
            next_sequence: None,
            received_sequences: Vec::with_capacity(1000),
            last_played_timestamp: None,
            timestamp_offset: 0,
            first_packet_received: false,
            jitter_calculator: JitterCalculator::new(),
            current_delay: initial_delay,
            adaptation_counter: 0,
            start_time: Instant::now(),
        }
    }
    
    /// Create a new jitter buffer with default configuration
    pub fn new_default() -> Self {
        Self::new(JitterBufferConfig::default())
    }
    
    /// Add an RTP packet to the jitter buffer
    pub fn add_packet(&mut self, packet: &RtpPacket) -> Result<()> {
        let sequence = packet.sequence();
        let timestamp = packet.timestamp();
        let payload = packet.payload();
        let marker = packet.marker();
        
        // Update stats
        self.stats.packets_received += 1;
        
        // Check for duplicates
        if self.is_duplicate(sequence) {
            self.stats.duplicate_packets += 1;
            return Ok(());
        }
        
        // Update next sequence if this is the first packet
        if !self.first_packet_received {
            self.next_sequence = Some(sequence.wrapping_add(1));
            self.first_packet_received = true;
            
            // Initialize timestamp offset for playout timing
            self.timestamp_offset = timestamp;
        }
        
        // Check if packet is out of order
        if let Some(expected) = self.next_sequence {
            if sequence != expected {
                // Could be packet loss or reordering
                if self.sequence_greater_than(sequence, expected) {
                    // Out of order in the future - we missed some packets
                    let missed = self.sequence_distance(expected, sequence);
                    
                    // Update next expected sequence
                    self.next_sequence = Some(sequence.wrapping_add(1));
                    
                    // Generate padding for lost packets
                    if self.config.generate_padding {
                        for seq in 0..missed {
                            let lost_seq = expected.wrapping_add(seq as u16);
                            let estimated_ts = self.estimate_timestamp(lost_seq, timestamp);
                            self.handle_lost_packet(lost_seq, estimated_ts);
                        }
                    }
                } else {
                    // Out of order in the past - reordered packet
                    self.stats.reordered_packets += 1;
                    
                    // Check if it's too late
                    if self.is_too_late(timestamp) {
                        self.stats.late_packets += 1;
                        return Ok(());
                    }
                }
            } else {
                // In sequence - update next expected
                self.next_sequence = Some(expected.wrapping_add(1));
            }
        }
        
        // Track received sequence
        self.received_sequences.push(sequence);
        if self.received_sequences.len() > 1000 {
            self.received_sequences.remove(0);
        }
        
        // Create buffered packet
        let buffered_packet = BufferedPacket::new(
            payload.clone(),
            sequence,
            timestamp,
            marker,
        );
        
        // Update jitter calculation
        self.jitter_calculator.update(timestamp, buffered_packet.arrival_time, self.config.clock_rate);
        self.stats.jitter_ms = self.jitter_calculator.jitter_ms();
        
        // Add to frame or create new frame
        if let Some(frame) = self.frames.get_mut(&timestamp) {
            // Add to existing frame
            frame.add_packet(buffered_packet);
        } else {
            // Create new frame
            let frame = BufferedFrame::new(buffered_packet);
            self.frames.insert(timestamp, frame);
            self.frame_queue.push_back(timestamp);
            
            // Keep buffer size in check
            while self.frames.len() > self.config.max_frames {
                if let Some(oldest_ts) = self.frame_queue.pop_front() {
                    self.frames.remove(&oldest_ts);
                }
            }
        }
        
        // Adapt buffer if in adaptive mode
        if self.config.mode == JitterBufferMode::Adaptive {
            self.adaptation_counter += 1;
            if self.adaptation_counter >= self.config.adaptation_trigger_count {
                self.adapt_delay();
                self.adaptation_counter = 0;
            }
        }
        
        // Update current delay in statistics
        self.stats.current_delay_ms = self.current_delay * 1000 / self.config.clock_rate;
        self.stats.buffer_size = self.frames.len();
        
        Ok(())
    }
    
    /// Get the next frame from the jitter buffer if it's ready for playout
    pub fn get_next_frame(&mut self) -> Option<Bytes> {
        // Nothing to play if buffer is empty
        if self.frames.is_empty() {
            return None;
        }
        
        // Get the oldest timestamp
        let oldest_ts = match self.frame_queue.front() {
            Some(&ts) => ts,
            None => return None,
        };
        
        // Check if we've waited long enough
        let playout_point = self.get_playout_point();
        if !self.is_ready_for_playout(oldest_ts, playout_point) {
            return None;
        }
        
        // Retrieve the frame
        if let Some(ts) = self.frame_queue.pop_front() {
            if let Some(frame) = self.frames.remove(&ts) {
                // Update stats
                self.stats.packets_played += frame.packet_count() as u64;
                
                // Update last played timestamp
                self.last_played_timestamp = Some(ts);
                
                // Return the combined data
                return Some(frame.data());
            }
        }
        
        None
    }
    
    /// Get the current jitter buffer statistics
    pub fn stats(&self) -> &JitterBufferStats {
        &self.stats
    }
    
    /// Reset the jitter buffer
    pub fn reset(&mut self) {
        self.frames.clear();
        self.frame_queue.clear();
        self.next_sequence = None;
        self.received_sequences.clear();
        self.last_played_timestamp = None;
        self.timestamp_offset = 0;
        self.first_packet_received = false;
        self.jitter_calculator.reset();
        self.adaptation_counter = 0;
        
        let timestamp_units_per_ms = self.config.clock_rate / 1000;
        self.current_delay = self.config.initial_delay_ms * timestamp_units_per_ms;
        
        self.stats = JitterBufferStats::default();
        self.start_time = Instant::now();
    }
    
    /// Check if a sequence number is a duplicate
    fn is_duplicate(&self, sequence: u16) -> bool {
        self.received_sequences.contains(&sequence)
    }
    
    /// Check if a timestamp is too late for playback
    fn is_too_late(&self, timestamp: u32) -> bool {
        if let Some(last_ts) = self.last_played_timestamp {
            return timestamp <= last_ts;
        }
        false
    }
    
    /// Get the current playout point
    fn get_playout_point(&self) -> u32 {
        // The playout point is the current time plus the buffer delay
        let elapsed = self.start_time.elapsed();
        let elapsed_ms = elapsed.as_millis() as u32;
        let elapsed_units = elapsed_ms * self.config.clock_rate / 1000;
        
        // Add timestamp offset and subtract the delay
        let mut playout_point = self.timestamp_offset.wrapping_add(elapsed_units);
        playout_point = playout_point.wrapping_sub(self.current_delay);
        
        playout_point
    }
    
    /// Check if a frame is ready for playout
    fn is_ready_for_playout(&self, timestamp: u32, playout_point: u32) -> bool {
        // We can play this frame if its timestamp is less than or equal to the playout point
        self.sequence_greater_than(playout_point, timestamp)
    }
    
    /// Handle a lost packet
    fn handle_lost_packet(&mut self, sequence: u16, timestamp: u32) {
        self.stats.lost_packets += 1;
        
        // Create a padding packet
        let padding_packet = BufferedPacket::padding(sequence, timestamp);
        
        // Add to frame or create new frame
        if let Some(frame) = self.frames.get_mut(&timestamp) {
            frame.add_packet(padding_packet);
        } else {
            let frame = BufferedFrame::new(padding_packet);
            self.frames.insert(timestamp, frame);
            self.frame_queue.push_back(timestamp);
        }
    }
    
    /// Estimate the timestamp for a sequence number
    fn estimate_timestamp(&self, sequence: u16, reference_ts: u32) -> u32 {
        // Try to find a nearby packet to estimate from
        for &seq in &self.received_sequences {
            if let Some(frame) = self.frame_queue.iter()
                .filter_map(|&ts| self.frames.get(&ts))
                .find(|f| f.contains_sequence(seq))
            {
                // Found a reference frame, estimate based on sequence difference
                let seq_diff = self.sequence_distance(seq, sequence);
                // In timestamp units, each packet is typically a fixed duration
                // For 20ms frames at 8kHz, this would be 160 timestamp units
                const TIMESTAMP_UNITS_PER_PACKET: u32 = 160;
                return frame.timestamp.wrapping_add(seq_diff as u32 * TIMESTAMP_UNITS_PER_PACKET);
            }
        }
        
        // Fallback - use the reference timestamp
        reference_ts
    }
    
    /// Compare sequence numbers accounting for wraparound
    fn sequence_greater_than(&self, a: u16, b: u16) -> bool {
        // RFC 3550 algorithm for sequence comparison
        (a > b && a - b < 32768) || (a < b && b - a > 32768)
    }
    
    /// Calculate the distance between two sequence numbers
    fn sequence_distance(&self, a: u16, b: u16) -> u32 {
        if a <= b {
            (b - a) as u32
        } else {
            (65536 - a as u32) + b as u32
        }
    }
    
    /// Adapt the buffer delay based on current jitter
    fn adapt_delay(&mut self) {
        // Current jitter measured in timestamp units
        let jitter_units = (self.stats.jitter_ms * self.config.clock_rate as f64 / 1000.0) as u32;
        
        // Target delay is jitter plus a safety margin
        let safety_factor = 2.0;
        let target_delay = (jitter_units as f32 * safety_factor as f32) as u32;
        
        // Clamp to configured min/max
        let min_delay = self.config.min_delay_ms * self.config.clock_rate / 1000;
        let max_delay = self.config.max_delay_ms * self.config.clock_rate / 1000;
        let target_delay = max(min_delay, min(target_delay, max_delay));
        
        // Slowly adapt to the target
        let delta = target_delay as f32 - self.current_delay as f32;
        let adjustment = delta * self.config.adaptation_rate;
        self.current_delay = (self.current_delay as f32 + adjustment) as u32;
        
        // Update stats
        self.stats.current_delay_ms = self.current_delay * 1000 / self.config.clock_rate;
        
        trace!("Adapted jitter buffer: jitter={}ms, delay={}ms", 
               self.stats.jitter_ms, self.stats.current_delay_ms);
    }
    
    /// Set the buffer mode
    pub fn set_mode(&mut self, mode: JitterBufferMode) {
        self.config.mode = mode;
    }
    
    /// Get the current buffer mode
    pub fn mode(&self) -> JitterBufferMode {
        self.config.mode
    }
    
    /// Set fixed buffer delay
    pub fn set_fixed_delay(&mut self, delay_ms: u32) -> Result<()> {
        if delay_ms < self.config.min_delay_ms || delay_ms > self.config.max_delay_ms {
            return Err(Error::InvalidParameter(format!(
                "Delay must be between {} and {} ms", 
                self.config.min_delay_ms, 
                self.config.max_delay_ms
            )));
        }
        
        self.config.mode = JitterBufferMode::Fixed;
        let delay_units = delay_ms * self.config.clock_rate / 1000;
        self.current_delay = delay_units;
        self.stats.current_delay_ms = delay_ms;
        
        Ok(())
    }
} 
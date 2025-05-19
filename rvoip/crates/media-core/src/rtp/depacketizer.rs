//! RTP Depacketizer
//!
//! This module provides the RTP depacketization functionality for
//! converting RTP packets back to media data.

use std::sync::Arc;
use std::collections::VecDeque;
use bytes::{Bytes, BytesMut};
use std::time::{Duration, Instant};

use tracing::{debug, trace, warn};

use crate::error::{Error, Result};
use crate::codec::{Codec, CodecParameters};
use rvoip_rtp_core::packet::RtpPacket;
use crate::{AudioBuffer, AudioFormat, Sample, SampleRate};

/// Configuration for the RTP depacketizer
#[derive(Debug, Clone)]
pub struct DepacketizerConfig {
    /// Expected payload type
    pub payload_type: u8,
    /// Expected clock rate in Hz
    pub clock_rate: u32,
    /// Expected audio format for decoded output
    pub audio_format: AudioFormat,
    /// Maximum time to wait for packet reordering (ms)
    pub reordering_time_ms: u32,
    /// Maximum jitter buffer size in packets
    pub max_jitter_packets: usize,
    /// Whether to perform packet reordering
    pub reorder_packets: bool,
}

impl Default for DepacketizerConfig {
    fn default() -> Self {
        Self {
            payload_type: 0, // Default to PCMU
            clock_rate: 8000, // Default to 8kHz
            audio_format: AudioFormat::telephony(),
            reordering_time_ms: 50, // 50ms default for reordering
            max_jitter_packets: 5, // 5 packets max in jitter buffer
            reorder_packets: true,
        }
    }
}

/// Structure for tracking RTP sequence and timing
#[derive(Debug)]
struct RtpState {
    /// Last seen sequence number
    last_seq: u16,
    /// Highest sequence seen
    highest_seq: u16,
    /// Last seen timestamp
    last_ts: u32,
    /// Whether state is initialized
    initialized: bool,
    /// Packets waiting for processing (for reordering)
    packet_queue: VecDeque<(RtpPacket, Instant)>,
    /// Packets received out of order
    out_of_order_count: u32,
    /// Packets too late to be reordered
    late_packets: u32,
    /// Whether the sequence has wrapped around
    wrapped: bool,
}

impl RtpState {
    fn new() -> Self {
        Self {
            last_seq: 0,
            highest_seq: 0,
            last_ts: 0,
            initialized: false,
            packet_queue: VecDeque::new(),
            out_of_order_count: 0,
            late_packets: 0,
            wrapped: false,
        }
    }
    
    /// Check if a packet is in sequence
    fn is_in_sequence(&self, seq: u16) -> bool {
        if !self.initialized {
            return true;
        }
        
        // Handle sequence wraparound
        if self.wrapped {
            // If we've wrapped and seq is high, it's likely an old packet
            if seq > 65000 && self.highest_seq < 1000 {
                return false;
            }
            
            // If we've wrapped and seq is low, it should be higher than highest
            if seq < 1000 && self.highest_seq > 65000 {
                return seq > self.last_seq || seq < self.highest_seq;
            }
        }
        
        // Normal case - should be next in sequence
        seq == self.last_seq.wrapping_add(1)
    }
    
    /// Check if a packet is too old (compared to highest seen)
    fn is_too_old(&self, seq: u16) -> bool {
        if !self.initialized {
            return false;
        }
        
        // Handle sequence wraparound
        if self.wrapped {
            // If we've wrapped and seq is high, it's likely an old packet
            if seq > 65000 && self.highest_seq < 1000 {
                return true;
            }
        }
        
        // Determine sequence distance
        let distance = if seq > self.highest_seq {
            seq - self.highest_seq
        } else {
            // Wrapped around
            (65535 - self.highest_seq) + seq + 1
        };
        
        // Too old if more than 100 packets behind highest
        distance > 100
    }
    
    /// Update state with a new packet
    fn update(&mut self, packet: &RtpPacket) {
        let seq = packet.sequence();
        
        if !self.initialized {
            self.last_seq = seq;
            self.highest_seq = seq;
            self.last_ts = packet.timestamp();
            self.initialized = true;
            return;
        }
        
        // Check for wraparound
        if self.highest_seq > 65000 && seq < 1000 {
            self.wrapped = true;
        }
        
        if seq > self.highest_seq || 
           (self.wrapped && seq < 1000 && self.highest_seq > 65000) {
            self.highest_seq = seq;
        }
        
        self.last_seq = seq;
        self.last_ts = packet.timestamp();
    }
}

/// RTP depacketizer that converts RTP packets back into media frames
pub struct Depacketizer {
    /// Configuration
    config: DepacketizerConfig,
    /// RTP state tracking
    state: RtpState,
    /// Codec for depacketization (if needed)
    codec: Option<Arc<dyn Codec>>,
    /// Buffer for assembling packets
    assembly_buffer: BytesMut,
}

impl Depacketizer {
    /// Create a new depacketizer with the given configuration
    pub fn new(config: DepacketizerConfig) -> Self {
        Self {
            config,
            state: RtpState::new(),
            codec: None,
            assembly_buffer: BytesMut::with_capacity(1024),
        }
    }
    
    /// Create a new depacketizer with default configuration
    pub fn new_default() -> Self {
        Self::new(DepacketizerConfig::default())
    }
    
    /// Set the codec for depacketization
    pub fn set_codec(&mut self, codec: Arc<dyn Codec>) {
        self.codec = Some(codec);
    }
    
    /// Process an RTP packet, returning audio buffers if available
    /// 
    /// Returns None if the packet isn't ready to be processed yet (waiting for reordering)
    /// or if it doesn't produce a complete audio frame.
    pub fn process_packet(&mut self, packet: RtpPacket) -> Result<Option<AudioBuffer>> {
        // Check payload type
        if packet.payload_type().value() != self.config.payload_type {
            warn!("Unexpected payload type: {}, expected {}", 
                 packet.payload_type().value(), self.config.payload_type);
            return Ok(None);
        }
        
        // If reordering is enabled, queue the packet
        if self.config.reorder_packets {
            return self.process_with_reordering(packet);
        }
        
        // Process immediately (no reordering)
        self.process_packet_internal(packet)
    }
    
    /// Process a packet with the reordering queue
    fn process_with_reordering(&mut self, packet: RtpPacket) -> Result<Option<AudioBuffer>> {
        let seq = packet.sequence();
        let now = Instant::now();
        
        // Drop packets that are too old
        if self.state.is_too_old(seq) {
            self.state.late_packets += 1;
            trace!("Dropping packet with sequence {}, too old (highest: {})",
                  seq, self.state.highest_seq);
            return Ok(None);
        }
        
        // If this packet is the next in sequence, process it immediately
        if self.state.is_in_sequence(seq) {
            // First process this packet
            let result = self.process_packet_internal(packet.clone())?;
            
            // Then check the queue for packets that might be ready now
            return self.process_queued_packets(result);
        }
        
        // Out of order packet - add to queue
        self.state.out_of_order_count += 1;
        self.state.packet_queue.push_back((packet, now));
        
        // Sort queue by sequence number, taking wraparound into account
        self.state.packet_queue.make_contiguous();
        self.state.packet_queue.sort_by(|a, b| {
            let seq_a = a.0.sequence();
            let seq_b = b.0.sequence();
            
            // Handle wraparound case
            if (seq_a > 65000 && seq_b < 1000) {
                std::cmp::Ordering::Greater
            } else if (seq_a < 1000 && seq_b > 65000) {
                std::cmp::Ordering::Less
            } else {
                seq_a.cmp(&seq_b)
            }
        });
        
        // Check if any packets in queue are ready to be processed due to timeout
        self.process_queue_timeouts()
    }
    
    /// Process any queued packets that have timed out
    fn process_queue_timeouts(&mut self) -> Result<Option<AudioBuffer>> {
        let timeout_duration = Duration::from_millis(self.config.reordering_time_ms as u64);
        let now = Instant::now();
        let mut result = None;
        
        // Process packets that have been waiting too long
        while let Some((packet, timestamp)) = self.state.packet_queue.front() {
            if now.duration_since(*timestamp) >= timeout_duration {
                // Timeout expired, process this packet
                let (packet, _) = self.state.packet_queue.pop_front().unwrap();
                
                debug!("Processing queued packet seq={} (timeout)", packet.sequence());
                
                // Process the packet
                let packet_result = self.process_packet_internal(packet)?;
                
                // Combine results if needed
                if packet_result.is_some() {
                    result = packet_result;
                }
            } else {
                // This and subsequent packets haven't timed out yet
                break;
            }
        }
        
        // Limit queue size to avoid memory issues
        if self.state.packet_queue.len() > self.config.max_jitter_packets {
            // Remove oldest packets
            while self.state.packet_queue.len() > self.config.max_jitter_packets {
                self.state.packet_queue.pop_front();
            }
        }
        
        Ok(result)
    }
    
    /// Process queued packets that might be in sequence now
    fn process_queued_packets(&mut self, mut result: Option<AudioBuffer>) -> Result<Option<AudioBuffer>> {
        let mut processed_any = false;
        
        // Keep processing packets as long as we find ones that are next in sequence
        loop {
            let mut found_next = false;
            
            if let Some((packet, _)) = self.state.packet_queue.front() {
                if self.state.is_in_sequence(packet.sequence()) {
                    // This packet is next in sequence
                    let (packet, _) = self.state.packet_queue.pop_front().unwrap();
                    
                    debug!("Processing queued packet seq={} (in sequence)", packet.sequence());
                    
                    // Process the packet
                    let packet_result = self.process_packet_internal(packet)?;
                    
                    // Combine results if needed
                    if packet_result.is_some() {
                        result = packet_result;
                    }
                    
                    processed_any = true;
                    found_next = true;
                }
            }
            
            if !found_next {
                break;
            }
        }
        
        // If we processed anything or have a result, return it
        if processed_any || result.is_some() {
            Ok(result)
        } else {
            // Otherwise, try to process timed-out packets
            self.process_queue_timeouts()
        }
    }
    
    /// Process a packet directly (without reordering)
    fn process_packet_internal(&mut self, packet: RtpPacket) -> Result<Option<AudioBuffer>> {
        let seq = packet.sequence();
        let ts = packet.timestamp();
        
        // Update RTP state
        self.state.update(&packet);
        
        // Get payload
        let payload = packet.payload();
        
        if payload.is_empty() {
            // Empty payload, nothing to do
            return Ok(None);
        }
        
        trace!("Processing RTP packet: seq={}, ts={}, len={}", 
              seq, ts, payload.len());
        
        // If we have a codec, use it for depacketization
        if let Some(codec) = &self.codec {
            return self.depacketize_with_codec(packet, codec.as_ref());
        }
        
        // Simple depacketization (raw PCM)
        let audio_buffer = AudioBuffer::new(
            payload.clone(),
            self.config.audio_format
        );
        
        Ok(Some(audio_buffer))
    }
    
    /// Depacketize using a codec
    fn depacketize_with_codec(&mut self, packet: RtpPacket, codec: &dyn Codec) -> Result<Option<AudioBuffer>> {
        // Get the encoded data
        let payload = packet.payload();
        
        // Try to decode directly
        let buffer = codec.decode(&[payload.clone()])?;
        
        if let Some(buffer) = buffer {
            return Ok(Some(buffer));
        }
        
        // If direct decoding failed, append to assembly buffer and try again
        self.assembly_buffer.extend_from_slice(&payload);
        
        // Try to decode from assembly buffer
        let result = codec.decode(&[Bytes::from(self.assembly_buffer.clone())])?;
        
        // If successful, clear assembly buffer
        if result.is_some() {
            self.assembly_buffer.clear();
        }
        
        Ok(result)
    }
    
    /// Reset the depacketizer state
    pub fn reset(&mut self) {
        self.state = RtpState::new();
        self.assembly_buffer.clear();
    }
    
    /// Get the current depacketization statistics
    pub fn stats(&self) -> DepacketizerStats {
        DepacketizerStats {
            packets_queued: self.state.packet_queue.len(),
            out_of_order_count: self.state.out_of_order_count,
            late_packets: self.state.late_packets,
        }
    }
}

/// Statistics for the depacketizer
#[derive(Debug, Clone, Copy)]
pub struct DepacketizerStats {
    /// Number of packets currently queued for reordering
    pub packets_queued: usize,
    /// Total number of packets received out of order
    pub out_of_order_count: u32,
    /// Total number of packets that arrived too late to be reordered
    pub late_packets: u32,
} 
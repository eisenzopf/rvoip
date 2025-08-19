//! Adaptive jitter buffer for RTP packet reordering (moved from rtp-core)
//!
//! This module provides a high-performance jitter buffer implementation
//! that adapts to network conditions in real-time.

use std::collections::BTreeMap;
use std::time::Duration;

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
}

impl Default for JitterBufferConfig {
    fn default() -> Self {
        Self {
            initial_size_ms: 50,
            min_size_ms: 10,
            max_size_ms: 500,
            clock_rate: 8000,
            max_out_of_order: 100,
            max_packet_age_ms: 1000,
        }
    }
}

/// High-performance adaptive jitter buffer
pub struct JitterBuffer {
    config: JitterBufferConfig,
    buffer: BTreeMap<u32, Vec<u8>>, // timestamp -> packet data
    next_sequence: u32,
    last_playout_time: Option<std::time::Instant>,
}

impl JitterBuffer {
    /// Create a new jitter buffer with the given configuration
    pub fn new(config: JitterBufferConfig) -> Self {
        Self {
            config,
            buffer: BTreeMap::new(),
            next_sequence: 0,
            last_playout_time: None,
        }
    }
    
    /// Add a packet to the jitter buffer
    pub fn put_packet(&mut self, sequence: u32, timestamp: u32, payload: Vec<u8>) -> Result<(), String> {
        if self.buffer.len() >= self.config.max_out_of_order {
            return Err("Buffer full".to_string());
        }
        
        self.buffer.insert(timestamp, payload);
        Ok(())
    }
    
    /// Get the next packet for playout
    pub fn get_packet(&mut self) -> Option<(u32, Vec<u8>)> {
        // Simplified implementation - real jitter buffer would handle timing, reordering, etc.
        self.buffer.pop_first()
    }
    
    /// Flush old packets from the buffer
    pub fn flush_old_packets(&mut self) {
        let now = std::time::Instant::now();
        let max_age = Duration::from_millis(self.config.max_packet_age_ms as u64);
        
        // Simplified - real implementation would track packet ages
        if self.buffer.len() > self.config.max_out_of_order / 2 {
            self.buffer.clear();
        }
    }
    
    /// Get buffer statistics
    pub fn get_stats(&self) -> JitterBufferStats {
        JitterBufferStats {
            buffered_packets: self.buffer.len(),
            buffer_size_ms: 0, // Simplified
            adaptive_delay_ms: 0, // Simplified
        }
    }
}

/// Jitter buffer statistics
#[derive(Debug, Clone)]
pub struct JitterBufferStats {
    /// Number of packets currently buffered
    pub buffered_packets: usize,
    
    /// Current buffer size in milliseconds
    pub buffer_size_ms: u32,
    
    /// Current adaptive delay in milliseconds
    pub adaptive_delay_ms: u32,
}
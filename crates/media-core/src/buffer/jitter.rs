//! Adaptive Jitter Buffer
//!
//! This module implements an adaptive jitter buffer for VoIP that handles
//! packet reordering, network jitter compensation, and smooth audio playback.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, warn, trace};

use crate::error::{Result, BufferError};
use crate::types::{AudioFrame, MediaPacket};
use crate::quality::metrics::QualityMetrics;

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

/// Buffered frame with metadata
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

/// Adaptive jitter buffer for smooth audio playback
pub struct JitterBuffer {
    /// Configuration
    config: JitterBufferConfig,
    /// Buffered frames (indexed by sequence number)
    buffer: RwLock<BTreeMap<u16, BufferedFrame>>,
    /// Current target buffer depth
    target_depth: RwLock<usize>,
    /// Statistics
    stats: RwLock<JitterBufferStats>,
    /// Next expected sequence number
    next_sequence: RwLock<Option<u16>>,
    /// Jitter calculation state
    jitter_state: RwLock<JitterState>,
    /// Last playout timestamp
    last_playout_time: RwLock<Option<Instant>>,
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
}

impl JitterBuffer {
    /// Create a new jitter buffer
    pub fn new(config: JitterBufferConfig) -> Self {
        debug!("Creating JitterBuffer with config: {:?}", config);
        
        Self {
            target_depth: RwLock::new(config.initial_depth),
            config,
            buffer: RwLock::new(BTreeMap::new()),
            stats: RwLock::new(JitterBufferStats::default()),
            next_sequence: RwLock::new(None),
            jitter_state: RwLock::new(JitterState::default()),
            last_playout_time: RwLock::new(None),
        }
    }
    
    /// Add a frame to the buffer
    pub async fn add_frame(&self, packet: MediaPacket, frame: AudioFrame) -> Result<()> {
        let arrival_time = Instant::now();
        
        // Update jitter calculation
        self.update_jitter(packet.timestamp, arrival_time).await;
        
        // Check if packet is too late
        if self.is_packet_too_late(packet.sequence_number, arrival_time).await? {
            let mut stats = self.stats.write().await;
            stats.frames_dropped_late += 1;
            trace!("Dropped late packet: seq={}", packet.sequence_number);
            return Ok(());
        }
        
        // Create buffered frame
        let buffered_frame = BufferedFrame {
            frame,
            arrival_time,
            rtp_timestamp: packet.timestamp,
            sequence_number: packet.sequence_number,
        };
        
        // Add to buffer
        {
            let mut buffer = self.buffer.write().await;
            
            // Check for buffer overflow
            if buffer.len() >= self.config.max_depth {
                // Drop oldest frame
                if let Some((oldest_seq, _)) = buffer.iter().next() {
                    let oldest_seq = *oldest_seq;
                    buffer.remove(&oldest_seq);
                    let mut stats = self.stats.write().await;
                    stats.frames_dropped_overflow += 1;
                    warn!("Buffer overflow, dropped frame: seq={}", oldest_seq);
                }
            }
            
            buffer.insert(packet.sequence_number, buffered_frame);
        }
        
        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.frames_received += 1;
            stats.current_depth = self.buffer.read().await.len();
        }
        
        trace!("Added frame to jitter buffer: seq={}, buffer_depth={}", 
               packet.sequence_number, self.get_current_depth().await);
        
        Ok(())
    }
    
    /// Get the next frame for playout
    pub async fn get_next_frame(&self) -> Result<Option<AudioFrame>> {
        let next_seq = {
            let next_sequence = self.next_sequence.read().await;
            *next_sequence
        };
        
        // If we don't have a next sequence, start with the oldest in buffer
        let target_seq = if let Some(seq) = next_seq {
            seq
        } else {
            let buffer = self.buffer.read().await;
            if let Some((&first_seq, _)) = buffer.iter().next() {
                let mut next_sequence = self.next_sequence.write().await;
                *next_sequence = Some(first_seq);
                first_seq
            } else {
                return Ok(None); // Buffer empty
            }
        };
        
        // Check if we're ready for playout
        if !self.is_ready_for_playout().await {
            return Ok(None); // Not enough frames buffered
        }
        
        // Try to get the next expected frame
        let frame = {
            let mut buffer = self.buffer.write().await;
            if let Some(buffered_frame) = buffer.remove(&target_seq) {
                Some(buffered_frame.frame)
            } else {
                // Frame missing, check for gaps
                self.handle_missing_frame(target_seq, &mut buffer).await
            }
        };
        
        // Update next sequence number
        {
            let mut next_sequence = self.next_sequence.write().await;
            *next_sequence = Some(target_seq.wrapping_add(1));
        }
        
        // Update statistics and timestamps
        if frame.is_some() {
            let mut stats = self.stats.write().await;
            stats.frames_played += 1;
            
            let mut last_playout = self.last_playout_time.write().await;
            *last_playout = Some(Instant::now());
        }
        
        // Adapt buffer if needed
        self.adapt_buffer_depth().await?;
        
        Ok(frame)
    }
    
    /// Check if buffer is ready for playout
    async fn is_ready_for_playout(&self) -> bool {
        let buffer = self.buffer.read().await;
        let target_depth = *self.target_depth.read().await;
        
        buffer.len() >= target_depth
    }
    
    /// Handle missing frame (gap in sequence)
    async fn handle_missing_frame(
        &self,
        missing_seq: u16,
        buffer: &mut BTreeMap<u16, BufferedFrame>,
    ) -> Option<AudioFrame> {
        // Look ahead for the next available frame
        let next_available = buffer
            .range(missing_seq.wrapping_add(1)..)
            .next()
            .map(|(&seq, frame)| (seq, frame.clone()));
        
        if let Some((next_seq, buffered_frame)) = next_available {
            // Use the next available frame and remove it
            buffer.remove(&next_seq);
            
            warn!("Missing frame seq={}, using seq={} instead", missing_seq, next_seq);
            
            // Could implement packet loss concealment here
            Some(buffered_frame.frame)
        } else {
            // No frames available, underrun
            let mut stats = self.stats.write().await;
            stats.underrun_count += 1;
            warn!("Buffer underrun at seq={}", missing_seq);
            None
        }
    }
    
    /// Update jitter calculation (RFC 3550)
    async fn update_jitter(&self, rtp_timestamp: u32, arrival_time: Instant) {
        let mut jitter_state = self.jitter_state.write().await;
        
        if let (Some(prev_arrival), Some(prev_timestamp)) = 
            (jitter_state.prev_arrival, jitter_state.prev_timestamp) {
            
            // Calculate arrival time difference (in RTP timestamp units)
            let arrival_diff = arrival_time.duration_since(prev_arrival).as_millis() as i32;
            let timestamp_diff = rtp_timestamp.wrapping_sub(prev_timestamp) as i32;
            
            // RFC 3550 jitter calculation
            let d = (arrival_diff - timestamp_diff).abs() as f32;
            jitter_state.jitter += (d - jitter_state.jitter) / 16.0;
            
            // Update statistics
            {
                let mut stats = self.stats.write().await;
                stats.average_jitter_ms = jitter_state.jitter;
                if jitter_state.jitter > stats.max_jitter_ms {
                    stats.max_jitter_ms = jitter_state.jitter;
                }
            }
        }
        
        jitter_state.prev_arrival = Some(arrival_time);
        jitter_state.prev_timestamp = Some(rtp_timestamp);
    }
    
    /// Check if packet is too late to be useful
    async fn is_packet_too_late(&self, sequence_number: u16, arrival_time: Instant) -> Result<bool> {
        let next_seq = self.next_sequence.read().await;
        
        if let Some(expected_seq) = *next_seq {
            // Check sequence number distance
            let seq_distance = sequence_number.wrapping_sub(expected_seq);
            
            // If sequence number is very old (wrapped around), it's too late
            if seq_distance > 32768 {
                return Ok(true);
            }
        }
        
        // Check time-based lateness
        if let Some(last_playout) = *self.last_playout_time.read().await {
            let age = arrival_time.duration_since(last_playout);
            if age.as_millis() > self.config.max_late_packet_age_ms as u128 {
                return Ok(true);
            }
        }
        
        Ok(false)
    }
    
    /// Adapt buffer depth based on network conditions
    async fn adapt_buffer_depth(&self) -> Result<()> {
        let jitter = {
            let jitter_state = self.jitter_state.read().await;
            jitter_state.jitter
        };
        
        let current_target = *self.target_depth.read().await;
        let frame_duration = self.config.frame_duration_ms as f32;
        
        // Calculate desired depth based on jitter
        let jitter_frames = (jitter / frame_duration).ceil() as usize;
        let desired_depth = match self.config.adaptation_strategy {
            AdaptationStrategy::Conservative => {
                (current_target + jitter_frames * 2).max(self.config.min_depth)
            }
            AdaptationStrategy::Balanced => {
                (jitter_frames * 3 + 2).max(self.config.min_depth)
            }
            AdaptationStrategy::Aggressive => {
                (jitter_frames * 2 + 1).max(self.config.min_depth)
            }
        }.min(self.config.max_depth);
        
        // Only adapt if change is significant
        if desired_depth != current_target && 
           (desired_depth as i32 - current_target as i32).abs() > 1 {
            
            let mut target_depth = self.target_depth.write().await;
            *target_depth = desired_depth;
            
            let mut stats = self.stats.write().await;
            stats.adaptation_count += 1;
            
            debug!("Adapted jitter buffer depth: {} -> {} (jitter: {:.1}ms)", 
                   current_target, desired_depth, jitter);
        }
        
        Ok(())
    }
    
    /// Get current buffer depth
    pub async fn get_current_depth(&self) -> usize {
        self.buffer.read().await.len()
    }
    
    /// Get target buffer depth
    pub async fn get_target_depth(&self) -> usize {
        *self.target_depth.read().await
    }
    
    /// Get buffer statistics
    pub async fn get_statistics(&self) -> JitterBufferStats {
        let mut stats = self.stats.read().await.clone();
        stats.current_depth = self.get_current_depth().await;
        stats
    }
    
    /// Reset the buffer
    pub async fn reset(&self) {
        {
            let mut buffer = self.buffer.write().await;
            buffer.clear();
        }
        
        {
            let mut next_sequence = self.next_sequence.write().await;
            *next_sequence = None;
        }
        
        {
            let mut target_depth = self.target_depth.write().await;
            *target_depth = self.config.initial_depth;
        }
        
        {
            let mut jitter_state = self.jitter_state.write().await;
            *jitter_state = JitterState::default();
        }
        
        debug!("Jitter buffer reset");
    }
    
    /// Check if buffer is empty
    pub async fn is_empty(&self) -> bool {
        self.buffer.read().await.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SampleRate, MediaType};
    
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
        
        let packet2 = MediaPacket { sequence_number: 1001, timestamp: 8160, ..packet1.clone() };
        let packet3 = MediaPacket { sequence_number: 1002, timestamp: 8320, ..packet1.clone() };
        
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
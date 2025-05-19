use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, RwLock};
use tokio::sync::Notify;
use tracing::{debug, warn, info, error};

use crate::api::buffer::{
    MediaBuffer, MediaBufferConfig, BufferError, BufferStats
};
use crate::api::transport::MediaFrame;
use crate::buffer::jitter::AdaptiveJitterBuffer;
use crate::buffer::transmit::TransmitBuffer;

/// Default implementation of MediaBuffer
pub struct DefaultMediaBuffer {
    /// Jitter buffer for incoming frames
    jitter_buffer: RwLock<AdaptiveJitterBuffer>,
    
    /// Transmit buffer for outgoing frames
    transmit_buffer: Option<Arc<TransmitBuffer>>,
    
    /// Configuration for the buffer
    config: RwLock<MediaBufferConfig>,
    
    /// Queue for frames extracted from the jitter buffer
    frame_queue: Mutex<VecDeque<MediaFrame>>,
    
    /// Signal for frame availability
    frame_signal: Arc<Notify>,
    
    /// Statistics
    stats: RwLock<BufferStats>,
}

impl DefaultMediaBuffer {
    /// Create a new DefaultMediaBuffer
    pub fn new(config: MediaBufferConfig) -> Result<Arc<Self>, BufferError> {
        // Create jitter buffer configuration
        let jitter_config = crate::buffer::jitter::JitterBufferConfig {
            initial_size_ms: config.min_delay_ms,
            min_size_ms: config.min_delay_ms,
            max_size_ms: config.max_delay_ms,
            max_out_of_order: config.max_packet_count,
            adaptive: config.adaptive,
            ..Default::default()
        };
        
        // Create jitter buffer with configuration
        let jitter_buffer = AdaptiveJitterBuffer::new(jitter_config);
        
        // Create initial stats
        let stats = BufferStats {
            current_delay_ms: config.min_delay_ms,
            packet_count: 0,
            max_delay_seen_ms: 0,
            min_delay_seen_ms: config.min_delay_ms,
            late_packet_count: 0,
            overflow_discard_count: 0,
            average_occupancy: 0.0,
            underrun_count: 0,
        };
        
        Ok(Arc::new(Self {
            jitter_buffer: RwLock::new(jitter_buffer),
            transmit_buffer: None, // Initialize if needed
            config: RwLock::new(config),
            frame_queue: Mutex::new(VecDeque::with_capacity(100)),
            frame_signal: Arc::new(Notify::new()),
            stats: RwLock::new(stats),
        }))
    }
    
    /// Update statistics based on current state
    async fn update_stats(&self) {
        let jitter = self.jitter_buffer.read().await;
        let mut stats = self.stats.write().await;
        
        // Update current delay
        let jitter_stats = jitter.get_stats();
        stats.current_delay_ms = jitter_stats.buffer_size_ms;
        
        // Update packet count
        stats.packet_count = jitter_stats.buffered_packets;
        
        // Update max/min delay
        stats.max_delay_seen_ms = stats.max_delay_seen_ms.max(stats.current_delay_ms);
        stats.min_delay_seen_ms = stats.min_delay_seen_ms.min(stats.current_delay_ms);
        
        // Update occupancy
        let capacity = jitter_stats.buffered_packets;
        let max_capacity = self.config.read().await.max_packet_count;
        if max_capacity > 0 {
            stats.average_occupancy = (stats.average_occupancy * 0.95) + 
                                     (capacity as f32 / max_capacity as f32 * 100.0 * 0.05);
        }
    }
    
    /// Convert a media frame to internal packet format
    fn frame_to_packet(&self, frame: &MediaFrame) -> crate::packet::rtp::RtpPacket {
        // Create header from frame data
        let mut header = crate::packet::header::RtpHeader::new(
            frame.payload_type,
            frame.sequence,
            frame.timestamp,
            frame.ssrc,
        );
        header.marker = frame.marker;
        
        // Create payload
        let payload = bytes::Bytes::copy_from_slice(&frame.data);
        
        // Create packet
        crate::packet::rtp::RtpPacket::new(header, payload)
    }
    
    /// Convert internal packet to media frame
    fn packet_to_frame(&self, packet: crate::packet::rtp::RtpPacket, frame_type: crate::api::transport::MediaFrameType) -> MediaFrame {
        MediaFrame {
            frame_type,
            data: packet.payload.to_vec(),
            timestamp: packet.header.timestamp,
            sequence: packet.header.sequence_number,
            marker: packet.header.marker,
            payload_type: packet.header.payload_type,
            ssrc: packet.header.ssrc,
        }
    }
}

#[async_trait::async_trait]
impl MediaBuffer for DefaultMediaBuffer {
    async fn put_frame(&self, frame: MediaFrame) -> Result<(), BufferError> {
        // Convert frame to packet for internal jitter buffer
        let packet = self.frame_to_packet(&frame);
        
        // Put packet in jitter buffer
        let mut jitter = self.jitter_buffer.write().await;
        let added = jitter.add_packet(packet).await;
        if added {
            // Frame added successfully
            drop(jitter); // Release the lock before notification
            
            // Signal that a frame was added
            self.frame_signal.notify_one();
            
            // Update stats
            self.update_stats().await;
            
            Ok(())
        } else {
            // Failed to add packet (full buffer or other reason)
            drop(jitter); // Release the lock before updating stats
            let mut stats = self.stats.write().await;
            stats.overflow_discard_count += 1;
            Err(BufferError::BufferFull)
        }
    }
    
    async fn get_frame(&self, timeout: Duration) -> Result<MediaFrame, BufferError> {
        // Check if we already have frames queued
        {
            let mut queue = self.frame_queue.lock().await;
            if let Some(frame) = queue.pop_front() {
                return Ok(frame);
            }
        }
        
        // No frames queued, try to get from jitter buffer
        let start_time = Instant::now();
        
        loop {
            // Check if we've exceeded the timeout
            if start_time.elapsed() >= timeout {
                // Timeout occurred without getting a frame
                let mut stats = self.stats.write().await;
                stats.underrun_count += 1;
                return Err(BufferError::BufferEmpty);
            }
            
            // Try to get a packet from the jitter buffer
            let mut jitter = self.jitter_buffer.write().await;
            match jitter.get_next_packet().await {
                Some(packet) => {
                    // Determine frame type based on payload type (simplified)
                    let frame_type = {
                        if packet.header.payload_type >= 96 {
                            // Dynamic payload types
                            crate::api::transport::MediaFrameType::Video
                        } else {
                            // Static payload types
                            crate::api::transport::MediaFrameType::Audio
                        }
                    };
                    
                    // Convert to media frame
                    let frame = self.packet_to_frame(packet, frame_type);
                    return Ok(frame);
                },
                None => {
                    // No packet available, release the lock and wait
                    drop(jitter);
                    
                    // Wait for notification or timeout
                    let remaining = timeout.checked_sub(start_time.elapsed())
                        .unwrap_or(Duration::from_millis(1));
                    
                    // Wait for notification or timeout
                    tokio::select! {
                        _ = self.frame_signal.notified() => {
                            // A frame might be available now, loop and try again
                            continue;
                        }
                        _ = tokio::time::sleep(remaining) => {
                            // Timeout occurred
                            let mut stats = self.stats.write().await;
                            stats.underrun_count += 1;
                            return Err(BufferError::BufferEmpty);
                        }
                    }
                }
            }
        }
    }
    
    async fn get_stats(&self) -> BufferStats {
        // Update stats first
        self.update_stats().await;
        
        // Return a copy of the current stats
        self.stats.read().await.clone()
    }
    
    async fn reset(&self) -> Result<(), BufferError> {
        // Reset jitter buffer
        {
            let mut jitter = self.jitter_buffer.write().await;
            jitter.reset();
        }
        
        // Clear frame queue
        {
            let mut queue = self.frame_queue.lock().await;
            queue.clear();
        }
        
        // Reset stats
        {
            let mut stats = self.stats.write().await;
            let config = self.config.read().await;
            
            stats.current_delay_ms = config.min_delay_ms;
            stats.packet_count = 0;
            stats.average_occupancy = 0.0;
            // Don't reset counters, just current state
        }
        
        Ok(())
    }
    
    async fn flush(&self) -> Result<Vec<MediaFrame>, BufferError> {
        let mut frames = Vec::new();
        
        // First add any queued frames
        {
            let mut queue = self.frame_queue.lock().await;
            frames.extend(queue.drain(..));
        }
        
        // Then drain the jitter buffer
        {
            let mut jitter = self.jitter_buffer.write().await;
            
            // Keep getting packets until empty
            while let Some(packet) = jitter.get_next_packet().await {
                // Determine frame type based on payload type (simplified)
                let frame_type = {
                    if packet.header.payload_type >= 96 {
                        // Dynamic payload types
                        crate::api::transport::MediaFrameType::Video
                    } else {
                        // Static payload types
                        crate::api::transport::MediaFrameType::Audio
                    }
                };
                
                // Convert to media frame
                let frame = self.packet_to_frame(packet, frame_type);
                frames.push(frame);
            }
        }
        
        // Update stats
        self.update_stats().await;
        
        Ok(frames)
    }
    
    async fn update_config(&self, config: MediaBufferConfig) -> Result<(), BufferError> {
        // Validate configuration
        if config.min_delay_ms > config.max_delay_ms {
            return Err(BufferError::ConfigurationError(
                "Minimum delay must be less than or equal to maximum delay".to_string()
            ));
        }
        
        if config.max_packet_count == 0 {
            return Err(BufferError::ConfigurationError(
                "Maximum packet count must be greater than zero".to_string()
            ));
        }
        
        // Create new jitter buffer configuration
        let jitter_config = crate::buffer::jitter::JitterBufferConfig {
            initial_size_ms: config.min_delay_ms,
            min_size_ms: config.min_delay_ms,
            max_size_ms: config.max_delay_ms,
            max_out_of_order: config.max_packet_count,
            adaptive: config.adaptive,
            ..Default::default()
        };
        
        // Update jitter buffer configuration - recreate it with new config
        {
            let mut jitter = self.jitter_buffer.write().await;
            *jitter = AdaptiveJitterBuffer::new(jitter_config);
        }
        
        // Store new configuration
        {
            let mut cfg = self.config.write().await;
            *cfg = config;
        }
        
        // Update stats
        self.update_stats().await;
        
        Ok(())
    }
} 
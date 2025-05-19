//! Buffer API
//!
//! This module provides a simplified interface for buffer management,
//! including jitter buffer and transmit buffer configuration.

use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

use crate::api::transport::MediaFrame;

/// Error types for buffer operations
#[derive(Error, Debug)]
pub enum BufferError {
    /// Buffer is full
    #[error("Buffer is full")]
    BufferFull,
    
    /// Buffer is empty
    #[error("Buffer is empty")]
    BufferEmpty,
    
    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigurationError(String),
    
    /// Other buffer operation error
    #[error("Buffer operation error: {0}")]
    OperationError(String),
}

/// Network condition preset for buffer configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkPreset {
    /// Minimal latency, good for LAN
    LowLatency,
    
    /// Balanced preset, good for stable broadband
    Balanced,
    
    /// Resilient preset, good for mobile or unstable networks
    Resilient,
    
    /// Maximum protection, for very unstable networks
    HighProtection,
}

/// Media buffer configuration
#[derive(Debug, Clone)]
pub struct MediaBufferConfig {
    /// Jitter buffer minimum delay in milliseconds
    pub min_delay_ms: u32,
    
    /// Jitter buffer maximum delay in milliseconds
    pub max_delay_ms: u32,
    
    /// Whether to use adaptive jitter buffer sizing
    pub adaptive: bool,
    
    /// Target jitter buffer occupancy percentage (0-100)
    pub target_occupancy: u8,
    
    /// Maximum number of packets that can be stored
    pub max_packet_count: usize,
    
    /// Transmit buffer maximum latency in milliseconds
    pub transmit_max_latency_ms: u32,
    
    /// Whether to prioritize I-frames for video
    pub prioritize_keyframes: bool,
}

impl Default for MediaBufferConfig {
    fn default() -> Self {
        Self {
            min_delay_ms: 20,
            max_delay_ms: 120,
            adaptive: true,
            target_occupancy: 50,
            max_packet_count: 1000,
            transmit_max_latency_ms: 100,
            prioritize_keyframes: true,
        }
    }
}

/// Builder for MediaBufferConfig
pub struct MediaBufferConfigBuilder {
    config: MediaBufferConfig,
}

impl MediaBufferConfigBuilder {
    /// Create a new builder with default configuration
    pub fn new() -> Self {
        Self {
            config: MediaBufferConfig::default(),
        }
    }
    
    /// Apply a network preset
    pub fn preset(mut self, preset: NetworkPreset) -> Self {
        match preset {
            NetworkPreset::LowLatency => {
                self.config.min_delay_ms = 10;
                self.config.max_delay_ms = 50;
                self.config.adaptive = false;
                self.config.target_occupancy = 30;
            },
            NetworkPreset::Balanced => {
                // Default values are already balanced
            },
            NetworkPreset::Resilient => {
                self.config.min_delay_ms = 50;
                self.config.max_delay_ms = 200;
                self.config.adaptive = true;
                self.config.target_occupancy = 70;
            },
            NetworkPreset::HighProtection => {
                self.config.min_delay_ms = 100;
                self.config.max_delay_ms = 500;
                self.config.adaptive = true;
                self.config.target_occupancy = 80;
            },
        }
        self
    }
    
    /// Set audio specific preset
    pub fn audio(mut self) -> Self {
        // Audio typically needs less buffering but is more sensitive to jitter
        self.config.min_delay_ms = 20;
        self.config.max_delay_ms = 120;
        self.config.target_occupancy = 50;
        self
    }
    
    /// Set video specific preset
    pub fn video(mut self) -> Self {
        // Video can tolerate more latency but needs more buffer space
        self.config.min_delay_ms = 50;
        self.config.max_delay_ms = 300;
        self.config.target_occupancy = 60;
        self.config.max_packet_count = 5000;
        self.config.prioritize_keyframes = true;
        self
    }
    
    /// Set minimum delay in milliseconds
    pub fn min_delay_ms(mut self, delay: u32) -> Self {
        self.config.min_delay_ms = delay;
        self
    }
    
    /// Set maximum delay in milliseconds
    pub fn max_delay_ms(mut self, delay: u32) -> Self {
        self.config.max_delay_ms = delay;
        self
    }
    
    /// Set whether to use adaptive jitter buffer sizing
    pub fn adaptive(mut self, adaptive: bool) -> Self {
        self.config.adaptive = adaptive;
        self
    }
    
    /// Set target buffer occupancy percentage (0-100)
    pub fn target_occupancy(mut self, occupancy: u8) -> Self {
        self.config.target_occupancy = occupancy.min(100);
        self
    }
    
    /// Set maximum packet count
    pub fn max_packet_count(mut self, count: usize) -> Self {
        self.config.max_packet_count = count;
        self
    }
    
    /// Set transmit buffer maximum latency in milliseconds
    pub fn transmit_max_latency_ms(mut self, latency: u32) -> Self {
        self.config.transmit_max_latency_ms = latency;
        self
    }
    
    /// Set whether to prioritize keyframes for video
    pub fn prioritize_keyframes(mut self, prioritize: bool) -> Self {
        self.config.prioritize_keyframes = prioritize;
        self
    }
    
    /// Build the configuration
    pub fn build(self) -> Result<MediaBufferConfig, BufferError> {
        // Validate configuration
        if self.config.min_delay_ms > self.config.max_delay_ms {
            return Err(BufferError::ConfigurationError(
                "Minimum delay must be less than or equal to maximum delay".to_string()
            ));
        }
        
        if self.config.max_packet_count == 0 {
            return Err(BufferError::ConfigurationError(
                "Maximum packet count must be greater than zero".to_string()
            ));
        }
        
        Ok(self.config)
    }
}

/// Buffer statistics
#[derive(Debug, Clone)]
pub struct BufferStats {
    /// Current buffer delay in milliseconds
    pub current_delay_ms: u32,
    
    /// Current number of packets in buffer
    pub packet_count: usize,
    
    /// Maximum delay seen in the current session
    pub max_delay_seen_ms: u32,
    
    /// Minimum delay seen in the current session
    pub min_delay_seen_ms: u32,
    
    /// Number of late packets (arrived too late to be used)
    pub late_packet_count: u64,
    
    /// Number of packets discarded due to buffer overflow
    pub overflow_discard_count: u64,
    
    /// Average occupancy percentage
    pub average_occupancy: f32,
    
    /// Number of buffer underruns
    pub underrun_count: u64,
}

/// Media buffer interface for jitter buffering and transmit buffering
pub trait MediaBuffer: Send + Sync {
    /// Put a media frame into the buffer
    fn put_frame(&self, frame: MediaFrame) -> Result<(), BufferError>;
    
    /// Get the next media frame from the buffer, waiting up to the specified timeout
    fn get_frame(&self, timeout: Duration) -> Result<MediaFrame, BufferError>;
    
    /// Get current buffer statistics
    fn get_stats(&self) -> BufferStats;
    
    /// Reset the buffer, discarding all frames
    fn reset(&self) -> Result<(), BufferError>;
    
    /// Flush the buffer, returning all frames in order
    fn flush(&self) -> Result<Vec<MediaFrame>, BufferError>;
    
    /// Update buffer configuration
    fn update_config(&self, config: MediaBufferConfig) -> Result<(), BufferError>;
}

/// Factory for creating MediaBuffer instances
pub struct MediaBufferFactory;

impl MediaBufferFactory {
    /// Create a new MediaBuffer
    pub fn create_buffer(
        config: MediaBufferConfig,
    ) -> Result<Arc<dyn MediaBuffer>, BufferError> {
        // This is a placeholder that will be implemented to create the actual buffer
        // based on the internal buffer implementation
        todo!("Implement buffer creation using internal components")
    }
} 
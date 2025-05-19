use std::time::{Duration, Instant};
use std::cmp::{min, max};
use std::collections::VecDeque;

use bytes::Bytes;
use tracing::{debug, trace, warn};

use crate::error::{Error, Result};
use crate::buffer::common::JitterCalculator;

/// Configuration for the adaptive buffer
#[derive(Debug, Clone)]
pub struct AdaptiveBufferConfig {
    /// Initial buffer size in milliseconds
    pub initial_size_ms: u32,
    /// Minimum buffer size in milliseconds
    pub min_size_ms: u32,
    /// Maximum buffer size in milliseconds
    pub max_size_ms: u32,
    /// Target occupancy (0.0-1.0)
    pub target_occupancy: f32,
    /// How quickly to adapt buffer size (higher = faster) [0.0-1.0]
    pub adaptation_rate: f32,
    /// How often to check for adaptation (in milliseconds)
    pub adaptation_interval_ms: u32,
    /// Whether to use variable bitrate adaptation
    pub use_vbr: bool,
    /// Maximum data size in bytes before scaling buffer
    pub max_bytes: usize,
}

impl Default for AdaptiveBufferConfig {
    fn default() -> Self {
        Self {
            initial_size_ms: 100,
            min_size_ms: 20,
            max_size_ms: 500,
            target_occupancy: 0.5,
            adaptation_rate: 0.1,
            adaptation_interval_ms: 500, // Check every 500ms
            use_vbr: true,
            max_bytes: 1024 * 100, // 100KB
        }
    }
}

/// Adaptive buffer for managing media data with variable network conditions
pub struct AdaptiveBuffer {
    /// Configuration
    config: AdaptiveBufferConfig,
    /// Buffer data
    buffer: VecDeque<Bytes>,
    /// Current buffer size in milliseconds
    current_size_ms: u32,
    /// Total bytes in buffer
    bytes_in_buffer: usize,
    /// Jitter calculator
    jitter_calculator: JitterCalculator,
    /// Last adaptation time
    last_adaptation: Instant,
    /// Clock rate (for timestamp calculations)
    clock_rate: u32,
    /// Last added timestamp
    last_timestamp: Option<u32>,
    /// Duration of data in buffer (in timestamp units)
    buffer_duration: u32,
}

impl AdaptiveBuffer {
    /// Create a new adaptive buffer with the given configuration
    pub fn new(config: AdaptiveBufferConfig, clock_rate: u32) -> Self {
        Self {
            config,
            buffer: VecDeque::new(),
            current_size_ms: config.initial_size_ms,
            bytes_in_buffer: 0,
            jitter_calculator: JitterCalculator::new(),
            last_adaptation: Instant::now(),
            clock_rate,
            last_timestamp: None,
            buffer_duration: 0,
        }
    }
    
    /// Create a new adaptive buffer with default configuration
    pub fn new_default(clock_rate: u32) -> Self {
        Self::new(AdaptiveBufferConfig::default(), clock_rate)
    }
    
    /// Add data to the buffer
    pub fn add(&mut self, data: Bytes, timestamp: u32) -> Result<()> {
        // Update jitter calculation if we have previous timestamps
        if let Some(last_ts) = self.last_timestamp {
            let arrival_time = Instant::now();
            self.jitter_calculator.update(timestamp, arrival_time, self.clock_rate);
            
            // Update buffer duration
            let duration = timestamp.wrapping_sub(last_ts);
            self.buffer_duration = self.buffer_duration.wrapping_add(duration);
        }
        
        // Store this timestamp
        self.last_timestamp = Some(timestamp);
        
        // Add to buffer
        self.buffer.push_back(data.clone());
        self.bytes_in_buffer += data.len();
        
        // Check if we need to adapt the buffer
        let now = Instant::now();
        if now.duration_since(self.last_adaptation).as_millis() >= self.config.adaptation_interval_ms as u128 {
            self.adapt_buffer();
            self.last_adaptation = now;
        }
        
        // Check if buffer is too large
        self.check_buffer_size();
        
        Ok(())
    }
    
    /// Get data from the buffer if available
    pub fn get(&mut self) -> Option<Bytes> {
        // Check if we should return data based on buffer filling
        if self.should_output_data() {
            if let Some(data) = self.buffer.pop_front() {
                // Update buffer stats
                self.bytes_in_buffer = self.bytes_in_buffer.saturating_sub(data.len());
                
                // Update buffer duration (rough estimate assuming constant sizes)
                if !self.buffer.is_empty() {
                    let frames_remaining = self.buffer.len() as f32;
                    let frames_total = frames_remaining + 1.0;
                    let duration_per_frame = self.buffer_duration as f32 / frames_total;
                    self.buffer_duration = (duration_per_frame * frames_remaining) as u32;
                } else {
                    self.buffer_duration = 0;
                }
                
                return Some(data);
            }
        }
        
        None
    }
    
    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
    
    /// Get the current buffer size in milliseconds
    pub fn size_ms(&self) -> u32 {
        self.current_size_ms
    }
    
    /// Get the current buffer occupancy (0.0-1.0)
    pub fn occupancy(&self) -> f32 {
        if self.current_size_ms == 0 {
            return 0.0;
        }
        
        // Calculate how full the buffer is in time units
        let buffer_duration_ms = self.buffer_duration_ms();
        let occupancy = buffer_duration_ms as f32 / self.current_size_ms as f32;
        
        // Clamp to 0.0-1.0 range
        occupancy.clamp(0.0, 1.0)
    }
    
    /// Get the number of bytes in the buffer
    pub fn bytes_in_buffer(&self) -> usize {
        self.bytes_in_buffer
    }
    
    /// Get the number of items in the buffer
    pub fn items_in_buffer(&self) -> usize {
        self.buffer.len()
    }
    
    /// Reset the buffer
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.bytes_in_buffer = 0;
        self.current_size_ms = self.config.initial_size_ms;
        self.jitter_calculator.reset();
        self.last_adaptation = Instant::now();
        self.last_timestamp = None;
        self.buffer_duration = 0;
    }
    
    /// Set the target buffer size in milliseconds
    pub fn set_target_size(&mut self, size_ms: u32) -> Result<()> {
        if size_ms < self.config.min_size_ms || size_ms > self.config.max_size_ms {
            return Err(Error::InvalidParameter(format!(
                "Buffer size must be between {} and {} ms", 
                self.config.min_size_ms, 
                self.config.max_size_ms
            )));
        }
        
        self.current_size_ms = size_ms;
        
        Ok(())
    }
    
    /// Check if the buffer is too large and trim if needed
    fn check_buffer_size(&mut self) {
        // Check if buffer has too many bytes
        if self.bytes_in_buffer > self.config.max_bytes {
            warn!("Buffer too large ({}KB), trimming to keep memory usage in check", 
                  self.bytes_in_buffer / 1024);
            
            // Remove oldest data until we're under the threshold
            let target_bytes = self.config.max_bytes / 2; // Aim for half the max
            while self.bytes_in_buffer > target_bytes && !self.buffer.is_empty() {
                if let Some(data) = self.buffer.pop_front() {
                    self.bytes_in_buffer = self.bytes_in_buffer.saturating_sub(data.len());
                    
                    // Also update buffer duration (rough estimate)
                    if !self.buffer.is_empty() && self.buffer_duration > 0 {
                        let remaining_ratio = self.buffer.len() as f32 / (self.buffer.len() + 1) as f32;
                        self.buffer_duration = (self.buffer_duration as f32 * remaining_ratio) as u32;
                    }
                }
            }
        }
    }
    
    /// Determine if we should output data based on buffer fullness
    fn should_output_data(&self) -> bool {
        // Always output if we have more than target occupancy
        let occupancy = self.occupancy();
        
        if occupancy >= self.config.target_occupancy {
            return true;
        }
        
        // If buffer is too small, don't output yet unless we have a lot of data
        if occupancy < 0.2 && self.buffer.len() < 5 {
            return false;
        }
        
        // If we have a very large buffer, output to avoid memory issues
        if self.bytes_in_buffer > self.config.max_bytes / 2 {
            return true;
        }
        
        // Default to true if we have any data
        !self.buffer.is_empty()
    }
    
    /// Adapt the buffer size based on network conditions
    fn adapt_buffer(&mut self) {
        // Only adapt if we have enough data
        if self.buffer.len() < 2 {
            return;
        }
        
        // Get current jitter in milliseconds
        let jitter_ms = self.jitter_calculator.jitter_ms();
        
        // Calculate target buffer size based on jitter
        // We want buffer size to be jitter * safety_factor
        let safety_factor = 3.0;
        let target_size_ms = (jitter_ms * safety_factor) as u32;
        
        // Clamp to configured min/max
        let target_size_ms = max(
            self.config.min_size_ms,
            min(target_size_ms, self.config.max_size_ms)
        );
        
        // If using VBR, also consider occupancy
        let mut final_target_ms = target_size_ms;
        if self.config.use_vbr {
            let occupancy = self.occupancy();
            
            // If buffer is too empty, increase size
            if occupancy < self.config.target_occupancy * 0.5 {
                final_target_ms = (final_target_ms as f32 * 1.5) as u32;
            }
            
            // If buffer is too full, decrease size
            if occupancy > self.config.target_occupancy * 1.5 {
                final_target_ms = (final_target_ms as f32 * 0.8) as u32;
            }
            
            // Clamp again
            final_target_ms = max(
                self.config.min_size_ms,
                min(final_target_ms, self.config.max_size_ms)
            );
        }
        
        // Gradual adaptation
        let delta = final_target_ms as f32 - self.current_size_ms as f32;
        let adjustment = delta * self.config.adaptation_rate;
        
        // Only adapt if change is significant
        if adjustment.abs() >= 1.0 {
            self.current_size_ms = (self.current_size_ms as f32 + adjustment) as u32;
            
            trace!("Adapted buffer size: jitter={:.1}ms, target={}ms, actual={}ms, occupancy={:.2}", 
                   jitter_ms, final_target_ms, self.current_size_ms, self.occupancy());
        }
    }
    
    /// Get the duration of data in the buffer in milliseconds
    fn buffer_duration_ms(&self) -> u32 {
        // Convert from timestamp units to milliseconds
        self.buffer_duration * 1000 / self.clock_rate
    }
} 
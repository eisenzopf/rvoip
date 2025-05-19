use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use bytes::Bytes;

use tracing::{debug, trace};

use crate::AudioBuffer;
use crate::error::{Error, Result};
use crate::processing::audio::AudioProcessor;
use crate::codec::audio::common::AudioFormat;
use crate::processing::audio::vad::{VoiceActivityDetector, VadState};

/// A pipeline of audio processors that can be applied in sequence
pub struct AudioProcessingPipeline {
    /// Audio processors in the pipeline
    processors: Vec<Arc<Mutex<dyn AudioProcessor>>>,
    /// Enabled state of each processor
    enabled: Vec<bool>,
    /// Name of each processor for configuration
    names: Vec<String>,
}

impl AudioProcessingPipeline {
    /// Create a new empty audio processing pipeline
    pub fn new() -> Self {
        Self {
            processors: Vec::new(),
            enabled: Vec::new(),
            names: Vec::new(),
        }
    }
    
    /// Add a processor to the pipeline
    pub fn add<T: AudioProcessor + 'static>(&mut self, name: &str, processor: T) {
        self.processors.push(Arc::new(Mutex::new(processor)));
        self.enabled.push(true);
        self.names.push(name.to_string());
    }
    
    /// Process an audio buffer through the pipeline
    pub fn process(&self, buffer: &mut AudioBuffer) -> Result<bool> {
        let mut modified = false;
        
        for (i, processor) in self.processors.iter().enumerate() {
            if !self.enabled[i] {
                continue;
            }
            
            let processor = processor.lock().unwrap();
            if processor.process(buffer)? {
                modified = true;
            }
        }
        
        Ok(modified)
    }
    
    /// Enable or disable a processor by name
    pub fn set_enabled(&mut self, name: &str, enabled: bool) {
        for (i, processor_name) in self.names.iter().enumerate() {
            if processor_name == name {
                self.enabled[i] = enabled;
                break;
            }
        }
    }
    
    /// Check if a processor is enabled
    pub fn is_enabled(&self, name: &str) -> bool {
        for (i, processor_name) in self.names.iter().enumerate() {
            if processor_name == name {
                return self.enabled[i];
            }
        }
        false
    }
    
    /// Reset all processors in the pipeline
    pub fn reset(&mut self) {
        for processor in &self.processors {
            let mut processor = processor.lock().unwrap();
            processor.reset();
        }
    }
    
    /// Configure a processor by name
    pub fn configure(&mut self, name: &str, config: &HashMap<String, String>) -> Result<()> {
        for (i, processor_name) in self.names.iter().enumerate() {
            if processor_name == name {
                let mut processor = self.processors[i].lock().unwrap();
                return processor.configure(config);
            }
        }
        
        Ok(()) // Processor not found, no-op
    }
    
    /// Get the names of all processors in the pipeline
    pub fn processor_names(&self) -> Vec<String> {
        self.names.clone()
    }
    
    /// Get the number of processors in the pipeline
    pub fn len(&self) -> usize {
        self.processors.len()
    }
    
    /// Check if the pipeline is empty
    pub fn is_empty(&self) -> bool {
        self.processors.is_empty()
    }
}

impl Default for AudioProcessingPipeline {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a standard voice processing pipeline for telephony
pub fn create_telephony_pipeline() -> AudioProcessingPipeline {
    use crate::processing::audio::{
        aec::EchoCanceller,
        agc::GainControl,
        vad::VoiceActivityDetector,
        ns::NoiseSupressor,
        plc::PacketLossConcealor,
    };
    
    let mut pipeline = AudioProcessingPipeline::new();
    
    // Add standard processors for VoIP
    pipeline.add("echo_canceller", EchoCanceller::new());
    pipeline.add("noise_suppression", NoiseSupressor::new());
    pipeline.add("gain_control", GainControl::new());
    pipeline.add("voice_activity", VoiceActivityDetector::new());
    pipeline.add("packet_loss", PacketLossConcealor::new());
    
    pipeline
}

/// An audio processing unit that can be inserted into a processing pipeline
pub trait AudioProcessor: Send + Sync {
    /// Process audio data
    fn process(&mut self, input: &[u8], output: &mut [u8]) -> Result<usize>;
    
    /// Get the output format
    fn output_format(&self) -> AudioFormat;
    
    /// Get the input format
    fn input_format(&self) -> AudioFormat;
    
    /// Reset the processor state
    fn reset(&mut self) -> Result<()>;
    
    /// Check if the processor modifies audio data
    fn modifies_audio(&self) -> bool;
    
    /// Get processor name/type
    fn name(&self) -> &'static str;
}

/// Audio pipeline configuration
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Input audio format
    pub input_format: AudioFormat,
    /// Output audio format
    pub output_format: AudioFormat,
    /// Whether to use VAD-based processing
    pub use_vad: bool,
    /// Whether to apply noise reduction
    pub noise_reduction: bool,
    /// Whether to apply automatic gain control
    pub auto_gain: bool,
    /// Whether to apply echo cancellation
    pub echo_cancellation: bool,
    /// Whether to enable packet loss concealment
    pub packet_loss_concealment: bool,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            input_format: AudioFormat::pcm_telephony(),
            output_format: AudioFormat::pcm_telephony(),
            use_vad: true,
            noise_reduction: true,
            auto_gain: true,
            echo_cancellation: false,
            packet_loss_concealment: true,
        }
    }
}

/// Callback for pipeline events
pub type PipelineEventCallback = Arc<dyn Fn(PipelineEvent) + Send + Sync>;

/// Pipeline events
#[derive(Debug, Clone)]
pub enum PipelineEvent {
    /// Voice activity state changed
    VoiceActivityChanged(VadState),
    /// Audio level changed
    LevelChanged(f32),
    /// Clipping detected
    ClippingDetected,
    /// Frame processed
    FrameProcessed {
        /// Frame number
        frame: u64,
        /// Processing duration in milliseconds
        processing_ms: f32,
    },
}

/// Audio processing pipeline that combines multiple processors
pub struct AudioPipeline {
    /// Processing units in the pipeline
    processors: Vec<Box<dyn AudioProcessor>>,
    /// Input audio format
    input_format: AudioFormat,
    /// Output audio format
    output_format: AudioFormat,
    /// Intermediate buffers for processing
    buffers: Vec<Bytes>,
    /// Voice activity detector (if enabled)
    vad: Option<VoiceActivityDetector>,
    /// Event callback
    event_callback: Option<PipelineEventCallback>,
    /// Frame counter
    frame_counter: u64,
    /// Whether the pipeline is enabled
    enabled: bool,
}

impl AudioPipeline {
    /// Create a new audio processing pipeline
    pub fn new(config: PipelineConfig) -> Self {
        let vad = if config.use_vad {
            Some(VoiceActivityDetector::new_default())
        } else {
            None
        };
        
        let mut processors = Vec::new();
        
        // Here we would normally create and add processors based on config
        // For stub purposes, we'll just create an empty pipeline
        
        Self {
            processors,
            input_format: config.input_format,
            output_format: config.output_format,
            buffers: Vec::new(),
            vad,
            event_callback: None,
            frame_counter: 0,
            enabled: true,
        }
    }
    
    /// Add a processor to the pipeline
    pub fn add_processor(&mut self, processor: Box<dyn AudioProcessor>) -> Result<()> {
        // Check format compatibility
        if !self.processors.is_empty() {
            let last_processor = self.processors.last().unwrap();
            if last_processor.output_format() != processor.input_format() {
                return Err(Error::FormatMismatch(
                    format!("Format mismatch between processors: {} outputs {:?}, but {} expects {:?}",
                            last_processor.name(), 
                            last_processor.output_format(),
                            processor.name(),
                            processor.input_format())
                ));
            }
        } else {
            // First processor should match pipeline input format
            if self.input_format != processor.input_format() {
                return Err(Error::FormatMismatch(
                    format!("Format mismatch: pipeline input is {:?}, but processor expects {:?}",
                            self.input_format, 
                            processor.input_format())
                ));
            }
        }
        
        // Add to the pipeline
        self.processors.push(processor);
        
        // Resize intermediate buffers
        self.resize_buffers();
        
        Ok(())
    }
    
    /// Process audio data through the pipeline
    pub fn process(&mut self, input: &[u8]) -> Result<Bytes> {
        if !self.enabled {
            // If pipeline is disabled, pass through the audio
            return Ok(Bytes::copy_from_slice(input));
        }
        
        let start_time = std::time::Instant::now();
        
        // Update VAD if enabled
        if let Some(vad) = &mut self.vad {
            let prev_state = vad.state();
            let new_state = vad.process(input)?;
            
            if prev_state != new_state {
                if let Some(callback) = &self.event_callback {
                    callback(PipelineEvent::VoiceActivityChanged(new_state));
                }
            }
            
            // If VAD is inactive, we might want to skip processing
            // For now, we'll continue processing anyway
        }
        
        // If we have no processors, just return the input
        if self.processors.is_empty() {
            self.frame_counter += 1;
            let elapsed = start_time.elapsed();
            
            if let Some(callback) = &self.event_callback {
                callback(PipelineEvent::FrameProcessed {
                    frame: self.frame_counter,
                    processing_ms: elapsed.as_secs_f32() * 1000.0,
                });
            }
            
            return Ok(Bytes::copy_from_slice(input));
        }
        
        // Process through pipeline
        let mut input_data = Bytes::copy_from_slice(input);
        
        for i in 0..self.processors.len() {
            let processor = &mut self.processors[i];
            let output_buf = &mut self.buffers[i];
            
            // Get mutable access to buffer
            let output_slice = unsafe {
                std::slice::from_raw_parts_mut(
                    output_buf.as_ptr() as *mut u8,
                    output_buf.len()
                )
            };
            
            // Process data
            let bytes_written = processor.process(&input_data, output_slice)?;
            
            // Use this buffer as input for next stage
            input_data = Bytes::copy_from_slice(&output_slice[..bytes_written]);
        }
        
        // Update frame counter and elapsed time
        self.frame_counter += 1;
        let elapsed = start_time.elapsed();
        
        if let Some(callback) = &self.event_callback {
            callback(PipelineEvent::FrameProcessed {
                frame: self.frame_counter,
                processing_ms: elapsed.as_secs_f32() * 1000.0,
            });
        }
        
        trace!("Processed frame {} in {:.3}ms", 
              self.frame_counter, elapsed.as_secs_f32() * 1000.0);
        
        Ok(input_data)
    }
    
    /// Set event callback
    pub fn set_event_callback(&mut self, callback: PipelineEventCallback) {
        self.event_callback = Some(callback);
    }
    
    /// Reset all processors
    pub fn reset(&mut self) -> Result<()> {
        for processor in &mut self.processors {
            processor.reset()?;
        }
        
        if let Some(vad) = &mut self.vad {
            vad.reset();
        }
        
        self.frame_counter = 0;
        
        Ok(())
    }
    
    /// Enable or disable the pipeline
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
    
    /// Check if the pipeline is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
    
    /// Get the VAD state (if VAD is enabled)
    pub fn vad_state(&self) -> Option<VadState> {
        self.vad.as_ref().map(|vad| vad.state())
    }
    
    /// Resize intermediate buffers
    fn resize_buffers(&mut self) {
        // Create intermediate buffers for each processor
        self.buffers.clear();
        
        for processor in &self.processors {
            // Allocate a generous buffer size based on format
            let format = processor.output_format();
            let buffer_size = format.bytes_per_frame(100); // 100ms buffer
            self.buffers.push(Bytes::from(vec![0u8; buffer_size]));
            
            debug!("Created {}KB buffer for {} processor",
                  buffer_size / 1024,
                  processor.name());
        }
    }
    
    /// Get input format
    pub fn input_format(&self) -> AudioFormat {
        self.input_format
    }
    
    /// Get output format
    pub fn output_format(&self) -> AudioFormat {
        if let Some(processor) = self.processors.last() {
            processor.output_format()
        } else {
            self.input_format
        }
    }
    
    /// Get the number of processors in the pipeline
    pub fn processor_count(&self) -> usize {
        self.processors.len()
    }
} 
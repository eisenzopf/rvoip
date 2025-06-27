//! Processing Pipeline - Orchestrates all media processing
//!
//! This module contains the ProcessingPipeline which coordinates audio processing,
//! format conversion, and other media processing operations.

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::error::{Result, AudioProcessingError};
use crate::types::{AudioFrame, SampleRate};
use super::audio::{AudioProcessor, AudioProcessingConfig, AudioProcessingResult};
use super::format::{FormatConverter, ConversionParams};

/// Configuration for the processing pipeline
#[derive(Debug, Clone)]
pub struct ProcessingConfig {
    /// Audio processing configuration
    pub audio_config: AudioProcessingConfig,
    /// Enable format conversion
    pub enable_format_conversion: bool,
    /// Target format for processed audio
    pub target_format: TargetFormat,
    /// Maximum processing latency in milliseconds
    pub max_latency_ms: u32,
}

/// Target format specification
#[derive(Debug, Clone)]
pub struct TargetFormat {
    /// Target sample rate
    pub sample_rate: SampleRate,
    /// Target number of channels
    pub channels: u8,
}

impl Default for ProcessingConfig {
    fn default() -> Self {
        Self {
            audio_config: AudioProcessingConfig::default(),
            enable_format_conversion: true,
            target_format: TargetFormat {
                sample_rate: SampleRate::Rate8000,
                channels: 1,
            },
            max_latency_ms: 50, // 50ms max latency
        }
    }
}

/// Result of pipeline processing
#[derive(Debug, Clone)]
pub struct PipelineResult {
    /// Processed audio frame
    pub frame: AudioFrame,
    /// Audio processing result (if audio processing was applied)
    pub audio_result: Option<AudioProcessingResult>,
    /// Whether format conversion was applied
    pub format_converted: bool,
    /// Total processing time in microseconds
    pub total_processing_time_us: u64,
}

/// Media processing pipeline
pub struct ProcessingPipeline {
    /// Pipeline configuration
    config: ProcessingConfig,
    /// Audio processor
    audio_processor: Option<Arc<RwLock<AudioProcessor>>>,
    /// Format converter
    format_converter: Arc<RwLock<FormatConverter>>,
    /// Pipeline statistics
    stats: RwLock<PipelineStats>,
}

/// Pipeline processing statistics
#[derive(Debug, Default)]
pub struct PipelineStats {
    /// Total frames processed
    pub frames_processed: u64,
    /// Total processing time
    pub total_processing_time_us: u64,
    /// Frames that required format conversion
    pub format_conversions: u64,
    /// Audio processing operations
    pub audio_processing_operations: u64,
}

impl ProcessingPipeline {
    /// Create a new processing pipeline
    pub async fn new(config: ProcessingConfig) -> Result<Self> {
        info!("Creating ProcessingPipeline with config: {:?}", config);
        
        // Create audio processor if audio processing is enabled
        let audio_processor = if config.audio_config.enable_vad {
            let processor = AudioProcessor::new(config.audio_config.clone())?;
            Some(Arc::new(RwLock::new(processor)))
        } else {
            None
        };
        
        // Create format converter
        let format_converter = Arc::new(RwLock::new(FormatConverter::new()));
        
        Ok(Self {
            config,
            audio_processor,
            format_converter,
            stats: RwLock::new(PipelineStats::default()),
        })
    }
    
    /// Process capture audio (from microphone/input)
    pub async fn process_capture(&self, input: &AudioFrame) -> Result<PipelineResult> {
        let start_time = std::time::Instant::now();
        
        debug!("Processing capture audio: {}Hz, {}ch, {} samples", 
               input.sample_rate, input.channels, input.samples.len());
        
        let mut current_frame = input.clone();
        let mut audio_result = None;
        let mut format_converted = false;
        
        // Step 1: Audio processing (VAD, etc.)
        if let Some(audio_processor) = &self.audio_processor {
            let processor = audio_processor.read().await;
            let result = processor.process_capture_audio(&current_frame).await?;
            current_frame = result.frame.clone();
            audio_result = Some(result);
            
            // Update stats
            let mut stats = self.stats.write().await;
            stats.audio_processing_operations += 1;
        }
        
        // Step 2: Format conversion (if needed and enabled)
        if self.config.enable_format_conversion {
            current_frame = self.apply_format_conversion(&current_frame, &mut format_converted).await?;
        }
        
        let total_time = start_time.elapsed();
        
        // Update general stats
        {
            let mut stats = self.stats.write().await;
            stats.frames_processed += 1;
            stats.total_processing_time_us += total_time.as_micros() as u64;
            if format_converted {
                stats.format_conversions += 1;
            }
        }
        
        // Check latency constraint
        if total_time.as_millis() > self.config.max_latency_ms as u128 {
            warn!("Processing latency exceeded limit: {}ms > {}ms", 
                  total_time.as_millis(), self.config.max_latency_ms);
        }
        
        Ok(PipelineResult {
            frame: current_frame,
            audio_result,
            format_converted,
            total_processing_time_us: total_time.as_micros() as u64,
        })
    }
    
    /// Process playback audio (to speaker/output)
    pub async fn process_playback(&self, input: &AudioFrame) -> Result<PipelineResult> {
        let start_time = std::time::Instant::now();
        
        debug!("Processing playback audio: {}Hz, {}ch, {} samples", 
               input.sample_rate, input.channels, input.samples.len());
        
        let mut current_frame = input.clone();
        let mut audio_result = None;
        let mut format_converted = false;
        
        // Step 1: Format conversion (if needed and enabled)
        if self.config.enable_format_conversion {
            current_frame = self.apply_format_conversion(&current_frame, &mut format_converted).await?;
        }
        
        // Step 2: Audio processing for playback
        if let Some(audio_processor) = &self.audio_processor {
            let processor = audio_processor.read().await;
            let result = processor.process_playback_audio(&current_frame).await?;
            current_frame = result.frame.clone();
            audio_result = Some(result);
        }
        
        let total_time = start_time.elapsed();
        
        // Update stats
        {
            let mut stats = self.stats.write().await;
            stats.frames_processed += 1;
            stats.total_processing_time_us += total_time.as_micros() as u64;
            if format_converted {
                stats.format_conversions += 1;
            }
        }
        
        Ok(PipelineResult {
            frame: current_frame,
            audio_result,
            format_converted,
            total_processing_time_us: total_time.as_micros() as u64,
        })
    }
    
    /// Get pipeline statistics
    pub async fn get_stats(&self) -> PipelineStats {
        self.stats.read().await.clone()
    }
    
    /// Reset pipeline statistics
    pub async fn reset_stats(&self) {
        let mut stats = self.stats.write().await;
        *stats = PipelineStats::default();
        debug!("Pipeline statistics reset");
    }
    
    /// Apply format conversion if needed
    async fn apply_format_conversion(
        &self, 
        input: &AudioFrame, 
        was_converted: &mut bool
    ) -> Result<AudioFrame> {
        let target_sample_rate = self.config.target_format.sample_rate.as_hz();
        let target_channels = self.config.target_format.channels;
        
        // Check if conversion is needed
        if input.sample_rate == target_sample_rate && input.channels == target_channels {
            return Ok(input.clone());
        }
        
        // Apply format conversion
        let conversion_params = ConversionParams::new(
            self.config.target_format.sample_rate,
            target_channels,
        );
        
        let mut converter = self.format_converter.write().await;
        let result = converter.convert_frame(input, &conversion_params)?;
        
        *was_converted = result.was_converted;
        Ok(result.frame)
    }
}

impl Clone for PipelineStats {
    fn clone(&self) -> Self {
        Self {
            frames_processed: self.frames_processed,
            total_processing_time_us: self.total_processing_time_us,
            format_conversions: self.format_conversions,
            audio_processing_operations: self.audio_processing_operations,
        }
    }
} 
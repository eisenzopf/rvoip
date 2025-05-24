//! AudioProcessor - Main audio processing pipeline
//!
//! This module contains the AudioProcessor which coordinates all audio processing
//! operations including VAD, format conversion, and future AEC/AGC/NS components.

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn, error};

use crate::error::{Result, AudioProcessingError};
use crate::types::{AudioFrame, SampleRate};
use super::vad::{VoiceActivityDetector, VadConfig, VadResult};

/// Configuration for audio processing
#[derive(Debug, Clone)]
pub struct AudioProcessingConfig {
    /// Enable voice activity detection
    pub enable_vad: bool,
    /// VAD configuration
    pub vad_config: VadConfig,
    /// Target sample rate for processing
    pub target_sample_rate: SampleRate,
    /// Enable automatic gain control (future)
    pub enable_agc: bool,
    /// Enable echo cancellation (future)
    pub enable_aec: bool,
    /// Enable noise suppression (future)
    pub enable_noise_suppression: bool,
}

impl Default for AudioProcessingConfig {
    fn default() -> Self {
        Self {
            enable_vad: true,
            vad_config: VadConfig::default(),
            target_sample_rate: SampleRate::Rate8000,
            enable_agc: false,  // Disabled for Phase 2
            enable_aec: false,  // Disabled for Phase 2
            enable_noise_suppression: false,  // Disabled for Phase 2
        }
    }
}

/// Result of audio processing operations
#[derive(Debug, Clone)]
pub struct AudioProcessingResult {
    /// Processed audio frame
    pub frame: AudioFrame,
    /// Voice activity detection result
    pub vad_result: Option<VadResult>,
    /// Processing metrics
    pub metrics: AudioProcessingMetrics,
}

/// Metrics from audio processing
#[derive(Debug, Clone, Default)]
pub struct AudioProcessingMetrics {
    /// Processing time in microseconds
    pub processing_time_us: u64,
    /// Whether frame was modified
    pub frame_modified: bool,
    /// Number of samples processed
    pub samples_processed: usize,
}

/// Main audio processing pipeline
pub struct AudioProcessor {
    /// Processing configuration
    config: AudioProcessingConfig,
    /// Voice activity detector
    vad: Option<Arc<RwLock<VoiceActivityDetector>>>,
    /// Processing statistics
    stats: RwLock<AudioProcessingStats>,
}

/// Audio processing statistics
#[derive(Debug, Default, Clone)]
struct AudioProcessingStats {
    /// Total frames processed
    frames_processed: u64,
    /// Total processing time
    total_processing_time_us: u64,
    /// Frames with voice activity detected
    voice_frames: u64,
    /// Frames without voice activity
    silence_frames: u64,
}

impl AudioProcessor {
    /// Create a new audio processor with the given configuration
    pub fn new(config: AudioProcessingConfig) -> Result<Self> {
        debug!("Creating AudioProcessor with config: {:?}", config);
        
        // Initialize VAD if enabled
        let vad = if config.enable_vad {
            let vad_detector = VoiceActivityDetector::new(config.vad_config.clone())?;
            Some(Arc::new(RwLock::new(vad_detector)))
        } else {
            None
        };
        
        Ok(Self {
            config,
            vad,
            stats: RwLock::new(AudioProcessingStats::default()),
        })
    }
    
    /// Process capture audio (from microphone/input)
    pub async fn process_capture_audio(&self, input: &AudioFrame) -> Result<AudioProcessingResult> {
        let start_time = std::time::Instant::now();
        
        // Validate input frame
        self.validate_audio_frame(input)?;
        
        // Start with a copy of the input frame
        let mut processed_frame = input.clone();
        let mut frame_modified = false;
        let mut vad_result = None;
        
        // Run VAD if enabled
        if let Some(vad) = &self.vad {
            let mut vad_detector = vad.write().await;
            vad_result = Some(vad_detector.analyze_frame(&processed_frame)?);
        }
        
        // TODO: Add more processing stages in Phase 3:
        // - Automatic Gain Control (AGC)
        // - Noise Suppression (NS)
        // - Echo Cancellation (AEC) reference signal processing
        
        let processing_time = start_time.elapsed();
        
        // Update statistics
        self.update_stats(&processed_frame, &vad_result, processing_time).await;
        
        Ok(AudioProcessingResult {
            frame: processed_frame,
            vad_result,
            metrics: AudioProcessingMetrics {
                processing_time_us: processing_time.as_micros() as u64,
                frame_modified,
                samples_processed: input.samples.len(),
            },
        })
    }
    
    /// Process playback audio (to speaker/output)
    pub async fn process_playback_audio(&self, input: &AudioFrame) -> Result<AudioProcessingResult> {
        let start_time = std::time::Instant::now();
        
        // Validate input frame
        self.validate_audio_frame(input)?;
        
        // For playback, we mainly do format conversion and output processing
        let mut processed_frame = input.clone();
        let frame_modified = false;
        
        // TODO: Add playback processing in Phase 3:
        // - Echo Cancellation (AEC) playback signal processing
        // - Output gain control
        // - Packet Loss Concealment (PLC)
        
        let processing_time = start_time.elapsed();
        
        Ok(AudioProcessingResult {
            frame: processed_frame,
            vad_result: None, // No VAD on playback
            metrics: AudioProcessingMetrics {
                processing_time_us: processing_time.as_micros() as u64,
                frame_modified,
                samples_processed: input.samples.len(),
            },
        })
    }
    
    /// Get current processing statistics
    pub async fn get_stats(&self) -> AudioProcessingStats {
        self.stats.read().await.clone()
    }
    
    /// Reset processing statistics
    pub async fn reset_stats(&self) {
        let mut stats = self.stats.write().await;
        *stats = AudioProcessingStats::default();
    }
    
    /// Validate audio frame parameters
    fn validate_audio_frame(&self, frame: &AudioFrame) -> Result<()> {
        if frame.samples.is_empty() {
            return Err(AudioProcessingError::InvalidFormat {
                details: "Audio frame has no samples".to_string(),
            }.into());
        }
        
        if frame.channels == 0 {
            return Err(AudioProcessingError::InvalidFormat {
                details: "Audio frame has zero channels".to_string(),
            }.into());
        }
        
        if frame.sample_rate == 0 {
            return Err(AudioProcessingError::InvalidFormat {
                details: "Audio frame has zero sample rate".to_string(),
            }.into());
        }
        
        Ok(())
    }
    
    /// Update processing statistics
    async fn update_stats(
        &self,
        frame: &AudioFrame,
        vad_result: &Option<VadResult>,
        processing_time: std::time::Duration,
    ) {
        let mut stats = self.stats.write().await;
        stats.frames_processed += 1;
        stats.total_processing_time_us += processing_time.as_micros() as u64;
        
        if let Some(vad) = vad_result {
            if vad.is_voice {
                stats.voice_frames += 1;
            } else {
                stats.silence_frames += 1;
            }
        }
    }
} 
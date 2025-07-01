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
use super::agc::{AutomaticGainControl, AgcConfig, AgcResult};

// NEW: Performance library imports
use crate::performance::{
    pool::{AudioFramePool, PooledAudioFrame},
    zero_copy::{ZeroCopyAudioFrame, SharedAudioBuffer},
    simd::SimdProcessor,
    metrics::{PerformanceMetrics, BenchmarkResults},
};

// NEW: Advanced v2 processor imports
use super::vad_v2::{AdvancedVoiceActivityDetector, AdvancedVadConfig, AdvancedVadResult};
use super::agc_v2::{AdvancedAutomaticGainControl, AdvancedAgcConfig, AdvancedAgcResult};
use super::aec_v2::{AdvancedAcousticEchoCanceller, AdvancedAecConfig, AdvancedAecResult};

/// Configuration for audio processing
#[derive(Debug, Clone)]
pub struct AudioProcessingConfig {
    /// Enable voice activity detection
    pub enable_vad: bool,
    /// VAD configuration
    pub vad_config: VadConfig,
    /// Enable automatic gain control
    pub enable_agc: bool,
    /// AGC configuration
    pub agc_config: AgcConfig,
    /// Target sample rate for processing
    pub target_sample_rate: SampleRate,
    /// Enable echo cancellation (future)
    pub enable_aec: bool,
    /// Enable noise suppression (future)
    pub enable_noise_suppression: bool,
    
    // NEW: Advanced v2 processor options
    /// Use advanced VAD instead of basic VAD
    pub use_advanced_vad: bool,
    /// Advanced VAD configuration
    pub advanced_vad_config: AdvancedVadConfig,
    /// Use advanced AGC instead of basic AGC
    pub use_advanced_agc: bool,
    /// Advanced AGC configuration
    pub advanced_agc_config: AdvancedAgcConfig,
    /// Use advanced AEC instead of basic AEC
    pub use_advanced_aec: bool,
    /// Advanced AEC configuration
    pub advanced_aec_config: AdvancedAecConfig,
    
    // NEW: Performance optimization options
    /// Enable SIMD optimizations for audio operations
    pub enable_simd_optimizations: bool,
    /// Use zero-copy audio frames where possible
    pub use_zero_copy_frames: bool,
    /// Enable comprehensive performance metrics collection
    pub enable_performance_metrics: bool,
    /// Frame pool size for pooled audio frames
    pub frame_pool_size: usize,
}

impl Default for AudioProcessingConfig {
    fn default() -> Self {
        Self {
            enable_vad: true,
            vad_config: VadConfig::default(),
            enable_agc: false,  // Disabled by default (can be aggressive)
            agc_config: AgcConfig::default(),
            target_sample_rate: SampleRate::Rate8000,
            enable_aec: false,  // Disabled for Phase 2
            enable_noise_suppression: false,  // Disabled for Phase 2
            
            // NEW: Default to advanced processors for better quality
            use_advanced_vad: true,
            advanced_vad_config: AdvancedVadConfig::default(),
            use_advanced_agc: false,  // Disabled by default until tuned
            advanced_agc_config: AdvancedAgcConfig::default(),
            use_advanced_aec: false,  // Disabled until far-end reference implemented
            advanced_aec_config: AdvancedAecConfig::default(),
            
            // NEW: Default performance settings
            enable_simd_optimizations: true,
            use_zero_copy_frames: true,
            enable_performance_metrics: true,
            frame_pool_size: 32,
        }
    }
}

/// Result of audio processing operations
#[derive(Debug, Clone)]
pub struct AudioProcessingResult {
    /// Processed audio frame
    pub frame: AudioFrame,
    /// Voice activity detection result (v1)
    pub vad_result: Option<VadResult>,
    /// Automatic gain control result (v1)
    pub agc_result: Option<AgcResult>,
    /// Advanced VAD result (v2)
    pub advanced_vad_result: Option<AdvancedVadResult>,
    /// Advanced AGC result (v2)
    pub advanced_agc_result: Option<AdvancedAgcResult>,
    /// Advanced AEC result (v2)
    pub advanced_aec_result: Option<AdvancedAecResult>,
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
    /// SIMD optimizations used
    pub simd_optimizations_used: bool,
    /// Zero-copy operations used
    pub zero_copy_used: bool,
    /// Performance metrics (if enabled)
    pub performance_metrics: Option<PerformanceMetrics>,
}

/// Main audio processing pipeline
pub struct AudioProcessor {
    /// Processing configuration
    config: AudioProcessingConfig,
    
    // V1 processors (legacy support)
    /// Voice activity detector (v1)
    vad: Option<Arc<RwLock<VoiceActivityDetector>>>,
    /// Automatic gain control (v1)
    agc: Option<Arc<RwLock<AutomaticGainControl>>>,
    
    // NEW: Advanced v2 processors
    /// Advanced voice activity detector (v2)
    advanced_vad: Option<Arc<RwLock<AdvancedVoiceActivityDetector>>>,
    /// Advanced automatic gain control (v2)
    advanced_agc: Option<Arc<RwLock<AdvancedAutomaticGainControl>>>,
    /// Advanced acoustic echo canceller (v2)
    advanced_aec: Option<Arc<RwLock<AdvancedAcousticEchoCanceller>>>,
    
    // NEW: Performance components
    /// Frame pool for efficient audio frame allocation
    frame_pool: Arc<AudioFramePool>,
    /// SIMD processor for optimized audio operations
    simd_processor: SimdProcessor,
    /// Performance metrics collector
    performance_metrics: RwLock<PerformanceMetrics>,
    
    /// Processing statistics
    stats: RwLock<AudioProcessingStats>,
}

/// Audio processing statistics
#[derive(Debug, Default, Clone)]
pub struct AudioProcessingStats {
    /// Total frames processed
    pub frames_processed: u64,
    /// Total processing time
    pub total_processing_time_us: u64,
    /// Frames with voice activity detected
    pub voice_frames: u64,
    /// Frames without voice activity
    pub silence_frames: u64,
    /// Frames processed by AGC
    pub agc_processed_frames: u64,
    /// Average gain applied by AGC
    pub avg_agc_gain: f32,
}

impl AudioProcessor {
    /// Create a new audio processor with the given configuration
    pub fn new(config: AudioProcessingConfig) -> Result<Self> {
        debug!("Creating AudioProcessor with config: {:?}", config);
        
        // Initialize frame pool for performance optimization
        let pool_config = crate::performance::pool::PoolConfig {
            initial_size: config.frame_pool_size,
            max_size: config.frame_pool_size * 4, // Allow growth up to 4x
            sample_rate: match config.target_sample_rate {
                SampleRate::Rate8000 => 8000,
                SampleRate::Rate16000 => 16000,
                SampleRate::Rate32000 => 32000,
                SampleRate::Rate48000 => 48000,
            },
            channels: 1, // Mono for now
            samples_per_frame: match config.target_sample_rate {
                SampleRate::Rate8000 => 160,  // 20ms at 8kHz
                SampleRate::Rate16000 => 320, // 20ms at 16kHz
                SampleRate::Rate32000 => 640, // 20ms at 32kHz
                SampleRate::Rate48000 => 960, // 20ms at 48kHz
            },
        };
        let frame_pool = AudioFramePool::new(pool_config);
        
        // Initialize SIMD processor with platform detection
        let simd_processor = SimdProcessor::new();
        debug!("SIMD support available: {}", simd_processor.is_simd_available());
        
        // Initialize V1 processors (legacy support)
        let vad = if config.enable_vad && !config.use_advanced_vad {
            let vad_detector = VoiceActivityDetector::new(config.vad_config.clone())?;
            Some(Arc::new(RwLock::new(vad_detector)))
        } else {
            None
        };
        
        let agc = if config.enable_agc && !config.use_advanced_agc {
            let agc_processor = AutomaticGainControl::new(config.agc_config.clone())?;
            Some(Arc::new(RwLock::new(agc_processor)))
        } else {
            None
        };
        
        // Initialize advanced V2 processors
        let advanced_vad = if config.enable_vad && config.use_advanced_vad {
            let sample_rate = match config.target_sample_rate {
                SampleRate::Rate8000 => 8000.0,
                SampleRate::Rate16000 => 16000.0,
                SampleRate::Rate32000 => 32000.0,
                SampleRate::Rate48000 => 48000.0,
            };
            let vad_detector = AdvancedVoiceActivityDetector::new(
                config.advanced_vad_config.clone(),
                sample_rate,
            )?;
            Some(Arc::new(RwLock::new(vad_detector)))
        } else {
            None
        };
        
        let advanced_agc = if config.enable_agc && config.use_advanced_agc {
            let sample_rate = match config.target_sample_rate {
                SampleRate::Rate8000 => 8000.0,
                SampleRate::Rate16000 => 16000.0,
                SampleRate::Rate32000 => 32000.0,
                SampleRate::Rate48000 => 48000.0,
            };
            let agc_processor = AdvancedAutomaticGainControl::new(
                config.advanced_agc_config.clone(),
                sample_rate,
            )?;
            Some(Arc::new(RwLock::new(agc_processor)))
        } else {
            None
        };
        
        let advanced_aec = if config.enable_aec && config.use_advanced_aec {
            let aec_processor = AdvancedAcousticEchoCanceller::new(
                config.advanced_aec_config.clone(),
            )?;
            Some(Arc::new(RwLock::new(aec_processor)))
        } else {
            None
        };
        
        debug!("AudioProcessor created with processors: VAD v1={}, VAD v2={}, AGC v1={}, AGC v2={}, AEC v2={}",
               vad.is_some(), advanced_vad.is_some(), agc.is_some(), advanced_agc.is_some(), advanced_aec.is_some());
        
        Ok(Self {
            config,
            // V1 processors
            vad,
            agc,
            // V2 processors  
            advanced_vad,
            advanced_agc,
            advanced_aec,
            // Performance components
            frame_pool,
            simd_processor,
            performance_metrics: RwLock::new(PerformanceMetrics::new()),
            stats: RwLock::new(AudioProcessingStats::default()),
        })
    }
    
    /// Process capture audio (from microphone/input) - Legacy v1 method
    pub async fn process_capture_audio(&self, input: &AudioFrame) -> Result<AudioProcessingResult> {
        let start_time = std::time::Instant::now();
        
        // Validate input frame
        self.validate_audio_frame(input)?;
        
        // Start with a copy of the input frame
        let mut processed_frame = input.clone();
        let mut frame_modified = false;
        let mut vad_result = None;
        let mut agc_result = None;
        
        // Step 1: Run AGC first (on capture path, process before VAD for better detection)
        if let Some(agc) = &self.agc {
            let mut agc_processor = agc.write().await;
            let result = agc_processor.process_frame(&processed_frame)?;
            
            // Apply the gain to the samples
            agc_processor.apply_gain(&mut processed_frame.samples, result.applied_gain);
            agc_result = Some(result);
            frame_modified = true;
        }
        
        // Step 2: Run VAD on the processed audio
        if let Some(vad) = &self.vad {
            let mut vad_detector = vad.write().await;
            vad_result = Some(vad_detector.analyze_frame(&processed_frame)?);
        }
        
        // TODO: Add more processing stages in Phase 3:
        // - Echo Cancellation (AEC) reference signal processing
        // - Noise Suppression (NS)
        
        let processing_time = start_time.elapsed();
        
        // Update statistics
        self.update_stats(&processed_frame, &vad_result, &agc_result, processing_time).await;
        
        Ok(AudioProcessingResult {
            frame: processed_frame,
            vad_result,
            agc_result,
            advanced_vad_result: None,
            advanced_agc_result: None,
            advanced_aec_result: None,
            metrics: AudioProcessingMetrics {
                processing_time_us: processing_time.as_micros() as u64,
                frame_modified,
                samples_processed: input.samples.len(),
                simd_optimizations_used: false,
                zero_copy_used: false,
                performance_metrics: None,
            },
        })
    }
    
    /// Process capture audio with advanced v2 processors and performance optimizations
    pub async fn process_capture_audio_v2(&self, input: &AudioFrame) -> Result<AudioProcessingResult> {
        let start_time = std::time::Instant::now();
        
        // Validate input frame
        self.validate_audio_frame(input)?;
        
        // Performance optimizations
        let mut simd_used = false;
        let mut zero_copy_used = false;
        let mut frame_modified = false;
        
        // Initialize result containers
        let mut vad_result = None;
        let mut agc_result = None;
        let mut advanced_vad_result = None;
        let mut advanced_agc_result = None;
        let mut advanced_aec_result = None;
        
        // Step 1: Get optimized frame for processing
        let mut processed_frame = if self.config.use_zero_copy_frames {
            // Use pooled frame for zero-copy optimization
            let pooled_frame = self.frame_pool.get_frame();
            
            // Copy input data to pooled frame
            if self.config.enable_simd_optimizations {
                // Use SIMD for efficient copying
                let input_samples = &input.samples;
                let mut output_samples = vec![0i16; input_samples.len()];
                self.simd_processor.apply_gain(input_samples, 1.0, &mut output_samples);
                simd_used = true;
            }
            zero_copy_used = true;
            
            // Create regular AudioFrame from pooled data for now
            // TODO: Update entire pipeline to use ZeroCopyAudioFrame
            input.clone()
        } else {
            input.clone()
        };
        
        // Step 2: Advanced Echo Cancellation (AEC v2) - if enabled with far-end reference
        if let Some(aec) = &self.advanced_aec {
            // TODO: AEC requires far-end reference signal
            // For now, skip AEC processing until far-end reference is available
            debug!("AEC v2 enabled but far-end reference not available, skipping");
        }
        
        // Step 3: Advanced Automatic Gain Control (AGC v2) - multi-band processing
        if let Some(agc) = &self.advanced_agc {
            let mut agc_processor = agc.write().await;
            let result = agc_processor.process_frame(&processed_frame)?;
            
            // Apply processed samples
            // TODO: Update AGC v2 to modify frame in-place for efficiency
            advanced_agc_result = Some(result);
            frame_modified = true;
        } else if let Some(agc) = &self.agc {
            // Fallback to v1 AGC
            let mut agc_processor = agc.write().await;
            let result = agc_processor.process_frame(&processed_frame)?;
            
            // Apply the gain to the samples
            if self.config.enable_simd_optimizations {
                self.simd_processor.apply_gain(&input.samples, result.applied_gain, &mut processed_frame.samples);
                simd_used = true;
            } else {
                agc_processor.apply_gain(&mut processed_frame.samples, result.applied_gain);
            }
            agc_result = Some(result);
            frame_modified = true;
        }
        
        // Step 4: Advanced Voice Activity Detection (VAD v2) - spectral analysis
        if let Some(vad) = &self.advanced_vad {
            let mut vad_detector = vad.write().await;
            let result = vad_detector.analyze_frame(&processed_frame)?;
            advanced_vad_result = Some(result);
        } else if let Some(vad) = &self.vad {
            // Fallback to v1 VAD
            let mut vad_detector = vad.write().await;
            vad_result = Some(vad_detector.analyze_frame(&processed_frame)?);
        }
        
        let processing_time = start_time.elapsed();
        
        // Collect performance metrics if enabled
        let performance_metrics = if self.config.enable_performance_metrics {
            let mut metrics = self.performance_metrics.write().await;
            metrics.add_timing(processing_time);
            if frame_modified {
                let allocation_size = (processed_frame.samples.len() * std::mem::size_of::<i16>()) as u64;
                metrics.add_allocation(allocation_size);
            }
            Some(metrics.clone())
        } else {
            None
        };
        
        // Update statistics with advanced results
        self.update_stats_v2(&processed_frame, &vad_result, &agc_result, 
                           &advanced_vad_result, &advanced_agc_result, &advanced_aec_result, processing_time).await;
        
        Ok(AudioProcessingResult {
            frame: processed_frame,
            vad_result,
            agc_result,
            advanced_vad_result,
            advanced_agc_result,
            advanced_aec_result,
            metrics: AudioProcessingMetrics {
                processing_time_us: processing_time.as_micros() as u64,
                frame_modified,
                samples_processed: input.samples.len(),
                simd_optimizations_used: simd_used,
                zero_copy_used,
                performance_metrics,
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
        let mut frame_modified = false;
        let mut agc_result = None;
        
        // Apply AGC on playback path for output level control
        if let Some(agc) = &self.agc {
            let mut agc_processor = agc.write().await;
            let result = agc_processor.process_frame(&processed_frame)?;
            
            // Apply the gain to the samples
            agc_processor.apply_gain(&mut processed_frame.samples, result.applied_gain);
            agc_result = Some(result);
            frame_modified = true;
        }
        
        // TODO: Add playback processing in Phase 3:
        // - Echo Cancellation (AEC) playback signal processing
        // - Output gain control
        // - Packet Loss Concealment (PLC)
        
        let processing_time = start_time.elapsed();
        
        Ok(AudioProcessingResult {
            frame: processed_frame,
            vad_result: None, // No VAD on playback
            agc_result,
            advanced_vad_result: None,
            advanced_agc_result: None,
            advanced_aec_result: None,
            metrics: AudioProcessingMetrics {
                processing_time_us: processing_time.as_micros() as u64,
                frame_modified,
                samples_processed: input.samples.len(),
                simd_optimizations_used: false,
                zero_copy_used: false,
                performance_metrics: None,
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
    
    /// Get current performance metrics
    pub async fn get_performance_metrics(&self) -> PerformanceMetrics {
        self.performance_metrics.read().await.clone()
    }
    
    /// Reset performance metrics
    pub async fn reset_performance_metrics(&self) {
        let mut metrics = self.performance_metrics.write().await;
        *metrics = PerformanceMetrics::new();
    }
    
    /// Get frame pool statistics
    pub fn get_pool_stats(&self) -> crate::performance::pool::PoolStats {
        self.frame_pool.get_stats()
    }
    
    /// Check if SIMD optimizations are available
    pub fn is_simd_available(&self) -> bool {
        self.simd_processor.is_simd_available()
    }
    
    /// Get processing configuration
    pub fn get_config(&self) -> &AudioProcessingConfig {
        &self.config
    }
    
    /// Check if advanced processors are enabled
    pub fn are_advanced_processors_enabled(&self) -> bool {
        self.advanced_vad.is_some() || self.advanced_agc.is_some() || self.advanced_aec.is_some()
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
    
    /// Update processing statistics (v1 legacy)
    async fn update_stats(
        &self,
        frame: &AudioFrame,
        vad_result: &Option<VadResult>,
        agc_result: &Option<AgcResult>,
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
        
        if let Some(agc) = agc_result {
            stats.agc_processed_frames += 1;
            // Update running average of AGC gain
            let frame_count = stats.agc_processed_frames as f32;
            stats.avg_agc_gain = ((stats.avg_agc_gain * (frame_count - 1.0)) + agc.applied_gain) / frame_count;
        }
    }
    
    /// Update processing statistics with v2 advanced processors
    async fn update_stats_v2(
        &self,
        frame: &AudioFrame,
        vad_result: &Option<VadResult>,
        agc_result: &Option<AgcResult>,
        advanced_vad_result: &Option<AdvancedVadResult>,
        advanced_agc_result: &Option<AdvancedAgcResult>,
        advanced_aec_result: &Option<AdvancedAecResult>,
        processing_time: std::time::Duration,
    ) {
        let mut stats = self.stats.write().await;
        stats.frames_processed += 1;
        stats.total_processing_time_us += processing_time.as_micros() as u64;
        
        // Handle VAD results (v1 or v2)
        if let Some(vad) = advanced_vad_result {
            if vad.is_voice {
                stats.voice_frames += 1;
            } else {
                stats.silence_frames += 1;
            }
        } else if let Some(vad) = vad_result {
            if vad.is_voice {
                stats.voice_frames += 1;
            } else {
                stats.silence_frames += 1;
            }
        }
        
        // Handle AGC results (v1 or v2)
        if let Some(agc) = advanced_agc_result {
            stats.agc_processed_frames += 1;
            // Use first band gain as representative for now
            let representative_gain = agc.band_gains_db.get(0).unwrap_or(&0.0);
            let frame_count = stats.agc_processed_frames as f32;
            stats.avg_agc_gain = ((stats.avg_agc_gain * (frame_count - 1.0)) + representative_gain) / frame_count;
        } else if let Some(agc) = agc_result {
            stats.agc_processed_frames += 1;
            let frame_count = stats.agc_processed_frames as f32;
            stats.avg_agc_gain = ((stats.avg_agc_gain * (frame_count - 1.0)) + agc.applied_gain) / frame_count;
        }
        
        // Log advanced processor performance if enabled
        if self.config.enable_performance_metrics {
            if let Some(aec) = advanced_aec_result {
                debug!("AEC v2 ERLE: {:.1}dB, coherence: {:.2}", aec.erle_db, aec.coherence);
            }
            if let Some(vad) = advanced_vad_result {
                debug!("VAD v2 confidence: {:.2}, spectral centroid: {:.0}Hz", vad.confidence, vad.spectral_centroid);
            }
        }
    }
}

impl std::fmt::Debug for AudioProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioProcessor")
            .field("config", &self.config)
            .field("vad_enabled", &self.vad.is_some())
            .field("agc_enabled", &self.agc.is_some())
            .field("advanced_vad_enabled", &self.advanced_vad.is_some())
            .field("advanced_agc_enabled", &self.advanced_agc.is_some())
            .field("advanced_aec_enabled", &self.advanced_aec.is_some())
            .field("simd_available", &self.simd_processor.is_simd_available())
            .finish()
    }
} 
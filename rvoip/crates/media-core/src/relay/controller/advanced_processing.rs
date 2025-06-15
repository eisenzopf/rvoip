//! Advanced audio processing functionality
//!
//! This module provides advanced audio processing capabilities including
//! VAD, AGC, AEC, and performance monitoring.

use std::sync::Arc;
use tracing::{debug, info};

use crate::error::{Error, Result};
use crate::types::{DialogId, AudioFrame};
use crate::processing::audio::{
    AdvancedVoiceActivityDetector, AdvancedVadConfig,
    AdvancedAutomaticGainControl, AdvancedAgcConfig,
    AdvancedAcousticEchoCanceller, AdvancedAecConfig,
};
use crate::performance::{
    metrics::PerformanceMetrics,
    pool::{AudioFramePool, PoolConfig},
    simd::SimdProcessor,
};
use tokio::sync::RwLock;

use super::{MediaSessionController, MediaConfig, AdvancedProcessorConfig, AdvancedProcessorSet};

impl AdvancedProcessorSet {
    /// Create a new advanced processor set
    pub async fn new(config: AdvancedProcessorConfig, frame_pool: Arc<AudioFramePool>) -> Result<Self> {
        debug!("Creating AdvancedProcessorSet with config: {:?}", config);
        
        // Create SIMD processor
        let simd_processor = SimdProcessor::new();
        
        // Create performance metrics
        let metrics = Arc::new(RwLock::new(PerformanceMetrics::new()));
        
        // Create advanced processors based on configuration
        let vad = if config.enable_advanced_vad {
            let vad_detector = AdvancedVoiceActivityDetector::new(
                config.vad_config.clone(),
                config.sample_rate as f32,
            )?;
            Some(Arc::new(RwLock::new(vad_detector)))
        } else {
            None
        };
        
        let agc = if config.enable_advanced_agc {
            let agc_processor = AdvancedAutomaticGainControl::new(
                config.agc_config.clone(),
                config.sample_rate as f32,
            )?;
            Some(Arc::new(RwLock::new(agc_processor)))
        } else {
            None
        };
        
        let aec = if config.enable_advanced_aec {
            let aec_processor = AdvancedAcousticEchoCanceller::new(
                config.aec_config.clone(),
            )?;
            Some(Arc::new(RwLock::new(aec_processor)))
        } else {
            None
        };
        
        debug!("AdvancedProcessorSet created: VAD={}, AGC={}, AEC={}, SIMD={}",
               vad.is_some(), agc.is_some(), aec.is_some(), simd_processor.is_simd_available());
        
        Ok(Self {
            vad,
            agc,
            aec,
            frame_pool,
            simd_processor,
            metrics,
            config,
        })
    }
    
    /// Process audio frame with advanced processors
    pub async fn process_audio(&self, input_frame: &AudioFrame) -> Result<AudioFrame> {
        let start_time = std::time::Instant::now();
        
        let mut processed_frame = input_frame.clone();
        
        // Process with advanced AEC first (if enabled and far-end reference available)
        if let Some(aec) = &self.aec {
            // TODO: Add far-end reference when available
            debug!("AEC v2 processing skipped - far-end reference not available");
        }
        
        // Process with advanced AGC
        if let Some(agc) = &self.agc {
            let mut agc_processor = agc.write().await;
            let result = agc_processor.process_frame(&processed_frame)?;
            // TODO: Apply AGC result to frame
            debug!("AGC v2 processed frame with {} band gains", result.band_gains_db.len());
        }
        
        // Process with advanced VAD
        let mut vad_result = None;
        if let Some(vad) = &self.vad {
            let mut vad_detector = vad.write().await;
            vad_result = Some(vad_detector.analyze_frame(&processed_frame)?);
        }
        
        // Apply SIMD optimizations if enabled
        if self.config.enable_simd && self.simd_processor.is_simd_available() {
            // Apply SIMD-optimized operations
            let mut simd_samples = vec![0i16; processed_frame.samples.len()];
            self.simd_processor.apply_gain(&processed_frame.samples, 1.0, &mut simd_samples);
            processed_frame.samples = simd_samples;
        }
        
        // Update performance metrics
        let processing_time = start_time.elapsed();
        {
            let mut metrics = self.metrics.write().await;
            metrics.add_timing(processing_time);
            metrics.add_allocation(processed_frame.samples.len() as u64 * 2); // 2 bytes per i16
        }
        
        if let Some(vad) = vad_result {
            debug!("Advanced VAD result: voice={}, confidence={:.2}", vad.is_voice, vad.confidence);
        }
        
        Ok(processed_frame)
    }
    
    /// Get performance metrics for this processor set
    pub async fn get_metrics(&self) -> PerformanceMetrics {
        self.metrics.read().await.clone()
    }
    
    /// Reset performance metrics
    pub async fn reset_metrics(&self) {
        let mut metrics = self.metrics.write().await;
        *metrics = PerformanceMetrics::new();
    }
    
    /// Check if any advanced processors are enabled
    pub fn has_advanced_processors(&self) -> bool {
        self.vad.is_some() || self.agc.is_some() || self.aec.is_some()
    }
}

impl MediaSessionController {
    /// Start advanced media session with custom processor configuration
    pub async fn start_advanced_media(&self, dialog_id: DialogId, config: MediaConfig, processor_config: Option<AdvancedProcessorConfig>) -> Result<()> {
        info!("Starting advanced media session for dialog: {}", dialog_id);
        
        // Start regular media session first
        self.start_media(dialog_id.clone(), config).await?;
        
        // Create advanced processors if configuration provided
        if let Some(proc_config) = processor_config {
            // Create session-specific frame pool for advanced processors or use global pool
            let session_frame_pool: Arc<AudioFramePool> = if proc_config.frame_pool_size > 0 {
                // Create dedicated pool for this session
                let session_pool_config = PoolConfig {
                    initial_size: proc_config.frame_pool_size,
                    max_size: proc_config.frame_pool_size * 2,
                    sample_rate: proc_config.sample_rate,
                    channels: 1,
                    samples_per_frame: 160, // 20ms at 8kHz
                };
                AudioFramePool::new(session_pool_config)
            } else {
                // Use global shared pool
                self.frame_pool.clone()
            };
            
            let processor_set = AdvancedProcessorSet::new(proc_config, session_frame_pool).await?;
            
            {
                let mut processors = self.advanced_processors.write().await;
                processors.insert(dialog_id.clone(), processor_set);
            }
            
            info!("✅ Created advanced processors for dialog: {}", dialog_id);
        } else {
            info!("⚠️ No processor configuration provided - using basic media session");
        }
        
        Ok(())
    }
    
    /// Process audio frame with advanced processors (if enabled for this dialog)
    pub async fn process_advanced_audio(&self, dialog_id: &DialogId, audio_frame: AudioFrame) -> Result<AudioFrame> {
        let start_time = std::time::Instant::now();
        
        // Check if dialog has advanced processors
        let processed_frame = {
            let processors = self.advanced_processors.read().await;
            if let Some(processor_set) = processors.get(dialog_id) {
                // Process with session-specific advanced processors
                let processed = processor_set.process_audio(&audio_frame).await?;
                debug!("Processed audio frame for {} with advanced processors", dialog_id);
                processed
            } else {
                // Use global frame pool for zero-copy optimization even without advanced processors
                debug!("Processed audio frame for {} with global pool only", dialog_id);
                audio_frame // Return as-is if no advanced processors
            }
        };
        
        // Update global performance metrics
        let processing_time = start_time.elapsed();
        {
            let mut metrics = self.performance_metrics.write().await;
            metrics.add_timing(processing_time);
            metrics.add_allocation(processed_frame.samples.len() as u64 * 2); // 2 bytes per i16
        }
        
        Ok(processed_frame)
    }
    
    /// Get performance metrics for a specific dialog
    pub async fn get_dialog_performance_metrics(&self, dialog_id: &DialogId) -> Option<PerformanceMetrics> {
        let processors = self.advanced_processors.read().await;
        if let Some(processor_set) = processors.get(dialog_id) {
            Some(processor_set.get_metrics().await)
        } else {
            None
        }
    }
    
    /// Get global performance metrics for all sessions
    pub async fn get_global_performance_metrics(&self) -> PerformanceMetrics {
        self.performance_metrics.read().await.clone()
    }
    
    /// Reset performance metrics for a specific dialog
    pub async fn reset_dialog_metrics(&self, dialog_id: &DialogId) -> Result<()> {
        let processors = self.advanced_processors.read().await;
        if let Some(processor_set) = processors.get(dialog_id) {
            processor_set.reset_metrics().await;
            Ok(())
        } else {
            Err(Error::session_not_found(&format!("No advanced processors for dialog: {}", dialog_id)))
        }
    }
    
    /// Reset global performance metrics
    pub async fn reset_global_metrics(&self) {
        let mut metrics = self.performance_metrics.write().await;
        *metrics = PerformanceMetrics::new();
    }
    
    /// Check if dialog has advanced processors enabled
    pub async fn has_advanced_processors(&self, dialog_id: &DialogId) -> bool {
        let processors = self.advanced_processors.read().await;
        processors.get(dialog_id)
            .map(|p| p.has_advanced_processors())
            .unwrap_or(false)
    }
    
    /// Get frame pool statistics
    pub fn get_frame_pool_stats(&self) -> crate::performance::pool::PoolStats {
        self.frame_pool.get_stats()
    }
    
    /// Update default processor configuration for new sessions
    pub async fn set_default_processor_config(&mut self, config: AdvancedProcessorConfig) {
        self.default_processor_config = config;
        info!("Updated default processor configuration");
    }
    
    /// Get current default processor configuration
    pub fn get_default_processor_config(&self) -> &AdvancedProcessorConfig {
        &self.default_processor_config
    }
} 
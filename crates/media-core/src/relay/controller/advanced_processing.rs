//! Advanced audio processing functionality
//!
//! This module provides advanced audio processing capabilities including
//! VAD, AGC, AEC, and performance monitoring.

use std::sync::Arc;
use tracing::{debug, info};

use crate::error::{Error, Result};
use crate::performance::{
    metrics::{ConcurrentPerformanceMetrics, PerformanceMetrics},
    pool::{AudioFramePool, PoolConfig},
    simd::SimdProcessor,
};
use crate::processing::audio::{
    AdvancedAcousticEchoCanceller, AdvancedAecConfig, AdvancedAgcConfig,
    AdvancedAutomaticGainControl, AdvancedVadConfig, AdvancedVoiceActivityDetector,
};
use crate::types::{AudioFrame, DialogId};
use tokio::sync::RwLock;

use super::{AdvancedProcessorConfig, AdvancedProcessorSet, MediaConfig, MediaSessionController};

impl AdvancedProcessorSet {
    /// Create a new advanced processor set
    pub async fn new(
        config: AdvancedProcessorConfig,
        frame_pool: Arc<AudioFramePool>,
    ) -> Result<Self> {
        debug!("Creating AdvancedProcessorSet with config: {:?}", config);

        // Create SIMD processor
        let simd_processor = SimdProcessor::new();

        // Create performance metrics (lock-free padded atomics, C20).
        let metrics = Arc::new(ConcurrentPerformanceMetrics::new());

        // Create advanced processors based on configuration. Inner
        // locks are `parking_lot::RwLock` because per-frame DSP work
        // is CPU-only with no `.await` held across the guard.
        let vad = if config.enable_advanced_vad {
            let vad_detector = AdvancedVoiceActivityDetector::new(
                config.vad_config.clone(),
                config.sample_rate as f32,
            )?;
            Some(Arc::new(parking_lot::RwLock::new(vad_detector)))
        } else {
            None
        };

        let agc = if config.enable_advanced_agc {
            let agc_processor = AdvancedAutomaticGainControl::new(
                config.agc_config.clone(),
                config.sample_rate as f32,
            )?;
            Some(Arc::new(parking_lot::RwLock::new(agc_processor)))
        } else {
            None
        };

        let aec = if config.enable_advanced_aec {
            let aec_processor = AdvancedAcousticEchoCanceller::new(config.aec_config.clone())?;
            Some(Arc::new(parking_lot::RwLock::new(aec_processor)))
        } else {
            None
        };

        debug!(
            "AdvancedProcessorSet created: VAD={}, AGC={}, AEC={}, SIMD={}",
            vad.is_some(),
            agc.is_some(),
            aec.is_some(),
            simd_processor.is_simd_available()
        );

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

        // Process with advanced AGC (parking_lot — sync write).
        if let Some(agc) = &self.agc {
            let mut agc_processor = agc.write();
            let result = agc_processor.process_frame(&processed_frame)?;
            debug!(
                "AGC v2 processed frame with {} band gains",
                result.band_gains_db.len()
            );
        }

        // Process with advanced VAD (parking_lot — sync write).
        let mut vad_result = None;
        if let Some(vad) = &self.vad {
            let mut vad_detector = vad.write();
            vad_result = Some(vad_detector.analyze_frame(&processed_frame)?);
        }

        // Apply SIMD optimizations if enabled
        if self.config.enable_simd && self.simd_processor.is_simd_available() {
            // Apply SIMD-optimized operations
            let mut simd_samples = vec![0i16; processed_frame.samples.len()];
            self.simd_processor
                .apply_gain(&processed_frame.samples, 1.0, &mut simd_samples);
            processed_frame.samples = simd_samples;
        }

        // Update performance metrics (lock-free padded atomics).
        let processing_time = start_time.elapsed();
        self.metrics.add_timing(processing_time);
        self.metrics
            .add_allocation(processed_frame.samples.len() as u64 * 2); // 2 bytes per i16

        if let Some(vad) = vad_result {
            debug!(
                "Advanced VAD result: voice={}, confidence={:.2}",
                vad.is_voice, vad.confidence
            );
        }

        Ok(processed_frame)
    }

    /// Get performance metrics for this processor set
    pub async fn get_metrics(&self) -> PerformanceMetrics {
        self.metrics.snapshot()
    }

    /// Reset performance metrics
    pub async fn reset_metrics(&self) {
        self.metrics.reset();
    }

    /// Check if any advanced processors are enabled
    pub fn has_advanced_processors(&self) -> bool {
        self.vad.is_some() || self.agc.is_some() || self.aec.is_some()
    }
}

impl MediaSessionController {
    /// Start advanced media session with custom processor configuration
    pub async fn start_advanced_media(
        &self,
        dialog_id: DialogId,
        config: MediaConfig,
        processor_config: Option<AdvancedProcessorConfig>,
    ) -> Result<()> {
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
            self.advanced_processors
                .insert(dialog_id.clone(), Arc::new(processor_set));

            info!("✅ Created advanced processors for dialog: {}", dialog_id);
        } else {
            info!("⚠️ No processor configuration provided - using basic media session");
        }

        Ok(())
    }

    /// Process audio frame with advanced processors (if enabled for this dialog)
    pub async fn process_advanced_audio(
        &self,
        dialog_id: &DialogId,
        audio_frame: AudioFrame,
    ) -> Result<AudioFrame> {
        let start_time = std::time::Instant::now();

        // Clone the Arc out of the DashMap shard, drop the shard guard,
        // then run the async `process_audio` without serialising other
        // dialogs that happen to land in the same shard.
        let processor_arc = self
            .advanced_processors
            .get(dialog_id)
            .map(|r| r.value().clone());
        let processed_frame = if let Some(processor_set) = processor_arc {
            let processed = processor_set.process_audio(&audio_frame).await?;
            debug!(
                "Processed audio frame for {} with advanced processors",
                dialog_id
            );
            processed
        } else {
            debug!(
                "Processed audio frame for {} with global pool only",
                dialog_id
            );
            audio_frame
        };

        // Update global performance metrics — lock-free padded atomics.
        let processing_time = start_time.elapsed();
        self.performance_metrics.add_timing(processing_time);
        self.performance_metrics
            .add_allocation(processed_frame.samples.len() as u64 * 2); // 2 bytes per i16

        Ok(processed_frame)
    }

    /// Get performance metrics for a specific dialog
    pub async fn get_dialog_performance_metrics(
        &self,
        dialog_id: &DialogId,
    ) -> Option<PerformanceMetrics> {
        let processor_arc = self
            .advanced_processors
            .get(dialog_id)
            .map(|r| r.value().clone())?;
        Some(processor_arc.get_metrics().await)
    }

    /// Get global performance metrics for all sessions
    pub async fn get_global_performance_metrics(&self) -> PerformanceMetrics {
        self.performance_metrics.snapshot()
    }

    /// Reset performance metrics for a specific dialog
    pub async fn reset_dialog_metrics(&self, dialog_id: &DialogId) -> Result<()> {
        let processor_arc = self
            .advanced_processors
            .get(dialog_id)
            .map(|r| r.value().clone())
            .ok_or_else(|| {
                Error::session_not_found(&format!(
                    "No advanced processors for dialog: {}",
                    dialog_id
                ))
            })?;
        processor_arc.reset_metrics().await;
        Ok(())
    }

    /// Reset global performance metrics
    pub async fn reset_global_metrics(&self) {
        self.performance_metrics.reset();
    }

    /// Check if dialog has advanced processors enabled
    pub async fn has_advanced_processors(&self, dialog_id: &DialogId) -> bool {
        self.advanced_processors
            .get(dialog_id)
            .map(|p| p.value().has_advanced_processors())
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

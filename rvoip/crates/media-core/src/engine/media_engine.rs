//! MediaEngine - Central orchestrator for media processing
//!
//! This is the main entry point for all media processing operations.
//! It coordinates codec management, session management, audio processing,
//! and integration with other crates.

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock;
use tracing::{debug, info, warn, error};

use crate::error::{Result, Error};
use crate::types::{DialogId, MediaSessionId, PayloadType, SampleRate};
use super::config::{MediaEngineConfig, EngineCapabilities, AudioCodecCapability};
use super::lifecycle::{LifecycleManager, EngineState};

// NEW: Performance library imports
use crate::performance::{
    pool::{AudioFramePool, PoolConfig},
    metrics::PerformanceMetrics,
};

// NEW: Audio processing imports
use crate::processing::audio::{
    AudioProcessor, AudioProcessingConfig,
    AdvancedVoiceActivityDetector, AdvancedVadConfig,
    AdvancedAutomaticGainControl, AdvancedAgcConfig,
    AdvancedAcousticEchoCanceller, AdvancedAecConfig,
};

/// Performance optimization level for media sessions
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PerformanceLevel {
    /// Basic processing, minimal CPU usage
    Basic,
    /// Balanced processing with some optimizations
    Balanced,
    /// High-performance processing with all optimizations
    HighPerformance,
}

impl Default for PerformanceLevel {
    fn default() -> Self {
        PerformanceLevel::Balanced
    }
}

/// Parameters for creating a media session
#[derive(Debug, Clone)]
pub struct MediaSessionParams {
    /// Preferred codec payload type
    pub preferred_codec: Option<PayloadType>,
    /// Enable audio processing
    pub enable_processing: bool,
    /// Custom session configuration
    pub custom_config: Option<serde_json::Value>,
    
    // NEW: Advanced processing configuration
    /// Audio processing configuration for this session
    pub audio_processing_config: AudioProcessingConfig,
    /// Enable advanced echo cancellation
    pub enable_advanced_echo_cancellation: bool,
    /// Enable multi-band AGC
    pub enable_multi_band_agc: bool,
    /// Enable spectral VAD
    pub enable_spectral_vad: bool,
    /// Performance optimization level
    pub performance_optimization_level: PerformanceLevel,
}

impl MediaSessionParams {
    /// Create audio-only session parameters
    pub fn audio_only() -> Self {
        Self {
            preferred_codec: Some(0), // PCMU by default
            enable_processing: true,
            custom_config: None,
            // NEW: Default advanced processing configuration
            audio_processing_config: AudioProcessingConfig::default(),
            enable_advanced_echo_cancellation: false,
            enable_multi_band_agc: false,
            enable_spectral_vad: true, // Enable advanced VAD by default
            performance_optimization_level: PerformanceLevel::default(),
        }
    }
    
    /// Set preferred codec
    pub fn with_preferred_codec(mut self, payload_type: PayloadType) -> Self {
        self.preferred_codec = Some(payload_type);
        self
    }
    
    /// Enable/disable audio processing
    pub fn with_processing_enabled(mut self, enabled: bool) -> Self {
        self.enable_processing = enabled;
        self
    }
    
    /// Configure advanced audio processing
    pub fn with_advanced_processing(mut self, config: AudioProcessingConfig) -> Self {
        self.audio_processing_config = config;
        self
    }
    
    /// Enable advanced echo cancellation
    pub fn with_advanced_aec(mut self, enabled: bool) -> Self {
        self.enable_advanced_echo_cancellation = enabled;
        self.audio_processing_config.use_advanced_aec = enabled;
        self
    }
    
    /// Enable multi-band AGC
    pub fn with_multi_band_agc(mut self, enabled: bool) -> Self {
        self.enable_multi_band_agc = enabled;
        self.audio_processing_config.use_advanced_agc = enabled;
        self
    }
    
    /// Enable spectral VAD
    pub fn with_spectral_vad(mut self, enabled: bool) -> Self {
        self.enable_spectral_vad = enabled;
        self.audio_processing_config.use_advanced_vad = enabled;
        self
    }
    
    /// Set performance optimization level
    pub fn with_performance_level(mut self, level: PerformanceLevel) -> Self {
        self.performance_optimization_level = level;
        
        // Update audio processing config based on performance level
        match level {
            PerformanceLevel::Basic => {
                self.audio_processing_config.enable_simd_optimizations = false;
                self.audio_processing_config.use_zero_copy_frames = false;
                self.audio_processing_config.enable_performance_metrics = false;
                self.audio_processing_config.frame_pool_size = 8;
            }
            PerformanceLevel::Balanced => {
                self.audio_processing_config.enable_simd_optimizations = true;
                self.audio_processing_config.use_zero_copy_frames = true;
                self.audio_processing_config.enable_performance_metrics = true;
                self.audio_processing_config.frame_pool_size = 16;
            }
            PerformanceLevel::HighPerformance => {
                self.audio_processing_config.enable_simd_optimizations = true;
                self.audio_processing_config.use_zero_copy_frames = true;
                self.audio_processing_config.enable_performance_metrics = true;
                self.audio_processing_config.frame_pool_size = 32;
            }
        }
        
        self
    }
}

/// Factory for creating advanced audio processors
#[derive(Debug)]
pub struct AdvancedProcessorFactory {
    /// Default performance configuration
    default_performance_config: PerformanceMetrics,
    /// Advanced processing configuration
    advanced_config: Option<super::config::AdvancedProcessingConfig>,
}

impl AdvancedProcessorFactory {
    /// Create a new processor factory
    pub fn new() -> Self {
        Self {
            default_performance_config: PerformanceMetrics::new(),
            advanced_config: None,
        }
    }
    
    /// Create a new processor factory with advanced configuration
    pub fn new_with_config(config: super::config::AdvancedProcessingConfig) -> Self {
        Self {
            default_performance_config: PerformanceMetrics::new(),
            advanced_config: Some(config),
        }
    }
    
    /// Create a processor set for a session
    pub async fn create_processor_set(
        &self,
        config: AudioProcessingConfig,
        sample_rate: f32,
    ) -> Result<AdvancedProcessorSet> {
        // Use factory config if available, otherwise use passed config
        let use_advanced = if let Some(factory_config) = &self.advanced_config {
            factory_config.use_advanced_processors
        } else {
            config.use_advanced_vad || config.use_advanced_agc || config.use_advanced_aec
        };

        if !use_advanced {
            // Return empty processor set if advanced processing is disabled
            return Ok(AdvancedProcessorSet { vad: None, agc: None, aec: None });
        }
        
        let vad = if config.use_advanced_vad && config.enable_vad {
            let vad_config = if let Some(factory_config) = &self.advanced_config {
                factory_config.advanced_vad_config.clone()
            } else {
                config.advanced_vad_config.clone()
            };
            
            match AdvancedVoiceActivityDetector::new(vad_config, sample_rate) {
                Ok(detector) => Some(Arc::new(RwLock::new(detector))),
                Err(e) => {
                    if let Some(factory_config) = &self.advanced_config {
                        if factory_config.fallback_to_v1_on_error {
                            warn!("Advanced VAD failed to initialize, fallback enabled but v1 VAD not implemented in factory: {}", e);
                            None
                        } else {
                            return Err(e);
                        }
                    } else {
                        return Err(e);
                    }
                }
            }
        } else {
            None
        };
        
        let agc = if config.use_advanced_agc && config.enable_agc {
            let agc_config = if let Some(factory_config) = &self.advanced_config {
                factory_config.advanced_agc_config.clone()
            } else {
                config.advanced_agc_config.clone()
            };
            
            match AdvancedAutomaticGainControl::new(agc_config, sample_rate) {
                Ok(processor) => Some(Arc::new(RwLock::new(processor))),
                Err(e) => {
                    if let Some(factory_config) = &self.advanced_config {
                        if factory_config.fallback_to_v1_on_error {
                            warn!("Advanced AGC failed to initialize, fallback enabled but v1 AGC not implemented in factory: {}", e);
                            None
                        } else {
                            return Err(e);
                        }
                    } else {
                        return Err(e);
                    }
                }
            }
        } else {
            None
        };
        
        let aec = if config.use_advanced_aec && config.enable_aec {
            let aec_config = if let Some(factory_config) = &self.advanced_config {
                factory_config.advanced_aec_config.clone()
            } else {
                config.advanced_aec_config.clone()
            };
            
            match AdvancedAcousticEchoCanceller::new(aec_config) {
                Ok(processor) => Some(Arc::new(RwLock::new(processor))),
                Err(e) => {
                    if let Some(factory_config) = &self.advanced_config {
                        if factory_config.fallback_to_v1_on_error {
                            warn!("Advanced AEC failed to initialize, fallback enabled but v1 AEC not implemented in factory: {}", e);
                            None
                        } else {
                            return Err(e);
                        }
                    } else {
                        return Err(e);
                    }
                }
            }
        } else {
            None
        };
        
        Ok(AdvancedProcessorSet { vad, agc, aec })
    }
    
    /// Get the advanced processing configuration
    pub fn get_advanced_config(&self) -> Option<&super::config::AdvancedProcessingConfig> {
        self.advanced_config.as_ref()
    }
}

/// Set of advanced audio processors for a session
pub struct AdvancedProcessorSet {
    pub vad: Option<Arc<RwLock<AdvancedVoiceActivityDetector>>>,
    pub agc: Option<Arc<RwLock<AdvancedAutomaticGainControl>>>,
    pub aec: Option<Arc<RwLock<AdvancedAcousticEchoCanceller>>>,
}

/// Handle to a media session for external operations
#[derive(Debug, Clone)]
pub struct MediaSessionHandle {
    /// Session identifier
    pub session_id: MediaSessionId,
    /// Reference to the engine for operations
    engine: Arc<MediaEngine>,
    /// Session-specific audio processor
    audio_processor: Option<Arc<AudioProcessor>>,
    /// Session-specific performance metrics
    performance_metrics: Option<Arc<RwLock<PerformanceMetrics>>>,
}

impl MediaSessionHandle {
    /// Create a new session handle
    fn new(session_id: MediaSessionId, engine: Arc<MediaEngine>) -> Self {
        Self { 
            session_id, 
            engine,
            audio_processor: None,
            performance_metrics: None,
        }
    }
    
    /// Create a new session handle with advanced processing
    fn new_with_advanced_processing(
        session_id: MediaSessionId,
        engine: Arc<MediaEngine>,
        audio_processor: Arc<AudioProcessor>,
        performance_metrics: Arc<RwLock<PerformanceMetrics>>,
    ) -> Self {
        Self {
            session_id,
            engine,
            audio_processor: Some(audio_processor),
            performance_metrics: Some(performance_metrics),
        }
    }
    
    /// Get the session ID
    pub fn id(&self) -> &MediaSessionId {
        &self.session_id
    }
    
    /// Get session statistics with performance metrics
    pub async fn get_stats(&self) -> Result<serde_json::Value> {
        let mut stats = serde_json::json!({
            "session_id": self.session_id.as_str(),
            "status": "active"
        });
        
        // Add audio processor stats if available
        if let Some(processor) = &self.audio_processor {
            let audio_stats = processor.get_stats().await;
            let performance_metrics = processor.get_performance_metrics().await;
            let pool_stats = processor.get_pool_stats();
            
            stats["audio_processing"] = serde_json::json!({
                "frames_processed": audio_stats.frames_processed,
                "total_processing_time_us": audio_stats.total_processing_time_us,
                "voice_frames": audio_stats.voice_frames,
                "silence_frames": audio_stats.silence_frames,
                "avg_agc_gain": audio_stats.avg_agc_gain,
                "performance": {
                    "operation_count": performance_metrics.operation_count,
                    "avg_time_us": performance_metrics.avg_time.as_micros(),
                    "allocation_count": performance_metrics.allocation_count,
                    "memory_allocated": performance_metrics.memory_allocated,
                },
                "pool": {
                    "pool_size": pool_stats.pool_size,
                    "pool_hits": pool_stats.pool_hits,
                    "pool_misses": pool_stats.pool_misses,
                },
                "simd_available": processor.is_simd_available(),
                "advanced_processors_enabled": processor.are_advanced_processors_enabled(),
            });
        }
        
        Ok(stats)
    }
    
    /// Get audio processor for this session
    pub fn audio_processor(&self) -> Option<&Arc<AudioProcessor>> {
        self.audio_processor.as_ref()
    }
    
    /// Get performance metrics for this session
    pub async fn get_performance_metrics(&self) -> Option<PerformanceMetrics> {
        if let Some(metrics) = &self.performance_metrics {
            Some(metrics.read().await.clone())
        } else {
            None
        }
    }
}

/// Central MediaEngine for coordinating all media processing
#[derive(Debug)]
pub struct MediaEngine {
    /// Engine configuration
    config: MediaEngineConfig,
    
    /// Lifecycle manager
    lifecycle: Arc<LifecycleManager>,
    
    /// Active media sessions
    sessions: RwLock<HashMap<MediaSessionId, MediaSessionHandle>>,
    
    /// Engine capabilities
    capabilities: EngineCapabilities,
    
    // NEW: Performance infrastructure
    /// Global frame pool for all sessions
    global_frame_pool: Arc<AudioFramePool>,
    /// Performance metrics for the entire engine
    performance_metrics: Arc<RwLock<PerformanceMetrics>>,
    /// Session-specific frame pools
    session_pools: RwLock<HashMap<MediaSessionId, Arc<AudioFramePool>>>,
    /// Factory for creating advanced processors
    advanced_processor_factory: Arc<AdvancedProcessorFactory>,
    
    // TODO: Add component managers when implemented
    // codec_manager: Arc<CodecManager>,
    // session_manager: Arc<SessionManager>,
    // quality_monitor: Arc<QualityMonitor>,
}

impl MediaEngine {
    /// Create a new MediaEngine with the given configuration
    pub async fn new(config: MediaEngineConfig) -> Result<Arc<Self>> {
        info!("Creating new MediaEngine with enhanced configuration");
        
        // Create engine capabilities based on config
        let capabilities = Self::build_capabilities(&config);
        
        // Initialize global frame pool based on performance config
        let global_pool_config = PoolConfig {
            initial_size: if config.performance.enable_frame_pooling { 
                config.performance.frame_pool_size 
            } else { 
                8 // Minimal pool if pooling disabled
            },
            max_size: if config.performance.enable_frame_pooling {
                config.performance.frame_pool_size * 4 // Allow 4x growth
            } else {
                16 // Minimal max if pooling disabled
            },
            sample_rate: match config.audio.default_sample_rate {
                SampleRate::Rate8000 => 8000,
                SampleRate::Rate16000 => 16000,
                SampleRate::Rate32000 => 32000,
                SampleRate::Rate48000 => 48000,
            },
            channels: 1,
            samples_per_frame: match config.audio.default_sample_rate {
                SampleRate::Rate8000 => 160,  // 20ms at 8kHz
                SampleRate::Rate16000 => 320, // 20ms at 16kHz
                SampleRate::Rate32000 => 640, // 20ms at 32kHz
                SampleRate::Rate48000 => 960, // 20ms at 48kHz
            },
        };
        let global_frame_pool = AudioFramePool::new(global_pool_config);
        
        // Initialize performance infrastructure based on config
        let performance_metrics = Arc::new(RwLock::new(
            if config.performance.enable_performance_metrics {
                PerformanceMetrics::new()
            } else {
                PerformanceMetrics::disabled() // Create disabled metrics collector
            }
        ));
        let session_pools = RwLock::new(HashMap::new());
        let advanced_processor_factory = Arc::new(AdvancedProcessorFactory::new_with_config(
            config.advanced_processing.clone()
        ));
        
        let engine = Arc::new(Self {
            config,
            lifecycle: Arc::new(LifecycleManager::new()),
            sessions: RwLock::new(HashMap::new()),
            capabilities,
            // Performance infrastructure
            global_frame_pool,
            performance_metrics,
            session_pools,
            advanced_processor_factory,
        });
        
        debug!("MediaEngine created with performance optimizations: zero_copy={}, simd={}, pooling={}, advanced_processors={}",
               engine.config.performance.enable_zero_copy,
               engine.config.performance.enable_simd_optimizations,
               engine.config.performance.enable_frame_pooling,
               engine.config.advanced_processing.use_advanced_processors);
        
        Ok(engine)
    }
    
    /// Start the MediaEngine
    pub async fn start(self: &Arc<Self>) -> Result<()> {
        info!("Starting MediaEngine");
        self.lifecycle.start().await?;
        info!("MediaEngine started and ready");
        Ok(())
    }
    
    /// Stop the MediaEngine
    pub async fn stop(&self) -> Result<()> {
        info!("Stopping MediaEngine");
        
        // Close all active sessions first
        self.close_all_sessions().await?;
        
        // Then stop the engine
        self.lifecycle.stop().await?;
        info!("MediaEngine stopped");
        Ok(())
    }
    
    /// Get the current engine state
    pub async fn state(&self) -> EngineState {
        self.lifecycle.state().await
    }
    
    /// Check if the engine is running
    pub async fn is_running(&self) -> bool {
        self.lifecycle.is_running().await
    }
    
    /// Create a new media session for a SIP dialog (legacy method)
    pub async fn create_media_session(
        self: &Arc<Self>,
        dialog_id: DialogId,
        params: MediaSessionParams,
    ) -> Result<MediaSessionHandle> {
        // Check if engine is running
        if !self.is_running().await {
            return Err(Error::config("MediaEngine is not running"));
        }
        
        let session_id = MediaSessionId::from_dialog(&dialog_id);
        
        // Check if session already exists
        {
            let sessions = self.sessions.read().await;
            if sessions.contains_key(&session_id) {
                return Err(Error::session_not_found(session_id.as_str()));
            }
        }
        
        info!("Creating media session for dialog: {}", dialog_id);
        
        // TODO: Create actual MediaSession with components
        // For now, create a placeholder handle
        let handle = MediaSessionHandle::new(session_id.clone(), self.clone());
        
        // Store the session
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(session_id.clone(), handle.clone());
        }
        
        debug!("Media session created: {}", session_id);
        Ok(handle)
    }
    
    /// Create a new media session with advanced v2 processors and performance optimizations
    pub async fn create_media_session_v2(
        self: &Arc<Self>,
        dialog_id: DialogId,
        params: MediaSessionParams,
    ) -> Result<MediaSessionHandle> {
        // Check if engine is running
        if !self.is_running().await {
            return Err(Error::config("MediaEngine is not running"));
        }
        
        let session_id = MediaSessionId::from_dialog(&dialog_id);
        
        // Check if session already exists
        {
            let sessions = self.sessions.read().await;
            if sessions.contains_key(&session_id) {
                return Err(Error::session_not_found(session_id.as_str()));
            }
        }
        
        info!("Creating advanced media session for dialog: {} with performance level: {:?}", 
              dialog_id, params.performance_optimization_level);
        
        // 1. Create session-specific performance pool
        let session_pool = self.create_session_pool(&session_id, &params).await?;
        
        // 2. Create audio processor with advanced configuration
        let audio_processor = AudioProcessor::new(params.audio_processing_config.clone())?;
        let audio_processor = Arc::new(audio_processor);
        
        // 3. Set up performance monitoring for the session
        let session_metrics = Arc::new(RwLock::new(PerformanceMetrics::new()));
        
        // 4. Create session handle with advanced capabilities
        let handle = MediaSessionHandle::new_with_advanced_processing(
            session_id.clone(),
            self.clone(),
            audio_processor,
            session_metrics,
        );
        
        // Store the session
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(session_id.clone(), handle.clone());
        }
        
        debug!("Advanced media session created: {} with advanced processors: VAD={}, AGC={}, AEC={}",
               session_id, params.enable_spectral_vad, params.enable_multi_band_agc, params.enable_advanced_echo_cancellation);
        
        Ok(handle)
    }
    
    /// Create session-specific performance pool
    async fn create_session_pool(&self, session_id: &MediaSessionId, params: &MediaSessionParams) -> Result<Arc<AudioFramePool>> {
        let pool_config = match params.performance_optimization_level {
            PerformanceLevel::Basic => PoolConfig {
                initial_size: 8,
                max_size: 16,
                sample_rate: 8000,
                channels: 1,
                samples_per_frame: 160,
            },
            PerformanceLevel::Balanced => PoolConfig {
                initial_size: 16,
                max_size: 32,
                sample_rate: 8000,
                channels: 1,
                samples_per_frame: 160,
            },
            PerformanceLevel::HighPerformance => PoolConfig {
                initial_size: 32,
                max_size: 64,
                sample_rate: 8000,
                channels: 1,
                samples_per_frame: 160,
            },
        };
        
        let session_pool = AudioFramePool::new(pool_config);
        
        // Store the pool for later cleanup
        {
            let mut pools = self.session_pools.write().await;
            pools.insert(session_id.clone(), session_pool.clone());
        }
        
        Ok(session_pool)
    }
    
    /// Destroy a media session
    pub async fn destroy_media_session(&self, dialog_id: DialogId) -> Result<()> {
        let session_id = MediaSessionId::from_dialog(&dialog_id);
        
        info!("Destroying media session for dialog: {}", dialog_id);
        
        // Remove from active sessions
        let session_handle = {
            let mut sessions = self.sessions.write().await;
            sessions.remove(&session_id)
        };
        
        if session_handle.is_none() {
            warn!("Attempted to destroy non-existent session: {}", session_id);
            return Err(Error::session_not_found(session_id.as_str()));
        }
        
        // Clean up session-specific resources
        {
            let mut pools = self.session_pools.write().await;
            if let Some(session_pool) = pools.remove(&session_id) {
                // Clear the session pool to free memory
                session_pool.clear();
                debug!("Session pool cleared for: {}", session_id);
            }
        }
        
        debug!("Media session destroyed with resource cleanup: {}", session_id);
        Ok(())
    }
    
    /// Get supported codec capabilities
    pub fn get_supported_codecs(&self) -> Vec<AudioCodecCapability> {
        self.capabilities.audio_codecs.clone()
    }
    
    /// Get complete engine capabilities
    pub fn get_media_capabilities(&self) -> &EngineCapabilities {
        &self.capabilities
    }
    
    /// Get number of active sessions
    pub async fn session_count(&self) -> usize {
        self.sessions.read().await.len()
    }
    
    /// Get global performance metrics
    pub async fn get_global_performance_metrics(&self) -> PerformanceMetrics {
        self.performance_metrics.read().await.clone()
    }
    
    /// Get global frame pool statistics
    pub fn get_global_pool_stats(&self) -> crate::performance::pool::PoolStats {
        self.global_frame_pool.get_stats()
    }
    
    /// Get session pool statistics
    pub async fn get_session_pool_stats(&self) -> HashMap<MediaSessionId, crate::performance::pool::PoolStats> {
        let pools = self.session_pools.read().await;
        pools.iter()
            .map(|(id, pool)| (id.clone(), pool.get_stats()))
            .collect()
    }
    
    /// Get comprehensive engine performance report
    pub async fn get_performance_report(&self) -> serde_json::Value {
        let global_metrics = self.get_global_performance_metrics().await;
        let global_pool_stats = self.get_global_pool_stats();
        let session_pool_stats = self.get_session_pool_stats().await;
        let session_count = self.session_count().await;
        
        serde_json::json!({
            "engine": {
                "session_count": session_count,
                "global_performance": {
                    "operation_count": global_metrics.operation_count,
                    "avg_time_us": global_metrics.avg_time.as_micros(),
                    "total_time_us": global_metrics.total_time.as_micros(),
                    "allocation_count": global_metrics.allocation_count,
                    "memory_allocated": global_metrics.memory_allocated,
                },
                "global_pool": {
                    "pool_size": global_pool_stats.pool_size,
                    "allocated_count": global_pool_stats.allocated_count,
                    "returned_count": global_pool_stats.returned_count,
                    "pool_hits": global_pool_stats.pool_hits,
                    "pool_misses": global_pool_stats.pool_misses,
                    "max_pool_size": global_pool_stats.max_pool_size,
                },
                "session_pools": session_pool_stats.iter().map(|(id, stats)| {
                    serde_json::json!({
                        "session_id": id.as_str(),
                        "pool_size": stats.pool_size,
                        "pool_hits": stats.pool_hits,
                        "pool_misses": stats.pool_misses,
                    })
                }).collect::<Vec<_>>(),
            }
        })
    }
    
    /// Close all active sessions
    async fn close_all_sessions(&self) -> Result<()> {
        let session_ids: Vec<MediaSessionId> = {
            let sessions = self.sessions.read().await;
            sessions.keys().cloned().collect()
        };
        
        for session_id in session_ids {
            // TODO: Gracefully close each session
            debug!("Closing session: {}", session_id);
        }
        
        // Clear all sessions
        {
            let mut sessions = self.sessions.write().await;
            sessions.clear();
        }
        
        info!("All media sessions closed");
        Ok(())
    }
    
    /// Build engine capabilities from configuration
    fn build_capabilities(config: &MediaEngineConfig) -> EngineCapabilities {
        use crate::types::{SampleRate, payload_types};
        
        let audio_codecs = config.codecs.enabled_payload_types.iter()
            .filter_map(|&pt| {
                match pt {
                    payload_types::PCMU => Some(AudioCodecCapability {
                        payload_type: pt,
                        name: "PCMU".to_string(),
                        sample_rates: vec![SampleRate::Rate8000],
                        channels: 1,
                        clock_rate: 8000,
                    }),
                    payload_types::PCMA => Some(AudioCodecCapability {
                        payload_type: pt,
                        name: "PCMA".to_string(),
                        sample_rates: vec![SampleRate::Rate8000],
                        channels: 1,
                        clock_rate: 8000,
                    }),
                    payload_types::OPUS => Some(AudioCodecCapability {
                        payload_type: pt,
                        name: "opus".to_string(),
                        sample_rates: vec![
                            SampleRate::Rate8000,
                            SampleRate::Rate16000,
                            SampleRate::Rate48000,
                        ],
                        channels: 1, // Mono for now
                        clock_rate: 48000,
                    }),
                    _ => None,
                }
            })
            .collect();
        
        EngineCapabilities {
            audio_codecs,
            audio_processing: Default::default(),
            sample_rates: vec![
                SampleRate::Rate8000,
                SampleRate::Rate16000,
                SampleRate::Rate48000,
            ],
            max_sessions: config.performance.max_sessions,
        }
    }
} 
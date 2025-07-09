//! Codec Fallback Mechanism for RTP Streams
//!
//! This module provides fallback handling when incoming RTP streams use different
//! codecs than negotiated during SDP. It supports transcoding between compatible
//! codecs and graceful degradation to passthrough mode when transcoding fails.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, Mutex};
use tracing::{debug, error, info, warn};

use crate::codec::mapping::CodecMapper;
use crate::codec::transcoding::{Transcoder, TranscodingSession, TranscodingPath};
use crate::processing::format::FormatConverter;
use crate::types::{DialogId, AudioFrame};
use crate::error::{Error, Result};

use super::codec_detection::{CodecDetector, CodecDetectionResult};

/// Fallback operation mode
#[derive(Debug, Clone, PartialEq)]
pub enum FallbackMode {
    /// No fallback needed - codecs match
    None,
    /// Transcoding between different codecs
    Transcoding { 
        from_codec: String,
        to_codec: String,
        from_payload_type: u8,
        to_payload_type: u8,
    },
    /// Passthrough mode - no transcoding, just forward packets
    Passthrough {
        detected_codec: String,
        detected_payload_type: u8,
    },
}

/// Statistics for fallback operations
#[derive(Debug, Clone)]
pub struct FallbackStats {
    /// Total number of packets processed
    pub total_packets: u64,
    /// Number of packets successfully transcoded
    pub transcoded_packets: u64,
    /// Number of packets passed through unchanged
    pub passthrough_packets: u64,
    /// Number of packets dropped due to errors
    pub dropped_packets: u64,
    /// Number of transcoding errors
    pub transcoding_errors: u64,
    /// Success rate for transcoding operations
    pub transcoding_success_rate: f32,
    /// Average transcoding latency in microseconds
    pub avg_transcoding_latency_us: u64,
    /// Current fallback mode
    pub current_mode: FallbackMode,
    /// Whether transcoding is currently active
    pub transcoding_active: bool,
    /// Total transcoding time in microseconds
    pub total_transcoding_time_us: u64,
    /// Last statistics update time
    pub last_updated: Instant,
}

impl Default for FallbackStats {
    fn default() -> Self {
        Self {
            total_packets: 0,
            transcoded_packets: 0,
            passthrough_packets: 0,
            dropped_packets: 0,
            transcoding_errors: 0,
            transcoding_success_rate: 0.0,
            avg_transcoding_latency_us: 0,
            current_mode: FallbackMode::None,
            transcoding_active: false,
            total_transcoding_time_us: 0,
            last_updated: Instant::now(),
        }
    }
}

impl FallbackStats {
    /// Update statistics with transcoding result
    pub fn update_transcoding(&mut self, success: bool, latency_us: u64) {
        self.total_packets += 1;
        self.total_transcoding_time_us += latency_us;
        
        if success {
            self.transcoded_packets += 1;
        } else {
            self.transcoding_errors += 1;
            self.dropped_packets += 1;
        }
        
        self.update_success_rate();
        self.update_avg_latency();
        self.last_updated = Instant::now();
    }
    
    /// Update statistics with passthrough result
    pub fn update_passthrough(&mut self) {
        self.total_packets += 1;
        self.passthrough_packets += 1;
        self.last_updated = Instant::now();
    }
    
    /// Update success rate calculation
    fn update_success_rate(&mut self) {
        if self.total_packets > 0 {
            self.transcoding_success_rate = self.transcoded_packets as f32 / self.total_packets as f32;
        }
    }
    
    /// Update average latency calculation
    fn update_avg_latency(&mut self) {
        if self.transcoded_packets > 0 {
            self.avg_transcoding_latency_us = self.total_transcoding_time_us / self.transcoded_packets;
        }
    }
    
    /// Get fallback efficiency (0.0 to 1.0)
    pub fn get_efficiency(&self) -> f32 {
        if self.total_packets == 0 {
            return 1.0;
        }
        
        let successful_packets = self.transcoded_packets + self.passthrough_packets;
        successful_packets as f32 / self.total_packets as f32
    }
    
    /// Check if fallback is performing well
    pub fn is_performing_well(&self) -> bool {
        self.get_efficiency() > 0.95 && self.avg_transcoding_latency_us < 5000 // 5ms threshold
    }
}

/// Configuration for fallback handling
#[derive(Debug, Clone)]
pub struct FallbackConfig {
    /// Maximum number of transcoding errors before switching to passthrough
    pub max_transcoding_errors: u32,
    /// Minimum confidence threshold for codec detection
    pub min_detection_confidence: f32,
    /// Maximum allowed transcoding latency in microseconds
    pub max_transcoding_latency_us: u64,
    /// Whether to enable automatic fallback to passthrough mode
    pub enable_auto_passthrough: bool,
    /// Time window for error rate calculation
    pub error_rate_window: Duration,
    /// Maximum error rate before fallback (0.0 to 1.0)
    pub max_error_rate: f32,
    /// Whether to enable fallback statistics collection
    pub enable_statistics: bool,
}

impl Default for FallbackConfig {
    fn default() -> Self {
        Self {
            max_transcoding_errors: 10,
            min_detection_confidence: 0.8,
            max_transcoding_latency_us: 10000, // 10ms
            enable_auto_passthrough: true,
            error_rate_window: Duration::from_secs(30),
            max_error_rate: 0.1, // 10% error rate
            enable_statistics: true,
        }
    }
}

/// Fallback handler for a specific dialog
pub struct FallbackHandler {
    /// Dialog ID
    dialog_id: DialogId,
    /// Expected codec based on SDP negotiation
    expected_codec: Option<String>,
    /// Expected payload type
    expected_payload_type: u8,
    /// Current fallback mode
    current_mode: FallbackMode,
    /// Transcoding session (if active)
    transcoder: Option<Arc<Mutex<Transcoder>>>,
    /// Fallback statistics
    stats: FallbackStats,
    /// Configuration
    config: FallbackConfig,
    /// Error count in current window
    error_count: u32,
    /// Last error reset time
    last_error_reset: Instant,
    /// Whether fallback is active
    active: bool,
}

impl FallbackHandler {
    /// Create a new fallback handler
    pub fn new(
        dialog_id: DialogId,
        expected_codec: Option<String>,
        expected_payload_type: u8,
        config: FallbackConfig,
    ) -> Self {
        let mut stats = FallbackStats::default();
        stats.current_mode = FallbackMode::None;
        stats.last_updated = Instant::now();
        
        Self {
            dialog_id,
            expected_codec,
            expected_payload_type,
            current_mode: FallbackMode::None,
            transcoder: None,
            stats,
            config,
            error_count: 0,
            last_error_reset: Instant::now(),
            active: false,
        }
    }
    
    /// Activate fallback with detected codec information
    pub async fn activate_fallback(
        &mut self,
        detected_codec: String,
        detected_payload_type: u8,
        confidence: f32,
        codec_mapper: &CodecMapper,
    ) -> Result<()> {
        if confidence < self.config.min_detection_confidence {
            return Err(Error::config(format!(
                "Detection confidence {} below threshold {}",
                confidence, self.config.min_detection_confidence
            )));
        }
        
        info!("🔄 Activating fallback for dialog {}: {} (PT:{}) → {} (PT:{})",
              self.dialog_id, detected_codec, detected_payload_type,
              self.expected_codec.as_deref().unwrap_or("unknown"), self.expected_payload_type);
        
        // Determine fallback mode
        let fallback_mode = if let Some(expected_codec) = self.expected_codec.clone() {
            if self.can_transcode(&detected_codec, &expected_codec) {
                // Attempt transcoding
                match self.setup_transcoding(&detected_codec, &expected_codec, detected_payload_type).await {
                    Ok(()) => {
                        FallbackMode::Transcoding {
                            from_codec: detected_codec,
                            to_codec: expected_codec,
                            from_payload_type: detected_payload_type,
                            to_payload_type: self.expected_payload_type,
                        }
                    },
                    Err(e) => {
                        warn!("Failed to setup transcoding for dialog {}: {}, falling back to passthrough",
                              self.dialog_id, e);
                        FallbackMode::Passthrough {
                            detected_codec,
                            detected_payload_type,
                        }
                    }
                }
            } else {
                // Transcoding not supported, use passthrough
                FallbackMode::Passthrough {
                    detected_codec,
                    detected_payload_type,
                }
            }
        } else {
            // No expected codec, use passthrough
            FallbackMode::Passthrough {
                detected_codec,
                detected_payload_type,
            }
        };
        
        // Update state
        self.current_mode = fallback_mode.clone();
        self.stats.current_mode = fallback_mode;
        self.stats.transcoding_active = matches!(self.current_mode, FallbackMode::Transcoding { .. });
        self.active = true;
        
        info!("✅ Fallback activated for dialog {}: mode={:?}", self.dialog_id, self.current_mode);
        Ok(())
    }
    
    /// Check if transcoding is supported between two codecs
    fn can_transcode(&self, from_codec: &str, to_codec: &str) -> bool {
        // Support transcoding between G.711 variants and G.729
        let supported_codecs = ["PCMU", "PCMA", "G729"];
        
        supported_codecs.contains(&from_codec) && supported_codecs.contains(&to_codec)
    }
    
    /// Setup transcoding session
    async fn setup_transcoding(
        &mut self,
        from_codec: &str,
        to_codec: &str,
        from_payload_type: u8,
    ) -> Result<()> {
        debug!("Setting up transcoding session: {} → {} for dialog {}", 
               from_codec, to_codec, self.dialog_id);
        
        // Create format converter and transcoder
        let format_converter = Arc::new(tokio::sync::RwLock::new(FormatConverter::new()));
        let transcoder = Transcoder::new(format_converter);
        
        self.transcoder = Some(Arc::new(Mutex::new(transcoder)));
        
        debug!("✅ Transcoding session setup complete for dialog {}", self.dialog_id);
        Ok(())
    }
    
    /// Process an audio frame through the fallback mechanism
    pub async fn process_frame(&mut self, frame: AudioFrame) -> Result<Option<AudioFrame>> {
        if !self.active {
            return Ok(Some(frame)); // No fallback needed
        }
        
        let start_time = Instant::now();
        
        match &self.current_mode {
            FallbackMode::None => Ok(Some(frame)),
            FallbackMode::Transcoding { .. } => {
                self.process_transcoding(frame, start_time).await
            },
            FallbackMode::Passthrough { .. } => {
                self.process_passthrough(frame).await
            },
        }
    }
    
    /// Process frame through transcoding
    async fn process_transcoding(&mut self, frame: AudioFrame, start_time: Instant) -> Result<Option<AudioFrame>> {
        // Extract codec information for transcoding
        let (from_payload_type, to_payload_type) = match &self.current_mode {
            FallbackMode::Transcoding { from_payload_type, to_payload_type, .. } => {
                (*from_payload_type, *to_payload_type)
            },
            _ => return Ok(Some(frame)), // Not in transcoding mode
        };
        
        // Clone the transcoder Arc to avoid borrow checker issues
        let transcoder = match self.transcoder.clone() {
            Some(transcoder) => transcoder,
            None => {
                error!("No transcoding session available for dialog {}", self.dialog_id);
                return Ok(None);
            }
        };
        
        // Convert AudioFrame to bytes for transcoding
        // Note: This is a simplified conversion - convert i16 samples to u8 bytes
        let input_bytes: Vec<u8> = frame.samples.iter()
            .flat_map(|&sample| sample.to_le_bytes().to_vec())
            .collect();
        
        // Perform transcoding
        let transcoding_result = {
            let mut transcoder_guard = transcoder.lock().await;
            transcoder_guard.transcode(&input_bytes, from_payload_type, to_payload_type).await
        };
        
        match transcoding_result {
            Ok(transcoded_bytes) => {
                let latency_us = start_time.elapsed().as_micros() as u64;
                
                // Check latency threshold
                if latency_us > self.config.max_transcoding_latency_us {
                    warn!("High transcoding latency for dialog {}: {}μs", self.dialog_id, latency_us);
                    
                    if self.config.enable_auto_passthrough {
                        self.switch_to_passthrough().await?;
                        return self.process_passthrough(frame).await;
                    }
                }
                
                if self.config.enable_statistics {
                    self.stats.update_transcoding(true, latency_us);
                }
                
                // Convert bytes back to AudioFrame
                // Note: This is a simplified conversion - convert u8 bytes back to i16 samples
                let transcoded_samples: Vec<i16> = transcoded_bytes.chunks_exact(2)
                    .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
                    .collect();
                
                let transcoded_frame = AudioFrame {
                    samples: transcoded_samples,
                    sample_rate: frame.sample_rate, // May need adjustment based on target codec
                    channels: frame.channels,
                    duration: frame.duration,
                    timestamp: frame.timestamp,
                };
                
                Ok(Some(transcoded_frame))
            },
            Err(e) => {
                let latency_us = start_time.elapsed().as_micros() as u64;
                
                error!("Transcoding error for dialog {}: {}", self.dialog_id, e);
                
                if self.config.enable_statistics {
                    self.stats.update_transcoding(false, latency_us);
                }
                
                self.handle_transcoding_error().await?;
                
                // Try passthrough as fallback
                if matches!(self.current_mode, FallbackMode::Passthrough { .. }) {
                    self.process_passthrough(frame).await
                } else {
                    Ok(None) // Drop frame
                }
            }
        }
    }
    
    /// Process frame through passthrough
    async fn process_passthrough(&mut self, frame: AudioFrame) -> Result<Option<AudioFrame>> {
        if self.config.enable_statistics {
            self.stats.update_passthrough();
        }
        
        debug!("Passthrough frame for dialog {}", self.dialog_id);
        Ok(Some(frame))
    }
    
    /// Handle transcoding errors with automatic fallback
    async fn handle_transcoding_error(&mut self) -> Result<()> {
        self.error_count += 1;
        
        // Reset error count if window expired
        if self.last_error_reset.elapsed() > self.config.error_rate_window {
            self.error_count = 1;
            self.last_error_reset = Instant::now();
        }
        
        // Check if we should switch to passthrough
        if self.error_count >= self.config.max_transcoding_errors {
            warn!("Too many transcoding errors ({}) for dialog {}, switching to passthrough",
                  self.error_count, self.dialog_id);
            
            if self.config.enable_auto_passthrough {
                self.switch_to_passthrough().await?;
            }
        }
        
        Ok(())
    }
    
    /// Switch to passthrough mode
    async fn switch_to_passthrough(&mut self) -> Result<()> {
        if let FallbackMode::Transcoding { from_codec, from_payload_type, .. } = &self.current_mode {
            info!("🔄 Switching to passthrough mode for dialog {}: {} (PT:{})",
                  self.dialog_id, from_codec, from_payload_type);
            
            // Update mode
            self.current_mode = FallbackMode::Passthrough {
                detected_codec: from_codec.clone(),
                detected_payload_type: *from_payload_type,
            };
            
            // Update statistics
            self.stats.current_mode = self.current_mode.clone();
            self.stats.transcoding_active = false;
            
            // Clean up transcoding session
            self.transcoder = None;
            
            info!("✅ Switched to passthrough mode for dialog {}", self.dialog_id);
        }
        
        Ok(())
    }
    
    /// Get current fallback statistics
    pub fn get_stats(&self) -> &FallbackStats {
        &self.stats
    }
    
    /// Check if fallback is active
    pub fn is_active(&self) -> bool {
        self.active
    }
    
    /// Get current fallback mode
    pub fn get_mode(&self) -> &FallbackMode {
        &self.current_mode
    }
    
    /// Deactivate fallback
    pub async fn deactivate(&mut self) {
        if self.active {
            info!("⏹️ Deactivating fallback for dialog {}", self.dialog_id);
            
            self.active = false;
            self.current_mode = FallbackMode::None;
            self.stats.current_mode = FallbackMode::None;
            self.stats.transcoding_active = false;
            self.transcoder = None;
            
            debug!("✅ Fallback deactivated for dialog {}", self.dialog_id);
        }
    }
}

/// Codec fallback manager
pub struct CodecFallbackManager {
    /// Fallback handlers per dialog
    handlers: RwLock<HashMap<DialogId, Arc<Mutex<FallbackHandler>>>>,
    /// Codec detector
    detector: Arc<CodecDetector>,
    /// Codec mapper
    mapper: Arc<CodecMapper>,
    /// Default configuration
    config: FallbackConfig,
}

impl CodecFallbackManager {
    /// Create a new codec fallback manager
    pub fn new(detector: Arc<CodecDetector>, mapper: Arc<CodecMapper>) -> Self {
        Self {
            handlers: RwLock::new(HashMap::new()),
            detector,
            mapper,
            config: FallbackConfig::default(),
        }
    }
    
    /// Create a new codec fallback manager with custom configuration
    pub fn with_config(
        detector: Arc<CodecDetector>,
        mapper: Arc<CodecMapper>,
        config: FallbackConfig,
    ) -> Self {
        Self {
            handlers: RwLock::new(HashMap::new()),
            detector,
            mapper,
            config,
        }
    }
    
    /// Initialize fallback for a dialog
    pub async fn initialize_fallback(
        &self,
        dialog_id: DialogId,
        expected_codec: Option<String>,
    ) -> Result<()> {
        let expected_payload_type = expected_codec
            .as_ref()
            .and_then(|codec| self.mapper.codec_to_payload(codec))
            .unwrap_or(0);
        
        let handler = FallbackHandler::new(
            dialog_id.clone(),
            expected_codec.clone(),
            expected_payload_type,
            self.config.clone(),
        );
        
        let mut handlers = self.handlers.write().await;
        handlers.insert(dialog_id.clone(), Arc::new(Mutex::new(handler)));
        
        debug!("🔧 Initialized fallback for dialog {}: expected codec={:?}, PT={}",
               dialog_id, expected_codec, expected_payload_type);
        
        Ok(())
    }
    
    /// Process a frame with fallback handling
    pub async fn process_frame(
        &self,
        dialog_id: &DialogId,
        frame: AudioFrame,
        payload_type: u8,
    ) -> Result<Option<AudioFrame>> {
        // Check for codec detection results
        if let Some(detection_result) = self.detector.process_packet(dialog_id, payload_type).await {
            self.handle_detection_result(dialog_id, detection_result).await?;
        }
        
        // Process frame through fallback handler
        let handlers = self.handlers.read().await;
        if let Some(handler) = handlers.get(dialog_id) {
            let mut handler_guard = handler.lock().await;
            handler_guard.process_frame(frame).await
        } else {
            // No handler, pass through unchanged
            Ok(Some(frame))
        }
    }
    
    /// Handle codec detection results
    async fn handle_detection_result(
        &self,
        dialog_id: &DialogId,
        detection_result: CodecDetectionResult,
    ) -> Result<()> {
        match detection_result {
            CodecDetectionResult::UnexpectedCodec {
                detected_payload_type,
                detected_codec,
                confidence,
                ..
            } => {
                info!("🔍 Unexpected codec detected for dialog {}: {} (PT:{}, confidence: {:.2})",
                      dialog_id, detected_codec.as_deref().unwrap_or("unknown"), detected_payload_type, confidence);
                
                if let Some(detected_codec) = detected_codec {
                    let handlers = self.handlers.read().await;
                    if let Some(handler) = handlers.get(dialog_id) {
                        let mut handler_guard = handler.lock().await;
                        handler_guard.activate_fallback(
                            detected_codec,
                            detected_payload_type,
                            confidence,
                            &self.mapper,
                        ).await?;
                    }
                }
            },
            CodecDetectionResult::Expected { .. } => {
                // Codec matches expectation, ensure fallback is deactivated
                let handlers = self.handlers.read().await;
                if let Some(handler) = handlers.get(dialog_id) {
                    let mut handler_guard = handler.lock().await;
                    handler_guard.deactivate().await;
                }
            },
            CodecDetectionResult::InsufficientData { .. } => {
                // Not enough data yet, continue monitoring
                debug!("Insufficient data for codec detection on dialog {}", dialog_id);
            },
        }
        
        Ok(())
    }
    
    /// Get fallback statistics for a dialog
    pub async fn get_stats(&self, dialog_id: &DialogId) -> Option<FallbackStats> {
        let handlers = self.handlers.read().await;
        if let Some(handler) = handlers.get(dialog_id) {
            let handler_guard = handler.lock().await;
            Some(handler_guard.get_stats().clone())
        } else {
            None
        }
    }
    
    /// Check if fallback is active for a dialog
    pub async fn is_active(&self, dialog_id: &DialogId) -> bool {
        let handlers = self.handlers.read().await;
        if let Some(handler) = handlers.get(dialog_id) {
            let handler_guard = handler.lock().await;
            handler_guard.is_active()
        } else {
            false
        }
    }
    
    /// Get current fallback mode for a dialog
    pub async fn get_mode(&self, dialog_id: &DialogId) -> Option<FallbackMode> {
        let handlers = self.handlers.read().await;
        if let Some(handler) = handlers.get(dialog_id) {
            let handler_guard = handler.lock().await;
            Some(handler_guard.get_mode().clone())
        } else {
            None
        }
    }
    
    /// Cleanup fallback for a dialog
    pub async fn cleanup_fallback(&self, dialog_id: &DialogId) -> bool {
        let mut handlers = self.handlers.write().await;
        if let Some(handler) = handlers.remove(dialog_id) {
            let mut handler_guard = handler.lock().await;
            handler_guard.deactivate().await;
            debug!("🧹 Cleaned up fallback for dialog {}", dialog_id);
            true
        } else {
            false
        }
    }
    
    /// Get statistics for all active fallback handlers
    pub async fn get_all_stats(&self) -> HashMap<DialogId, FallbackStats> {
        let handlers = self.handlers.read().await;
        let mut stats = HashMap::new();
        
        for (dialog_id, handler) in handlers.iter() {
            let handler_guard = handler.lock().await;
            stats.insert(dialog_id.clone(), handler_guard.get_stats().clone());
        }
        
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::mapping::CodecMapper;
    use crate::types::DialogId;
    
    #[tokio::test]
    async fn test_fallback_handler_creation() {
        let dialog_id = DialogId::new("test_dialog");
        let config = FallbackConfig::default();
        
        let handler = FallbackHandler::new(
            dialog_id.clone(),
            Some("PCMU".to_string()),
            0,
            config,
        );
        
        assert_eq!(handler.dialog_id, dialog_id);
        assert_eq!(handler.expected_codec, Some("PCMU".to_string()));
        assert_eq!(handler.expected_payload_type, 0);
        assert_eq!(handler.current_mode, FallbackMode::None);
        assert!(!handler.is_active());
    }
    
    #[tokio::test]
    async fn test_fallback_stats() {
        let mut stats = FallbackStats::default();
        
        // Test transcoding success
        stats.update_transcoding(true, 1000);
        assert_eq!(stats.total_packets, 1);
        assert_eq!(stats.transcoded_packets, 1);
        assert_eq!(stats.transcoding_errors, 0);
        assert_eq!(stats.avg_transcoding_latency_us, 1000);
        
        // Test transcoding failure
        stats.update_transcoding(false, 2000);
        assert_eq!(stats.total_packets, 2);
        assert_eq!(stats.transcoded_packets, 1);
        assert_eq!(stats.transcoding_errors, 1);
        assert_eq!(stats.dropped_packets, 1);
        
        // Test passthrough
        stats.update_passthrough();
        assert_eq!(stats.total_packets, 3);
        assert_eq!(stats.passthrough_packets, 1);
        
        // Test efficiency calculation
        let efficiency = stats.get_efficiency();
        assert_eq!(efficiency, 2.0 / 3.0); // 2 successful out of 3 total
    }
    
    #[tokio::test]
    async fn test_fallback_mode_matching() {
        let none_mode = FallbackMode::None;
        let transcoding_mode = FallbackMode::Transcoding {
            from_codec: "PCMU".to_string(),
            to_codec: "PCMA".to_string(),
            from_payload_type: 0,
            to_payload_type: 8,
        };
        let passthrough_mode = FallbackMode::Passthrough {
            detected_codec: "Opus".to_string(),
            detected_payload_type: 111,
        };
        
        assert_eq!(none_mode, FallbackMode::None);
        assert!(matches!(transcoding_mode, FallbackMode::Transcoding { .. }));
        assert!(matches!(passthrough_mode, FallbackMode::Passthrough { .. }));
    }
    
    #[tokio::test]
    async fn test_fallback_config_defaults() {
        let config = FallbackConfig::default();
        
        assert_eq!(config.max_transcoding_errors, 10);
        assert_eq!(config.min_detection_confidence, 0.8);
        assert_eq!(config.max_transcoding_latency_us, 10000);
        assert!(config.enable_auto_passthrough);
        assert_eq!(config.max_error_rate, 0.1);
        assert!(config.enable_statistics);
    }
    
    #[tokio::test]
    async fn test_can_transcode() {
        let dialog_id = DialogId::new("test_dialog");
        let config = FallbackConfig::default();
        
        let handler = FallbackHandler::new(
            dialog_id,
            Some("PCMU".to_string()),
            0,
            config,
        );
        
        // Test supported transcoding pairs
        assert!(handler.can_transcode("PCMU", "PCMA"));
        assert!(handler.can_transcode("PCMA", "G729"));
        assert!(handler.can_transcode("G729", "PCMU"));
        
        // Test unsupported transcoding pairs
        assert!(!handler.can_transcode("Opus", "PCMU"));
        assert!(!handler.can_transcode("PCMU", "unknown"));
    }
    
    #[tokio::test]
    async fn test_fallback_manager_creation() {
        let mapper = Arc::new(CodecMapper::new());
        let detector = Arc::new(CodecDetector::new(mapper.clone()));
        
        let manager = CodecFallbackManager::new(detector, mapper);
        
        // Test initialization
        let dialog_id = DialogId::new("test_dialog");
        let result = manager.initialize_fallback(dialog_id.clone(), Some("PCMU".to_string())).await;
        assert!(result.is_ok());
        
        // Test stats retrieval
        let stats = manager.get_stats(&dialog_id).await;
        assert!(stats.is_some());
        
        // Test cleanup
        let cleaned = manager.cleanup_fallback(&dialog_id).await;
        assert!(cleaned);
    }
    
    #[tokio::test]
    async fn test_fallback_performance_check() {
        let mut stats = FallbackStats::default();
        
        // Good performance scenario
        for _ in 0..100 {
            stats.update_transcoding(true, 1000); // 1ms latency
        }
        
        assert!(stats.is_performing_well());
        assert!(stats.get_efficiency() > 0.95);
        
        // Bad performance scenario
        stats = FallbackStats::default();
        for _ in 0..50 {
            stats.update_transcoding(true, 10000); // 10ms latency
        }
        for _ in 0..50 {
            stats.update_transcoding(false, 1000); // Failures
        }
        
        assert!(!stats.is_performing_well());
        assert!(stats.get_efficiency() < 0.95);
    }
} 
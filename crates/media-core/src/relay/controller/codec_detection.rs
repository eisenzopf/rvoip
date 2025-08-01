//! Dynamic Codec Detection for RTP Streams
//!
//! This module provides functionality to detect when incoming RTP streams use
//! different codecs than what was negotiated during SDP. This is the "just in case"
//! mechanism to handle unexpected codec formats gracefully.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::codec::mapping::CodecMapper;
use crate::types::DialogId;

/// Detection state for a specific dialog
#[derive(Debug, Clone)]
pub struct DetectionState {
    /// Expected payload type based on SDP negotiation
    pub expected_payload_type: u8,
    /// Expected codec name
    pub expected_codec: Option<String>,
    /// Detected payload type from actual packets
    pub detected_payload_type: Option<u8>,
    /// Confidence level in detection (0.0 to 1.0)
    pub confidence: f32,
    /// Number of packets analyzed
    pub packets_analyzed: u64,
    /// Number of packets matching expected payload type
    pub packets_matching: u64,
    /// Number of packets with unexpected payload type
    pub packets_unexpected: u64,
    /// Last detection update time
    pub last_update: Instant,
    /// Whether detection is active
    pub detection_active: bool,
}

impl DetectionState {
    /// Create a new detection state
    pub fn new(expected_payload_type: u8, expected_codec: Option<String>) -> Self {
        Self {
            expected_payload_type,
            expected_codec,
            detected_payload_type: None,
            confidence: 0.0,
            packets_analyzed: 0,
            packets_matching: 0,
            packets_unexpected: 0,
            last_update: Instant::now(),
            detection_active: true,
        }
    }

    /// Update detection state with a new packet
    pub fn update_with_packet(&mut self, payload_type: u8) {
        if !self.detection_active {
            return;
        }

        self.packets_analyzed += 1;
        self.last_update = Instant::now();

        if payload_type == self.expected_payload_type {
            self.packets_matching += 1;
        } else {
            self.packets_unexpected += 1;
            self.detected_payload_type = Some(payload_type);
        }

        // Update confidence based on packet analysis
        self.update_confidence();
    }

    /// Update confidence level based on packet statistics
    fn update_confidence(&mut self) {
        if self.packets_analyzed == 0 {
            self.confidence = 0.0;
            return;
        }

        // Calculate confidence based on consistency of detection
        let total_packets = self.packets_analyzed as f32;
        let unexpected_ratio = self.packets_unexpected as f32 / total_packets;
        let matching_ratio = self.packets_matching as f32 / total_packets;

        // High confidence if we consistently see the same unexpected payload type
        if unexpected_ratio > 0.8 && self.packets_analyzed >= 10 {
            self.confidence = 0.9;
        } else if unexpected_ratio > 0.6 && self.packets_analyzed >= 5 {
            self.confidence = 0.7;
        } else if unexpected_ratio > 0.4 && self.packets_analyzed >= 3 {
            self.confidence = 0.5;
        } else if matching_ratio > 0.9 && self.packets_analyzed >= 5 {
            // High confidence that we're receiving expected codec
            self.confidence = 0.95;
        } else if self.packets_analyzed >= 20 {
            // After many packets, we should have high confidence either way
            self.confidence = if unexpected_ratio > 0.5 { 0.8 } else { 0.9 };
        } else {
            // Lower confidence for small sample sizes
            self.confidence = (self.packets_analyzed as f32) / 20.0;
        }
    }

    /// Check if detection state is stale (no updates for a while)
    pub fn is_stale(&self, max_age: Duration) -> bool {
        self.last_update.elapsed() > max_age
    }

    /// Get detection summary
    pub fn get_summary(&self) -> String {
        format!(
            "Expected PT: {}, Detected PT: {:?}, Confidence: {:.2}, Packets: {}/{}/{}", 
            self.expected_payload_type,
            self.detected_payload_type,
            self.confidence,
            self.packets_matching,
            self.packets_unexpected,
            self.packets_analyzed
        )
    }
}

/// Result of codec detection analysis
#[derive(Debug, Clone)]
pub enum CodecDetectionResult {
    /// Packets match expected codec
    Expected { 
        payload_type: u8, 
        codec: Option<String>,
        confidence: f32,
    },
    /// Unexpected codec detected
    UnexpectedCodec { 
        expected_payload_type: u8,
        detected_payload_type: u8,
        expected_codec: Option<String>,
        detected_codec: Option<String>,
        confidence: f32,
    },
    /// Not enough data to make a determination
    InsufficientData {
        packets_analyzed: u64,
    },
}

/// Configuration for codec detection
#[derive(Debug, Clone)]
pub struct CodecDetectionConfig {
    /// Minimum confidence threshold for detection
    pub confidence_threshold: f32,
    /// Minimum number of packets before making a decision
    pub min_packets_for_detection: u64,
    /// Maximum age for detection state before cleanup
    pub max_state_age: Duration,
    /// Whether to automatically clean up stale states
    pub auto_cleanup: bool,
}

impl Default for CodecDetectionConfig {
    fn default() -> Self {
        Self {
            confidence_threshold: 0.7,
            min_packets_for_detection: 5,
            max_state_age: Duration::from_secs(300), // 5 minutes
            auto_cleanup: true,
        }
    }
}

/// Dynamic codec detector for RTP streams
pub struct CodecDetector {
    /// Codec mapper for payload type resolution
    mapper: Arc<CodecMapper>,
    /// Detection state cache per dialog
    detection_cache: Arc<RwLock<HashMap<DialogId, DetectionState>>>,
    /// Detection configuration
    config: CodecDetectionConfig,
}

impl CodecDetector {
    /// Create a new codec detector
    pub fn new(mapper: Arc<CodecMapper>) -> Self {
        Self {
            mapper,
            detection_cache: Arc::new(RwLock::new(HashMap::new())),
            config: CodecDetectionConfig::default(),
        }
    }

    /// Create a new codec detector with custom configuration
    pub fn with_config(mapper: Arc<CodecMapper>, config: CodecDetectionConfig) -> Self {
        Self {
            mapper,
            detection_cache: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    /// Initialize detection for a dialog with expected codec
    pub async fn initialize_detection(&self, dialog_id: DialogId, expected_codec: Option<String>) {
        let expected_payload_type = expected_codec
            .as_ref()
            .and_then(|codec| self.mapper.codec_to_payload(codec))
            .unwrap_or(0); // Default to PCMU

        let state = DetectionState::new(expected_payload_type, expected_codec.clone());
        
        debug!("🔍 Initialized codec detection for dialog {}: expected codec={:?}, PT={}", 
               dialog_id, expected_codec, expected_payload_type);

        let mut cache = self.detection_cache.write().await;
        cache.insert(dialog_id, state);
    }

    /// Process an RTP packet for codec detection
    pub async fn process_packet(&self, dialog_id: &DialogId, payload_type: u8) -> Option<CodecDetectionResult> {
        let mut cache = self.detection_cache.write().await;
        
        if let Some(state) = cache.get_mut(dialog_id) {
            state.update_with_packet(payload_type);
            
            // Check if we have enough data for a decision
            if state.packets_analyzed >= self.config.min_packets_for_detection {
                return Some(self.analyze_detection_state(state));
            }
        }

        None
    }

    /// Analyze current detection state and return result
    fn analyze_detection_state(&self, state: &DetectionState) -> CodecDetectionResult {
        if state.confidence < self.config.confidence_threshold {
            return CodecDetectionResult::InsufficientData {
                packets_analyzed: state.packets_analyzed,
            };
        }

        // Check if we detected an unexpected codec
        if let Some(detected_payload_type) = state.detected_payload_type {
            if detected_payload_type != state.expected_payload_type && 
               state.packets_unexpected > state.packets_matching {
                
                let detected_codec = self.mapper.payload_to_codec(detected_payload_type);
                
                return CodecDetectionResult::UnexpectedCodec {
                    expected_payload_type: state.expected_payload_type,
                    detected_payload_type,
                    expected_codec: state.expected_codec.clone(),
                    detected_codec,
                    confidence: state.confidence,
                };
            }
        }

        // Expected codec confirmed
        CodecDetectionResult::Expected {
            payload_type: state.expected_payload_type,
            codec: state.expected_codec.clone(),
            confidence: state.confidence,
        }
    }

    /// Get detection result for a dialog
    pub async fn get_detection_result(&self, dialog_id: &DialogId) -> Option<CodecDetectionResult> {
        let cache = self.detection_cache.read().await;
        
        if let Some(state) = cache.get(dialog_id) {
            if state.packets_analyzed >= self.config.min_packets_for_detection {
                return Some(self.analyze_detection_state(state));
            }
        }

        None
    }

    /// Get detection state for a dialog
    pub async fn get_detection_state(&self, dialog_id: &DialogId) -> Option<DetectionState> {
        let cache = self.detection_cache.read().await;
        cache.get(dialog_id).cloned()
    }

    /// Clean up detection state for a dialog
    pub async fn cleanup_detection(&self, dialog_id: &DialogId) -> bool {
        let mut cache = self.detection_cache.write().await;
        if let Some(state) = cache.remove(dialog_id) {
            debug!("🧹 Cleaned up codec detection for dialog {}: {}", dialog_id, state.get_summary());
            true
        } else {
            false
        }
    }

    /// Clean up stale detection states
    pub async fn cleanup_stale_states(&self) -> usize {
        if !self.config.auto_cleanup {
            return 0;
        }

        let mut cache = self.detection_cache.write().await;
        let mut to_remove = Vec::new();

        for (dialog_id, state) in cache.iter() {
            if state.is_stale(self.config.max_state_age) {
                to_remove.push(dialog_id.clone());
            }
        }

        let removed_count = to_remove.len();
        for dialog_id in to_remove {
            if let Some(state) = cache.remove(&dialog_id) {
                debug!("🧹 Cleaned up stale codec detection for dialog {}: {}", dialog_id, state.get_summary());
            }
        }

        if removed_count > 0 {
            info!("🧹 Cleaned up {} stale codec detection states", removed_count);
        }

        removed_count
    }

    /// Get statistics about detection cache
    pub async fn get_cache_stats(&self) -> HashMap<String, u64> {
        let cache = self.detection_cache.read().await;
        let mut stats = HashMap::new();
        
        stats.insert("total_states".to_string(), cache.len() as u64);
        
        let mut active_states = 0;
        let mut stale_states = 0;
        let mut total_packets = 0;

        for state in cache.values() {
            if state.detection_active {
                active_states += 1;
            }
            if state.is_stale(self.config.max_state_age) {
                stale_states += 1;
            }
            total_packets += state.packets_analyzed;
        }

        stats.insert("active_states".to_string(), active_states);
        stats.insert("stale_states".to_string(), stale_states);
        stats.insert("total_packets_analyzed".to_string(), total_packets);

        stats
    }

    /// Pause detection for a dialog
    pub async fn pause_detection(&self, dialog_id: &DialogId) -> bool {
        let mut cache = self.detection_cache.write().await;
        if let Some(state) = cache.get_mut(dialog_id) {
            state.detection_active = false;
            debug!("⏸️ Paused codec detection for dialog {}", dialog_id);
            true
        } else {
            false
        }
    }

    /// Resume detection for a dialog
    pub async fn resume_detection(&self, dialog_id: &DialogId) -> bool {
        let mut cache = self.detection_cache.write().await;
        if let Some(state) = cache.get_mut(dialog_id) {
            state.detection_active = true;
            debug!("▶️ Resumed codec detection for dialog {}", dialog_id);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::mapping::CodecMapper;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn test_codec_detector_creation() {
        let mapper = Arc::new(CodecMapper::new());
        let detector = CodecDetector::new(mapper.clone());
        
        let stats = detector.get_cache_stats().await;
        assert_eq!(stats.get("total_states"), Some(&0));
    }

    #[tokio::test]
    async fn test_detection_initialization() {
        let mapper = Arc::new(CodecMapper::new());
        let detector = CodecDetector::new(mapper.clone());
        
        let dialog_id = DialogId::new("test_dialog");
        detector.initialize_detection(dialog_id.clone(), Some("PCMU".to_string())).await;
        
        let state = detector.get_detection_state(&dialog_id).await;
        assert!(state.is_some());
        let state = state.unwrap();
        assert_eq!(state.expected_payload_type, 0); // PCMU
        assert_eq!(state.expected_codec, Some("PCMU".to_string()));
    }

    #[tokio::test]
    async fn test_expected_codec_detection() {
        let mapper = Arc::new(CodecMapper::new());
        let detector = CodecDetector::new(mapper.clone());
        
        let dialog_id = DialogId::new("test_dialog");
        detector.initialize_detection(dialog_id.clone(), Some("PCMU".to_string())).await;
        
        // Send packets with expected payload type
        for _ in 0..10 {
            detector.process_packet(&dialog_id, 0).await; // PCMU
        }
        
        let result = detector.get_detection_result(&dialog_id).await;
        assert!(result.is_some());
        
        match result.unwrap() {
            CodecDetectionResult::Expected { payload_type, codec, confidence } => {
                assert_eq!(payload_type, 0);
                assert_eq!(codec, Some("PCMU".to_string()));
                assert!(confidence > 0.7);
            },
            other => panic!("Expected Expected result, got {:?}", other),
        }
    }

    // Test removed - Opus codec is not supported

    #[tokio::test]
    async fn test_insufficient_data() {
        let mapper = Arc::new(CodecMapper::new());
        let detector = CodecDetector::new(mapper.clone());
        
        let dialog_id = DialogId::new("test_dialog");
        detector.initialize_detection(dialog_id.clone(), Some("PCMU".to_string())).await;
        
        // Send only a few packets
        for _ in 0..3 {
            detector.process_packet(&dialog_id, 0).await;
        }
        
        let result = detector.get_detection_result(&dialog_id).await;
        assert!(result.is_none()); // Not enough packets for detection
    }

    #[tokio::test]
    async fn test_mixed_codec_detection() {
        let mapper = Arc::new(CodecMapper::new());
        let detector = CodecDetector::new(mapper.clone());
        
        let dialog_id = DialogId::new("test_dialog");
        detector.initialize_detection(dialog_id.clone(), Some("PCMU".to_string())).await;
        
        // Send mixed packets - mostly unexpected
        for _ in 0..8 {
            detector.process_packet(&dialog_id, 111).await; // Opus
        }
        for _ in 0..2 {
            detector.process_packet(&dialog_id, 0).await; // PCMU
        }
        
        let result = detector.get_detection_result(&dialog_id).await;
        assert!(result.is_some());
        
        match result.unwrap() {
            CodecDetectionResult::UnexpectedCodec { detected_payload_type, .. } => {
                assert_eq!(detected_payload_type, 111); // Should detect Opus as primary
            },
            other => panic!("Expected UnexpectedCodec result, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_detection_cleanup() {
        let mapper = Arc::new(CodecMapper::new());
        let detector = CodecDetector::new(mapper.clone());
        
        let dialog_id = DialogId::new("test_dialog");
        detector.initialize_detection(dialog_id.clone(), Some("PCMU".to_string())).await;
        
        // Verify state exists
        let state = detector.get_detection_state(&dialog_id).await;
        assert!(state.is_some());
        
        // Clean up
        let cleaned = detector.cleanup_detection(&dialog_id).await;
        assert!(cleaned);
        
        // Verify state is gone
        let state = detector.get_detection_state(&dialog_id).await;
        assert!(state.is_none());
    }

    #[tokio::test]
    async fn test_stale_state_cleanup() {
        let mapper = Arc::new(CodecMapper::new());
        let config = CodecDetectionConfig {
            max_state_age: Duration::from_millis(50),
            ..Default::default()
        };
        let detector = CodecDetector::with_config(mapper.clone(), config);
        
        let dialog_id = DialogId::new("test_dialog");
        detector.initialize_detection(dialog_id.clone(), Some("PCMU".to_string())).await;
        
        // Wait for state to become stale
        sleep(Duration::from_millis(100)).await;
        
        // Clean up stale states
        let cleaned = detector.cleanup_stale_states().await;
        assert_eq!(cleaned, 1);
        
        // Verify state is gone
        let state = detector.get_detection_state(&dialog_id).await;
        assert!(state.is_none());
    }

    #[tokio::test]
    async fn test_pause_resume_detection() {
        let mapper = Arc::new(CodecMapper::new());
        let detector = CodecDetector::new(mapper.clone());
        
        let dialog_id = DialogId::new("test_dialog");
        detector.initialize_detection(dialog_id.clone(), Some("PCMU".to_string())).await;
        
        // Pause detection
        let paused = detector.pause_detection(&dialog_id).await;
        assert!(paused);
        
        // Send packets while paused - should not be analyzed
        for _ in 0..10 {
            detector.process_packet(&dialog_id, 111).await;
        }
        
        let state = detector.get_detection_state(&dialog_id).await;
        assert!(state.is_some());
        assert_eq!(state.unwrap().packets_analyzed, 0); // No packets analyzed while paused
        
        // Resume detection
        let resumed = detector.resume_detection(&dialog_id).await;
        assert!(resumed);
        
        // Send packets while resumed - should be analyzed
        for _ in 0..10 {
            detector.process_packet(&dialog_id, 111).await;
        }
        
        let state = detector.get_detection_state(&dialog_id).await;
        assert!(state.is_some());
        assert_eq!(state.unwrap().packets_analyzed, 10); // Packets analyzed after resume
    }

    #[tokio::test]
    async fn test_detection_state_summary() {
        let state = DetectionState::new(0, Some("PCMU".to_string()));
        let summary = state.get_summary();
        assert!(summary.contains("Expected PT: 0"));
        assert!(summary.contains("Confidence: 0.00"));
        assert!(summary.contains("Packets: 0/0/0"));
    }

    #[tokio::test]
    async fn test_confidence_calculation() {
        let mapper = Arc::new(CodecMapper::new());
        let detector = CodecDetector::new(mapper.clone());
        
        let dialog_id = DialogId::new("test_dialog");
        detector.initialize_detection(dialog_id.clone(), Some("PCMU".to_string())).await;
        
        // Send consistent expected packets
        for _ in 0..20 {
            detector.process_packet(&dialog_id, 0).await; // PCMU
        }
        
        let state = detector.get_detection_state(&dialog_id).await;
        assert!(state.is_some());
        let state = state.unwrap();
        assert!(state.confidence > 0.9); // High confidence for consistent packets
        
        // Reset and send mixed packets
        detector.cleanup_detection(&dialog_id).await;
        detector.initialize_detection(dialog_id.clone(), Some("PCMU".to_string())).await;
        
        for _ in 0..5 {
            detector.process_packet(&dialog_id, 0).await; // PCMU
        }
        for _ in 0..5 {
            detector.process_packet(&dialog_id, 111).await; // Opus
        }
        
        let state = detector.get_detection_state(&dialog_id).await;
        assert!(state.is_some());
        let state = state.unwrap();
        assert!(state.confidence < 0.9); // Lower confidence for mixed packets
    }
} 
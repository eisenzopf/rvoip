//! Graceful degradation implementation
//!
//! This module provides codec fallback, quality reduction, and other
//! degradation strategies to maintain call continuity under poor conditions.

use crate::{
    error::{SipClientError, SipClientResult},
    events::{SipClientEvent, EventEmitter},
    recovery::{NetworkMetrics, DegradationActions},
    types::CallId,
};
use codec_core::CodecType;
use std::{
    sync::Arc,
    collections::HashMap,
};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Codec fallback chain for graceful degradation
#[derive(Debug, Clone)]
pub struct CodecFallbackChain {
    /// Ordered list of codecs from highest to lowest quality
    codecs: Vec<CodecType>,
    
    /// Current codec index
    current_index: usize,
}

impl CodecFallbackChain {
    /// Create a new codec fallback chain
    pub fn new(preferred_codecs: Vec<CodecType>) -> Self {
        // Ensure we have at least PCMU as fallback
        let mut codecs = preferred_codecs;
        if !codecs.contains(&CodecType::G711Pcmu) {
            codecs.push(CodecType::G711Pcmu);
        }
        
        Self {
            codecs,
            current_index: 0,
        }
    }
    
    /// Get the current codec
    pub fn current_codec(&self) -> CodecType {
        self.codecs.get(self.current_index)
            .copied()
            .unwrap_or(CodecType::G711Pcmu)
    }
    
    /// Downgrade to next codec in chain
    pub fn downgrade(&mut self) -> Option<CodecType> {
        if self.current_index < self.codecs.len() - 1 {
            self.current_index += 1;
            Some(self.current_codec())
        } else {
            None
        }
    }
    
    /// Upgrade to previous codec in chain
    pub fn upgrade(&mut self) -> Option<CodecType> {
        if self.current_index > 0 {
            self.current_index -= 1;
            Some(self.current_codec())
        } else {
            None
        }
    }
    
    /// Reset to highest quality codec
    pub fn reset(&mut self) {
        self.current_index = 0;
    }
}

/// Quality adaptation manager
pub struct QualityAdaptationManager {
    /// Event emitter
    event_emitter: EventEmitter,
    
    /// Codec fallback chains per call
    codec_chains: Arc<RwLock<HashMap<CallId, CodecFallbackChain>>>,
    
    /// Quality settings per call
    quality_settings: Arc<RwLock<HashMap<CallId, QualitySettings>>>,
    
    /// Network metrics history
    metrics_history: Arc<RwLock<Vec<NetworkMetrics>>>,
    
    /// Maximum history size
    max_history_size: usize,
}

/// Quality settings for a call
#[derive(Debug, Clone)]
pub struct QualitySettings {
    /// Target bitrate in bps
    pub bitrate: u32,
    
    /// Packet time in ms
    pub ptime: u32,
    
    /// Enable FEC (Forward Error Correction)
    pub fec_enabled: bool,
    
    /// Enable DTX (Discontinuous Transmission)
    pub dtx_enabled: bool,
    
    /// Audio processing enabled
    pub audio_processing: AudioProcessingSettings,
}

/// Audio processing settings
#[derive(Debug, Clone)]
pub struct AudioProcessingSettings {
    /// Echo cancellation enabled
    pub echo_cancellation: bool,
    
    /// Noise suppression enabled
    pub noise_suppression: bool,
    
    /// Automatic gain control enabled
    pub agc: bool,
    
    /// Voice activity detection enabled
    pub vad: bool,
}

impl Default for QualitySettings {
    fn default() -> Self {
        Self {
            bitrate: 64000, // 64 kbps
            ptime: 20, // 20ms
            fec_enabled: false,
            dtx_enabled: false,
            audio_processing: AudioProcessingSettings {
                echo_cancellation: true,
                noise_suppression: true,
                agc: true,
                vad: true,
            },
        }
    }
}

impl QualityAdaptationManager {
    /// Create a new quality adaptation manager
    pub fn new(event_emitter: EventEmitter) -> Self {
        Self {
            event_emitter,
            codec_chains: Arc::new(RwLock::new(HashMap::new())),
            quality_settings: Arc::new(RwLock::new(HashMap::new())),
            metrics_history: Arc::new(RwLock::new(Vec::new())),
            max_history_size: 100,
        }
    }
    
    /// Initialize quality settings for a call
    pub async fn initialize_call(&self, call_id: CallId, preferred_codecs: Vec<CodecType>) {
        let mut chains = self.codec_chains.write().await;
        chains.insert(call_id, CodecFallbackChain::new(preferred_codecs));
        
        let mut settings = self.quality_settings.write().await;
        settings.insert(call_id, QualitySettings::default());
    }
    
    /// Update network metrics and adapt quality
    pub async fn update_metrics(&self, metrics: NetworkMetrics) -> HashMap<CallId, DegradationActions> {
        // Store metrics in history
        {
            let mut history = self.metrics_history.write().await;
            history.push(metrics.clone());
            if history.len() > self.max_history_size {
                history.remove(0);
            }
        }
        
        // Analyze trends
        let trend = self.analyze_network_trend().await;
        
        // Apply adaptations to all active calls
        let chains = self.codec_chains.read().await;
        let mut actions_map = HashMap::new();
        
        for call_id in chains.keys() {
            let actions = self.adapt_quality_for_call(call_id, &metrics, &trend).await;
            if !actions.is_default() {
                actions_map.insert(*call_id, actions);
            }
        }
        
        actions_map
    }
    
    /// Adapt quality for a specific call
    async fn adapt_quality_for_call(
        &self,
        call_id: &CallId,
        metrics: &NetworkMetrics,
        trend: &NetworkTrend,
    ) -> DegradationActions {
        let mut actions = DegradationActions::default();
        
        // Determine if we need to degrade or improve
        match trend {
            NetworkTrend::Degrading => {
                info!("Network degrading for call {}, applying quality reduction", call_id);
                
                // Try codec downgrade first
                let mut chains = self.codec_chains.write().await;
                if let Some(chain) = chains.get_mut(call_id) {
                    if let Some(new_codec) = chain.downgrade() {
                        info!("Downgrading codec to {:?}", new_codec);
                        actions.codec_downgrade = true;
                        
                        self.event_emitter.emit(SipClientEvent::CodecChanged {
                            call_id: *call_id,
                            old_codec: chain.codecs[chain.current_index - 1].to_string(),
                            new_codec: new_codec.to_string(),
                            reason: "Network degradation".to_string(),
                        });
                    }
                }
                
                // Adjust quality settings
                let mut settings = self.quality_settings.write().await;
                if let Some(quality) = settings.get_mut(call_id) {
                    // Reduce bitrate
                    if metrics.packet_loss_percent > 5.0 {
                        quality.bitrate = (quality.bitrate / 2).max(8000);
                        actions.target_bitrate = Some(quality.bitrate);
                        actions.reduce_quality = true;
                    }
                    
                    // Disable audio enhancements to save CPU
                    if metrics.packet_loss_percent > 10.0 {
                        quality.audio_processing.echo_cancellation = false;
                        quality.audio_processing.noise_suppression = false;
                        actions.disable_enhancements = true;
                    }
                    
                    // Enable DTX to reduce bandwidth
                    quality.dtx_enabled = true;
                }
            }
            
            NetworkTrend::Improving => {
                debug!("Network improving for call {}, considering quality increase", call_id);
                
                // Only upgrade if metrics are consistently good
                if metrics.packet_loss_percent < 1.0 && metrics.jitter_ms < 20.0 {
                    let mut chains = self.codec_chains.write().await;
                    if let Some(chain) = chains.get_mut(call_id) {
                        if let Some(new_codec) = chain.upgrade() {
                            info!("Upgrading codec to {:?}", new_codec);
                            
                            self.event_emitter.emit(SipClientEvent::CodecChanged {
                                call_id: *call_id,
                                old_codec: chain.codecs[chain.current_index + 1].to_string(),
                                new_codec: new_codec.to_string(),
                                reason: "Network improvement".to_string(),
                            });
                        }
                    }
                    
                    // Restore quality settings
                    let mut settings = self.quality_settings.write().await;
                    if let Some(quality) = settings.get_mut(call_id) {
                        quality.bitrate = 64000;
                        quality.audio_processing.echo_cancellation = true;
                        quality.audio_processing.noise_suppression = true;
                    }
                }
            }
            
            NetworkTrend::Stable => {
                // No action needed
                debug!("Network stable for call {}", call_id);
            }
        }
        
        actions
    }
    
    /// Analyze network trend from history
    async fn analyze_network_trend(&self) -> NetworkTrend {
        let history = self.metrics_history.read().await;
        
        if history.len() < 3 {
            return NetworkTrend::Stable;
        }
        
        // Compare recent metrics to older ones
        let recent_avg = self.calculate_average_score(&history[history.len() - 3..]);
        let older_avg = self.calculate_average_score(&history[history.len().saturating_sub(10)..history.len() - 3]);
        
        if recent_avg > older_avg * 1.2 {
            NetworkTrend::Degrading
        } else if recent_avg < older_avg * 0.8 {
            NetworkTrend::Improving
        } else {
            NetworkTrend::Stable
        }
    }
    
    /// Calculate network quality score (higher is worse)
    fn calculate_average_score(&self, metrics: &[NetworkMetrics]) -> f64 {
        if metrics.is_empty() {
            return 0.0;
        }
        
        let sum: f64 = metrics.iter()
            .map(|m| {
                m.packet_loss_percent * 10.0 +
                m.jitter_ms * 0.1 +
                m.rtt_ms * 0.01 +
                m.consecutive_errors as f64 * 5.0
            })
            .sum();
        
        sum / metrics.len() as f64
    }
    
    /// Get current quality settings for a call
    pub async fn get_quality_settings(&self, call_id: &CallId) -> Option<QualitySettings> {
        let settings = self.quality_settings.read().await;
        settings.get(call_id).cloned()
    }
    
    /// Clean up resources for a call
    pub async fn cleanup_call(&self, call_id: &CallId) {
        let mut chains = self.codec_chains.write().await;
        chains.remove(call_id);
        
        let mut settings = self.quality_settings.write().await;
        settings.remove(call_id);
    }
}

/// Network quality trend
#[derive(Debug, Clone, PartialEq)]
enum NetworkTrend {
    /// Network quality is getting worse
    Degrading,
    /// Network quality is improving
    Improving,
    /// Network quality is stable
    Stable,
}

/// Default implementation for DegradationActions
impl Default for DegradationActions {
    fn default() -> Self {
        Self {
            codec_downgrade: false,
            reduce_quality: false,
            target_bitrate: None,
            disable_enhancements: false,
            disable_video: false,
            reduce_frame_rate: false,
        }
    }
}

impl DegradationActions {
    /// Check if any actions are set
    fn is_default(&self) -> bool {
        !self.codec_downgrade &&
        !self.reduce_quality &&
        self.target_bitrate.is_none() &&
        !self.disable_enhancements &&
        !self.disable_video &&
        !self.reduce_frame_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_codec_fallback_chain() {
        let codecs = vec![
            CodecType::G711Pcma,
            CodecType::G711Pcmu,
        ];
        
        let mut chain = CodecFallbackChain::new(codecs);
        
        // Initial codec should be first in list
        assert_eq!(chain.current_codec(), CodecType::G711Pcma);
        
        // Downgrade should move to next codec
        assert_eq!(chain.downgrade(), Some(CodecType::G711Pcmu));
        assert_eq!(chain.current_codec(), CodecType::G711Pcmu);
        
        // Can't downgrade past last codec
        assert_eq!(chain.downgrade(), None);
        
        // Upgrade should move back
        assert_eq!(chain.upgrade(), Some(CodecType::G711Pcma));
        assert_eq!(chain.current_codec(), CodecType::G711Pcma);
    }
    
    #[test]
    fn test_network_score_calculation() {
        let manager = QualityAdaptationManager::new(EventEmitter::default());
        
        let good_metrics = vec![
            NetworkMetrics {
                packet_loss_percent: 0.5,
                jitter_ms: 10.0,
                rtt_ms: 50.0,
                available_bandwidth_bps: Some(128000),
                consecutive_errors: 0,
            },
        ];
        
        let bad_metrics = vec![
            NetworkMetrics {
                packet_loss_percent: 10.0,
                jitter_ms: 100.0,
                rtt_ms: 300.0,
                available_bandwidth_bps: Some(32000),
                consecutive_errors: 3,
            },
        ];
        
        let good_score = manager.calculate_average_score(&good_metrics);
        let bad_score = manager.calculate_average_score(&bad_metrics);
        
        assert!(good_score < bad_score);
    }
}
//! Quality Adaptation Engine
//!
//! This module implements adaptive quality management that automatically adjusts
//! media parameters based on real-time quality monitoring and network conditions.

use tracing::{debug, info, warn};
use crate::error::Result;
use crate::types::{MediaSessionId, SampleRate};
use super::metrics::{QualityMetrics, QualityTrend, QualityGrade};

/// Quality adjustment recommendations
#[derive(Debug, Clone)]
pub struct QualityAdjustment {
    /// Session to adjust
    pub session_id: MediaSessionId,
    /// Adjustment type
    pub adjustment_type: AdjustmentType,
    /// Confidence in this adjustment (0.0-1.0)
    pub confidence: f32,
    /// Human-readable reason
    pub reason: String,
}

/// Types of quality adjustments
#[derive(Debug, Clone)]
pub enum AdjustmentType {
    /// Reduce bitrate
    ReduceBitrate { new_bitrate: u32 },
    /// Increase bitrate
    IncreaseBitrate { new_bitrate: u32 },
    /// Change codec
    ChangeCodec { codec_name: String },
    /// Adjust sample rate
    AdjustSampleRate { new_rate: SampleRate },
    /// Enable/disable processing features
    ProcessingAdjustment { feature: ProcessingFeature, enable: bool },
    /// Change packet size/interval
    AdjustPacketTiming { frame_size_ms: f32 },
}

/// Audio processing features that can be adjusted
#[derive(Debug, Clone)]
pub enum ProcessingFeature {
    EchoCancellation,
    NoiseSupression,
    AutomaticGainControl,
    VoiceActivityDetection,
}

/// Adaptation strategies
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdaptationStrategy {
    /// Conservative - only make small adjustments
    Conservative,
    /// Balanced - moderate adjustments based on quality
    Balanced,
    /// Aggressive - make large adjustments quickly
    Aggressive,
}

/// Quality adaptation engine configuration
#[derive(Debug, Clone)]
pub struct AdaptationConfig {
    /// Adaptation strategy
    pub strategy: AdaptationStrategy,
    /// Minimum confidence required for adjustments
    pub min_confidence: f32,
    /// Maximum bitrate allowed
    pub max_bitrate: u32,
    /// Minimum bitrate allowed
    pub min_bitrate: u32,
    /// Enable automatic codec switching
    pub enable_codec_switching: bool,
}

impl Default for AdaptationConfig {
    fn default() -> Self {
        Self {
            strategy: AdaptationStrategy::Balanced,
            min_confidence: 0.7, // 70% confidence required
            max_bitrate: 320_000, // 320 kbps max
            min_bitrate: 32_000,  // 32 kbps min
            enable_codec_switching: true,
        }
    }
}

/// Adaptation engine that analyzes quality and suggests adjustments
pub struct AdaptationEngine {
    /// Engine configuration
    config: AdaptationConfig,
}

impl AdaptationEngine {
    /// Create a new adaptation engine
    pub fn new(config: AdaptationConfig) -> Self {
        debug!("Creating AdaptationEngine with strategy: {:?}", config.strategy);
        Self { config }
    }
    
    /// Suggest quality adjustments based on current metrics
    pub fn suggest_adjustments(
        &self,
        session_id: &MediaSessionId,
        current_metrics: &QualityMetrics,
        trend: QualityTrend,
        current_bitrate: u32,
    ) -> Vec<QualityAdjustment> {
        let mut adjustments = Vec::new();
        
        let quality_grade = current_metrics.get_quality_grade();
        
        // Analyze packet loss
        if current_metrics.packet_loss > 5.0 {
            adjustments.extend(self.handle_packet_loss(session_id, current_metrics, current_bitrate));
        }
        
        // Analyze jitter
        if current_metrics.jitter_ms > 30.0 {
            adjustments.extend(self.handle_high_jitter(session_id, current_metrics));
        }
        
        // Analyze overall quality and trend
        match (quality_grade, trend) {
            (QualityGrade::Poor | QualityGrade::Bad, _) => {
                adjustments.extend(self.handle_poor_quality(session_id, current_metrics, current_bitrate));
            }
            (QualityGrade::Fair, QualityTrend::Degrading) => {
                adjustments.extend(self.handle_degrading_quality(session_id, current_metrics, current_bitrate));
            }
            (QualityGrade::Good | QualityGrade::Excellent, QualityTrend::Stable | QualityTrend::Improving) => {
                adjustments.extend(self.handle_good_quality(session_id, current_metrics, current_bitrate));
            }
            _ => {} // No adjustments needed
        }
        
        // Sort by priority and filter by confidence
        adjustments.sort_by(|a: &QualityAdjustment, b: &QualityAdjustment| {
            b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal)
        });
        let filtered: Vec<_> = adjustments
            .into_iter()
            .filter(|adj| adj.confidence >= self.config.min_confidence)
            .collect();
        
        filtered
    }
    
    /// Handle high packet loss situations
    fn handle_packet_loss(
        &self,
        session_id: &MediaSessionId,
        metrics: &QualityMetrics,
        current_bitrate: u32,
    ) -> Vec<QualityAdjustment> {
        let mut adjustments = Vec::new();
        
        let loss_severity = metrics.packet_loss / 10.0; // Normalize to 0-1+ range
        let confidence = (loss_severity * 0.8).min(1.0);
        
        // Reduce bitrate to help with congestion
        if current_bitrate > self.config.min_bitrate {
            let reduction = match self.config.strategy {
                AdaptationStrategy::Conservative => 0.9,
                AdaptationStrategy::Balanced => 0.8,
                AdaptationStrategy::Aggressive => 0.7,
            };
            
            let new_bitrate = ((current_bitrate as f32 * reduction) as u32).max(self.config.min_bitrate);
            
            adjustments.push(QualityAdjustment {
                session_id: session_id.clone(),
                adjustment_type: AdjustmentType::ReduceBitrate { new_bitrate },
                confidence,
                reason: format!("High packet loss: {:.1}%", metrics.packet_loss),
            });
        }
        
        // Suggest enabling processing features for robustness
        if metrics.packet_loss > 8.0 {
            adjustments.push(QualityAdjustment {
                session_id: session_id.clone(),
                adjustment_type: AdjustmentType::ProcessingAdjustment {
                    feature: ProcessingFeature::VoiceActivityDetection,
                    enable: true,
                },
                confidence: confidence * 0.8,
                reason: "Enable VAD for packet loss robustness".to_string(),
            });
        }
        
        adjustments
    }
    
    /// Handle high jitter situations
    fn handle_high_jitter(&self, session_id: &MediaSessionId, metrics: &QualityMetrics) -> Vec<QualityAdjustment> {
        let mut adjustments = Vec::new();
        
        if metrics.jitter_ms > 50.0 {
            // Suggest larger frame sizes to reduce jitter sensitivity
            adjustments.push(QualityAdjustment {
                session_id: session_id.clone(),
                adjustment_type: AdjustmentType::AdjustPacketTiming { frame_size_ms: 40.0 },
                confidence: 0.75,
                reason: format!("High jitter: {:.1}ms", metrics.jitter_ms),
            });
        }
        
        adjustments
    }
    
    /// Handle poor quality situations
    fn handle_poor_quality(
        &self,
        session_id: &MediaSessionId,
        metrics: &QualityMetrics,
        current_bitrate: u32,
    ) -> Vec<QualityAdjustment> {
        let mut adjustments = Vec::new();
        
        // Switch to more robust codec if available
        if self.config.enable_codec_switching && metrics.mos_score < 2.0 {
            adjustments.push(QualityAdjustment {
                session_id: session_id.clone(),
                adjustment_type: AdjustmentType::ChangeCodec {
                    codec_name: "PCMU".to_string(), // Fall back to G.711
                },
                confidence: 0.8,
                reason: format!("Poor MOS score: {:.2}", metrics.mos_score),
            });
        }
        
        // Reduce sample rate to improve stability
        adjustments.push(QualityAdjustment {
            session_id: session_id.clone(),
            adjustment_type: AdjustmentType::AdjustSampleRate {
                new_rate: SampleRate::Rate8000, // Fall back to narrowband
            },
            confidence: 0.7,
            reason: "Reduce sample rate for stability".to_string(),
        });
        
        adjustments
    }
    
    /// Handle degrading quality situations
    fn handle_degrading_quality(
        &self,
        session_id: &MediaSessionId,
        _metrics: &QualityMetrics,
        current_bitrate: u32,
    ) -> Vec<QualityAdjustment> {
        let mut adjustments = Vec::new();
        
        // Preemptively reduce bitrate
        if current_bitrate > self.config.min_bitrate {
            let new_bitrate = ((current_bitrate as f32 * 0.9) as u32).max(self.config.min_bitrate);
            
            adjustments.push(QualityAdjustment {
                session_id: session_id.clone(),
                adjustment_type: AdjustmentType::ReduceBitrate { new_bitrate },
                confidence: 0.6,
                reason: "Preemptive adjustment for degrading quality".to_string(),
            });
        }
        
        adjustments
    }
    
    /// Handle good quality situations (opportunities to improve)
    fn handle_good_quality(
        &self,
        session_id: &MediaSessionId,
        metrics: &QualityMetrics,
        current_bitrate: u32,
    ) -> Vec<QualityAdjustment> {
        let mut adjustments = Vec::new();
        
        // Only suggest improvements if quality is consistently good
        if metrics.mos_score >= 4.0 && current_bitrate < self.config.max_bitrate {
            let increase_factor = match self.config.strategy {
                AdaptationStrategy::Conservative => 1.1,
                AdaptationStrategy::Balanced => 1.2,
                AdaptationStrategy::Aggressive => 1.3,
            };
            
            let new_bitrate = ((current_bitrate as f32 * increase_factor) as u32).min(self.config.max_bitrate);
            
            if new_bitrate > current_bitrate {
                adjustments.push(QualityAdjustment {
                    session_id: session_id.clone(),
                    adjustment_type: AdjustmentType::IncreaseBitrate { new_bitrate },
                    confidence: 0.5, // Lower confidence for improvements
                    reason: format!("Good quality - opportunity to improve (MOS: {:.2})", metrics.mos_score),
                });
            }
        }
        
        adjustments
    }
} 
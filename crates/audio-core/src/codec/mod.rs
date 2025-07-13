use crate::types::{AudioFrame, AudioFormat};
use crate::error::AudioError;
use std::collections::HashMap;

pub mod g711;
pub mod g722;
pub mod g729;
pub mod opus;

/// Represents different audio codecs supported by the codec engine
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodecType {
    /// G.711 PCMU (μ-law) - 8kHz, 64kbps
    G711Pcmu,
    /// G.711 PCMA (A-law) - 8kHz, 64kbps
    G711Pcma,
    /// G.722 - 16kHz, 64kbps
    G722,
    /// G.729 - 8kHz, 8kbps
    G729,
    /// Opus - Variable bitrate, 8-48kHz
    Opus,
}

impl CodecType {
    /// Get the default sample rate for this codec
    pub fn default_sample_rate(&self) -> u32 {
        match self {
            CodecType::G711Pcmu | CodecType::G711Pcma => 8000,
            CodecType::G722 => 16000,
            CodecType::G729 => 8000,
            CodecType::Opus => 48000,
        }
    }

    /// Get the default bitrate for this codec
    pub fn default_bitrate(&self) -> u32 {
        match self {
            CodecType::G711Pcmu | CodecType::G711Pcma => 64000,
            CodecType::G722 => 64000,
            CodecType::G729 => 8000,
            CodecType::Opus => 32000,
        }
    }

    /// Get the RTP payload type for this codec
    pub fn payload_type(&self) -> u8 {
        match self {
            CodecType::G711Pcmu => 0,
            CodecType::G711Pcma => 8,
            CodecType::G722 => 9,
            CodecType::G729 => 18,
            CodecType::Opus => 111, // Dynamic payload type
        }
    }

    /// Get the codec name as used in SDP
    pub fn sdp_name(&self) -> &'static str {
        match self {
            CodecType::G711Pcmu => "PCMU",
            CodecType::G711Pcma => "PCMA",
            CodecType::G722 => "G722",
            CodecType::G729 => "G729",
            CodecType::Opus => "opus",
        }
    }
}

/// Configuration for codec encoding/decoding
#[derive(Debug, Clone)]
pub struct CodecConfig {
    /// Codec type
    pub codec: CodecType,
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Number of audio channels
    pub channels: u32,
    /// Bitrate in bits per second
    pub bitrate: u32,
    /// Additional codec-specific parameters
    pub params: HashMap<String, String>,
}

impl Default for CodecConfig {
    fn default() -> Self {
        Self {
            codec: CodecType::G711Pcmu,
            sample_rate: 8000,
            channels: 1,
            bitrate: 64000,
            params: HashMap::new(),
        }
    }
}

/// Trait for audio codec implementations
pub trait AudioCodecTrait: Send + Sync {
    /// Encode PCM audio frame to compressed format
    fn encode(&mut self, frame: &AudioFrame) -> Result<Vec<u8>, AudioError>;
    
    /// Decode compressed audio data to PCM frame
    fn decode(&mut self, data: &[u8]) -> Result<AudioFrame, AudioError>;
    
    /// Get codec configuration
    fn config(&self) -> &CodecConfig;
    
    /// Reset codec state
    fn reset(&mut self) -> Result<(), AudioError>;
    
    /// Get codec type
    fn codec_type(&self) -> CodecType;
}

/// Factory for creating codec instances
pub struct CodecFactory;

impl CodecFactory {
    /// Create a new codec instance with the specified configuration
    pub fn create(config: CodecConfig) -> Result<Box<dyn AudioCodecTrait>, AudioError> {
        match config.codec {
            CodecType::G711Pcmu => Ok(Box::new(g711::G711Encoder::new(config, true)?)),
            CodecType::G711Pcma => Ok(Box::new(g711::G711Encoder::new(config, false)?)),
            CodecType::G722 => Ok(Box::new(g722::G722Encoder::new(config)?)),
            CodecType::G729 => Ok(Box::new(g729::G729Encoder::new(config)?)),
            CodecType::Opus => Ok(Box::new(opus::OpusEncoder::new(config)?)),
        }
    }

    /// Get all supported codecs
    pub fn supported_codecs() -> Vec<CodecType> {
        vec![
            CodecType::G711Pcmu,
            CodecType::G711Pcma,
            CodecType::G722,
            CodecType::G729,
            CodecType::Opus,
        ]
    }

    /// Check if a codec is supported
    pub fn is_supported(codec: CodecType) -> bool {
        Self::supported_codecs().contains(&codec)
    }
}

/// Codec capability negotiation
#[derive(Debug, Clone)]
pub struct CodecCapability {
    /// Codec type
    pub codec: CodecType,
    /// Supported sample rates
    pub sample_rates: Vec<u32>,
    /// Supported channel counts
    pub channels: Vec<u32>,
    /// Supported bitrates
    pub bitrates: Vec<u32>,
    /// Quality score (0-100)
    pub quality_score: u8,
}

impl CodecCapability {
    /// Create capability for G.711 PCMU
    pub fn g711_pcmu() -> Self {
        Self {
            codec: CodecType::G711Pcmu,
            sample_rates: vec![8000],
            channels: vec![1],
            bitrates: vec![64000],
            quality_score: 70,
        }
    }

    /// Create capability for G.711 PCMA
    pub fn g711_pcma() -> Self {
        Self {
            codec: CodecType::G711Pcma,
            sample_rates: vec![8000],
            channels: vec![1],
            bitrates: vec![64000],
            quality_score: 70,
        }
    }

    /// Create capability for G.722
    pub fn g722() -> Self {
        Self {
            codec: CodecType::G722,
            sample_rates: vec![16000],
            channels: vec![1],
            bitrates: vec![64000],
            quality_score: 80,
        }
    }

    /// Create capability for G.729
    pub fn g729() -> Self {
        Self {
            codec: CodecType::G729,
            sample_rates: vec![8000],
            channels: vec![1],
            bitrates: vec![8000],
            quality_score: 85,
        }
    }

    /// Create capability for Opus
    pub fn opus() -> Self {
        Self {
            codec: CodecType::Opus,
            sample_rates: vec![8000, 12000, 16000, 24000, 48000],
            channels: vec![1, 2],
            bitrates: vec![6000, 8000, 16000, 24000, 32000, 64000, 128000],
            quality_score: 95,
        }
    }
}

/// Codec negotiation engine
pub struct CodecNegotiator {
    local_capabilities: Vec<CodecCapability>,
}

impl CodecNegotiator {
    /// Create a new codec negotiator with default capabilities
    pub fn new() -> Self {
        Self {
            local_capabilities: vec![
                CodecCapability::opus(),
                CodecCapability::g729(),
                CodecCapability::g722(),
                CodecCapability::g711_pcmu(),
                CodecCapability::g711_pcma(),
            ],
        }
    }

    /// Create negotiator with custom capabilities
    pub fn with_capabilities(capabilities: Vec<CodecCapability>) -> Self {
        Self {
            local_capabilities: capabilities,
        }
    }

    /// Negotiate best codec with remote capabilities
    pub fn negotiate(&self, remote_capabilities: &[CodecCapability]) -> Option<CodecConfig> {
        // Find the best matching codec based on quality score
        let mut best_match: Option<(CodecCapability, CodecCapability)> = None;
        let mut best_score = 0;

        for local_cap in &self.local_capabilities {
            for remote_cap in remote_capabilities {
                if local_cap.codec == remote_cap.codec {
                    let score = std::cmp::min(local_cap.quality_score, remote_cap.quality_score);
                    if score > best_score {
                        best_score = score;
                        best_match = Some((local_cap.clone(), remote_cap.clone()));
                    }
                }
            }
        }

        // Create configuration for best match
        if let Some((local_cap, remote_cap)) = best_match {
            // Find compatible sample rate
            let sample_rate = local_cap.sample_rates.iter()
                .find(|&rate| remote_cap.sample_rates.contains(rate))
                .copied()
                .unwrap_or(local_cap.codec.default_sample_rate());

            // Find compatible channel count
            let channels = local_cap.channels.iter()
                .find(|&ch| remote_cap.channels.contains(ch))
                .copied()
                .unwrap_or(1);

            // Find compatible bitrate
            let bitrate = local_cap.bitrates.iter()
                .find(|&br| remote_cap.bitrates.contains(br))
                .copied()
                .unwrap_or(local_cap.codec.default_bitrate());

            Some(CodecConfig {
                codec: local_cap.codec,
                sample_rate,
                channels,
                bitrate,
                params: HashMap::new(),
            })
        } else {
            None
        }
    }

    /// Get local capabilities
    pub fn local_capabilities(&self) -> &[CodecCapability] {
        &self.local_capabilities
    }
}

impl Default for CodecNegotiator {
    fn default() -> Self {
        Self::new()
    }
}

/// Quality metrics for codec performance
#[derive(Debug, Default, Clone)]
pub struct CodecQualityMetrics {
    /// Mean Opinion Score (1.0 - 5.0)
    pub mos_score: f32,
    /// Packet loss rate (0.0 - 1.0)
    pub packet_loss_rate: f32,
    /// Jitter in milliseconds
    pub jitter_ms: f32,
    /// Round-trip time in milliseconds
    pub rtt_ms: f32,
    /// Bitrate utilization (0.0 - 1.0)
    pub bitrate_utilization: f32,
    /// Audio quality score (0-100)
    pub quality_score: u8,
}

impl CodecQualityMetrics {
    /// Calculate overall quality score
    pub fn calculate_quality_score(&mut self) {
        let mos_weight = 0.4;
        let loss_weight = 0.3;
        let jitter_weight = 0.2;
        let rtt_weight = 0.1;

        let mos_score = (self.mos_score / 5.0) * 100.0;
        let loss_score = (1.0 - self.packet_loss_rate) * 100.0;
        let jitter_score = ((50.0 - self.jitter_ms.min(50.0)) / 50.0) * 100.0;
        let rtt_score = ((200.0 - self.rtt_ms.min(200.0)) / 200.0) * 100.0;

        self.quality_score = (mos_weight * mos_score + 
                             loss_weight * loss_score + 
                             jitter_weight * jitter_score + 
                             rtt_weight * rtt_score) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codec_properties() {
        assert_eq!(CodecType::G711Pcmu.default_sample_rate(), 8000);
        assert_eq!(CodecType::G711Pcmu.payload_type(), 0);
        assert_eq!(CodecType::G711Pcmu.sdp_name(), "PCMU");
        
        assert_eq!(CodecType::G729.default_sample_rate(), 8000);
        assert_eq!(CodecType::G729.payload_type(), 18);
        assert_eq!(CodecType::G729.sdp_name(), "G729");
        
        assert_eq!(CodecType::Opus.default_sample_rate(), 48000);
        assert_eq!(CodecType::Opus.payload_type(), 111);
        assert_eq!(CodecType::Opus.sdp_name(), "opus");
    }

    #[test]
    fn test_codec_negotiation() {
        let negotiator = CodecNegotiator::new();
        
        let remote_caps = vec![
            CodecCapability::g711_pcmu(),
            CodecCapability::g729(),
            CodecCapability::opus(),
        ];
        
        let result = negotiator.negotiate(&remote_caps);
        assert!(result.is_some());
        
        let config = result.unwrap();
        assert_eq!(config.codec, CodecType::Opus); // Should prefer Opus (higher quality)
    }

    #[test]
    fn test_quality_metrics() {
        let mut metrics = CodecQualityMetrics {
            mos_score: 4.0,
            packet_loss_rate: 0.01,
            jitter_ms: 10.0,
            rtt_ms: 50.0,
            bitrate_utilization: 0.8,
            quality_score: 0,
        };
        
        metrics.calculate_quality_score();
        assert!(metrics.quality_score > 80);
    }
} 
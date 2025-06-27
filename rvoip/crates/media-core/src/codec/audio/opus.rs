//! Opus Audio Codec Implementation
//!
//! This module implements the Opus codec, a modern low-latency audio codec
//! optimized for VoIP applications with excellent quality and adaptability.

use tracing::{debug, warn, error};
use crate::error::{Result, CodecError};
use crate::types::{AudioFrame, SampleRate};
use super::common::{AudioCodec, CodecInfo};

/// Opus codec configuration
#[derive(Debug, Clone)]
pub struct OpusConfig {
    /// Target bitrate (6000-510000 bps)
    pub bitrate: u32,
    /// Encoding complexity (0-10, higher = better quality/more CPU)
    pub complexity: u32,
    /// Use variable bitrate
    pub vbr: bool,
    /// Application type
    pub application: OpusApplication,
    /// Frame size in milliseconds (2.5, 5, 10, 20, 40, 60)
    pub frame_size_ms: f32,
}

/// Opus application types
#[derive(Debug, Clone, Copy)]
pub enum OpusApplication {
    /// Voice over IP
    Voip,
    /// Audio streaming
    Audio,
}

impl Default for OpusConfig {
    fn default() -> Self {
        Self {
            bitrate: 64000,           // 64 kbps - good quality for VoIP
            complexity: 5,            // Balanced complexity
            vbr: true,               // Variable bitrate for efficiency
            application: OpusApplication::Voip,
            frame_size_ms: 20.0,     // 20ms frames (standard for VoIP)
        }
    }
}

/// Opus audio codec implementation
pub struct OpusCodec {
    /// Codec configuration
    config: OpusConfig,
    /// Sample rate
    sample_rate: u32,
    /// Number of channels
    channels: u8,
    /// Opus encoder (when available)
    #[cfg(feature = "opus")]
    encoder: Option<opus::Encoder>,
    /// Opus decoder (when available)
    #[cfg(feature = "opus")]
    decoder: Option<opus::Decoder>,
    /// Frame size in samples
    frame_size: usize,
}

impl OpusCodec {
    /// Create a new Opus codec
    pub fn new(sample_rate: SampleRate, channels: u8, config: OpusConfig) -> Result<Self> {
        let sample_rate_hz = sample_rate.as_hz();
        
        // Validate parameters
        if channels == 0 || channels > 2 {
            return Err(CodecError::InvalidParameters {
                details: format!("Invalid channel count: {}", channels),
            }.into());
        }
        
        // Validate sample rate (Opus supports 8, 12, 16, 24, 48 kHz)
        if !matches!(sample_rate_hz, 8000 | 12000 | 16000 | 24000 | 48000) {
            return Err(CodecError::InvalidParameters {
                details: format!("Invalid sample rate: {}", sample_rate_hz),
            }.into());
        }
        
        // Calculate frame size
        let frame_size = ((sample_rate_hz as f32 * config.frame_size_ms / 1000.0) as usize) * channels as usize;
        
        debug!("Creating Opus codec: {}Hz, {}ch, {}ms frames", 
               sample_rate_hz, channels, config.frame_size_ms);
        
        Ok(Self {
            config,
            sample_rate: sample_rate_hz,
            channels,
            #[cfg(feature = "opus")]
            encoder: None,
            #[cfg(feature = "opus")]
            decoder: None,
            frame_size,
        })
    }
    
    /// Initialize encoder
    #[cfg(feature = "opus")]
    fn ensure_encoder(&mut self) -> Result<()> {
        if self.encoder.is_none() {
            let app = match self.config.application {
                OpusApplication::Voip => opus::Application::Voip,
                OpusApplication::Audio => opus::Application::Audio,
            };
            
            let mut encoder = opus::Encoder::new(
                self.sample_rate,
                match self.channels {
                    1 => opus::Channels::Mono,
                    2 => opus::Channels::Stereo,
                    _ => return Err(CodecError::InitializationFailed {
                        reason: "Unsupported channel count".to_string(),
                    }.into()),
                },
                app,
            ).map_err(|e| CodecError::InitializationFailed {
                reason: format!("Opus encoder creation failed: {:?}", e),
            })?;
            
            // Configure encoder
            encoder.set_bitrate(opus::Bitrate::Bits(self.config.bitrate as i32))
                .map_err(|e| CodecError::InitializationFailed {
                    reason: format!("Failed to set bitrate: {:?}", e),
                })?;
            
            encoder.set_vbr(self.config.vbr)
                .map_err(|e| CodecError::InitializationFailed {
                    reason: format!("Failed to set VBR: {:?}", e),
                })?;
            
            // Note: complexity setting not available in this opus crate version
            // The complexity value is used as a hint but not directly configurable
            
            self.encoder = Some(encoder);
            debug!("Opus encoder initialized");
        }
        
        Ok(())
    }
    
    /// Initialize decoder
    #[cfg(feature = "opus")]
    fn ensure_decoder(&mut self) -> Result<()> {
        if self.decoder.is_none() {
            let decoder = opus::Decoder::new(
                self.sample_rate,
                match self.channels {
                    1 => opus::Channels::Mono,
                    2 => opus::Channels::Stereo,
                    _ => return Err(CodecError::InitializationFailed {
                        reason: "Unsupported channel count".to_string(),
                    }.into()),
                },
            ).map_err(|e| CodecError::InitializationFailed {
                reason: format!("Opus decoder creation failed: {:?}", e),
            })?;
            
            self.decoder = Some(decoder);
            debug!("Opus decoder initialized");
        }
        
        Ok(())
    }
}

impl AudioCodec for OpusCodec {
    fn encode(&mut self, audio_frame: &AudioFrame) -> Result<Vec<u8>> {
        #[cfg(feature = "opus")]
        {
            self.ensure_encoder()?;
            
            if audio_frame.samples.len() != self.frame_size {
                return Err(CodecError::InvalidFrameSize {
                    expected: self.frame_size,
                    actual: audio_frame.samples.len(),
                }.into());
            }
            
            let encoder = self.encoder.as_mut().unwrap();
            let mut output = vec![0u8; 4000]; // Max Opus packet size
            
            let encoded_size = encoder.encode(&audio_frame.samples, &mut output)
                .map_err(|e| CodecError::EncodingFailed {
                    reason: format!("Opus encoding failed: {:?}", e),
                })?;
            
            output.truncate(encoded_size);
            Ok(output)
        }
        
        #[cfg(not(feature = "opus"))]
        {
            warn!("Opus codec not available - feature 'opus' not enabled");
            Err(CodecError::NotFound {
                name: "Opus".to_string(),
            }.into())
        }
    }
    
    fn decode(&mut self, encoded_data: &[u8]) -> Result<AudioFrame> {
        #[cfg(feature = "opus")]
        {
            self.ensure_decoder()?;
            
            let decoder = self.decoder.as_mut().unwrap();
            let mut output = vec![0i16; self.frame_size];
            
            let decoded_samples = decoder.decode(encoded_data, &mut output, false)
                .map_err(|e| CodecError::DecodingFailed {
                    reason: format!("Opus decoding failed: {:?}", e),
                })?;
            
            output.truncate(decoded_samples * self.channels as usize);
            
            Ok(AudioFrame::new(
                output,
                self.sample_rate,
                self.channels,
                0, // Timestamp to be set by caller
            ))
        }
        
        #[cfg(not(feature = "opus"))]
        {
            warn!("Opus codec not available - feature 'opus' not enabled");
            Err(CodecError::NotFound {
                name: "Opus".to_string(),
            }.into())
        }
    }
    
    fn get_info(&self) -> CodecInfo {
        CodecInfo {
            name: "Opus".to_string(),
            sample_rate: self.sample_rate,
            channels: self.channels,
            bitrate: self.config.bitrate,
        }
    }
    
    fn reset(&mut self) {
        #[cfg(feature = "opus")]
        {
            if let Some(encoder) = &mut self.encoder {
                let _ = encoder.reset_state();
            }
            if let Some(decoder) = &mut self.decoder {
                let _ = decoder.reset_state();
            }
        }
        debug!("Opus codec reset");
    }
} 
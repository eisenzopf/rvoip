//! Audio channel conversion
//!
//! This module provides utilities for converting between different channel configurations
//! such as mono, stereo, and multichannel audio.

use std::sync::Arc;
use bytes::Bytes;
use tracing::{debug, error, info, warn};

use crate::error::{Error, Result};
use crate::{Sample, SampleRate, AudioBuffer, AudioFormat};

/// Channel conversion direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelConversion {
    /// Convert mono to stereo
    MonoToStereo,
    /// Convert stereo to mono
    StereoToMono,
    /// Convert between specific channel counts
    Custom { src_channels: u8, dst_channels: u8 },
}

/// Channel mixing method
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixingMethod {
    /// Simple average (equal weights)
    Average,
    /// Weighted mix (custom weights for each channel)
    Weighted,
    /// Drop channels (keep only specified channels)
    Drop,
}

impl Default for MixingMethod {
    fn default() -> Self {
        Self::Average
    }
}

/// Channel converter implementation
#[derive(Debug)]
pub struct ChannelConverter {
    /// Conversion direction
    conversion: ChannelConversion,
    
    /// Mixing method for downmixing
    mixing_method: MixingMethod,
    
    /// Mixing weights for each channel
    mixing_weights: Vec<f32>,
    
    /// Channels to keep when using Drop method
    channels_to_keep: Vec<u8>,
}

impl ChannelConverter {
    /// Create a new channel converter
    pub fn new(conversion: ChannelConversion, mixing_method: MixingMethod) -> Self {
        let mixing_weights = match conversion {
            ChannelConversion::StereoToMono => vec![0.5, 0.5], // Equal weights for L/R
            ChannelConversion::Custom { src_channels, .. } => {
                // Create equal weights for all source channels
                vec![1.0 / src_channels as f32; src_channels as usize]
            },
            _ => Vec::new(), // Not needed for upmixing
        };
        
        Self {
            conversion,
            mixing_method,
            mixing_weights,
            channels_to_keep: vec![0], // By default, keep first channel
        }
    }
    
    /// Create a simple mono to stereo converter
    pub fn mono_to_stereo() -> Self {
        Self::new(ChannelConversion::MonoToStereo, MixingMethod::Average)
    }
    
    /// Create a simple stereo to mono converter
    pub fn stereo_to_mono() -> Self {
        Self::new(ChannelConversion::StereoToMono, MixingMethod::Average)
    }
    
    /// Create a custom channel converter
    pub fn custom(src_channels: u8, dst_channels: u8, method: MixingMethod) -> Self {
        Self::new(
            ChannelConversion::Custom { 
                src_channels, 
                dst_channels 
            },
            method
        )
    }
    
    /// Set custom mixing weights
    pub fn set_mixing_weights(&mut self, weights: Vec<f32>) -> Result<()> {
        let expected_channels = match self.conversion {
            ChannelConversion::StereoToMono => 2,
            ChannelConversion::Custom { src_channels, .. } => src_channels,
            _ => return Err(Error::InvalidArgument(
                "Mixing weights only applicable for downmixing".to_string()
            )),
        };
        
        if weights.len() != expected_channels as usize {
            return Err(Error::InvalidArgument(format!(
                "Expected {} weights, got {}", 
                expected_channels, weights.len()
            )));
        }
        
        self.mixing_weights = weights;
        Ok(())
    }
    
    /// Set channels to keep for Drop mixing method
    pub fn set_channels_to_keep(&mut self, channels: Vec<u8>) -> Result<()> {
        let max_channel = match self.conversion {
            ChannelConversion::StereoToMono => 1, // 0-based, so stereo has 0,1
            ChannelConversion::Custom { src_channels, .. } => src_channels - 1,
            _ => return Err(Error::InvalidArgument(
                "Channel selection only applicable for downmixing".to_string()
            )),
        };
        
        // Validate channel indices
        for &ch in &channels {
            if ch > max_channel {
                return Err(Error::InvalidArgument(format!(
                    "Channel index {} out of bounds (max: {})",
                    ch, max_channel
                )));
            }
        }
        
        if channels.is_empty() {
            return Err(Error::InvalidArgument(
                "Must keep at least one channel".to_string()
            ));
        }
        
        self.channels_to_keep = channels;
        Ok(())
    }
    
    /// Get the source channel count
    pub fn source_channels(&self) -> u8 {
        match self.conversion {
            ChannelConversion::MonoToStereo => 1,
            ChannelConversion::StereoToMono => 2,
            ChannelConversion::Custom { src_channels, .. } => src_channels,
        }
    }
    
    /// Get the destination channel count
    pub fn destination_channels(&self) -> u8 {
        match self.conversion {
            ChannelConversion::MonoToStereo => 2,
            ChannelConversion::StereoToMono => 1,
            ChannelConversion::Custom { dst_channels, .. } => dst_channels,
        }
    }
    
    /// Calculate the output size in samples
    pub fn calculate_output_size(&self, input_size: usize) -> usize {
        let src_channels = self.source_channels() as usize;
        let dst_channels = self.destination_channels() as usize;
        
        if src_channels == 0 || input_size % src_channels != 0 {
            return 0; // Invalid input
        }
        
        let frames = input_size / src_channels;
        frames * dst_channels
    }
    
    /// Convert interleaved audio samples
    pub fn process(&self, input: &[Sample]) -> Result<Vec<Sample>> {
        let src_channels = self.source_channels() as usize;
        let dst_channels = self.destination_channels() as usize;
        
        if input.len() % src_channels != 0 {
            return Err(Error::InvalidArgument(format!(
                "Input size ({}) is not a multiple of source channels ({})",
                input.len(), src_channels
            )));
        }
        
        // Calculate number of frames (samples per channel)
        let frames = input.len() / src_channels;
        let mut output = Vec::with_capacity(frames * dst_channels);
        
        // Stub implementation - in a real implementation, we would:
        // 1. For each frame, extract samples from all source channels
        // 2. Apply the appropriate mixing/duplication rules
        // 3. Generate the output frame with the destination channel count
        
        // For the stub, just return silence with the correct size
        output.resize(frames * dst_channels, 0);
        
        Ok(output)
    }
    
    /// Convert an audio buffer
    pub fn process_buffer(&self, input: &AudioBuffer) -> Result<AudioBuffer> {
        if input.format.channels != self.source_channels() {
            return Err(Error::InvalidArgument(format!(
                "Buffer has {} channels, but converter expects {}",
                input.format.channels, self.source_channels()
            )));
        }
        
        // Process the audio data
        let output_data = self.process(
            bytemuck::cast_slice(&input.data)
        )?;
        
        // Create new output format with updated channel count
        let output_format = AudioFormat {
            channels: self.destination_channels(),
            ..input.format
        };
        
        // Create output buffer
        let output_data_bytes = Bytes::from(
            bytemuck::cast_slice(&output_data).to_vec()
        );
        
        Ok(AudioBuffer {
            data: output_data_bytes,
            format: output_format,
        })
    }
}

/// Builder for channel converter
pub struct ChannelConverterBuilder {
    conversion: ChannelConversion,
    mixing_method: MixingMethod,
    mixing_weights: Option<Vec<f32>>,
    channels_to_keep: Option<Vec<u8>>,
}

impl ChannelConverterBuilder {
    /// Create a new channel converter builder
    pub fn new() -> Self {
        Self {
            conversion: ChannelConversion::MonoToStereo,
            mixing_method: MixingMethod::default(),
            mixing_weights: None,
            channels_to_keep: None,
        }
    }
    
    /// Set mono to stereo conversion
    pub fn mono_to_stereo(mut self) -> Self {
        self.conversion = ChannelConversion::MonoToStereo;
        self
    }
    
    /// Set stereo to mono conversion
    pub fn stereo_to_mono(mut self) -> Self {
        self.conversion = ChannelConversion::StereoToMono;
        self
    }
    
    /// Set custom channel conversion
    pub fn custom(mut self, src_channels: u8, dst_channels: u8) -> Self {
        self.conversion = ChannelConversion::Custom { 
            src_channels, 
            dst_channels 
        };
        self
    }
    
    /// Set mixing method
    pub fn with_mixing_method(mut self, method: MixingMethod) -> Self {
        self.mixing_method = method;
        self
    }
    
    /// Set custom mixing weights
    pub fn with_mixing_weights(mut self, weights: Vec<f32>) -> Self {
        self.mixing_weights = Some(weights);
        self
    }
    
    /// Set channels to keep for Drop mixing method
    pub fn with_channels_to_keep(mut self, channels: Vec<u8>) -> Self {
        self.channels_to_keep = Some(channels);
        self
    }
    
    /// Build the channel converter
    pub fn build(self) -> Result<ChannelConverter> {
        let mut converter = ChannelConverter::new(
            self.conversion,
            self.mixing_method
        );
        
        // Apply mixing weights if provided
        if let Some(weights) = self.mixing_weights {
            converter.set_mixing_weights(weights)?;
        }
        
        // Apply channels to keep if provided
        if let Some(channels) = self.channels_to_keep {
            converter.set_channels_to_keep(channels)?;
        }
        
        Ok(converter)
    }
} 
//! Format Converter - Audio format conversion
//!
//! This module handles conversion between different audio formats including
//! sample rate conversion, channel layout changes, and bit depth conversion.

use tracing::{debug, warn};
use crate::error::{Result, AudioProcessingError};
use crate::types::{AudioFrame, SampleRate};
use super::resampler::Resampler;
use super::channel_mixer::{ChannelMixer, ChannelLayout};

/// Parameters for audio format conversion
#[derive(Debug, Clone)]
pub struct ConversionParams {
    /// Target sample rate
    pub target_sample_rate: SampleRate,
    /// Target number of channels
    pub target_channels: u8,
    /// Quality level for resampling (0-10, higher = better quality)
    pub quality: u8,
}

impl ConversionParams {
    /// Create new conversion parameters
    pub fn new(target_sample_rate: SampleRate, target_channels: u8) -> Self {
        Self {
            target_sample_rate,
            target_channels,
            quality: 5, // Medium quality by default
        }
    }
    
    /// High quality conversion parameters
    pub fn high_quality(target_sample_rate: SampleRate, target_channels: u8) -> Self {
        Self {
            target_sample_rate,
            target_channels,
            quality: 8,
        }
    }
    
    /// Fast conversion parameters (lower quality)
    pub fn fast(target_sample_rate: SampleRate, target_channels: u8) -> Self {
        Self {
            target_sample_rate,
            target_channels,
            quality: 3,
        }
    }
}

/// Result of format conversion
#[derive(Debug, Clone)]
pub struct ConversionResult {
    /// Converted audio frame
    pub frame: AudioFrame,
    /// Whether conversion was actually performed
    pub was_converted: bool,
    /// Conversion metrics
    pub metrics: ConversionMetrics,
}

/// Metrics from format conversion
#[derive(Debug, Clone, Default)]
pub struct ConversionMetrics {
    /// Whether sample rate was converted
    pub sample_rate_converted: bool,
    /// Whether channels were converted
    pub channels_converted: bool,
    /// Original sample count
    pub input_samples: usize,
    /// Converted sample count
    pub output_samples: usize,
    /// Conversion time in microseconds
    pub conversion_time_us: u64,
}

/// Audio format converter
pub struct FormatConverter {
    /// Resampler for sample rate conversion
    resampler: Option<Resampler>,
    /// Channel mixer for channel layout conversion
    channel_mixer: ChannelMixer,
    /// Current conversion parameters
    current_params: Option<ConversionParams>,
}

impl FormatConverter {
    /// Create a new format converter
    pub fn new() -> Self {
        Self {
            resampler: None,
            channel_mixer: ChannelMixer::new(),
            current_params: None,
        }
    }
    
    /// Convert audio frame to target format
    pub fn convert_frame(
        &mut self, 
        input: &AudioFrame, 
        params: &ConversionParams
    ) -> Result<ConversionResult> {
        let start_time = std::time::Instant::now();
        
        // Check if conversion is needed
        let needs_sample_rate_conversion = input.sample_rate != params.target_sample_rate.as_hz();
        let needs_channel_conversion = input.channels != params.target_channels;
        
        if !needs_sample_rate_conversion && !needs_channel_conversion {
            // No conversion needed
            return Ok(ConversionResult {
                frame: input.clone(),
                was_converted: false,
                metrics: ConversionMetrics {
                    input_samples: input.samples.len(),
                    output_samples: input.samples.len(),
                    conversion_time_us: start_time.elapsed().as_micros() as u64,
                    ..Default::default()
                },
            });
        }
        
        debug!("Converting audio: {}Hz,{}ch -> {}Hz,{}ch", 
               input.sample_rate, input.channels,
               params.target_sample_rate.as_hz(), params.target_channels);
        
        // Update resampler if needed
        if needs_sample_rate_conversion {
            self.update_resampler(input.sample_rate, params)?;
        }
        
        let mut converted_frame = input.clone();
        
        // Step 1: Sample rate conversion
        if needs_sample_rate_conversion {
            converted_frame = self.convert_sample_rate(&converted_frame, params)?;
        }
        
        // Step 2: Channel conversion
        if needs_channel_conversion {
            converted_frame = self.convert_channels(&converted_frame, params)?;
        }
        
        let conversion_time = start_time.elapsed();
        
        Ok(ConversionResult {
            frame: converted_frame.clone(),
            was_converted: true,
            metrics: ConversionMetrics {
                sample_rate_converted: needs_sample_rate_conversion,
                channels_converted: needs_channel_conversion,
                input_samples: input.samples.len(),
                output_samples: converted_frame.samples.len(),
                conversion_time_us: conversion_time.as_micros() as u64,
            },
        })
    }
    
    /// Reset converter state
    pub fn reset(&mut self) {
        if let Some(resampler) = &mut self.resampler {
            resampler.reset();
        }
        self.channel_mixer.reset();
        self.current_params = None;
        debug!("FormatConverter reset");
    }
    
    /// Update resampler configuration
    fn update_resampler(
        &mut self, 
        input_sample_rate: u32, 
        params: &ConversionParams
    ) -> Result<()> {
        let target_rate = params.target_sample_rate.as_hz();
        
        // Check if we need to create/update resampler
        let needs_update = match &self.current_params {
            None => true,
            Some(current) => {
                current.target_sample_rate != params.target_sample_rate ||
                current.quality != params.quality
            }
        };
        
        if needs_update {
            self.resampler = Some(Resampler::new(
                input_sample_rate,
                target_rate,
                params.quality,
            )?);
            self.current_params = Some(params.clone());
        }
        
        Ok(())
    }
    
    /// Convert sample rate
    fn convert_sample_rate(
        &mut self, 
        input: &AudioFrame, 
        params: &ConversionParams
    ) -> Result<AudioFrame> {
        let resampler = self.resampler.as_mut()
            .ok_or_else(|| AudioProcessingError::ProcessingFailed {
                reason: "Resampler not initialized".to_string(),
            })?;
        
        let resampled_samples = resampler.resample(&input.samples)?;
        
        Ok(AudioFrame::new(
            resampled_samples,
            params.target_sample_rate.as_hz(),
            input.channels,
            input.timestamp, // Keep original timestamp
        ))
    }
    
    /// Convert channel layout
    fn convert_channels(
        &mut self, 
        input: &AudioFrame, 
        params: &ConversionParams
    ) -> Result<AudioFrame> {
        let source_layout = match input.channels {
            1 => ChannelLayout::Mono,
            2 => ChannelLayout::Stereo,
            _ => return Err(AudioProcessingError::ChannelConversionFailed {
                from_channels: input.channels,
                to_channels: params.target_channels,
            }.into()),
        };
        
        let target_layout = match params.target_channels {
            1 => ChannelLayout::Mono,
            2 => ChannelLayout::Stereo,
            _ => return Err(AudioProcessingError::ChannelConversionFailed {
                from_channels: input.channels,
                to_channels: params.target_channels,
            }.into()),
        };
        
        let mixed_samples = self.channel_mixer.mix_channels(
            &input.samples,
            source_layout,
            target_layout,
        )?;
        
        Ok(AudioFrame::new(
            mixed_samples,
            input.sample_rate,
            params.target_channels,
            input.timestamp,
        ))
    }
}

impl Default for FormatConverter {
    fn default() -> Self {
        Self::new()
    }
} 
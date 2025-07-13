//! Audio format conversion and processing
//!
//! This module provides comprehensive audio format conversion capabilities including
//! sample rate conversion, channel mapping, bit depth conversion, and audio frame processing.

use crate::types::{AudioFormat, AudioFrame};
use crate::error::{AudioError, AudioResult};
use std::collections::VecDeque;

/// Audio format converter for handling different sample rates, channels, and bit depths
pub struct FormatConverter {
    /// Input format specification
    input_format: AudioFormat,
    /// Output format specification  
    output_format: AudioFormat,
    /// Internal conversion buffer
    conversion_buffer: VecDeque<i16>,
    /// Sample rate conversion ratio
    sample_rate_ratio: f64,
    /// Current conversion state
    conversion_state: ConversionState,
}

/// Internal state for format conversion
#[derive(Debug, Clone)]
struct ConversionState {
    /// Accumulated fractional sample position
    fractional_position: f64,
    /// Previous sample for interpolation
    previous_sample: Option<i16>,
    /// Channel mixing state
    channel_mix_buffer: Vec<f32>,
}

impl FormatConverter {
    /// Create a new format converter
    pub fn new(input_format: AudioFormat, output_format: AudioFormat) -> AudioResult<Self> {
        // Validate format compatibility
        if input_format.bits_per_sample != 16 || output_format.bits_per_sample != 16 {
            return Err(AudioError::FormatConversionFailed {
                source_format: input_format.description(),
                target_format: output_format.description(),
                reason: "Only 16-bit audio currently supported".to_string(),
            });
        }

        let sample_rate_ratio = output_format.sample_rate as f64 / input_format.sample_rate as f64;

        Ok(Self {
            input_format,
            output_format,
            conversion_buffer: VecDeque::new(),
            sample_rate_ratio,
            conversion_state: ConversionState {
                fractional_position: 0.0,
                previous_sample: None,
                channel_mix_buffer: Vec::new(),
            },
        })
    }

    /// Convert an audio frame from input format to output format
    pub fn convert_frame(&mut self, input_frame: &AudioFrame) -> AudioResult<AudioFrame> {
        // Validate input frame format
        if !input_frame.format.is_compatible_with(&self.input_format) {
            return Err(AudioError::FormatConversionFailed {
                source_format: input_frame.format.description(),
                target_format: self.input_format.description(),
                reason: "Input frame format doesn't match converter input format".to_string(),
            });
        }

        let mut converted_samples = input_frame.samples.clone();

        // Step 1: Channel conversion if needed
        if self.input_format.channels != self.output_format.channels {
            converted_samples = self.convert_channels(&converted_samples)?;
        }

        // Step 2: Sample rate conversion if needed
        if self.input_format.sample_rate != self.output_format.sample_rate {
            converted_samples = self.convert_sample_rate(&converted_samples)?;
        }

        // Create output frame
        let output_frame = AudioFrame::new(
            converted_samples,
            self.output_format.clone(),
            input_frame.timestamp,
        );

        Ok(output_frame)
    }

    /// Convert between different channel configurations
    fn convert_channels(&mut self, samples: &[i16]) -> AudioResult<Vec<i16>> {
        match (self.input_format.channels, self.output_format.channels) {
            (1, 2) => {
                // Mono to stereo: duplicate each sample
                let mut stereo_samples = Vec::with_capacity(samples.len() * 2);
                for &sample in samples {
                    stereo_samples.push(sample);
                    stereo_samples.push(sample);
                }
                Ok(stereo_samples)
            }
            (2, 1) => {
                // Stereo to mono: average left and right channels
                let mut mono_samples = Vec::with_capacity(samples.len() / 2);
                for chunk in samples.chunks_exact(2) {
                    let left = chunk[0] as i32;
                    let right = chunk[1] as i32;
                    let mixed = ((left + right) / 2) as i16;
                    mono_samples.push(mixed);
                }
                Ok(mono_samples)
            }
            (input_ch, output_ch) if input_ch == output_ch => {
                // No conversion needed
                Ok(samples.to_vec())
            }
            (input_ch, output_ch) => {
                Err(AudioError::FormatConversionFailed {
                    source_format: format!("{} channels", input_ch),
                    target_format: format!("{} channels", output_ch),
                    reason: "Unsupported channel configuration".to_string(),
                })
            }
        }
    }

    /// Convert sample rate using linear interpolation
    fn convert_sample_rate(&mut self, samples: &[i16]) -> AudioResult<Vec<i16>> {
        if self.sample_rate_ratio == 1.0 {
            return Ok(samples.to_vec());
        }

        let output_length = ((samples.len() as f64) * self.sample_rate_ratio) as usize;
        let mut output_samples = Vec::with_capacity(output_length);

        let mut input_position = 0.0;
        let step_size = 1.0 / self.sample_rate_ratio;

        while input_position < samples.len() as f64 - 1.0 {
            let index = input_position.floor() as usize;
            let fraction = input_position - input_position.floor();

            // Linear interpolation between adjacent samples
            let sample1 = samples.get(index).copied().unwrap_or(0) as f64;
            let sample2 = samples.get(index + 1).copied().unwrap_or(samples.get(index).copied().unwrap_or(0)) as f64;
            
            let interpolated = sample1 + (sample2 - sample1) * fraction;
            output_samples.push(interpolated as i16);

            input_position += step_size;
        }

        // Handle remaining samples
        if output_samples.len() < output_length {
            let last_sample = samples.last().copied().unwrap_or(0);
            while output_samples.len() < output_length {
                output_samples.push(last_sample);
            }
        }

        Ok(output_samples)
    }

    /// Get conversion statistics
    pub fn get_stats(&self) -> FormatConversionStats {
        FormatConversionStats {
            input_format: self.input_format.clone(),
            output_format: self.output_format.clone(),
            sample_rate_ratio: self.sample_rate_ratio,
            buffer_size: self.conversion_buffer.len(),
        }
    }

    /// Reset conversion state
    pub fn reset(&mut self) {
        self.conversion_buffer.clear();
        self.conversion_state.fractional_position = 0.0;
        self.conversion_state.previous_sample = None;
        self.conversion_state.channel_mix_buffer.clear();
    }
}

/// Statistics about format conversion
#[derive(Debug, Clone)]
pub struct FormatConversionStats {
    /// Input audio format
    pub input_format: AudioFormat,
    /// Output audio format
    pub output_format: AudioFormat,
    /// Sample rate conversion ratio
    pub sample_rate_ratio: f64,
    /// Current buffer size
    pub buffer_size: usize,
}

/// Audio frame buffer for managing multiple frames with format conversion
pub struct AudioFrameBuffer {
    /// Maximum number of frames to buffer
    max_frames: usize,
    /// Buffer of audio frames
    frames: VecDeque<AudioFrame>,
    /// Target format for buffered frames
    target_format: AudioFormat,
    /// Format converter
    converter: Option<FormatConverter>,
}

impl AudioFrameBuffer {
    /// Create a new audio frame buffer
    pub fn new(max_frames: usize, target_format: AudioFormat) -> Self {
        Self {
            max_frames,
            frames: VecDeque::with_capacity(max_frames),
            target_format,
            converter: None,
        }
    }

    /// Add a frame to the buffer, converting format if necessary
    pub fn push_frame(&mut self, frame: AudioFrame) -> AudioResult<()> {
        let converted_frame = if frame.format.is_compatible_with(&self.target_format) {
            frame
        } else {
            // Initialize converter if needed
            if self.converter.is_none() || 
               !self.converter.as_ref().unwrap().input_format.is_compatible_with(&frame.format) {
                self.converter = Some(FormatConverter::new(frame.format.clone(), self.target_format.clone())?);
            }

            self.converter.as_mut().unwrap().convert_frame(&frame)?
        };

        // Add frame to buffer
        if self.frames.len() >= self.max_frames {
            self.frames.pop_front();
        }
        self.frames.push_back(converted_frame);

        Ok(())
    }

    /// Pop a frame from the buffer
    pub fn pop_frame(&mut self) -> Option<AudioFrame> {
        self.frames.pop_front()
    }

    /// Get the number of frames in the buffer
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Get buffer statistics
    pub fn get_stats(&self) -> AudioFrameBufferStats {
        AudioFrameBufferStats {
            current_frames: self.frames.len(),
            max_frames: self.max_frames,
            target_format: self.target_format.clone(),
            converter_active: self.converter.is_some(),
        }
    }

    /// Clear all frames from the buffer
    pub fn clear(&mut self) {
        self.frames.clear();
        if let Some(ref mut converter) = self.converter {
            converter.reset();
        }
    }
}

/// Statistics about audio frame buffer
#[derive(Debug, Clone)]
pub struct AudioFrameBufferStats {
    /// Current number of frames in buffer
    pub current_frames: usize,
    /// Maximum number of frames
    pub max_frames: usize,
    /// Target format for buffered frames
    pub target_format: AudioFormat,
    /// Whether format converter is active
    pub converter_active: bool,
}

/// Utility functions for format conversion
pub mod utils {
    use super::*;

    /// Check if two formats require conversion
    pub fn requires_conversion(input: &AudioFormat, output: &AudioFormat) -> bool {
        !input.is_compatible_with(output)
    }

    /// Calculate conversion complexity score (higher = more complex)
    pub fn conversion_complexity(input: &AudioFormat, output: &AudioFormat) -> u32 {
        let mut complexity = 0;

        // Sample rate conversion complexity
        if input.sample_rate != output.sample_rate {
            let ratio = (output.sample_rate as f64 / input.sample_rate as f64).abs();
            complexity += if ratio > 2.0 || ratio < 0.5 { 3 } else { 1 };
        }

        // Channel conversion complexity
        if input.channels != output.channels {
            complexity += 1;
        }

        // Bit depth conversion complexity (not yet implemented)
        if input.bits_per_sample != output.bits_per_sample {
            complexity += 2;
        }

        complexity
    }

    /// Create optimal format converter configuration
    pub fn create_optimal_converter(
        input: AudioFormat,
        output: AudioFormat,
    ) -> AudioResult<FormatConverter> {
        // Validate conversion feasibility
        let complexity = conversion_complexity(&input, &output);
        if complexity > 6 {
            return Err(AudioError::FormatConversionFailed {
                source_format: input.description(),
                target_format: output.description(),
                reason: "Conversion too complex".to_string(),
            });
        }

        FormatConverter::new(input, output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_converter_creation() {
        let input_format = AudioFormat::pcm_8khz_mono();
        let output_format = AudioFormat::pcm_16khz_mono();
        
        let converter = FormatConverter::new(input_format, output_format);
        assert!(converter.is_ok());
        
        let converter = converter.unwrap();
        assert_eq!(converter.sample_rate_ratio, 2.0);
    }

    #[test]
    fn test_channel_conversion_mono_to_stereo() {
        let input_format = AudioFormat::pcm_8khz_mono();
        let output_format = AudioFormat::new(8000, 2, 16, 20);
        
        let mut converter = FormatConverter::new(input_format.clone(), output_format).unwrap();
        
        let input_samples = vec![100, 200, 300];
        let input_frame = AudioFrame::new(input_samples, input_format, 1000);
        
        let result = converter.convert_frame(&input_frame);
        assert!(result.is_ok());
        
        let output_frame = result.unwrap();
        assert_eq!(output_frame.samples, vec![100, 100, 200, 200, 300, 300]);
        assert_eq!(output_frame.format.channels, 2);
    }

    #[test]
    fn test_channel_conversion_stereo_to_mono() {
        let input_format = AudioFormat::new(8000, 2, 16, 20);
        let output_format = AudioFormat::pcm_8khz_mono();
        
        let mut converter = FormatConverter::new(input_format.clone(), output_format).unwrap();
        
        let input_samples = vec![100, 200, 300, 400]; // Two stereo samples
        let input_frame = AudioFrame::new(input_samples, input_format, 1000);
        
        let result = converter.convert_frame(&input_frame);
        assert!(result.is_ok());
        
        let output_frame = result.unwrap();
        assert_eq!(output_frame.samples, vec![150, 350]); // Averaged: (100+200)/2, (300+400)/2
        assert_eq!(output_frame.format.channels, 1);
    }

    #[test]
    fn test_sample_rate_conversion() {
        let input_format = AudioFormat::pcm_8khz_mono();
        let output_format = AudioFormat::pcm_16khz_mono();
        
        let mut converter = FormatConverter::new(input_format.clone(), output_format).unwrap();
        
        let input_samples = vec![100, 200, 300, 400];
        let input_frame = AudioFrame::new(input_samples, input_format, 1000);
        
        let result = converter.convert_frame(&input_frame);
        assert!(result.is_ok());
        
        let output_frame = result.unwrap();
        assert_eq!(output_frame.format.sample_rate, 16000);
        // Output should have approximately double the samples due to 2x upsampling
        assert!(output_frame.samples.len() >= 6 && output_frame.samples.len() <= 8);
    }

    #[test]
    fn test_no_conversion_needed() {
        let format = AudioFormat::pcm_8khz_mono();
        let mut converter = FormatConverter::new(format.clone(), format.clone()).unwrap();
        
        let input_samples = vec![100, 200, 300];
        let input_frame = AudioFrame::new(input_samples.clone(), format.clone(), 1000);
        
        let result = converter.convert_frame(&input_frame);
        assert!(result.is_ok());
        
        let output_frame = result.unwrap();
        assert_eq!(output_frame.samples, input_samples);
    }

    #[test]
    fn test_audio_frame_buffer() {
        let target_format = AudioFormat::pcm_8khz_mono();
        let mut buffer = AudioFrameBuffer::new(3, target_format.clone());
        
        // Add frames with same format
        for i in 0..4 {
            let samples = vec![i as i16; 160];
            let frame = AudioFrame::new(samples, target_format.clone(), i as u32);
            buffer.push_frame(frame).unwrap();
        }
        
        // Should only have 3 frames (max capacity)
        assert_eq!(buffer.len(), 3);
        
        // Pop a frame
        let frame = buffer.pop_frame();
        assert!(frame.is_some());
        assert_eq!(buffer.len(), 2);
    }

    #[test]
    fn test_conversion_complexity() {
        let simple = utils::conversion_complexity(
            &AudioFormat::pcm_8khz_mono(), 
            &AudioFormat::pcm_8khz_mono()
        );
        assert_eq!(simple, 0);
        
        let moderate = utils::conversion_complexity(
            &AudioFormat::pcm_8khz_mono(), 
            &AudioFormat::pcm_16khz_mono()
        );
        assert_eq!(moderate, 1);
        
        let complex = utils::conversion_complexity(
            &AudioFormat::pcm_8khz_mono(), 
            &AudioFormat::pcm_48khz_stereo()
        );
        assert!(complex > 1);
    }
} 
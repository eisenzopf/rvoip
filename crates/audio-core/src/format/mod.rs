//! Audio format conversion using dasp
//!
//! This module provides audio format conversion using the dasp library,
//! supporting sample rate conversion, channel mapping, and format changes.

use crate::types::{AudioFormat, AudioFrame};
use crate::error::{AudioError, AudioResult};

#[cfg(feature = "format-conversion")]
use dasp::{Sample, Signal};
#[cfg(feature = "format-conversion")]
use dasp::interpolate::linear::Linear;
#[cfg(feature = "format-conversion")]
use dasp::signal::{self, interpolate::Converter};

/// Audio format converter using dasp
pub struct FormatConverter {
    /// Input format specification
    input_format: AudioFormat,
    /// Output format specification  
    output_format: AudioFormat,
    /// Sample rate conversion ratio
    sample_rate_ratio: f64,
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
            sample_rate_ratio,
        })
    }

    /// Convert an audio frame to the target format
    #[cfg(feature = "format-conversion")]
    pub fn convert_frame(&mut self, input_frame: &AudioFrame) -> AudioResult<AudioFrame> {
        // Validate input
        if input_frame.format != self.input_format {
            return Err(AudioError::FormatConversionFailed {
                source_format: input_frame.format.description(),
                target_format: self.output_format.description(),
                reason: "Input frame format doesn't match converter input format".to_string(),
            });
        }

        let input_samples = &input_frame.samples;
        let output_samples = match (self.input_format.channels, self.output_format.channels) {
            (1, 1) => {
                // Mono to mono - handle sample rate conversion if needed
                if self.needs_sample_rate_conversion() {
                    self.convert_sample_rate_mono(input_samples)?
                } else {
                    input_samples.to_vec()
                }
            }
            (1, 2) => {
                // Mono to stereo
                self.convert_mono_to_stereo(input_samples)?
            }
            (2, 1) => {
                // Stereo to mono
                self.convert_stereo_to_mono(input_samples)?
            }
            (2, 2) => {
                // Stereo to stereo - handle sample rate conversion if needed
                if self.needs_sample_rate_conversion() {
                    self.convert_sample_rate_stereo(input_samples)?
                } else {
                    input_samples.to_vec()
                }
            }
            (1, n) if n > 2 => {
                // Mono to multi-channel (e.g., 5.1, 7.1)
                self.convert_mono_to_multichannel(input_samples, n)?
            }
            (2, n) if n > 2 => {
                // Stereo to multi-channel
                self.convert_stereo_to_multichannel(input_samples, n)?
            }
            (n, 1) if n > 2 => {
                // Multi-channel to mono
                self.convert_multichannel_to_mono(input_samples, n)?
            }
            (n, 2) if n > 2 => {
                // Multi-channel to stereo
                self.convert_multichannel_to_stereo(input_samples, n)?
            }
            (input_ch, output_ch) => {
                // Same channel count or unsupported conversion
                if input_ch == output_ch {
                    if self.needs_sample_rate_conversion() {
                        // Generic multi-channel sample rate conversion
                        self.convert_sample_rate_multichannel(input_samples, input_ch)?
                    } else {
                        input_samples.to_vec()
                    }
                } else {
                    return Err(AudioError::FormatConversionFailed {
                        source_format: format!("{} channels", input_ch),
                        target_format: format!("{} channels", output_ch),
                        reason: "Complex channel conversion not yet implemented".to_string(),
                    });
                }
            }
        };

        // Create output frame
        Ok(AudioFrame::new(
            output_samples,
            self.output_format.clone(),
            input_frame.timestamp,
        ))
    }

    /// Convert without dasp (fallback)
    #[cfg(not(feature = "format-conversion"))]
    pub fn convert_frame(&mut self, input_frame: &AudioFrame) -> AudioResult<AudioFrame> {
        // Basic conversion without dasp
        if input_frame.format != self.input_format {
            return Err(AudioError::FormatConversionFailed {
                source_format: input_frame.format.description(),
                target_format: self.output_format.description(),
                reason: "Input frame format doesn't match converter input format".to_string(),
            });
        }

        // Only support basic mono/stereo conversions without dasp
        let output_samples = match (self.input_format.channels, self.output_format.channels) {
            (1, 2) => {
                // Mono to stereo: duplicate each sample
                let mut stereo_samples = Vec::with_capacity(input_frame.samples.len() * 2);
                for &sample in &input_frame.samples {
                    stereo_samples.push(sample);
                    stereo_samples.push(sample);
                }
                stereo_samples
            }
            (2, 1) => {
                // Stereo to mono: average left and right channels
                let mut mono_samples = Vec::with_capacity(input_frame.samples.len() / 2);
                for chunk in input_frame.samples.chunks_exact(2) {
                    let left = chunk[0] as i32;
                    let right = chunk[1] as i32;
                    let mixed = ((left + right) / 2) as i16;
                    mono_samples.push(mixed);
                }
                mono_samples
            }
            (input_ch, output_ch) if input_ch == output_ch => {
                input_frame.samples.clone()
            }
            (input_ch, output_ch) => {
                return Err(AudioError::FormatConversionFailed {
                    source_format: format!("{} channels", input_ch),
                    target_format: format!("{} channels", output_ch),
                    reason: "Format conversion not available (dasp feature disabled)".to_string(),
                });
            }
        };

        Ok(AudioFrame::new(
            output_samples,
            self.output_format.clone(),
            input_frame.timestamp,
        ))
    }

    fn needs_sample_rate_conversion(&self) -> bool {
        self.input_format.sample_rate != self.output_format.sample_rate
    }

    #[cfg(feature = "format-conversion")]
    fn convert_mono_to_stereo(&self, samples: &[i16]) -> AudioResult<Vec<i16>> {
        let mut output = Vec::with_capacity(samples.len() * 2);
        
        if self.needs_sample_rate_conversion() {
            // Convert sample rate while expanding to stereo
            let signal = signal::from_iter(samples.iter().map(|&s| {
                let f = s.to_sample::<f32>();
                [f, f] // Duplicate mono to both channels
            }));
            
            let interpolator = Linear::new([0.0, 0.0], [0.0, 0.0]);
            let converter = signal.from_hz_to_hz(
                interpolator,
                self.input_format.sample_rate as f64,
                self.output_format.sample_rate as f64,
            );
            
            for frame in converter.until_exhausted() {
                output.push(frame[0].to_sample::<i16>());
                output.push(frame[1].to_sample::<i16>());
            }
        } else {
            // Just duplicate mono to stereo
            for &sample in samples {
                output.push(sample);
                output.push(sample);
            }
        }
        
        Ok(output)
    }

    #[cfg(feature = "format-conversion")]
    fn convert_stereo_to_mono(&self, samples: &[i16]) -> AudioResult<Vec<i16>> {
        let mut output = Vec::with_capacity(samples.len() / 2);
        
        if self.needs_sample_rate_conversion() {
            // Convert sample rate while mixing to mono
            let signal = signal::from_iter(samples.chunks_exact(2).map(|chunk| {
                let left = chunk[0].to_sample::<f32>();
                let right = chunk[1].to_sample::<f32>();
                (left + right) / 2.0
            }));
            
            let interpolator = Linear::new(0.0, 0.0);
            let converter = signal.from_hz_to_hz(
                interpolator,
                self.input_format.sample_rate as f64,
                self.output_format.sample_rate as f64,
            );
            
            for sample in converter.until_exhausted() {
                output.push(sample.to_sample::<i16>());
            }
        } else {
            // Just average stereo to mono
            for chunk in samples.chunks_exact(2) {
                let left = chunk[0] as i32;
                let right = chunk[1] as i32;
                output.push(((left + right) / 2) as i16);
            }
        }
        
        Ok(output)
    }

    #[cfg(feature = "format-conversion")]
    fn convert_mono_to_multichannel(&self, samples: &[i16], output_channels: u16) -> AudioResult<Vec<i16>> {
        let mut output = Vec::with_capacity(samples.len() * output_channels as usize);
        
        // Map mono to different speaker configurations
        let channel_map = match output_channels {
            // 5.1: FL, FR, C, LFE, RL, RR
            6 => vec![0.0, 0.0, 1.0, 0.1, 0.0, 0.0], // Mono -> Center + slight LFE
            // 7.1: FL, FR, C, LFE, RL, RR, SL, SR  
            8 => vec![0.0, 0.0, 1.0, 0.1, 0.0, 0.0, 0.0, 0.0], // Mono -> Center + slight LFE
            // Generic: put mono in first channel, silence others
            n => {
                let mut map = vec![0.0; n as usize];
                map[0] = 1.0;
                map
            }
        };
        
        for &sample in samples {
            let f = sample.to_sample::<f32>();
            for &gain in &channel_map {
                output.push((f * gain).to_sample::<i16>());
            }
        }
        
        // Apply sample rate conversion if needed
        if self.needs_sample_rate_conversion() {
            self.convert_sample_rate_multichannel(&output, output_channels)
        } else {
            Ok(output)
        }
    }

    #[cfg(feature = "format-conversion")]
    fn convert_stereo_to_multichannel(&self, samples: &[i16], output_channels: u16) -> AudioResult<Vec<i16>> {
        let mut output = Vec::with_capacity((samples.len() / 2) * output_channels as usize);
        
        // Map stereo to different speaker configurations
        for chunk in samples.chunks_exact(2) {
            let left = chunk[0].to_sample::<f32>();
            let right = chunk[1].to_sample::<f32>();
            
            match output_channels {
                // 5.1: FL, FR, C, LFE, RL, RR
                6 => {
                    output.push(left.to_sample::<i16>());       // FL
                    output.push(right.to_sample::<i16>());      // FR
                    output.push(((left + right) / 2.0).to_sample::<i16>()); // C
                    output.push(((left + right) / 20.0).to_sample::<i16>()); // LFE (reduced)
                    output.push((left * 0.7).to_sample::<i16>());  // RL
                    output.push((right * 0.7).to_sample::<i16>()); // RR
                }
                // 7.1: FL, FR, C, LFE, RL, RR, SL, SR
                8 => {
                    output.push(left.to_sample::<i16>());       // FL
                    output.push(right.to_sample::<i16>());      // FR
                    output.push(((left + right) / 2.0).to_sample::<i16>()); // C
                    output.push(((left + right) / 20.0).to_sample::<i16>()); // LFE (reduced)
                    output.push((left * 0.7).to_sample::<i16>());  // RL
                    output.push((right * 0.7).to_sample::<i16>()); // RR
                    output.push((left * 0.5).to_sample::<i16>());  // SL
                    output.push((right * 0.5).to_sample::<i16>()); // SR
                }
                // Generic: put left/right in first two channels, silence others
                n => {
                    output.push(left.to_sample::<i16>());
                    output.push(right.to_sample::<i16>());
                    for _ in 2..n {
                        output.push(0);
                    }
                }
            }
        }
        
        // Apply sample rate conversion if needed
        if self.needs_sample_rate_conversion() {
            self.convert_sample_rate_multichannel(&output, output_channels)
        } else {
            Ok(output)
        }
    }

    #[cfg(feature = "format-conversion")]
    fn convert_multichannel_to_mono(&self, samples: &[i16], input_channels: u16) -> AudioResult<Vec<i16>> {
        let mut output = Vec::with_capacity(samples.len() / input_channels as usize);
        
        // Average all channels to create mono
        for chunk in samples.chunks_exact(input_channels as usize) {
            let sum: i32 = chunk.iter().map(|&s| s as i32).sum();
            let avg = (sum / input_channels as i32) as i16;
            output.push(avg);
        }
        
        // Apply sample rate conversion if needed
        if self.needs_sample_rate_conversion() {
            self.convert_sample_rate_mono(&output)
        } else {
            Ok(output)
        }
    }

    #[cfg(feature = "format-conversion")]
    fn convert_multichannel_to_stereo(&self, samples: &[i16], input_channels: u16) -> AudioResult<Vec<i16>> {
        let mut output = Vec::with_capacity((samples.len() / input_channels as usize) * 2);
        
        // Downmix multi-channel to stereo based on standard mappings
        for chunk in samples.chunks_exact(input_channels as usize) {
            let (left, right) = match input_channels {
                // 5.1: FL, FR, C, LFE, RL, RR
                6 => {
                    let fl = chunk[0] as i32;
                    let fr = chunk[1] as i32;
                    let c = chunk[2] as i32;
                    let lfe = chunk[3] as i32;
                    let rl = chunk[4] as i32;
                    let rr = chunk[5] as i32;
                    
                    let left = (fl + (c / 2) + (lfe / 10) + (rl / 2)) / 2;
                    let right = (fr + (c / 2) + (lfe / 10) + (rr / 2)) / 2;
                    (left as i16, right as i16)
                }
                // 7.1: FL, FR, C, LFE, RL, RR, SL, SR
                8 => {
                    let fl = chunk[0] as i32;
                    let fr = chunk[1] as i32;
                    let c = chunk[2] as i32;
                    let lfe = chunk[3] as i32;
                    let rl = chunk[4] as i32;
                    let rr = chunk[5] as i32;
                    let sl = chunk[6] as i32;
                    let sr = chunk[7] as i32;
                    
                    let left = (fl + (c / 2) + (lfe / 10) + (rl / 3) + (sl / 3)) / 2;
                    let right = (fr + (c / 2) + (lfe / 10) + (rr / 3) + (sr / 3)) / 2;
                    (left as i16, right as i16)
                }
                // Generic: take first two channels if available
                _ => {
                    let left = chunk[0];
                    let right = if input_channels > 1 { chunk[1] } else { chunk[0] };
                    (left, right)
                }
            };
            
            output.push(left);
            output.push(right);
        }
        
        // Apply sample rate conversion if needed
        if self.needs_sample_rate_conversion() {
            self.convert_sample_rate_stereo(&output)
        } else {
            Ok(output)
        }
    }

    #[cfg(feature = "format-conversion")]
    fn convert_sample_rate_mono(&self, samples: &[i16]) -> AudioResult<Vec<i16>> {
        let signal = signal::from_iter(samples.iter().map(|&s| s.to_sample::<f32>()));
        let interpolator = Linear::new(0.0, 0.0);
        let converter = signal.from_hz_to_hz(
            interpolator,
            self.input_format.sample_rate as f64,
            self.output_format.sample_rate as f64,
        );
        
        Ok(converter.until_exhausted().map(|s| s.to_sample::<i16>()).collect())
    }

    #[cfg(feature = "format-conversion")]
    fn convert_sample_rate_stereo(&self, samples: &[i16]) -> AudioResult<Vec<i16>> {
        let signal = signal::from_iter(samples.chunks_exact(2).map(|chunk| {
            [chunk[0].to_sample::<f32>(), chunk[1].to_sample::<f32>()]
        }));
        
        let interpolator = Linear::new([0.0, 0.0], [0.0, 0.0]);
        let converter = signal.from_hz_to_hz(
            interpolator,
            self.input_format.sample_rate as f64,
            self.output_format.sample_rate as f64,
        );
        
        let mut output = Vec::new();
        for frame in converter.until_exhausted() {
            output.push(frame[0].to_sample::<i16>());
            output.push(frame[1].to_sample::<i16>());
        }
        
        Ok(output)
    }

    #[cfg(feature = "format-conversion")]
    fn convert_sample_rate_multichannel(&self, samples: &[i16], channels: u16) -> AudioResult<Vec<i16>> {
        // For now, do a simple linear interpolation per channel
        // This could be optimized with dasp's multi-channel support
        let mut output = Vec::new();
        
        for ch in 0..channels as usize {
            let channel_samples: Vec<f32> = samples.chunks_exact(channels as usize)
                .map(|chunk| chunk[ch].to_sample::<f32>())
                .collect();
            
            let signal = signal::from_iter(channel_samples.into_iter());
            let interpolator = Linear::new(0.0, 0.0);
            let converter = signal.from_hz_to_hz(
                interpolator,
                self.input_format.sample_rate as f64,
                self.output_format.sample_rate as f64,
            );
            
            let resampled: Vec<f32> = converter.until_exhausted().collect();
            
            // Interleave the resampled channel back
            if ch == 0 {
                output.resize(resampled.len() * channels as usize, 0);
            }
            
            for (i, sample) in resampled.iter().enumerate() {
                output[i * channels as usize + ch] = sample.to_sample::<i16>();
            }
        }
        
        Ok(output)
    }

    /// Check if two formats are compatible (no conversion needed)
    pub fn formats_compatible(input: &AudioFormat, output: &AudioFormat) -> bool {
        input.sample_rate == output.sample_rate &&
        input.channels == output.channels &&
        input.bits_per_sample == output.bits_per_sample
    }
}

/// Audio frame buffer for managing audio data flow
pub struct AudioFrameBuffer {
    /// Buffer of frames
    frames: std::collections::VecDeque<AudioFrame>,
    /// Maximum number of frames to buffer
    max_frames: usize,
    /// Expected format
    format: AudioFormat,
}

impl AudioFrameBuffer {
    /// Create a new audio frame buffer
    pub fn new(max_frames: usize, format: AudioFormat) -> Self {
        Self {
            frames: std::collections::VecDeque::with_capacity(max_frames),
            max_frames,
            format,
        }
    }

    /// Add a frame to the buffer
    pub fn push(&mut self, frame: AudioFrame) -> bool {
        if self.frames.len() >= self.max_frames {
            return false;
        }
        self.frames.push_back(frame);
        true
    }

    /// Remove and return the oldest frame
    pub fn pop(&mut self) -> Option<AudioFrame> {
        self.frames.pop_front()
    }

    /// Get the number of frames in the buffer
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Check if buffer is empty
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Check if buffer is full
    pub fn is_full(&self) -> bool {
        self.frames.len() >= self.max_frames
    }

    /// Clear all frames
    pub fn clear(&mut self) {
        self.frames.clear()
    }

    /// Get buffer statistics
    pub fn get_stats(&self) -> AudioFrameBufferStats {
        AudioFrameBufferStats {
            current_frames: self.frames.len(),
            max_frames: self.max_frames,
            format: self.format.clone(),
        }
    }
}

/// Statistics for audio frame buffer
#[derive(Debug, Clone)]
pub struct AudioFrameBufferStats {
    /// Current number of frames in buffer
    pub current_frames: usize,
    /// Maximum frames allowed
    pub max_frames: usize,
    /// Buffer format
    pub format: AudioFormat,
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
    fn test_formats_compatible() {
        let format1 = AudioFormat::pcm_8khz_mono();
        let format2 = AudioFormat::pcm_8khz_mono();
        let format3 = AudioFormat::pcm_16khz_mono();
        
        assert!(FormatConverter::formats_compatible(&format1, &format2));
        assert!(!FormatConverter::formats_compatible(&format1, &format3));
    }

    #[cfg(feature = "format-conversion")]
    #[test]
    fn test_mono_to_stereo_conversion() {
        let input_format = AudioFormat::pcm_8khz_mono();
        let output_format = AudioFormat::new(8000, 2, 16, 20);
        
        let mut converter = FormatConverter::new(input_format.clone(), output_format).unwrap();
        
        let input_samples = vec![100, 200, 300];
        let input_frame = AudioFrame::new(input_samples, input_format, 1000);
        
        let result = converter.convert_frame(&input_frame);
        assert!(result.is_ok());
        
        let output_frame = result.unwrap();
        assert_eq!(output_frame.samples.len(), 6); // 3 mono samples -> 6 stereo samples
        assert_eq!(output_frame.format.channels, 2);
    }

    #[cfg(feature = "format-conversion")]
    #[test]
    fn test_mono_to_8_channel_conversion() {
        let input_format = AudioFormat::pcm_8khz_mono();
        let output_format = AudioFormat::new(8000, 8, 16, 20);
        
        let mut converter = FormatConverter::new(input_format.clone(), output_format).unwrap();
        
        let input_samples = vec![100, 200];
        let input_frame = AudioFrame::new(input_samples, input_format, 1000);
        
        let result = converter.convert_frame(&input_frame);
        assert!(result.is_ok());
        
        let output_frame = result.unwrap();
        assert_eq!(output_frame.samples.len(), 16); // 2 mono samples -> 16 samples (2 * 8 channels)
        assert_eq!(output_frame.format.channels, 8);
        
        // Check that mono is mapped to center channel (index 2) with others silent
        for i in 0..2 {
            let base = i * 8;
            assert_eq!(output_frame.samples[base], 0);      // FL
            assert_eq!(output_frame.samples[base + 1], 0);  // FR
            assert_ne!(output_frame.samples[base + 2], 0);  // C (has audio)
            // LFE has reduced audio
            assert_eq!(output_frame.samples[base + 4], 0);  // RL
            assert_eq!(output_frame.samples[base + 5], 0);  // RR
            assert_eq!(output_frame.samples[base + 6], 0);  // SL
            assert_eq!(output_frame.samples[base + 7], 0);  // SR
        }
    }

    #[test]
    fn test_frame_buffer() {
        let format = AudioFormat::pcm_8khz_mono();
        let mut buffer = AudioFrameBuffer::new(5, format.clone());
        
        assert!(buffer.is_empty());
        assert_eq!(buffer.len(), 0);
        
        // Add frames
        for i in 0..5 {
            let frame = AudioFrame::new(vec![i as i16], format.clone(), i as u32);
            assert!(buffer.push(frame));
        }
        
        assert!(buffer.is_full());
        assert_eq!(buffer.len(), 5);
        
        // Try to add one more
        let frame = AudioFrame::new(vec![100], format.clone(), 100);
        assert!(!buffer.push(frame));
        
        // Pop frames
        let first = buffer.pop().unwrap();
        assert_eq!(first.samples[0], 0);
        assert_eq!(buffer.len(), 4);
    }
}
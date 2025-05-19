use std::cmp::min;

use crate::{AudioBuffer, AudioFormat, Sample, SampleRate};
use crate::error::{Error, Result};
use crate::codec::audio::common::{ChannelLayout, SampleFormat};

/// Audio sample converter for format conversions
pub struct AudioConverter {
    /// Source audio format
    source_format: AudioFormat,
    /// Target audio format
    target_format: AudioFormat,
    /// Resampling state (if needed)
    resampler: Option<Resampler>,
    /// Channel conversion state (if needed)
    channel_converter: Option<ChannelConverter>,
    /// Format conversion needed
    format_conversion_needed: bool,
}

impl AudioConverter {
    /// Create a new audio converter
    pub fn new(source_format: AudioFormat, target_format: AudioFormat) -> Result<Self> {
        // Check if we need resampling
        let resampler = if source_format.sample_rate != target_format.sample_rate {
            Some(Resampler::new(
                source_format.sample_rate.as_hz(),
                target_format.sample_rate.as_hz(),
            )?)
        } else {
            None
        };
        
        // Check if we need channel conversion
        let channel_converter = if source_format.channels != target_format.channels {
            Some(ChannelConverter::new(
                source_format.channels,
                target_format.channels,
            ))
        } else {
            None
        };
        
        // Check if we need format conversion
        let format_conversion_needed = source_format.format != target_format.format;
        
        Ok(Self {
            source_format,
            target_format,
            resampler,
            channel_converter,
            format_conversion_needed,
        })
    }
    
    /// Convert audio data from source to target format
    pub fn convert(&mut self, input: &[u8], output: &mut [u8]) -> Result<usize> {
        // Calculate frame sizes
        let src_bytes_per_sample = self.source_format.bytes_per_sample();
        let dst_bytes_per_sample = self.target_format.bytes_per_sample();
        
        let src_channels = self.source_format.channel_count() as usize;
        let dst_channels = self.target_format.channel_count() as usize;
        
        // Convert to intermediate format (S16 for simplicity)
        let src_samples = input.len() / src_bytes_per_sample / src_channels;
        
        // Create intermediate buffers for processing
        let mut intermediate = vec![0i16; src_samples * src_channels];
        
        // Convert input format to S16 for processing
        self.convert_to_s16(input, &mut intermediate)?;
        
        // Apply resampling if needed
        let mut resampled = if let Some(resampler) = &mut self.resampler {
            let mut buffer = vec![0i16; (src_samples * dst_channels * 2) as usize]; // Output buffer with 2x space for safety
            let resampled_frames = resampler.process(&intermediate, &mut buffer, src_channels)?;
            buffer.truncate(resampled_frames * src_channels);
            buffer
        } else {
            intermediate
        };
        
        // Apply channel conversion if needed
        let converted = if let Some(converter) = &mut self.channel_converter {
            let mut buffer = vec![0i16; resampled.len() / src_channels * dst_channels];
            converter.process(&resampled, &mut buffer, src_channels, dst_channels)?;
            buffer
        } else {
            resampled
        };
        
        // Convert from S16 to target format
        let bytes_written = self.convert_from_s16(&converted, output)?;
        
        Ok(bytes_written)
    }
    
    /// Convert input format to S16 intermediate format
    fn convert_to_s16(&self, input: &[u8], output: &mut [i16]) -> Result<()> {
        let src_format = self.source_format.format;
        let src_bytes_per_sample = src_format.bytes_per_sample();
        let samples = min(
            input.len() / src_bytes_per_sample,
            output.len()
        );
        
        match src_format {
            SampleFormat::S16 => {
                // Direct copy if already S16
                let src_slice = unsafe {
                    std::slice::from_raw_parts(
                        input.as_ptr() as *const i16,
                        samples
                    )
                };
                output[..samples].copy_from_slice(src_slice);
            },
            SampleFormat::U8 => {
                // Convert U8 to S16
                for i in 0..samples {
                    let u8_sample = input[i] as i16;
                    output[i] = ((u8_sample - 128) << 8) as i16;
                }
            },
            SampleFormat::S24 => {
                // Convert S24 to S16
                for i in 0..samples {
                    let idx = i * 3;
                    if idx + 2 < input.len() {
                        let sample = ((input[idx] as i32) << 8) |
                                     ((input[idx + 1] as i32) << 16) |
                                     ((input[idx + 2] as i32) << 24);
                        output[i] = (sample >> 16) as i16;
                    }
                }
            },
            SampleFormat::S32 => {
                // Convert S32 to S16
                let src_slice = unsafe {
                    std::slice::from_raw_parts(
                        input.as_ptr() as *const i32,
                        samples
                    )
                };
                for i in 0..samples {
                    output[i] = (src_slice[i] >> 16) as i16;
                }
            },
            SampleFormat::F32 => {
                // Convert F32 to S16
                let src_slice = unsafe {
                    std::slice::from_raw_parts(
                        input.as_ptr() as *const f32,
                        samples
                    )
                };
                for i in 0..samples {
                    let sample = (src_slice[i] * 32767.0).clamp(-32768.0, 32767.0);
                    output[i] = sample as i16;
                }
            },
            SampleFormat::F64 => {
                // Convert F64 to S16
                let src_slice = unsafe {
                    std::slice::from_raw_parts(
                        input.as_ptr() as *const f64,
                        samples
                    )
                };
                for i in 0..samples {
                    let sample = (src_slice[i] * 32767.0).clamp(-32768.0, 32767.0);
                    output[i] = sample as i16;
                }
            },
        }
        
        Ok(())
    }
    
    /// Convert from S16 intermediate format to output format
    fn convert_from_s16(&self, input: &[i16], output: &mut [u8]) -> Result<usize> {
        let dst_format = self.target_format.format;
        let dst_bytes_per_sample = dst_format.bytes_per_sample();
        let samples = min(
            input.len(),
            output.len() / dst_bytes_per_sample
        );
        
        match dst_format {
            SampleFormat::S16 => {
                // Direct copy if already S16
                let dst_slice = unsafe {
                    std::slice::from_raw_parts_mut(
                        output.as_mut_ptr() as *mut i16,
                        samples
                    )
                };
                dst_slice.copy_from_slice(&input[..samples]);
                Ok(samples * 2)
            },
            SampleFormat::U8 => {
                // Convert S16 to U8
                for i in 0..samples {
                    output[i] = ((input[i] >> 8) + 128) as u8;
                }
                Ok(samples)
            },
            SampleFormat::S24 => {
                // Convert S16 to S24
                for i in 0..samples {
                    let idx = i * 3;
                    if idx + 2 < output.len() {
                        let sample = input[i] as i32;
                        output[idx] = 0;
                        output[idx + 1] = (sample & 0xFF) as u8;
                        output[idx + 2] = ((sample >> 8) & 0xFF) as u8;
                    }
                }
                Ok(samples * 3)
            },
            SampleFormat::S32 => {
                // Convert S16 to S32
                let dst_slice = unsafe {
                    std::slice::from_raw_parts_mut(
                        output.as_mut_ptr() as *mut i32,
                        samples
                    )
                };
                for i in 0..samples {
                    dst_slice[i] = (input[i] as i32) << 16;
                }
                Ok(samples * 4)
            },
            SampleFormat::F32 => {
                // Convert S16 to F32
                let dst_slice = unsafe {
                    std::slice::from_raw_parts_mut(
                        output.as_mut_ptr() as *mut f32,
                        samples
                    )
                };
                for i in 0..samples {
                    dst_slice[i] = input[i] as f32 / 32768.0;
                }
                Ok(samples * 4)
            },
            SampleFormat::F64 => {
                // Convert S16 to F64
                let dst_slice = unsafe {
                    std::slice::from_raw_parts_mut(
                        output.as_mut_ptr() as *mut f64,
                        samples
                    )
                };
                for i in 0..samples {
                    dst_slice[i] = input[i] as f64 / 32768.0;
                }
                Ok(samples * 8)
            },
        }
    }
    
    /// Get the source format
    pub fn source_format(&self) -> AudioFormat {
        self.source_format
    }
    
    /// Get the target format
    pub fn target_format(&self) -> AudioFormat {
        self.target_format
    }
    
    /// Check if resampling is needed
    pub fn needs_resampling(&self) -> bool {
        self.resampler.is_some()
    }
    
    /// Check if channel conversion is needed
    pub fn needs_channel_conversion(&self) -> bool {
        self.channel_converter.is_some()
    }
    
    /// Check if format conversion is needed
    pub fn needs_format_conversion(&self) -> bool {
        self.format_conversion_needed
    }
    
    /// Calculate output buffer size needed for conversion
    pub fn calculate_output_size(&self, input_size: usize) -> usize {
        let src_bytes_per_sample = self.source_format.bytes_per_sample();
        let dst_bytes_per_sample = self.target_format.bytes_per_sample();
        
        let src_channels = self.source_format.channel_count() as usize;
        let dst_channels = self.target_format.channel_count() as usize;
        
        let src_samples = input_size / src_bytes_per_sample / src_channels;
        
        // Calculate samples after resampling
        let resampled_samples = if let Some(resampler) = &self.resampler {
            let ratio = self.target_format.sample_rate.as_hz() as f64 / 
                       self.source_format.sample_rate.as_hz() as f64;
            (src_samples as f64 * ratio).ceil() as usize
        } else {
            src_samples
        };
        
        // Calculate buffer size after channel conversion
        let channel_ratio = dst_channels as f64 / src_channels as f64;
        let output_samples = (resampled_samples as f64 * channel_ratio).ceil() as usize;
        
        // Final size
        output_samples * dst_bytes_per_sample
    }
}

/// Sample rate converter
struct Resampler {
    /// Source sample rate
    source_rate: u32,
    /// Target sample rate
    target_rate: u32,
    /// Resampling ratio
    ratio: f64,
    /// Resampling quality (0-10)
    quality: u8,
}

impl Resampler {
    /// Create a new resampler
    pub fn new(source_rate: u32, target_rate: u32) -> Result<Self> {
        if source_rate == 0 || target_rate == 0 {
            return Err(Error::InvalidParameter(
                format!("Invalid sample rate: src={}, dst={}", source_rate, target_rate)
            ));
        }
        
        let ratio = target_rate as f64 / source_rate as f64;
        
        Ok(Self {
            source_rate,
            target_rate,
            ratio,
            quality: 5, // Medium quality
        })
    }
    
    /// Process audio data
    pub fn process(&mut self, input: &[i16], output: &mut [i16], channels: usize) -> Result<usize> {
        if input.is_empty() {
            return Ok(0);
        }
        
        // Note: In a real implementation we might use a high-quality resampler library like libsamplerate
        // For stub purposes, we'll use a simple linear interpolation
        
        let frames_in = input.len() / channels;
        let max_frames_out = output.len() / channels;
        
        // Simple linear interpolation
        let mut frames_out = 0;
        for i in 0..max_frames_out {
            let src_pos = i as f64 / self.ratio;
            let src_frame = src_pos.floor() as usize;
            let next_frame = min(src_frame + 1, frames_in - 1);
            let fract = src_pos - src_frame as f64;
            
            if src_frame >= frames_in {
                break;
            }
            
            for ch in 0..channels {
                let cur_sample = input[src_frame * channels + ch] as f64;
                let next_sample = input[next_frame * channels + ch] as f64;
                let interp = cur_sample * (1.0 - fract) + next_sample * fract;
                output[i * channels + ch] = interp as i16;
            }
            
            frames_out += 1;
        }
        
        Ok(frames_out)
    }
    
    /// Set resampling quality
    pub fn set_quality(&mut self, quality: u8) {
        self.quality = quality.clamp(0, 10);
    }
    
    /// Get current conversion ratio
    pub fn ratio(&self) -> f64 {
        self.ratio
    }
}

/// Channel converter for mono/stereo conversion
struct ChannelConverter {
    /// Source channel layout
    source_layout: ChannelLayout,
    /// Target channel layout
    target_layout: ChannelLayout,
}

impl ChannelConverter {
    /// Create a new channel converter
    pub fn new(source_layout: ChannelLayout, target_layout: ChannelLayout) -> Self {
        Self {
            source_layout,
            target_layout,
        }
    }
    
    /// Process audio data
    pub fn process(
        &mut self,
        input: &[i16],
        output: &mut [i16],
        src_channels: usize,
        dst_channels: usize
    ) -> Result<usize> {
        let frames = input.len() / src_channels;
        let out_frames = min(frames, output.len() / dst_channels);
        
        match (src_channels, dst_channels) {
            (1, 2) => {
                // Mono to stereo (duplicate channel)
                for i in 0..out_frames {
                    let sample = input[i];
                    output[i * 2] = sample;
                    output[i * 2 + 1] = sample;
                }
            },
            (2, 1) => {
                // Stereo to mono (average channels)
                for i in 0..out_frames {
                    let left = input[i * 2] as i32;
                    let right = input[i * 2 + 1] as i32;
                    output[i] = ((left + right) / 2) as i16;
                }
            },
            _ => {
                // Simple mixer for other channel combinations
                // Real implementation would use a proper mixing matrix
                if src_channels == dst_channels {
                    // Direct copy if same number of channels
                    output[..out_frames * dst_channels].copy_from_slice(
                        &input[..out_frames * src_channels]
                    );
                } else if src_channels > dst_channels {
                    // Downmix (average extra channels)
                    for i in 0..out_frames {
                        for ch in 0..dst_channels {
                            let mut sum = 0i32;
                            let channels_to_mix = src_channels / dst_channels;
                            
                            for mix_ch in 0..channels_to_mix {
                                sum += input[i * src_channels + ch * channels_to_mix + mix_ch] as i32;
                            }
                            
                            output[i * dst_channels + ch] = (sum / channels_to_mix as i32) as i16;
                        }
                    }
                } else {
                    // Upmix (duplicate channels)
                    for i in 0..out_frames {
                        for ch in 0..dst_channels {
                            let src_ch = ch % src_channels;
                            output[i * dst_channels + ch] = input[i * src_channels + src_ch];
                        }
                    }
                }
            }
        }
        
        Ok(out_frames)
    }
}

impl Default for AudioConverter {
    fn default() -> Self {
        Self::new(AudioFormat::default(), AudioFormat::default()).unwrap()
    }
} 
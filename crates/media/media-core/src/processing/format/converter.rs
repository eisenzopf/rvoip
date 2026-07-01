//! Format Converter - Audio format conversion
//!
//! This module handles conversion between different audio formats including
//! sample rate conversion, channel layout changes, and bit depth conversion.

use super::channel_mixer::{ChannelLayout, ChannelMixer};
use super::resampler::Resampler;
use crate::error::{AudioProcessingError, Result};
use crate::types::{AudioFrame, SampleRate};
use tracing::debug;
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
        params: &ConversionParams,
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

        debug!(
            "Converting audio: {}Hz,{}ch -> {}Hz,{}ch",
            input.sample_rate,
            input.channels,
            params.target_sample_rate.as_hz(),
            params.target_channels
        );

        // The resampler treats its input as one flat stream, so it must only
        // ever see MONO: resampling an interleaved multi-channel buffer as a
        // flat array folds the channels into each other and corrupts the audio
        // (this is what garbled the 48 kHz-stereo Opus -> 8 kHz-mono G.711 leg).
        // Keep channels == 1 across the resample by down-mixing *before*
        // resampling and up-mixing *after*.
        let downmixing = needs_channel_conversion && params.target_channels < input.channels;

        let mut converted_frame = input.clone();

        // Step 1a: down-mix first when reducing channels (so the resampler sees mono).
        if downmixing {
            converted_frame = self.convert_channels(&converted_frame, params)?;
        }

        // Step 2: sample-rate conversion (mono in the down-mix case).
        if needs_sample_rate_conversion {
            self.update_resampler(converted_frame.sample_rate, params)?;
            converted_frame = self.convert_sample_rate(&converted_frame, params)?;
        }

        // Step 1b: up-mix (or any non-reducing channel change) after resampling.
        if needs_channel_conversion && !downmixing {
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
        params: &ConversionParams,
    ) -> Result<()> {
        let target_rate = params.target_sample_rate.as_hz();

        // Check if we need to create/update resampler
        let needs_update = match &self.current_params {
            None => true,
            Some(current) => {
                current.target_sample_rate != params.target_sample_rate
                    || current.quality != params.quality
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
        params: &ConversionParams,
    ) -> Result<AudioFrame> {
        let resampler =
            self.resampler
                .as_mut()
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
        params: &ConversionParams,
    ) -> Result<AudioFrame> {
        let source_layout = match input.channels {
            1 => ChannelLayout::Mono,
            2 => ChannelLayout::Stereo,
            _ => {
                return Err(AudioProcessingError::ChannelConversionFailed {
                    from_channels: input.channels,
                    to_channels: params.target_channels,
                }
                .into())
            }
        };

        let target_layout = match params.target_channels {
            1 => ChannelLayout::Mono,
            2 => ChannelLayout::Stereo,
            _ => {
                return Err(AudioProcessingError::ChannelConversionFailed {
                    from_channels: input.channels,
                    to_channels: params.target_channels,
                }
                .into())
            }
        };

        let mixed_samples =
            self.channel_mixer
                .mix_channels(&input.samples, source_layout, target_layout)?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_48k_to_mono_8k_downmixes_before_resampling() {
        // 20 ms of interleaved stereo @ 48 kHz = 960 frames * 2 = 1920 samples.
        // Correct output: 20 ms mono @ 8 kHz = 160 samples. (The old order
        // resampled the interleaved buffer as mono first, scrambling L/R.)
        let mut c = FormatConverter::new();
        let input = AudioFrame::new(vec![0i16; 1920], 48000, 2, 0);
        let params = ConversionParams::new(SampleRate::Rate8000, 1);
        let out = c.convert_frame(&input, &params).unwrap();
        assert_eq!(out.frame.sample_rate, 8000);
        assert_eq!(out.frame.channels, 1);
        assert_eq!(out.frame.samples.len(), 160);
    }

    /// Goertzel magnitude at `freq` for a mono i16 buffer — lets a test assert a
    /// tone survived conversion (it should dominate its own bin).
    fn goertzel_mag(samples: &[i16], sample_rate: f64, freq: f64) -> f64 {
        let n = samples.len() as f64;
        let k = (freq / sample_rate * n).round();
        let coeff = 2.0 * (2.0 * std::f64::consts::PI * k / n).cos();
        let (mut s1, mut s2) = (0.0_f64, 0.0_f64);
        for &x in samples {
            let s = x as f64 + coeff * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        (s1 * s1 + s2 * s2 - coeff * s1 * s2).sqrt()
    }

    fn sine_i16(freq: f64, sample_rate: f64, frames: usize, amp: f64) -> Vec<i16> {
        (0..frames)
            .map(|i| {
                (amp * (2.0 * std::f64::consts::PI * freq * (i as f64) / sample_rate).sin()) as i16
            })
            .collect()
    }

    #[test]
    fn downsample_48k_stereo_to_8k_mono_preserves_tone() {
        // 1 kHz tone, 48 kHz stereo (identical L/R), 100 ms.
        let mono = sine_i16(1000.0, 48_000.0, 4_800, 10_000.0);
        let mut interleaved = Vec::with_capacity(mono.len() * 2);
        for &s in &mono {
            interleaved.push(s);
            interleaved.push(s);
        }
        let input = AudioFrame::new(interleaved, 48_000, 2, 0);
        let params = ConversionParams::new(SampleRate::Rate8000, 1);
        let mut c = FormatConverter::new();
        let out = c.convert_frame(&input, &params).unwrap().frame;
        assert_eq!(out.sample_rate, 8000);
        assert_eq!(out.channels, 1);
        assert_eq!(out.samples.len(), 800);
        let tone = goertzel_mag(&out.samples, 8_000.0, 1_000.0);
        let off = goertzel_mag(&out.samples, 8_000.0, 3_000.0);
        assert!(
            tone > 1.0 && tone > off * 5.0,
            "1 kHz not dominant after downsample: tone={tone}, off={off}"
        );
    }

    #[test]
    fn upsample_8k_mono_to_48k_stereo_preserves_tone() {
        // 1 kHz tone, 8 kHz mono, 100 ms.
        let input = AudioFrame::new(sine_i16(1000.0, 8_000.0, 800, 10_000.0), 8_000, 1, 0);
        let params = ConversionParams::new(SampleRate::Rate48000, 2);
        let mut c = FormatConverter::new();
        let out = c.convert_frame(&input, &params).unwrap().frame;
        assert_eq!(out.sample_rate, 48000);
        assert_eq!(out.channels, 2);
        assert_eq!(out.samples.len(), 9_600); // 4800 frames * 2ch interleaved
                                              // De-interleave the left channel back to mono @ 48 kHz.
        let left: Vec<i16> = out.samples.iter().step_by(2).copied().collect();
        let tone = goertzel_mag(&left, 48_000.0, 1_000.0);
        let off = goertzel_mag(&left, 48_000.0, 3_000.0);
        assert!(
            tone > 1.0 && tone > off * 5.0,
            "1 kHz not dominant after upsample: tone={tone}, off={off}"
        );
    }
}

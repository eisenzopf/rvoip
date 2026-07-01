//! Audio Resampler - Sample rate conversion
//!
//! Up-sampling uses linear/cubic interpolation. Down-sampling additionally runs
//! the signal through an anti-aliasing low-pass **before** decimation — without
//! it, content above the output Nyquist folds back into the band as harsh
//! distortion (e.g. the 48 kHz -> 8 kHz Opus->G.711 leg).
//!
//! The resampler operates on a single (mono) stream; callers must de-interleave
//! multi-channel audio first (see `FormatConverter`).

use crate::error::{AudioProcessingError, Result};
use tracing::{debug, warn};

/// Configuration for resampler
#[derive(Debug, Clone)]
pub struct ResamplerConfig {
    /// Input sample rate
    pub input_rate: u32,
    /// Output sample rate
    pub output_rate: u32,
    /// Quality level (0-10, higher = better quality)
    pub quality: u8,
}

/// Audio resampler for sample rate conversion
pub struct Resampler {
    /// Resampler configuration
    config: ResamplerConfig,
    /// Conversion ratio (output_rate / input_rate)
    ratio: f64,
    /// Current position in the input stream (fractional)
    position: f64,
    /// Previous sample for interpolation
    prev_sample: i16,
    /// Whether this is the first sample
    first_sample: bool,
    /// Anti-aliasing low-pass applied before down-sampling (empty when
    /// up-sampling — interpolation doesn't alias). Cascaded biquads for a
    /// steeper roll-off; state persists across frames for continuity.
    antialias: Vec<Biquad>,
}

/// One RBJ-cookbook biquad section (Direct Form I), used to build the
/// anti-aliasing low-pass applied before down-sampling.
#[derive(Debug, Clone)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    /// Low-pass biquad at `cutoff_hz` for a stream sampled at `sample_rate_hz`.
    fn low_pass(sample_rate_hz: f64, cutoff_hz: f64, q: f64) -> Self {
        let w0 = 2.0 * std::f64::consts::PI * cutoff_hz / sample_rate_hz;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 - cos_w0) / 2.0) / a0,
            b1: (1.0 - cos_w0) / a0,
            b2: ((1.0 - cos_w0) / 2.0) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// Process one sample (Direct Form I), advancing filter state.
    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    /// Clear filter memory.
    fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

impl Resampler {
    /// Create a new resampler
    pub fn new(input_rate: u32, output_rate: u32, quality: u8) -> Result<Self> {
        if input_rate == 0 || output_rate == 0 {
            return Err(AudioProcessingError::ResamplingFailed {
                from_rate: input_rate,
                to_rate: output_rate,
            }
            .into());
        }

        if quality > 10 {
            warn!("Resampler quality {} clamped to 10", quality);
        }

        let ratio = output_rate as f64 / input_rate as f64;

        debug!(
            "Creating resampler: {}Hz -> {}Hz (ratio: {:.4})",
            input_rate, output_rate, ratio
        );

        // Anti-aliasing: down-sampling folds everything above the output Nyquist
        // back into the audible band. Pre-filter with a low-pass at ~0.42x the
        // output rate (keeps the voice band, kills the fold-back). Two
        // Butterworth-Q sections ~= 4th order (~24 dB/octave). Up-sampling does
        // not alias, so no filter is used there.
        let antialias = if output_rate < input_rate {
            let fc = 0.42 * output_rate as f64;
            let q = std::f64::consts::FRAC_1_SQRT_2;
            vec![
                Biquad::low_pass(input_rate as f64, fc, q),
                Biquad::low_pass(input_rate as f64, fc, q),
            ]
        } else {
            Vec::new()
        };

        Ok(Self {
            config: ResamplerConfig {
                input_rate,
                output_rate,
                quality: quality.min(10),
            },
            ratio,
            position: 0.0,
            prev_sample: 0,
            first_sample: true,
            antialias,
        })
    }

    /// Resample audio samples.
    ///
    /// The deterministic output count is `round(input_len * ratio)`. Using
    /// `floor((i+1) * input_len / output_len)` for the integer step avoids
    /// floating-point drift that previously caused off-by-one output sizes
    /// for integer-ratio cases (e.g. 8 kHz → 48 kHz: 160 in → 960 out, not
    /// 961). The fixed count is what downstream codecs (Opus 20 ms = 960
    /// samples @ 48 kHz) require to avoid `InvalidFrameSize` errors.
    pub fn resample(&mut self, input_samples: &[i16]) -> Result<Vec<i16>> {
        if input_samples.is_empty() {
            return Ok(Vec::new());
        }

        // Anti-alias pre-filter for down-sampling (no-op when up-sampling, where
        // `antialias` is empty). Filtered in f64 with state carried across
        // frames for continuity.
        let filtered: Option<Vec<i16>> = if self.antialias.is_empty() {
            None
        } else {
            Some(
                input_samples
                    .iter()
                    .map(|&s| {
                        let mut x = s as f64;
                        for bq in &mut self.antialias {
                            x = bq.process(x);
                        }
                        x.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16
                    })
                    .collect(),
            )
        };
        let source: &[i16] = filtered.as_deref().unwrap_or(input_samples);

        // Deterministic output length. Use round() not ceil() so that
        // exact integer ratios (6.0×) don't pick up a phantom extra
        // sample from floating-point representation drift.
        let output_len = ((source.len() as f64) * self.ratio).round() as usize;
        let mut output_samples = Vec::with_capacity(output_len);

        // Generate exactly `output_len` samples. Index the input by a
        // ratio-derived float position; the loop bound (an integer)
        // never drifts.
        let input_len = source.len();
        for i in 0..output_len {
            // Position in source frame for output sample `i`.
            self.position = (i as f64) / self.ratio;
            let sample = self.interpolate_sample(source)?;
            output_samples.push(sample);
        }

        // Update state for next frame
        self.prev_sample = source[input_len - 1];
        self.first_sample = false;

        Ok(output_samples)
    }

    /// Reset resampler state
    pub fn reset(&mut self) {
        self.position = 0.0;
        self.prev_sample = 0;
        self.first_sample = true;
        for bq in &mut self.antialias {
            bq.reset();
        }
        debug!("Resampler reset");
    }

    /// Get conversion ratio
    pub fn ratio(&self) -> f64 {
        self.ratio
    }

    /// Get configuration
    pub fn config(&self) -> &ResamplerConfig {
        &self.config
    }

    /// Interpolate sample at current position
    fn interpolate_sample(&self, input_samples: &[i16]) -> Result<i16> {
        let index = self.position as usize;
        let fraction = self.position - index as f64;

        // Handle edge cases
        if index >= input_samples.len() {
            return Ok(self.prev_sample);
        }

        let current_sample = input_samples[index];

        // If no interpolation needed (exact sample)
        if fraction == 0.0 {
            return Ok(current_sample);
        }

        // Get next sample for interpolation
        let next_sample = if index + 1 < input_samples.len() {
            input_samples[index + 1]
        } else {
            // Use previous sample if at end
            current_sample
        };

        // Linear interpolation based on quality setting
        let interpolated = match self.config.quality {
            0..=2 => {
                // Nearest neighbor (no interpolation)
                if fraction < 0.5 {
                    current_sample
                } else {
                    next_sample
                }
            }
            3..=6 => {
                // Linear interpolation
                self.linear_interpolate(current_sample, next_sample, fraction)
            }
            7..=10 => {
                // Enhanced linear interpolation with smoothing
                self.smooth_interpolate(input_samples, index, fraction)
            }
            _ => current_sample, // Should not happen due to clamping
        };

        Ok(interpolated)
    }

    /// Perform linear interpolation between two samples
    fn linear_interpolate(&self, sample1: i16, sample2: i16, fraction: f64) -> i16 {
        let result = sample1 as f64 + (sample2 as f64 - sample1 as f64) * fraction;
        result.round() as i16
    }

    /// Perform smooth interpolation with neighboring samples
    fn smooth_interpolate(&self, input_samples: &[i16], index: usize, fraction: f64) -> i16 {
        // Use more samples for smoother interpolation
        let prev_sample = if index > 0 {
            input_samples[index - 1]
        } else if !self.first_sample {
            self.prev_sample
        } else {
            input_samples[index]
        };

        let current_sample = input_samples[index];
        let next_sample = if index + 1 < input_samples.len() {
            input_samples[index + 1]
        } else {
            current_sample
        };

        let next_next_sample = if index + 2 < input_samples.len() {
            input_samples[index + 2]
        } else {
            next_sample
        };

        // Cubic interpolation (simplified)
        let t = fraction;
        let t2 = t * t;
        let t3 = t2 * t;

        // Catmull-Rom spline coefficients
        let a0 = -0.5 * prev_sample as f64 + 1.5 * current_sample as f64 - 1.5 * next_sample as f64
            + 0.5 * next_next_sample as f64;
        let a1 = prev_sample as f64 - 2.5 * current_sample as f64 + 2.0 * next_sample as f64
            - 0.5 * next_next_sample as f64;
        let a2 = -0.5 * prev_sample as f64 + 0.5 * next_sample as f64;
        let a3 = current_sample as f64;

        let result = a0 * t3 + a1 * t2 + a2 * t + a3;

        // Clamp to 16-bit range
        result.max(i16::MIN as f64).min(i16::MAX as f64).round() as i16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resampler_creation() {
        let resampler = Resampler::new(8000, 16000, 5);
        assert!(resampler.is_ok());

        let resampler = resampler.unwrap();
        assert_eq!(resampler.ratio(), 2.0);
    }

    #[test]
    fn test_upsampling() {
        let mut resampler = Resampler::new(8000, 16000, 5).unwrap();
        let input = vec![100, 200, 300, 400];
        let output = resampler.resample(&input).unwrap();

        // Should approximately double the number of samples
        assert!(output.len() >= input.len() * 2 - 1);
        assert!(output.len() <= input.len() * 2 + 1);
    }

    #[test]
    fn test_downsampling() {
        let mut resampler = Resampler::new(16000, 8000, 5).unwrap();
        let input = vec![100, 150, 200, 250, 300, 350, 400, 450];
        let output = resampler.resample(&input).unwrap();

        // Should approximately halve the number of samples
        assert!(output.len() >= input.len() / 2 - 1);
        assert!(output.len() <= input.len() / 2 + 1);
    }

    #[test]
    fn downsampling_attenuates_above_output_nyquist() {
        // A 6 kHz tone at 48 kHz is above the 4 kHz output Nyquist. Naive
        // decimation folds it to 2 kHz at full amplitude; the anti-alias
        // low-pass must suppress it instead.
        let fs = 48_000.0_f64;
        let f = 6_000.0_f64;
        let input: Vec<i16> = (0..4_800)
            .map(|i| ((2.0 * std::f64::consts::PI * f * (i as f64) / fs).sin() * 10_000.0) as i16)
            .collect();
        let mut rs = Resampler::new(48_000, 8_000, 5).unwrap();
        let out = rs.resample(&input).unwrap();
        let rms = (out.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / out.len() as f64).sqrt();
        assert!(
            rms < 2_500.0,
            "6 kHz tone not attenuated on downsample: rms={rms}"
        );
    }
}

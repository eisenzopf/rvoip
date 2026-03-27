//! WebRTC Audio Processing Module (APM) Adapter
//!
//! Production-quality audio processing using Google's WebRTC AudioProcessing Module,
//! the same engine tested in Chrome/Android on billions of devices.
//!
//! This module wraps the `webrtc-audio-processing` crate (Rust bindings for the native
//! C++ WebRTC APM) and provides a unified interface for:
//! - Acoustic Echo Cancellation (AEC)
//! - Automatic Gain Control (AGC)
//! - Noise Suppression (NS)
//! - Voice Activity Detection (VAD)
//!
//! Requires the `webrtc-apm` feature flag (needs CMake and C++ compiler at build time).

use tracing::{debug, trace};
use crate::error::{Result, AudioProcessingError};
use crate::types::AudioFrame;

use webrtc_audio_processing::{
    Processor,
    InitializationConfig,
    Config,
    EchoCancellation,
    EchoCancellationSuppressionLevel,
    GainControl,
    GainControlMode,
    NoiseSuppression,
    NoiseSuppressionLevel,
    VoiceDetection,
    VoiceDetectionLikelihood,
    Stats,
    NUM_SAMPLES_PER_FRAME,
};

/// Configuration for the WebRTC APM adapter.
#[derive(Debug, Clone)]
pub struct WebRtcApmConfig {
    /// Enable acoustic echo cancellation.
    pub echo_cancellation: bool,
    /// AEC suppression level (higher = more aggressive).
    pub echo_suppression_level: AecSuppressionLevel,
    /// Enable automatic gain control.
    pub gain_control: bool,
    /// AGC target level in dBFS (0 to 31, lower = louder).
    pub agc_target_level_dbfs: i32,
    /// AGC compression gain in dB (0 to 90).
    pub agc_compression_gain_db: i32,
    /// Enable AGC limiter to prevent clipping.
    pub agc_enable_limiter: bool,
    /// Enable noise suppression.
    pub noise_suppression: bool,
    /// Noise suppression aggressiveness level.
    pub noise_suppression_level: NsLevel,
    /// Enable voice activity detection.
    pub voice_detection: bool,
    /// VAD detection sensitivity.
    pub vad_likelihood: VadLikelihood,
    /// Enable high-pass filter (removes DC offset and low-frequency noise).
    pub high_pass_filter: bool,
    /// Audio sample rate in Hz (must match the APM's internal rate, typically 48000).
    pub sample_rate: u32,
    /// Number of audio channels (1 = mono, 2 = stereo).
    pub channels: u16,
}

/// AEC suppression aggressiveness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AecSuppressionLevel {
    /// Minimal suppression, preserves near-end speech.
    Low,
    /// Balanced suppression.
    Moderate,
    /// Aggressive suppression for strong echo environments.
    High,
}

/// Noise suppression level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NsLevel {
    /// Minimal noise removal.
    Low,
    /// Balanced noise removal.
    Moderate,
    /// Strong noise removal.
    High,
    /// Maximum noise removal (may affect speech quality).
    VeryHigh,
}

/// Voice activity detection sensitivity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadLikelihood {
    /// Most sensitive, detects faint speech.
    VeryLow,
    /// Sensitive detection.
    Low,
    /// Balanced detection.
    Moderate,
    /// Conservative, only detects clear speech.
    High,
}

impl Default for WebRtcApmConfig {
    fn default() -> Self {
        Self {
            echo_cancellation: true,
            echo_suppression_level: AecSuppressionLevel::Moderate,
            gain_control: true,
            agc_target_level_dbfs: 3,
            agc_compression_gain_db: 9,
            agc_enable_limiter: true,
            noise_suppression: true,
            noise_suppression_level: NsLevel::High,
            voice_detection: true,
            vad_likelihood: VadLikelihood::Moderate,
            high_pass_filter: true,
            sample_rate: 48000,
            channels: 1,
        }
    }
}

/// Result of processing a capture frame through the WebRTC APM.
#[derive(Debug, Clone)]
pub struct WebRtcApmResult {
    /// Whether voice was detected in the last processed capture frame.
    pub has_voice: bool,
    /// Whether echo was detected.
    pub has_echo: bool,
    /// RMS level in dBFS of the processed signal.
    pub rms_dbfs: Option<i32>,
    /// Speech probability (0.0 to 1.0).
    pub speech_probability: f32,
    /// Echo return loss enhancement in dB (higher = better cancellation).
    pub echo_return_loss_enhancement_db: Option<f64>,
}

/// Production-quality audio processor wrapping Google's WebRTC APM.
///
/// Provides AEC, AGC, noise suppression, and VAD in a single processing call,
/// matching the behavior used in Chrome and Android.
pub struct WebRtcAudioProcessor {
    processor: Processor,
    config: WebRtcApmConfig,
    /// Cached stats from last processing call.
    last_stats: Stats,
    /// Expected number of f32 samples per frame (from the APM).
    samples_per_frame: usize,
}

impl std::fmt::Debug for WebRtcAudioProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebRtcAudioProcessor")
            .field("config", &self.config)
            .field("samples_per_frame", &self.samples_per_frame)
            .finish()
    }
}

impl WebRtcAudioProcessor {
    /// Create a new WebRTC APM processor with the given configuration.
    ///
    /// This initializes the underlying native WebRTC AudioProcessing module.
    /// The processor expects audio frames of a fixed size determined by the
    /// sample rate (typically 480 samples for 48kHz at 10ms frames).
    pub fn new(config: WebRtcApmConfig) -> Result<Self> {
        debug!(
            "Creating WebRtcAudioProcessor: rate={}Hz, channels={}, aec={}, agc={}, ns={}, vad={}",
            config.sample_rate, config.channels,
            config.echo_cancellation, config.gain_control,
            config.noise_suppression, config.voice_detection
        );

        let init_config = InitializationConfig {
            num_capture_channels: config.channels as i32,
            num_render_channels: config.channels as i32,
            enable_experimental_agc: false,
            enable_intelligibility_enhancer: false,
        };

        let mut processor = Processor::new(&init_config).map_err(|e| {
            AudioProcessingError::ProcessingFailed {
                reason: format!("Failed to create WebRTC APM processor: {}", e),
            }
        })?;

        // Build and apply the runtime configuration.
        let apm_config = Self::build_config(&config);
        processor.set_config(apm_config);

        let samples_per_frame = NUM_SAMPLES_PER_FRAME as usize;

        debug!(
            "WebRTC APM initialized: samples_per_frame={}",
            samples_per_frame
        );

        Ok(Self {
            processor,
            config,
            last_stats: Stats {
                has_voice: None,
                has_echo: None,
                rms_dbfs: None,
                speech_probability: None,
                residual_echo_return_loss: None,
                echo_return_loss: None,
                echo_return_loss_enhancement: None,
                a_nlp: None,
                delay_median_ms: None,
                delay_standard_deviation_ms: None,
                delay_fraction_poor_delays: None,
            },
            samples_per_frame,
        })
    }

    /// Process a capture frame (microphone input).
    ///
    /// This applies the full WebRTC APM processing chain in one call:
    /// AEC, AGC, noise suppression, and VAD (as configured).
    ///
    /// The `samples` slice is modified in-place with the processed audio.
    /// Samples must be 16-bit signed integers; they are internally converted to
    /// f32 for the APM and back.
    ///
    /// Returns processing results including VAD and echo detection status.
    pub fn process_capture(&mut self, samples: &mut [i16]) -> Result<WebRtcApmResult> {
        // Convert i16 samples to interleaved f32 for the APM.
        let mut f32_samples = self.i16_to_f32_frame(samples);

        // Pad or truncate to the expected frame size if needed.
        self.align_frame_size(&mut f32_samples);

        self.processor
            .process_capture_frame(&mut f32_samples)
            .map_err(|e| AudioProcessingError::ProcessingFailed {
                reason: format!("WebRTC APM capture processing failed: {}", e),
            })?;

        // Convert processed f32 back to i16 and write into the caller's buffer.
        self.f32_to_i16_frame(&f32_samples, samples);

        // Retrieve stats after processing.
        self.last_stats = self.processor.get_stats();

        let result = WebRtcApmResult {
            has_voice: self.last_stats.has_voice.unwrap_or(false),
            has_echo: self.last_stats.has_echo.unwrap_or(false),
            rms_dbfs: self.last_stats.rms_dbfs,
            speech_probability: self.last_stats.speech_probability.unwrap_or(0.0) as f32,
            echo_return_loss_enhancement_db: self.last_stats.echo_return_loss_enhancement,
        };

        trace!(
            "WebRTC APM capture: voice={}, echo={}, rms={:?}dBFS",
            result.has_voice, result.has_echo, result.rms_dbfs
        );

        Ok(result)
    }

    /// Process a render frame (speaker/far-end output for AEC reference).
    ///
    /// This must be called with the audio being played to the speaker so the AEC
    /// can learn the echo path. Call this *before* `process_capture` for each
    /// corresponding time interval.
    ///
    /// The `samples` slice is not modified (render processing is analysis-only).
    pub fn process_render(&mut self, samples: &[i16]) -> Result<()> {
        let mut f32_samples = self.i16_to_f32_frame(samples);
        self.align_frame_size(&mut f32_samples);

        self.processor
            .process_render_frame(&mut f32_samples)
            .map_err(|e| AudioProcessingError::ProcessingFailed {
                reason: format!("WebRTC APM render processing failed: {}", e),
            })?;

        trace!("WebRTC APM render frame processed ({} samples)", samples.len());
        Ok(())
    }

    /// Check if voice was detected in the last processed capture frame.
    pub fn has_voice(&self) -> bool {
        self.last_stats.has_voice.unwrap_or(false)
    }

    /// Check if echo was detected in the last processed capture frame.
    pub fn has_echo(&self) -> bool {
        self.last_stats.has_echo.unwrap_or(false)
    }

    /// Get the speech probability from the last processed capture frame (0.0 to 1.0).
    pub fn speech_probability(&self) -> f32 {
        self.last_stats.speech_probability.unwrap_or(0.0) as f32
    }

    /// Get the current noise level estimate as RMS in dBFS.
    /// Returns 0.0 if not available.
    pub fn noise_level(&self) -> f32 {
        // The APM does not directly expose a noise level, but the RMS of the
        // processed signal gives an indication of the residual noise floor.
        self.last_stats.rms_dbfs.unwrap_or(-96) as f32
    }

    /// Get the expected number of samples per processing frame.
    pub fn samples_per_frame(&self) -> usize {
        self.samples_per_frame
    }

    /// Update the processing configuration at runtime.
    ///
    /// This can be called while processing is active to enable/disable
    /// components or change their parameters.
    pub fn update_config(&mut self, config: WebRtcApmConfig) {
        let apm_config = Self::build_config(&config);
        self.processor.set_config(apm_config);
        self.config = config;
        debug!("WebRTC APM configuration updated");
    }

    /// Get the current configuration.
    pub fn get_config(&self) -> &WebRtcApmConfig {
        &self.config
    }

    /// Get the full stats from the last capture processing call.
    pub fn get_stats(&self) -> &Stats {
        &self.last_stats
    }

    /// Process an `AudioFrame` capture (convenience wrapper).
    ///
    /// Converts the frame's i16 samples, processes through the APM, and writes
    /// the processed samples back into the frame.
    pub fn process_capture_frame(&mut self, frame: &mut AudioFrame) -> Result<WebRtcApmResult> {
        self.process_capture(&mut frame.samples)
    }

    /// Process an `AudioFrame` render (convenience wrapper).
    pub fn process_render_frame(&mut self, frame: &AudioFrame) -> Result<()> {
        self.process_render(&frame.samples)
    }

    // --- Private helpers ---

    /// Build the WebRTC APM `Config` from our adapter config.
    fn build_config(config: &WebRtcApmConfig) -> Config {
        let echo_cancellation = if config.echo_cancellation {
            Some(EchoCancellation {
                suppression_level: match config.echo_suppression_level {
                    AecSuppressionLevel::Low => EchoCancellationSuppressionLevel::Low,
                    AecSuppressionLevel::Moderate => EchoCancellationSuppressionLevel::Moderate,
                    AecSuppressionLevel::High => EchoCancellationSuppressionLevel::High,
                },
                enable_extended_filter: true,
                enable_delay_agnostic: true,
                stream_delay_ms: None, // Let APM auto-detect
            })
        } else {
            None
        };

        let gain_control = if config.gain_control {
            Some(GainControl {
                mode: GainControlMode::AdaptiveDigital,
                target_level_dbfs: config.agc_target_level_dbfs,
                compression_gain_db: config.agc_compression_gain_db,
                enable_limiter: config.agc_enable_limiter,
            })
        } else {
            None
        };

        let noise_suppression = if config.noise_suppression {
            Some(NoiseSuppression {
                suppression_level: match config.noise_suppression_level {
                    NsLevel::Low => NoiseSuppressionLevel::Low,
                    NsLevel::Moderate => NoiseSuppressionLevel::Moderate,
                    NsLevel::High => NoiseSuppressionLevel::High,
                    NsLevel::VeryHigh => NoiseSuppressionLevel::VeryHigh,
                },
            })
        } else {
            None
        };

        let voice_detection = if config.voice_detection {
            Some(VoiceDetection {
                detection_likelihood: match config.vad_likelihood {
                    VadLikelihood::VeryLow => VoiceDetectionLikelihood::VeryLow,
                    VadLikelihood::Low => VoiceDetectionLikelihood::Low,
                    VadLikelihood::Moderate => VoiceDetectionLikelihood::Moderate,
                    VadLikelihood::High => VoiceDetectionLikelihood::High,
                },
            })
        } else {
            None
        };

        Config {
            echo_cancellation,
            gain_control,
            noise_suppression,
            voice_detection,
            enable_transient_suppressor: false,
            enable_high_pass_filter: config.high_pass_filter,
        }
    }

    /// Convert i16 samples to interleaved f32 in [-1.0, 1.0] range.
    fn i16_to_f32_frame(&self, samples: &[i16]) -> Vec<f32> {
        samples.iter().map(|&s| s as f32 / 32768.0).collect()
    }

    /// Convert f32 samples back to i16, writing into the destination buffer.
    /// Clamps to valid i16 range.
    fn f32_to_i16_frame(&self, src: &[f32], dst: &mut [i16]) {
        let copy_len = src.len().min(dst.len());
        for i in 0..copy_len {
            let clamped = src[i].max(-1.0).min(1.0);
            dst[i] = (clamped * 32767.0) as i16;
        }
    }

    /// Pad or truncate frame to the expected APM frame size.
    fn align_frame_size(&self, frame: &mut Vec<f32>) {
        let expected = self.samples_per_frame * self.config.channels as usize;
        if frame.len() < expected {
            frame.resize(expected, 0.0);
        } else if frame.len() > expected {
            frame.truncate(expected);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = WebRtcApmConfig::default();
        assert!(config.echo_cancellation);
        assert!(config.gain_control);
        assert!(config.noise_suppression);
        assert!(config.voice_detection);
        assert_eq!(config.sample_rate, 48000);
        assert_eq!(config.channels, 1);
    }

    #[test]
    fn test_build_config_all_enabled() {
        let config = WebRtcApmConfig::default();
        let apm_config = WebRtcAudioProcessor::build_config(&config);
        assert!(apm_config.echo_cancellation.is_some());
        assert!(apm_config.gain_control.is_some());
        assert!(apm_config.noise_suppression.is_some());
        assert!(apm_config.voice_detection.is_some());
        assert!(apm_config.enable_high_pass_filter);
    }

    #[test]
    fn test_build_config_all_disabled() {
        let config = WebRtcApmConfig {
            echo_cancellation: false,
            gain_control: false,
            noise_suppression: false,
            voice_detection: false,
            high_pass_filter: false,
            ..Default::default()
        };
        let apm_config = WebRtcAudioProcessor::build_config(&config);
        assert!(apm_config.echo_cancellation.is_none());
        assert!(apm_config.gain_control.is_none());
        assert!(apm_config.noise_suppression.is_none());
        assert!(apm_config.voice_detection.is_none());
        assert!(!apm_config.enable_high_pass_filter);
    }
}

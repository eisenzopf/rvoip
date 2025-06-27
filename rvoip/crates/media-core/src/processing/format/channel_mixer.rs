//! Channel Mixer - Audio channel layout conversion
//!
//! This module handles conversion between different channel layouts such as
//! mono to stereo, stereo to mono, and other channel configurations.

use tracing::{debug, warn};
use crate::error::{Result, AudioProcessingError};

/// Supported channel layouts
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelLayout {
    /// Single channel (mono)
    Mono,
    /// Two channels (stereo)
    Stereo,
}

impl ChannelLayout {
    /// Get the number of channels for this layout
    pub fn channel_count(&self) -> u8 {
        match self {
            ChannelLayout::Mono => 1,
            ChannelLayout::Stereo => 2,
        }
    }
    
    /// Create channel layout from channel count
    pub fn from_channels(channels: u8) -> Option<Self> {
        match channels {
            1 => Some(ChannelLayout::Mono),
            2 => Some(ChannelLayout::Stereo),
            _ => None,
        }
    }
}

/// Channel mixer for audio channel conversions
pub struct ChannelMixer {
    /// Mixing gain for mono-to-stereo conversion
    mono_to_stereo_gain: f32,
    /// Mixing coefficients for stereo-to-mono conversion
    stereo_to_mono_coeffs: (f32, f32),
}

impl ChannelMixer {
    /// Create a new channel mixer
    pub fn new() -> Self {
        Self {
            mono_to_stereo_gain: 1.0,           // No gain change
            stereo_to_mono_coeffs: (0.5, 0.5), // Equal mix
        }
    }
    
    /// Create channel mixer with custom settings
    pub fn with_settings(
        mono_to_stereo_gain: f32,
        stereo_to_mono_coeffs: (f32, f32),
    ) -> Self {
        Self {
            mono_to_stereo_gain,
            stereo_to_mono_coeffs,
        }
    }
    
    /// Mix audio channels from source layout to target layout
    pub fn mix_channels(
        &mut self,
        input_samples: &[i16],
        source_layout: ChannelLayout,
        target_layout: ChannelLayout,
    ) -> Result<Vec<i16>> {
        if source_layout == target_layout {
            // No conversion needed
            return Ok(input_samples.to_vec());
        }
        
        debug!("Converting channels: {:?} -> {:?}", source_layout, target_layout);
        
        match (source_layout, target_layout) {
            (ChannelLayout::Mono, ChannelLayout::Stereo) => {
                self.mono_to_stereo(input_samples)
            }
            (ChannelLayout::Stereo, ChannelLayout::Mono) => {
                self.stereo_to_mono(input_samples)
            }
            _ => {
                // This should not happen due to the equality check above
                Ok(input_samples.to_vec())
            }
        }
    }
    
    /// Reset mixer state
    pub fn reset(&mut self) {
        // No state to reset for this simple mixer
        debug!("ChannelMixer reset");
    }
    
    /// Set mono-to-stereo gain
    pub fn set_mono_to_stereo_gain(&mut self, gain: f32) {
        self.mono_to_stereo_gain = gain.max(0.0).min(2.0); // Clamp to reasonable range
    }
    
    /// Set stereo-to-mono mixing coefficients
    pub fn set_stereo_to_mono_coeffs(&mut self, left: f32, right: f32) {
        // Normalize coefficients to avoid clipping
        let sum = left + right;
        if sum > 0.0 {
            self.stereo_to_mono_coeffs = (left / sum, right / sum);
        } else {
            warn!("Invalid stereo-to-mono coefficients, using defaults");
            self.stereo_to_mono_coeffs = (0.5, 0.5);
        }
    }
    
    /// Convert mono audio to stereo
    fn mono_to_stereo(&self, input_samples: &[i16]) -> Result<Vec<i16>> {
        let mut output_samples = Vec::with_capacity(input_samples.len() * 2);
        
        for &sample in input_samples {
            // Apply gain and duplicate to both channels
            let adjusted_sample = self.apply_gain(sample, self.mono_to_stereo_gain);
            output_samples.push(adjusted_sample); // Left channel
            output_samples.push(adjusted_sample); // Right channel
        }
        
        Ok(output_samples)
    }
    
    /// Convert stereo audio to mono
    fn stereo_to_mono(&self, input_samples: &[i16]) -> Result<Vec<i16>> {
        if input_samples.len() % 2 != 0 {
            return Err(AudioProcessingError::InvalidFormat {
                details: "Stereo input must have even number of samples".to_string(),
            }.into());
        }
        
        let mut output_samples = Vec::with_capacity(input_samples.len() / 2);
        
        // Process samples in pairs (left, right)
        for chunk in input_samples.chunks_exact(2) {
            let left = chunk[0];
            let right = chunk[1];
            
            // Mix channels according to coefficients
            let mixed_sample = self.mix_stereo_sample(left, right);
            output_samples.push(mixed_sample);
        }
        
        Ok(output_samples)
    }
    
    /// Mix a stereo sample pair to mono
    fn mix_stereo_sample(&self, left: i16, right: i16) -> i16 {
        let mixed = left as f32 * self.stereo_to_mono_coeffs.0 
                  + right as f32 * self.stereo_to_mono_coeffs.1;
        
        // Clamp to 16-bit range and round
        mixed.max(i16::MIN as f32).min(i16::MAX as f32).round() as i16
    }
    
    /// Apply gain to a sample with clipping protection
    fn apply_gain(&self, sample: i16, gain: f32) -> i16 {
        let adjusted = sample as f32 * gain;
        
        // Clamp to 16-bit range to prevent overflow
        adjusted.max(i16::MIN as f32).min(i16::MAX as f32).round() as i16
    }
}

impl Default for ChannelMixer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_channel_layout_from_channels() {
        assert_eq!(ChannelLayout::from_channels(1), Some(ChannelLayout::Mono));
        assert_eq!(ChannelLayout::from_channels(2), Some(ChannelLayout::Stereo));
        assert_eq!(ChannelLayout::from_channels(3), None);
    }
    
    #[test]
    fn test_mono_to_stereo() {
        let mut mixer = ChannelMixer::new();
        let input = vec![100, 200, 300];
        let output = mixer.mix_channels(
            &input,
            ChannelLayout::Mono,
            ChannelLayout::Stereo,
        ).unwrap();
        
        assert_eq!(output.len(), 6);
        assert_eq!(output, vec![100, 100, 200, 200, 300, 300]);
    }
    
    #[test]
    fn test_stereo_to_mono() {
        let mut mixer = ChannelMixer::new();
        let input = vec![100, 200, 300, 400]; // Two stereo samples
        let output = mixer.mix_channels(
            &input,
            ChannelLayout::Stereo,
            ChannelLayout::Mono,
        ).unwrap();
        
        assert_eq!(output.len(), 2);
        assert_eq!(output[0], 150); // (100 + 200) / 2
        assert_eq!(output[1], 350); // (300 + 400) / 2
    }
    
    #[test]
    fn test_no_conversion_needed() {
        let mut mixer = ChannelMixer::new();
        let input = vec![100, 200, 300];
        let output = mixer.mix_channels(
            &input,
            ChannelLayout::Mono,
            ChannelLayout::Mono,
        ).unwrap();
        
        assert_eq!(output, input);
    }
    
    #[test]
    fn test_custom_stereo_to_mono_coeffs() {
        let mut mixer = ChannelMixer::new();
        mixer.set_stereo_to_mono_coeffs(0.8, 0.2); // Favor left channel
        
        let input = vec![100, 200]; // One stereo sample
        let output = mixer.mix_channels(
            &input,
            ChannelLayout::Stereo,
            ChannelLayout::Mono,
        ).unwrap();
        
        assert_eq!(output.len(), 1);
        // Should be closer to 100 (left) than 200 (right)
        assert!(output[0] < 150);
    }
} 
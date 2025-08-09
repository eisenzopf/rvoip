//! WAV file loader and converter for music-on-hold
//!
//! This module provides functionality to load WAV files and convert them
//! to G.711 µ-law format suitable for RTP transmission during hold.

use std::path::Path;
use crate::error::{Error, Result};
use crate::codec::audio::{G711Codec, AudioCodec};
use crate::types::{AudioFrame, SampleRate};
use hound::{WavReader, WavSpec};
use tracing::{debug, info, warn};

/// Loaded WAV audio data
#[derive(Debug, Clone)]
pub struct WavAudio {
    /// PCM samples (i16 format)
    pub samples: Vec<i16>,
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Number of channels (1 = mono, 2 = stereo)
    pub channels: u16,
}

impl WavAudio {
    /// Get duration in seconds
    pub fn duration_secs(&self) -> f32 {
        self.samples.len() as f32 / (self.sample_rate as f32 * self.channels as f32)
    }
    
    /// Get total number of frames (samples per channel)
    pub fn frame_count(&self) -> usize {
        self.samples.len() / self.channels as usize
    }
}

/// Load a WAV file from disk
pub fn load_wav_file(path: &Path) -> Result<WavAudio> {
    info!("Loading WAV file: {}", path.display());
    
    let reader = WavReader::open(path)
        .map_err(|e| Error::config(format!("Failed to open WAV file: {}", e)))?;
    
    let spec = reader.spec();
    debug!("WAV spec: {} Hz, {} channels, {} bits", 
           spec.sample_rate, spec.channels, spec.bits_per_sample);
    
    // We need 16-bit samples for processing
    if spec.bits_per_sample != 16 {
        return Err(Error::config(format!(
            "Unsupported bit depth: {} (only 16-bit supported)", 
            spec.bits_per_sample
        )));
    }
    
    // Collect samples
    let samples: Vec<i16> = reader.into_samples::<i16>()
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| Error::config(format!("Failed to read WAV samples: {}", e)))?;
    
    info!("Loaded {} samples ({:.1}s) from WAV file", 
          samples.len(), 
          samples.len() as f32 / (spec.sample_rate as f32 * spec.channels as f32));
    
    Ok(WavAudio {
        samples,
        sample_rate: spec.sample_rate,
        channels: spec.channels,
    })
}

/// Convert WAV audio to G.711 µ-law format for RTP transmission
pub fn wav_to_ulaw(wav: &WavAudio) -> Result<Vec<u8>> {
    info!("Converting WAV to G.711 µ-law");
    
    // First, convert to 8kHz mono if needed
    let mono_8khz_samples = if wav.sample_rate != 8000 || wav.channels != 1 {
        resample_to_8khz_mono(wav)?
    } else {
        wav.samples.clone()
    };
    
    // Create G.711 codec
    let mut codec = G711Codec::mu_law(8000, 1)
        .map_err(|e| Error::config(format!("Failed to create G.711 codec: {}", e)))?;
    
    // Encode to µ-law
    let audio_frame = AudioFrame {
        samples: mono_8khz_samples.clone(),
        sample_rate: 8000,
        channels: 1,
        duration: std::time::Duration::from_secs_f32(mono_8khz_samples.len() as f32 / 8000.0),
        timestamp: 0, // timestamp not used for encoding
    };
    
    let encoded = codec.encode(&audio_frame)
        .map_err(|e| Error::config(format!("Failed to encode to µ-law: {}", e)))?;
    
    info!("Converted to {} bytes of µ-law audio", encoded.len());
    Ok(encoded)
}

/// Resample audio to 8kHz mono
fn resample_to_8khz_mono(wav: &WavAudio) -> Result<Vec<i16>> {
    info!("Resampling from {}Hz {} channels to 8kHz mono", 
          wav.sample_rate, wav.channels);
    
    // First, convert to mono if stereo
    let mono_samples = if wav.channels == 2 {
        stereo_to_mono(&wav.samples)
    } else if wav.channels == 1 {
        wav.samples.clone()
    } else {
        return Err(Error::config(format!(
            "Unsupported channel count: {}", wav.channels
        )));
    };
    
    // Then resample to 8kHz if needed
    if wav.sample_rate == 8000 {
        Ok(mono_samples)
    } else {
        simple_resample(&mono_samples, wav.sample_rate, 8000)
    }
}

/// Convert stereo samples to mono by averaging channels
fn stereo_to_mono(stereo_samples: &[i16]) -> Vec<i16> {
    stereo_samples
        .chunks_exact(2)
        .map(|chunk| {
            // Average left and right channels
            ((chunk[0] as i32 + chunk[1] as i32) / 2) as i16
        })
        .collect()
}

/// Simple linear resampling (quality is acceptable for hold music)
fn simple_resample(samples: &[i16], from_rate: u32, to_rate: u32) -> Result<Vec<i16>> {
    let ratio = from_rate as f64 / to_rate as f64;
    let output_len = (samples.len() as f64 / ratio).ceil() as usize;
    let mut output = Vec::with_capacity(output_len);
    
    for i in 0..output_len {
        let src_idx = i as f64 * ratio;
        let idx = src_idx as usize;
        
        if idx + 1 < samples.len() {
            // Linear interpolation between samples
            let frac = src_idx - idx as f64;
            let s1 = samples[idx] as f64;
            let s2 = samples[idx + 1] as f64;
            let interpolated = s1 + (s2 - s1) * frac;
            output.push(interpolated as i16);
        } else if idx < samples.len() {
            // Last sample
            output.push(samples[idx]);
        }
    }
    
    debug!("Resampled from {} to {} samples", samples.len(), output.len());
    Ok(output)
}

/// Load and prepare music-on-hold audio from a file
/// Returns G.711 µ-law encoded audio ready for RTP transmission
pub async fn load_music_on_hold(path: &Path) -> Result<Vec<u8>> {
    // Load WAV file
    let wav = load_wav_file(path)?;
    
    // Convert to µ-law
    wav_to_ulaw(&wav)
}

/// Cache for loaded music-on-hold files to avoid repeated disk I/O
pub struct MusicOnHoldCache {
    cache: std::sync::RwLock<std::collections::HashMap<std::path::PathBuf, Vec<u8>>>,
}

impl MusicOnHoldCache {
    pub fn new() -> Self {
        Self {
            cache: std::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }
    
    /// Get or load music-on-hold audio
    pub async fn get_or_load(&self, path: &Path) -> Result<Vec<u8>> {
        // Check cache first
        {
            let cache = self.cache.read().unwrap();
            if let Some(cached) = cache.get(path) {
                debug!("Using cached MoH audio for: {}", path.display());
                return Ok(cached.clone());
            }
        }
        
        // Load and cache
        let ulaw_audio = load_music_on_hold(path).await?;
        
        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(path.to_path_buf(), ulaw_audio.clone());
            info!("Cached MoH audio for: {}", path.display());
        }
        
        Ok(ulaw_audio)
    }
    
    /// Clear the cache
    pub fn clear(&self) {
        let mut cache = self.cache.write().unwrap();
        cache.clear();
        info!("Cleared MoH cache");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;
    
    #[test]
    fn test_stereo_to_mono() {
        let stereo = vec![100, 200, 300, 400, 500, 600];
        let mono = stereo_to_mono(&stereo);
        assert_eq!(mono, vec![150, 350, 550]); // Average of pairs
    }
    
    #[test]
    fn test_simple_resample_downsample() {
        // 16kHz to 8kHz (2:1 ratio)
        let samples = vec![100, 200, 300, 400, 500, 600, 700, 800];
        let resampled = simple_resample(&samples, 16000, 8000).unwrap();
        assert_eq!(resampled.len(), 4);
    }
    
    #[test]
    fn test_simple_resample_upsample() {
        // 8kHz to 16kHz (1:2 ratio)
        let samples = vec![100, 200, 300, 400];
        let resampled = simple_resample(&samples, 8000, 16000).unwrap();
        assert_eq!(resampled.len(), 8);
    }
    
    #[test]
    fn test_wav_duration() {
        let wav = WavAudio {
            samples: vec![0; 8000], // 1 second at 8kHz mono
            sample_rate: 8000,
            channels: 1,
        };
        assert_eq!(wav.duration_secs(), 1.0);
        
        let wav_stereo = WavAudio {
            samples: vec![0; 16000], // 1 second at 8kHz stereo
            sample_rate: 8000,
            channels: 2,
        };
        assert_eq!(wav_stereo.duration_secs(), 1.0);
    }
}
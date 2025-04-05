use std::sync::Arc;
use bytes::Bytes;
use tracing::{debug, error, info, warn};

use rvoip_media_core::{AudioBuffer, AudioFormat, SampleRate};
use rvoip_media_core::codec::{Codec, G711Codec, G711Variant};
use crate::config::CodecType;
use crate::error::{Error, Result};

/// Codec handler for encoding and decoding audio
pub struct CodecHandler {
    /// Type of codec
    codec_type: CodecType,
    
    /// G.711 codec (if used)
    g711_codec: Option<G711Codec>,
    
    /// Sample rate
    sample_rate: u32,
    
    /// Audio format
    audio_format: AudioFormat,
}

impl CodecHandler {
    /// Create a new codec handler
    pub fn new(codec_type: CodecType) -> Self {
        // Set up codec-specific parameters
        let (sample_rate, g711_codec) = match codec_type {
            CodecType::PCMU => {
                let codec = G711Codec::new(G711Variant::PCMU);
                (8000, Some(codec))
            },
            CodecType::PCMA => {
                let codec = G711Codec::new(G711Variant::PCMA);
                (8000, Some(codec))
            },
            // For other codecs, we would initialize their implementations here
            _ => (8000, None),
        };
        
        // Create audio format based on codec
        let audio_format = AudioFormat::mono_16bit(match sample_rate {
            8000 => SampleRate::Rate8000,
            16000 => SampleRate::Rate16000,
            32000 => SampleRate::Rate32000,
            44100 => SampleRate::Rate44100,
            48000 => SampleRate::Rate48000,
            _ => SampleRate::Rate8000,
        });
        
        Self {
            codec_type,
            g711_codec,
            sample_rate,
            audio_format,
        }
    }
    
    /// Encode audio data
    pub fn encode(&self, audio_data: &[i16]) -> Result<Bytes> {
        // Create audio buffer
        let buffer = AudioBuffer::new(
            Bytes::copy_from_slice(audio_data.as_bytes()),
            self.audio_format,
        );
        
        match self.codec_type {
            CodecType::PCMU | CodecType::PCMA => {
                if let Some(codec) = &self.g711_codec {
                    codec.encode(&buffer)
                        .map_err(|e| Error::Media(format!("G.711 encoding error: {}", e)))
                } else {
                    Err(Error::Media("G.711 codec not initialized".into()))
                }
            },
            _ => Err(Error::Media(format!("Unsupported codec: {:?}", self.codec_type))),
        }
    }
    
    /// Decode audio data
    pub fn decode(&self, encoded_data: &[u8]) -> Result<Vec<i16>> {
        match self.codec_type {
            CodecType::PCMU | CodecType::PCMA => {
                if let Some(codec) = &self.g711_codec {
                    let buffer = codec.decode(encoded_data)
                        .map_err(|e| Error::Media(format!("G.711 decoding error: {}", e)))?;
                    
                    // Convert AudioBuffer to i16 samples
                    let bytes = buffer.raw_data();
                    let mut samples = Vec::with_capacity(bytes.len() / 2);
                    
                    for i in (0..bytes.len()).step_by(2) {
                        if i + 1 < bytes.len() {
                            let sample = i16::from_le_bytes([bytes[i], bytes[i + 1]]);
                            samples.push(sample);
                        }
                    }
                    
                    Ok(samples)
                } else {
                    Err(Error::Media("G.711 codec not initialized".into()))
                }
            },
            _ => Err(Error::Media(format!("Unsupported codec: {:?}", self.codec_type))),
        }
    }
    
    /// Generate silent audio frame of specified duration in milliseconds
    pub fn generate_silence(&self, duration_ms: u32) -> Vec<i16> {
        let sample_count = (self.sample_rate * duration_ms) / 1000;
        vec![0; sample_count as usize]
    }
    
    /// Generate DTMF tone for a digit
    pub fn generate_dtmf(&self, digit: char, duration_ms: u32) -> Vec<i16> {
        // DTMF frequencies (row, column)
        let frequencies = match digit {
            '1' => (697.0, 1209.0),
            '2' => (697.0, 1336.0),
            '3' => (697.0, 1477.0),
            'A' => (697.0, 1633.0),
            '4' => (770.0, 1209.0),
            '5' => (770.0, 1336.0),
            '6' => (770.0, 1477.0),
            'B' => (770.0, 1633.0),
            '7' => (852.0, 1209.0),
            '8' => (852.0, 1336.0),
            '9' => (852.0, 1477.0),
            'C' => (852.0, 1633.0),
            '*' => (941.0, 1209.0),
            '0' => (941.0, 1336.0),
            '#' => (941.0, 1477.0),
            'D' => (941.0, 1633.0),
            _ => return self.generate_silence(duration_ms), // Invalid DTMF digit
        };
        
        let samples = (self.sample_rate * duration_ms) / 1000;
        let mut buffer = Vec::with_capacity(samples as usize);
        
        // Generate sine waves at the two frequencies
        let volume = 0.45; // Prevent clipping when adding two waves
        for i in 0..samples {
            let t = i as f64 / self.sample_rate as f64;
            let row_val = (2.0 * std::f64::consts::PI * frequencies.0 * t).sin() * volume;
            let col_val = (2.0 * std::f64::consts::PI * frequencies.1 * t).sin() * volume;
            let sample = ((row_val + col_val) * 32767.0) as i16;
            buffer.push(sample);
        }
        
        buffer
    }
    
    /// Get the codec type
    pub fn codec_type(&self) -> CodecType {
        self.codec_type
    }
    
    /// Get the sample rate
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    
    /// Get the audio format
    pub fn audio_format(&self) -> AudioFormat {
        self.audio_format
    }
}

/// Extension for easy access to bytes representation of i16 slices
trait AsBytes {
    fn as_bytes(&self) -> &[u8];
}

impl AsBytes for [i16] {
    fn as_bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self.as_ptr() as *const u8,
                self.len() * std::mem::size_of::<i16>(),
            )
        }
    }
} 
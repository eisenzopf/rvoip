use std::io;

#[cfg(feature = "opus")]
use opus::{Decoder, Encoder, Application, Channels, Error as OpusError};

use crate::error::{Error, Result};
use crate::media::codecs::{AudioCodec, CodecParams};

/// Sample rate for Opus codec (typically 48kHz)
pub const OPUS_SAMPLE_RATE: u32 = 48000;

/// Default bitrate for Opus codec (32kbps)
pub const OPUS_DEFAULT_BITRATE: u32 = 32000;

/// Default frame size for Opus (20ms at 48kHz = 960 samples)
pub const OPUS_FRAME_SIZE: usize = 960;

/// Opus codec implementation
#[cfg(feature = "opus")]
pub struct OpusCodec {
    /// Encoder for audio data
    encoder: Encoder,
    
    /// Decoder for audio data
    decoder: Decoder,
    
    /// Sample rate
    sample_rate: u32,
    
    /// Channels
    channels: u8,
    
    /// Frame size in samples
    frame_size: usize,
}

#[cfg(feature = "opus")]
impl OpusCodec {
    /// Create a new Opus codec
    pub fn new(params: CodecParams) -> Result<Self> {
        // Get codec parameters
        let sample_rate = params.sample_rate.unwrap_or(OPUS_SAMPLE_RATE);
        let channels = params.channels.unwrap_or(1);
        let bitrate = params.bitrate.unwrap_or(OPUS_DEFAULT_BITRATE);
        let frame_size = params.frame_size.unwrap_or(OPUS_FRAME_SIZE) as usize;
        
        // Create encoder
        let encoder = Encoder::new(
            sample_rate, 
            if channels == 1 { Channels::Mono } else { Channels::Stereo },
            Application::Voip
        ).map_err(|e| Error::Codec(format!("Failed to create Opus encoder: {}", e)))?;
        
        // Set bitrate
        encoder.set_bitrate(opus::Bitrate::Bits(bitrate as i32))
            .map_err(|e| Error::Codec(format!("Failed to set Opus bitrate: {}", e)))?;
        
        // Create decoder
        let decoder = Decoder::new(
            sample_rate,
            if channels == 1 { Channels::Mono } else { Channels::Stereo }
        ).map_err(|e| Error::Codec(format!("Failed to create Opus decoder: {}", e)))?;
        
        Ok(Self {
            encoder,
            decoder,
            sample_rate,
            channels,
            frame_size,
        })
    }
}

#[cfg(feature = "opus")]
impl AudioCodec for OpusCodec {
    fn encode(&mut self, pcm: &[i16]) -> Result<Vec<u8>> {
        // Allocate buffer for encoded data
        let mut output = vec![0u8; 4000]; // Large enough for any Opus frame
        
        // Encode audio
        let bytes = self.encoder.encode(pcm, &mut output)
            .map_err(|e| Error::Codec(format!("Opus encoding failed: {}", e)))?;
        
        // Resize to actual encoded size
        output.resize(bytes, 0);
        
        Ok(output)
    }
    
    fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        // Calculate output buffer size
        let buffer_size = self.frame_size * self.channels as usize;
        
        // Allocate buffer for decoded data
        let mut output = vec![0i16; buffer_size];
        
        // Decode audio
        let samples = self.decoder.decode(Some(data), &mut output, false)
            .map_err(|e| Error::Codec(format!("Opus decoding failed: {}", e)))?;
        
        // Resize to actual decoded size
        output.resize(samples * self.channels as usize, 0);
        
        Ok(output)
    }
    
    fn get_sample_rate(&self) -> u32 {
        self.sample_rate
    }
    
    fn get_channels(&self) -> u8 {
        self.channels
    }
    
    fn get_frame_size(&self) -> usize {
        self.frame_size
    }
}

/// Stub implementation when Opus feature is not enabled
#[cfg(not(feature = "opus"))]
pub struct OpusCodec;

#[cfg(not(feature = "opus"))]
impl OpusCodec {
    /// Create a new Opus codec
    pub fn new(_params: CodecParams) -> Result<Self> {
        Err(Error::Codec("Opus codec support not enabled".into()))
    }
}

#[cfg(not(feature = "opus"))]
impl AudioCodec for OpusCodec {
    fn encode(&mut self, _pcm: &[i16]) -> Result<Vec<u8>> {
        Err(Error::Codec("Opus codec support not enabled".into()))
    }
    
    fn decode(&mut self, _data: &[u8]) -> Result<Vec<i16>> {
        Err(Error::Codec("Opus codec support not enabled".into()))
    }
    
    fn get_sample_rate(&self) -> u32 {
        48000
    }
    
    fn get_channels(&self) -> u8 {
        1
    }
    
    fn get_frame_size(&self) -> usize {
        960
    }
} 
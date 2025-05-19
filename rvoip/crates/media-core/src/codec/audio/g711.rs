use std::fmt;
use std::sync::Arc;
use bytes::Bytes;

use crate::error::{Error, Result};
use crate::codec::traits::{AudioCodec, Codec, CodecCapability, CodecFactory, CodecFeatures, MediaType};
use crate::codec::audio::common::{AudioCodecParameters, BitrateMode, QualityMode};

/// G.711 µ-law (PCMU) and A-law (PCMA) codec implementation
#[derive(Clone)]
pub struct G711Codec {
    /// Whether to use A-law (true) or µ-law (false)
    alaw: bool,
    /// Current parameters
    params: AudioCodecParameters,
}

impl G711Codec {
    /// Create a new G.711 codec
    pub fn new(alaw: bool) -> Self {
        let bitrate = 64000; // 64 kbps - fixed for G.711
        let params = AudioCodecParameters {
            sample_rate: 8000,
            channels: 1,
            bitrate,
            complexity: 0,
            bitrate_mode: BitrateMode::Constant,
            quality_mode: QualityMode::Voice,
            fec_enabled: false,
            dtx_enabled: false,
            frame_duration_ms: 20,
            packet_loss_percentage: 0.0,
        };

        Self {
            alaw,
            params,
        }
    }
    
    /// Create a new G.711 µ-law (PCMU) codec
    pub fn new_ulaw() -> Self {
        Self::new(false)
    }
    
    /// Create a new G.711 A-law (PCMA) codec
    pub fn new_alaw() -> Self {
        Self::new(true)
    }
    
    /// Convert a 16-bit PCM sample to µ-law
    fn encode_ulaw(sample: i16) -> u8 {
        // Convert 16-bit PCM to µ-law
        // First, convert to 14-bit range
        let mut value = sample;
        
        // Apply bias to get better results around zero
        let bias = 0x84;
        let mut sign = (value >> 8) & 0x80;
        if sign != 0 {
            value = -value;
        }
        value += bias;
        
        // Clamp to 14 bits
        value = value.clamp(0, 0x7FFF);
        
        // Compress using logarithmic quantization
        let mut exponent = 7;
        let mut mantissa;
        let mut segment = 0x80;
        
        if value < segment {
            exponent = 0;
        } else {
            for i in 1..8 {
                segment <<= 1;
                if value < segment {
                    exponent = i;
                    break;
                }
            }
        }
        
        // Bits 1-4 are the mantissa
        if exponent == 0 {
            mantissa = (value >> 3) & 0x0F;
        } else {
            mantissa = (value >> (exponent + 3)) & 0x0F;
        }
        
        // Combine components and invert bits
        let result = !(sign | (exponent << 4) | mantissa);
        
        result as u8
    }
    
    /// Convert a µ-law byte to 16-bit PCM
    fn decode_ulaw(byte: u8) -> i16 {
        // First, invert all bits
        let mut value = !byte;
        
        // Extract sign, exponent, and mantissa
        let sign = (value & 0x80) >> 7;
        let exponent = (value & 0x70) >> 4;
        let mantissa = value & 0x0F;
        
        // Shift mantissa by exponent and add bias
        let mut result = ((mantissa << 3) | 0x84) << exponent;
        
        // Apply sign
        if sign == 1 {
            result = -result;
        }
        
        result
    }
    
    /// Convert a 16-bit PCM sample to A-law
    fn encode_alaw(sample: i16) -> u8 {
        // Convert 16-bit PCM to A-law
        // First, convert to 13-bit range
        let mut value = sample;
        
        let sign = (value >> 8) & 0x80;
        if sign != 0 {
            value = -value;
        }
        
        // Clamp to 13 bits
        value = value.clamp(0, 0x1FFF);
        
        // Compress using logarithmic quantization
        let mut exponent = 7;
        let mut mantissa;
        let mut segment = 0x100;
        
        if value < 16 {
            exponent = 0;
        } else {
            for i in 1..8 {
                segment <<= 1;
                if value < segment {
                    exponent = i;
                    break;
                }
            }
        }
        
        // Bits 1-4 are the mantissa
        if exponent == 0 {
            mantissa = (value >> 1) & 0x0F;
        } else {
            mantissa = (value >> (exponent + 1)) & 0x0F;
        }
        
        // Combine components and invert bits
        let result = (sign | (exponent << 4) | mantissa) ^ 0xD5;
        
        result as u8
    }
    
    /// Convert an A-law byte to 16-bit PCM
    fn decode_alaw(byte: u8) -> i16 {
        // First, invert alternate bits
        let mut value = byte ^ 0xD5;
        
        // Extract sign, exponent, and mantissa
        let sign = (value & 0x80) >> 7;
        let exponent = (value & 0x70) >> 4;
        let mantissa = value & 0x0F;
        
        // Shift mantissa by exponent
        let mut result = (1 << exponent) + ((mantissa << (exponent + 1)) >> 1);
        
        // Scale to 16-bit range
        result <<= 3;
        
        // Apply sign
        if sign == 1 {
            result = -result;
        }
        
        result
    }
}

impl fmt::Debug for G711Codec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("G711Codec")
            .field("type", if self.alaw { &"A-law" } else { &"µ-law" })
            .field("sample_rate", &self.params.sample_rate)
            .field("channels", &self.params.channels)
            .field("bitrate", &self.params.bitrate)
            .finish()
    }
}

impl Codec for G711Codec {
    fn capability(&self) -> CodecCapability {
        let (id, name, mime_type, payload_type) = if self.alaw {
            ("PCMA", "G.711 A-law", "audio/PCMA", Some(8))
        } else {
            ("PCMU", "G.711 µ-law", "audio/PCMU", Some(0))
        };
        
        CodecCapability {
            id: id.to_string(),
            name: name.to_string(),
            parameters: Bytes::new(),
            mime_type: mime_type.to_string(),
            clock_rate: 8000,
            payload_type,
            media_type: MediaType::Audio,
            features: CodecFeatures {
                has_fec: false,
                variable_bitrate: false,
                dtx: false,
                plc: true,
                vad: false,
                flexible_frames: true,
            },
            bandwidth: (64, 64, 64), // Fixed 64 kbps
        }
    }
    
    fn box_clone(&self) -> Box<dyn Codec> {
        Box::new(self.clone())
    }
}

impl AudioCodec for G711Codec {
    fn encode(&self, input: &[i16], output: &mut Bytes) -> Result<usize> {
        let mut buffer = Vec::with_capacity(input.len());
        
        for &sample in input {
            let encoded = if self.alaw {
                Self::encode_alaw(sample)
            } else {
                Self::encode_ulaw(sample)
            };
            
            buffer.push(encoded);
        }
        
        *output = Bytes::from(buffer);
        Ok(input.len())
    }
    
    fn decode(&self, input: &[u8], output: &mut [i16]) -> Result<usize> {
        let len = std::cmp::min(input.len(), output.len());
        
        for i in 0..len {
            output[i] = if self.alaw {
                Self::decode_alaw(input[i])
            } else {
                Self::decode_ulaw(input[i])
            };
        }
        
        Ok(len)
    }
    
    fn sample_rate(&self) -> u32 {
        self.params.sample_rate
    }
    
    fn channels(&self) -> u8 {
        self.params.channels
    }
    
    fn frame_size(&self) -> usize {
        // G.711 is typically used with 20ms frames at 8kHz
        // 8000Hz * 20ms = 160 samples
        (self.params.sample_rate as f32 * (self.params.frame_duration_ms as f32 / 1000.0)) as usize
    }
    
    fn bitrate(&self) -> u32 {
        self.params.bitrate
    }
    
    fn set_parameters(&mut self, params: &AudioCodecParameters) -> Result<()> {
        // G.711 only supports 8kHz sample rate and mono
        if params.sample_rate != 8000 {
            return Err(Error::UnsupportedSampleRate(params.sample_rate));
        }
        
        if params.channels > 1 {
            return Err(Error::UnsupportedChannelCount(params.channels));
        }
        
        self.params = params.clone();
        
        // Force constant bitrate at 64kbps
        self.params.bitrate = 64000;
        self.params.bitrate_mode = BitrateMode::Constant;
        
        Ok(())
    }
    
    fn parameters(&self) -> AudioCodecParameters {
        self.params.clone()
    }
    
    fn set_bitrate_mode(&mut self, _mode: BitrateMode) -> Result<()> {
        // G.711 only supports constant bitrate
        Ok(())
    }
    
    fn set_quality_mode(&mut self, mode: QualityMode) -> Result<()> {
        self.params.quality_mode = mode;
        Ok(())
    }
    
    fn enable_fec(&mut self, _enabled: bool) -> Result<()> {
        // G.711 doesn't support FEC
        Ok(())
    }
    
    fn enable_dtx(&mut self, _enabled: bool) -> Result<()> {
        // G.711 doesn't support DTX
        Ok(())
    }
    
    fn set_complexity(&mut self, _complexity: u8) -> Result<()> {
        // G.711 doesn't have complexity settings
        Ok(())
    }
    
    fn set_packet_loss(&mut self, packet_loss_pct: f32) -> Result<()> {
        self.params.packet_loss_percentage = packet_loss_pct.clamp(0.0, 100.0);
        Ok(())
    }
    
    fn reset(&mut self) -> Result<()> {
        // G.711 is stateless, so reset does nothing
        Ok(())
    }
}

/// Factory for creating G.711 codec instances
#[derive(Debug)]
pub struct G711Factory {
    /// Whether to create A-law (true) or µ-law (false) codecs
    alaw: bool,
}

impl G711Factory {
    /// Create a new G.711 µ-law factory
    pub fn new_ulaw() -> Self {
        Self { alaw: false }
    }
    
    /// Create a new G.711 A-law factory
    pub fn new_alaw() -> Self {
        Self { alaw: true }
    }
}

impl CodecFactory for G711Factory {
    fn id(&self) -> &str {
        if self.alaw { "PCMA" } else { "PCMU" }
    }
    
    fn name(&self) -> &str {
        if self.alaw { "G.711 A-law" } else { "G.711 µ-law" }
    }
    
    fn capabilities(&self) -> Vec<CodecCapability> {
        let codec = if self.alaw {
            G711Codec::new_alaw()
        } else {
            G711Codec::new_ulaw()
        };
        
        vec![codec.capability()]
    }
    
    fn create_default(&self) -> Result<Box<dyn Codec>> {
        let codec = if self.alaw {
            G711Codec::new_alaw()
        } else {
            G711Codec::new_ulaw()
        };
        
        Ok(Box::new(codec))
    }
    
    fn create_with_params(&self, _params: &[u8]) -> Result<Box<dyn Codec>> {
        // G.711 doesn't have configurable parameters via SDP
        self.create_default()
    }
} 
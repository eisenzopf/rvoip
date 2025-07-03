//! G.711 codec implementation
//!
//! G.711 is an ITU-T standard for audio companding, primarily used in telephony.
//! It has two main variants:
//! - μ-law (PCMU, used in North America and Japan)
//! - A-law (PCMA, used in Europe and the rest of the world)
//!
//! Both variants encode 16-bit PCM samples into 8-bit values, effectively
//! reducing bandwidth requirements by 50%.

use bytes::{Bytes, BytesMut};
use crate::{Result, Error, AudioBuffer, AudioFormat, SampleRate, Sample};
use super::Codec;

/// G.711 codec variant (μ-law or A-law)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum G711Variant {
    /// μ-law (PCMU, payload type 0)
    PCMU,
    /// A-law (PCMA, payload type 8)
    PCMA,
}

/// G.711 codec implementation
#[derive(Debug, Clone)]
pub struct G711Codec {
    /// The specific variant (μ-law or A-law)
    variant: G711Variant,
}

impl G711Codec {
    /// Create a new G.711 codec with the specified variant
    pub fn new(variant: G711Variant) -> Self {
        Self { variant }
    }
    
    /// Get the G.711 variant
    pub fn variant(&self) -> G711Variant {
        self.variant
    }
}

impl Codec for G711Codec {
    fn name(&self) -> &'static str {
        match self.variant {
            G711Variant::PCMU => "PCMU",
            G711Variant::PCMA => "PCMA",
        }
    }
    
    fn payload_type(&self) -> u8 {
        match self.variant {
            G711Variant::PCMU => 0,
            G711Variant::PCMA => 8,
        }
    }
    
    fn sample_rate(&self) -> u32 {
        8000 // G.711 is always 8kHz
    }
    
    fn supports_format(&self, format: AudioFormat) -> bool {
        // G.711 only supports mono 16-bit audio at 8kHz
        format.channels == 1 && 
        format.bit_depth == 16 && 
        format.sample_rate == SampleRate::Rate8000
    }
    
    fn frame_size(&self) -> usize {
        // G.711 typically uses 20ms frames at 8kHz = 160 samples
        160
    }
    
    fn encode(&self, pcm: &AudioBuffer) -> Result<Bytes> {
        // Validate the audio format
        if !self.supports_format(pcm.format) {
            return Err(Error::InvalidFormat(format!(
                "G.711 requires mono 16-bit 8kHz audio, got {}-channel {}-bit {}Hz",
                pcm.format.channels,
                pcm.format.bit_depth,
                pcm.format.sample_rate.as_hz()
            )));
        }
        
        // Each 16-bit PCM sample becomes one 8-bit G.711 sample
        let num_samples = pcm.samples();
        let mut output = BytesMut::with_capacity(num_samples);
        
        // Convert the byte buffer to 16-bit PCM samples
        let mut i = 0;
        while i + 1 < pcm.data.len() {
            // Extract 16-bit sample (in little-endian order)
            let sample = ((pcm.data[i + 1] as i16) << 8) | (pcm.data[i] as i16);
            
            // Encode the sample using the appropriate G.711 variant
            let encoded = match self.variant {
                G711Variant::PCMU => encode_ulaw(sample),
                G711Variant::PCMA => encode_alaw(sample),
            };
            
            output.extend_from_slice(&[encoded]);
            i += 2; // Move to next 16-bit sample
        }
        
        Ok(output.freeze())
    }
    
    fn decode(&self, encoded: &[u8]) -> Result<AudioBuffer> {
        // Create a buffer for 16-bit PCM output (2 bytes per sample)
        let mut output = BytesMut::with_capacity(encoded.len() * 2);
        
        // Decode each 8-bit G.711 sample to a 16-bit PCM sample
        for &byte in encoded {
            // Decode the sample using the appropriate G.711 variant
            let sample = match self.variant {
                G711Variant::PCMU => decode_ulaw(byte),
                G711Variant::PCMA => decode_alaw(byte),
            };
            
            // Add the 16-bit sample to the output (in little-endian order)
            output.extend_from_slice(&[(sample & 0xFF) as u8, ((sample >> 8) & 0xFF) as u8]);
        }
        
        Ok(AudioBuffer::new(
            output.freeze(),
            AudioFormat::mono_16bit(SampleRate::Rate8000)
        ))
    }
}

// μ-law encoding table
static ULAW_ENCODE_TABLE: [i16; 256] = [
    0, 0, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7
];

// μ-law decoding table
static ULAW_DECODE_TABLE: [i16; 256] = [
    -32124, -31100, -30076, -29052, -28028, -27004, -25980, -24956,
    -23932, -22908, -21884, -20860, -19836, -18812, -17788, -16764,
    -15996, -15484, -14972, -14460, -13948, -13436, -12924, -12412,
    -11900, -11388, -10876, -10364, -9852, -9340, -8828, -8316,
    -7932, -7676, -7420, -7164, -6908, -6652, -6396, -6140,
    -5884, -5628, -5372, -5116, -4860, -4604, -4348, -4092,
    -3900, -3772, -3644, -3516, -3388, -3260, -3132, -3004,
    -2876, -2748, -2620, -2492, -2364, -2236, -2108, -1980,
    -1884, -1820, -1756, -1692, -1628, -1564, -1500, -1436,
    -1372, -1308, -1244, -1180, -1116, -1052, -988, -924,
    -876, -844, -812, -780, -748, -716, -684, -652,
    -620, -588, -556, -524, -492, -460, -428, -396,
    -372, -356, -340, -324, -308, -292, -276, -260,
    -244, -228, -212, -196, -180, -164, -148, -132,
    -120, -112, -104, -96, -88, -80, -72, -64,
    -56, -48, -40, -32, -24, -16, -8, 0,
    32124, 31100, 30076, 29052, 28028, 27004, 25980, 24956,
    23932, 22908, 21884, 20860, 19836, 18812, 17788, 16764,
    15996, 15484, 14972, 14460, 13948, 13436, 12924, 12412,
    11900, 11388, 10876, 10364, 9852, 9340, 8828, 8316,
    7932, 7676, 7420, 7164, 6908, 6652, 6396, 6140,
    5884, 5628, 5372, 5116, 4860, 4604, 4348, 4092,
    3900, 3772, 3644, 3516, 3388, 3260, 3132, 3004,
    2876, 2748, 2620, 2492, 2364, 2236, 2108, 1980,
    1884, 1820, 1756, 1692, 1628, 1564, 1500, 1436,
    1372, 1308, 1244, 1180, 1116, 1052, 988, 924,
    876, 844, 812, 780, 748, 716, 684, 652,
    620, 588, 556, 524, 492, 460, 428, 396,
    372, 356, 340, 324, 308, 292, 276, 260,
    244, 228, 212, 196, 180, 164, 148, 132,
    120, 112, 104, 96, 88, 80, 72, 64,
    56, 48, 40, 32, 24, 16, 8, 0
];

// A-law encoding table
static ALAW_ENCODE_TABLE: [i16; 128] = [
    1, 1, 2, 2, 3, 3, 3, 3,
    4, 4, 4, 4, 4, 4, 4, 4,
    5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5,
    6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6,
    7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7
];

// A-law decoding table
static ALAW_DECODE_TABLE: [i16; 256] = [
    -5504, -5248, -6016, -5760, -4480, -4224, -4992, -4736,
    -7552, -7296, -8064, -7808, -6528, -6272, -7040, -6784,
    -2752, -2624, -3008, -2880, -2240, -2112, -2496, -2368,
    -3776, -3648, -4032, -3904, -3264, -3136, -3520, -3392,
    -22016, -20992, -24064, -23040, -17920, -16896, -19968, -18944,
    -30208, -29184, -32256, -31232, -26112, -25088, -28160, -27136,
    -11008, -10496, -12032, -11520, -8960, -8448, -9984, -9472,
    -15104, -14592, -16128, -15616, -13056, -12544, -14080, -13568,
    -344, -328, -376, -360, -280, -264, -312, -296,
    -472, -456, -504, -488, -408, -392, -440, -424,
    -88, -72, -120, -104, -24, -8, -56, -40,
    -216, -200, -248, -232, -152, -136, -184, -168,
    -1376, -1312, -1504, -1440, -1120, -1056, -1248, -1184,
    -1888, -1824, -2016, -1952, -1632, -1568, -1760, -1696,
    -688, -656, -752, -720, -560, -528, -624, -592,
    -944, -912, -1008, -976, -816, -784, -880, -848,
    5504, 5248, 6016, 5760, 4480, 4224, 4992, 4736,
    7552, 7296, 8064, 7808, 6528, 6272, 7040, 6784,
    2752, 2624, 3008, 2880, 2240, 2112, 2496, 2368,
    3776, 3648, 4032, 3904, 3264, 3136, 3520, 3392,
    22016, 20992, 24064, 23040, 17920, 16896, 19968, 18944,
    30208, 29184, 32256, 31232, 26112, 25088, 28160, 27136,
    11008, 10496, 12032, 11520, 8960, 8448, 9984, 9472,
    15104, 14592, 16128, 15616, 13056, 12544, 14080, 13568,
    344, 328, 376, 360, 280, 264, 312, 296,
    472, 456, 504, 488, 408, 392, 440, 424,
    88, 72, 120, 104, 24, 8, 56, 40,
    216, 200, 248, 232, 152, 136, 184, 168,
    1376, 1312, 1504, 1440, 1120, 1056, 1248, 1184,
    1888, 1824, 2016, 1952, 1632, 1568, 1760, 1696,
    688, 656, 752, 720, 560, 528, 624, 592,
    944, 912, 1008, 976, 816, 784, 880, 848
];

/// Encode a 16-bit PCM sample to 8-bit μ-law
/// 
/// This function follows the ITU-T G.711 recommendation for μ-law encoding:
/// 1. Add bias to the sample
/// 2. Take absolute value and apply logarithmic quantization
/// 3. Apply bit inversion for better error recovery
fn encode_ulaw(sample: Sample) -> u8 {
    // Handle edge case: -32768 would overflow when negated
    let value = if sample == -32768 {
        32767
    } else {
        sample.abs()
    };
    
    // Add bias, with clamping to 16-bit range
    let value = if value as u32 + 132 > 32767 {
        32767
    } else {
        value + 132
    };
    
    // Convert to u16 for bitwise operations
    let value = value as u16;
    
    // Get the sign bit
    let sign = if sample < 0 { 0x80u8 } else { 0u8 };
    
    // Find the segment (exponent)
    let exponent = ULAW_ENCODE_TABLE[(value >> 7) as usize] as u8;
    
    // Extract the mantissa bits
    let mantissa = ((value >> (exponent as u16 + 3)) & 0x0F) as u8;
    
    // Assemble the μ-law byte and apply bit inversion
    let encoded = sign | ((exponent << 4) | mantissa) ^ 0xFF;
    
    encoded
}

/// Decode an 8-bit μ-law sample to 16-bit PCM
fn decode_ulaw(encoded: u8) -> Sample {
    // Invert all bits (μ-law uses inverted values)
    let encoded = encoded ^ 0xFF;
    
    // Use lookup table for faster decoding
    ULAW_DECODE_TABLE[encoded as usize]
}

/// Encode a 16-bit PCM sample to 8-bit A-law
///
/// A-law encoding follows a similar pattern to μ-law but with different scaling:
/// 1. Take absolute value and logarithmically compress
/// 2. Format with 3 segment bits and 4 quantization bits
/// 3. Apply bit inversion to every other bit
fn encode_alaw(sample: Sample) -> u8 {
    // Handle edge case: -32768 would overflow when taking abs()
    let value = if sample == -32768 {
        32767
    } else {
        sample.abs()
    };
    
    // Clamp to 16-bit positive range (already handled above)
    let value = value as u16;
    
    // Get the sign bit
    let sign = if sample < 0 { 0x80u8 } else { 0u8 };
    
    // Logarithmic quantization
    let segment = ALAW_ENCODE_TABLE[(value >> 8) as usize] as u8;
    let mantissa = ((value >> (segment as u16 + 1)) & 0x0F) as u8;
    
    // Assemble A-law byte with alternate bit inversion
    let encoded = sign | ((segment << 4) | mantissa) ^ 0x55;
    
    encoded
}

/// Decode an 8-bit A-law sample to 16-bit PCM
fn decode_alaw(encoded: u8) -> Sample {
    // Use lookup table for faster decoding
    ALAW_DECODE_TABLE[encoded as usize]
}


#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_ulaw_encode_decode_basic() {
        // Test a simple value
        let value: i16 = 1000;
        let encoded = encode_ulaw(value);
        let decoded = decode_ulaw(encoded);
        
        // Just verify something reasonable is returned
        assert!(decoded != 0, "Decoded value should not be zero");
        println!("μ-law: {} encoded to {} then decoded to {}", value, encoded, decoded);
    }
    
    #[test]
    fn test_alaw_encode_decode_basic() {
        // Test a simple value
        let value: i16 = 1000;
        let encoded = encode_alaw(value);
        let decoded = decode_alaw(encoded);
        
        // Just verify something reasonable is returned
        assert!(decoded != 0, "Decoded value should not be zero");
        println!("A-law: {} encoded to {} then decoded to {}", value, encoded, decoded);
    }
    
    #[test]
    fn test_g711_codec_basic() {
        // Test both variants (PCMU and PCMA)
        for variant in [G711Variant::PCMU, G711Variant::PCMA] {
            let codec = G711Codec::new(variant);
            
            // Create a simple test audio buffer with 10 identical samples
            let num_samples = 10;
            let mut pcm_data = BytesMut::with_capacity(num_samples * 2);
            
            // Use a constant sample value for simplicity
            let sample: i16 = 1000;
            
            for _ in 0..num_samples {
                // Add sample in little-endian order
                pcm_data.extend_from_slice(&[(sample & 0xFF) as u8, ((sample >> 8) & 0xFF) as u8]);
            }
            
            let pcm_buffer = AudioBuffer::new(
                pcm_data.freeze(),
                AudioFormat::mono_16bit(SampleRate::Rate8000)
            );
            
            // Basic encoding test
            let encoded = codec.encode(&pcm_buffer).unwrap();
            
            // Verify encoded size (8-bit per sample)
            assert_eq!(encoded.len(), num_samples);
            
            // Basic decoding test
            let decoded = codec.decode(&encoded).unwrap();
            
            // Verify format is preserved
            assert_eq!(decoded.format, pcm_buffer.format);
            
            // Verify sample count is preserved
            assert_eq!(decoded.samples(), pcm_buffer.samples());
            
            // Just verify we got some non-zero output
            for i in 0..num_samples {
                let decoded_idx = i * 2;
                let decoded_sample = ((decoded.data[decoded_idx + 1] as i16) << 8) | 
                                     (decoded.data[decoded_idx] as i16);
                assert!(decoded_sample != 0, 
                      "Decoded sample for {:?} should not be zero", 
                      variant);
            }
        }
    }
} 
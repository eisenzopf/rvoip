use crate::types::{AudioFrame, AudioFormat};
use crate::error::AudioError;
use super::{CodecType, AudioCodecTrait, CodecConfig};

/// G.722 wideband audio codec implementation
/// 
/// G.722 uses sub-band coding to encode 16kHz audio at 64kbps
/// It splits the signal into high and low frequency bands
pub struct G722Encoder {
    config: CodecConfig,
    encoder_state: G722EncoderState,
    decoder_state: G722DecoderState,
}

/// G.722 encoder state
#[derive(Debug, Clone)]
struct G722EncoderState {
    // Low-band encoder state
    low_band_encoder: AdpcmEncoder,
    // High-band encoder state  
    high_band_encoder: AdpcmEncoder,
    // QMF analysis filter state
    qmf_state: [i32; 24],
    // Input buffer for QMF processing
    input_buffer: [i16; 2],
    buffer_index: usize,
}

/// G.722 decoder state
#[derive(Debug, Clone)]
struct G722DecoderState {
    // Low-band decoder state
    low_band_decoder: AdpcmDecoder,
    // High-band decoder state
    high_band_decoder: AdpcmDecoder,
    // QMF synthesis filter state
    qmf_state: [i32; 24],
    // Output buffer for QMF processing
    output_buffer: [i16; 2],
    buffer_index: usize,
}

/// ADPCM encoder state for each sub-band
#[derive(Debug, Clone)]
struct AdpcmEncoder {
    s: i32,        // Signal estimate
    sp: i32,       // Slow part of signal estimate
    sz: i32,       // Fast part of signal estimate
    r: [i32; 3],   // Delay line
    a: [i32; 3],   // Predictor coefficients
    b: [i32; 7],   // Predictor coefficients
    p: [i32; 7],   // Delay line
    d: [i32; 7],   // Quantized difference signal
    nb: i32,       // Scale factor
    det: i32,      // Quantizer scale factor
}

/// ADPCM decoder state for each sub-band
#[derive(Debug, Clone)]
struct AdpcmDecoder {
    s: i32,        // Signal estimate
    sp: i32,       // Slow part of signal estimate
    sz: i32,       // Fast part of signal estimate
    r: [i32; 3],   // Delay line
    a: [i32; 3],   // Predictor coefficients
    b: [i32; 7],   // Predictor coefficients
    p: [i32; 7],   // Delay line
    d: [i32; 7],   // Quantized difference signal
    nb: i32,       // Scale factor
    det: i32,      // Quantizer scale factor
}

impl G722Encoder {
    /// Create a new G.722 encoder
    pub fn new(config: CodecConfig) -> Result<Self, AudioError> {
        // Validate configuration
        if config.sample_rate != 16000 {
            return Err(AudioError::invalid_configuration(
                format!("G.722 only supports 16kHz sample rate, got {}", config.sample_rate)
            ));
        }
        
        if config.channels != 1 {
            return Err(AudioError::invalid_configuration(
                format!("G.722 only supports mono audio, got {} channels", config.channels)
            ));
        }

        Ok(Self {
            config,
            encoder_state: G722EncoderState::new(),
            decoder_state: G722DecoderState::new(),
        })
    }
}

impl AudioCodecTrait for G722Encoder {
    fn encode(&mut self, frame: &AudioFrame) -> Result<Vec<u8>, AudioError> {
        // Ensure frame format matches codec requirements
        if frame.format.sample_rate != self.config.sample_rate {
            return Err(AudioError::invalid_format(
                format!("Frame sample rate {} doesn't match codec sample rate {}", 
                       frame.format.sample_rate, self.config.sample_rate)
            ));
        }

        if frame.format.channels != self.config.channels as u16 {
            return Err(AudioError::invalid_format(
                format!("Frame channels {} doesn't match codec channels {}", 
                       frame.format.channels, self.config.channels)
            ));
        }

        // Use i16 samples directly
        let pcm_samples = &frame.samples;

        // Encode samples
        let mut encoded = Vec::with_capacity(pcm_samples.len() / 2);
        
        for chunk in pcm_samples.chunks(2) {
            let input = if chunk.len() == 2 {
                [chunk[0], chunk[1]]
            } else {
                [chunk[0], 0] // Pad with zero if odd number of samples
            };
            
            let encoded_byte = self.encoder_state.encode_sample(input);
            encoded.push(encoded_byte);
        }

        Ok(encoded)
    }

    fn decode(&mut self, data: &[u8]) -> Result<AudioFrame, AudioError> {
        let mut samples = Vec::with_capacity(data.len() * 2);
        
        for &byte in data {
            let decoded_samples = self.decoder_state.decode_sample(byte);
            // Use i16 samples directly
            samples.push(decoded_samples[0]);
            samples.push(decoded_samples[1]);
        }

        Ok(AudioFrame {
            samples,
            format: AudioFormat {
                sample_rate: self.config.sample_rate,
                channels: self.config.channels as u16,
                bits_per_sample: 16,
                frame_size_ms: 20,
            },
            timestamp: 0,
            sequence: 0,
            metadata: std::collections::HashMap::new(),
        })
    }

    fn config(&self) -> &CodecConfig {
        &self.config
    }

    fn reset(&mut self) -> Result<(), AudioError> {
        self.encoder_state = G722EncoderState::new();
        self.decoder_state = G722DecoderState::new();
        Ok(())
    }

    fn codec_type(&self) -> CodecType {
        CodecType::G722
    }
}

impl G722EncoderState {
    fn new() -> Self {
        Self {
            low_band_encoder: AdpcmEncoder::new(),
            high_band_encoder: AdpcmEncoder::new(),
            qmf_state: [0; 24],
            input_buffer: [0; 2],
            buffer_index: 0,
        }
    }

    fn encode_sample(&mut self, input: [i16; 2]) -> u8 {
        // Store input samples
        self.input_buffer = input;
        
        // QMF analysis - split into high and low frequency bands
        let (low_band, high_band) = self.qmf_analysis();
        
        // Encode each band using ADPCM
        let low_bits = self.low_band_encoder.encode(low_band);
        let high_bits = self.high_band_encoder.encode(high_band);
        
        // Combine bits: 6 bits for low band + 2 bits for high band
        ((low_bits & 0x3F) | ((high_bits & 0x03) << 6)) as u8
    }

    fn qmf_analysis(&mut self) -> (i32, i32) {
        // Simplified QMF analysis
        // In a real implementation, this would use proper QMF filter coefficients
        let low_band = (self.input_buffer[0] as i32 + self.input_buffer[1] as i32) / 2;
        let high_band = (self.input_buffer[0] as i32 - self.input_buffer[1] as i32) / 2;
        
        (low_band, high_band)
    }
}

impl G722DecoderState {
    fn new() -> Self {
        Self {
            low_band_decoder: AdpcmDecoder::new(),
            high_band_decoder: AdpcmDecoder::new(),
            qmf_state: [0; 24],
            output_buffer: [0; 2],
            buffer_index: 0,
        }
    }

    fn decode_sample(&mut self, input: u8) -> [i16; 2] {
        // Extract bits: 6 bits for low band + 2 bits for high band
        let low_bits = (input & 0x3F) as i32;
        let high_bits = ((input >> 6) & 0x03) as i32;
        
        // Decode each band using ADPCM
        let low_band = self.low_band_decoder.decode(low_bits);
        let high_band = self.high_band_decoder.decode(high_bits);
        
        // QMF synthesis - combine bands back to time domain
        self.qmf_synthesis(low_band, high_band)
    }

    fn qmf_synthesis(&mut self, low_band: i32, high_band: i32) -> [i16; 2] {
        // Simplified QMF synthesis
        // In a real implementation, this would use proper QMF filter coefficients
        let sample1 = ((low_band + high_band) / 2).clamp(-32768, 32767) as i16;
        let sample2 = ((low_band - high_band) / 2).clamp(-32768, 32767) as i16;
        
        [sample1, sample2]
    }
}

impl AdpcmEncoder {
    fn new() -> Self {
        Self {
            s: 0, sp: 0, sz: 0,
            r: [0; 3], a: [0; 3], b: [0; 7], p: [0; 7], d: [0; 7],
            nb: 0, det: 32,
        }
    }

    fn encode(&mut self, input: i32) -> i32 {
        // Simplified ADPCM encoding
        // This is a basic implementation - real G.722 would use more complex algorithms
        
        // Prediction
        let predicted = self.predict();
        
        // Difference
        let diff = input - predicted;
        
        // Quantization
        let quantized = self.quantize(diff);
        
        // Update state
        self.update_encoder(quantized, input);
        
        quantized
    }

    fn predict(&self) -> i32 {
        // Simple prediction based on previous samples
        self.s
    }

    fn quantize(&self, diff: i32) -> i32 {
        // Simple uniform quantization
        let scale = self.det;
        let quantized = (diff * 4) / scale;
        quantized.clamp(-8, 7)
    }

    fn update_encoder(&mut self, quantized: i32, input: i32) {
        // Update predictor state
        self.s = input;
        
        // Update scale factor
        self.det = (self.det * 15 + 8) / 16;
        if self.det < 1 {
            self.det = 1;
        }
    }
}

impl AdpcmDecoder {
    fn new() -> Self {
        Self {
            s: 0, sp: 0, sz: 0,
            r: [0; 3], a: [0; 3], b: [0; 7], p: [0; 7], d: [0; 7],
            nb: 0, det: 32,
        }
    }

    fn decode(&mut self, quantized: i32) -> i32 {
        // Simplified ADPCM decoding
        
        // Prediction
        let predicted = self.predict();
        
        // Dequantization
        let diff = self.dequantize(quantized);
        
        // Reconstruction
        let reconstructed = predicted + diff;
        
        // Update state
        self.update_decoder(quantized, reconstructed);
        
        reconstructed
    }

    fn predict(&self) -> i32 {
        // Simple prediction based on previous samples
        self.s
    }

    fn dequantize(&self, quantized: i32) -> i32 {
        // Simple uniform dequantization
        let scale = self.det;
        (quantized * scale) / 4
    }

    fn update_decoder(&mut self, quantized: i32, reconstructed: i32) {
        // Update predictor state
        self.s = reconstructed;
        
        // Update scale factor
        self.det = (self.det * 15 + 8) / 16;
        if self.det < 1 {
            self.det = 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_g722_encoder_creation() {
        let config = CodecConfig {
            codec: CodecType::G722,
            sample_rate: 16000,
            channels: 1,
            bitrate: 64000,
            params: std::collections::HashMap::new(),
        };
        
        let encoder = G722Encoder::new(config);
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_invalid_sample_rate() {
        let config = CodecConfig {
            codec: CodecType::G722,
            sample_rate: 8000, // Invalid for G.722
            channels: 1,
            bitrate: 64000,
            params: std::collections::HashMap::new(),
        };
        
        let encoder = G722Encoder::new(config);
        assert!(encoder.is_err());
    }

    #[test]
    fn test_g722_encoding_decoding() {
        let config = CodecConfig {
            codec: CodecType::G722,
            sample_rate: 16000,
            channels: 1,
            bitrate: 64000,
            params: std::collections::HashMap::new(),
        };
        
        let mut encoder = G722Encoder::new(config).unwrap();
        
        // Test with sine wave samples (even number for proper chunking)
        let samples = vec![0, 16384, 32767, 16384, 0, -16384, -32767, -16384];
        let frame = AudioFrame {
            samples: samples.clone(),
            format: AudioFormat {
                sample_rate: 16000,
                channels: 1,
                bits_per_sample: 16,
                frame_size_ms: 20,
            },
            timestamp: 0,
            sequence: 0,
            metadata: std::collections::HashMap::new(),
        };
        
        // Encode
        let encoded = encoder.encode(&frame).unwrap();
        assert_eq!(encoded.len(), samples.len() / 2); // G.722 compresses 2:1
        
        // Decode
        let decoded_frame = encoder.decode(&encoded).unwrap();
        assert_eq!(decoded_frame.samples.len(), samples.len());
        
        // Check that decoding produces reasonable output
        // G.722 is lossy, so we expect some distortion
        for (original, decoded) in samples.iter().zip(decoded_frame.samples.iter()) {
            let error = (original - decoded).abs();
            assert!(error < 8000, "Error too large: {} vs {}", original, decoded);
        }
    }

    #[test]
    fn test_codec_reset() {
        let config = CodecConfig {
            codec: CodecType::G722,
            sample_rate: 16000,
            channels: 1,
            bitrate: 64000,
            params: std::collections::HashMap::new(),
        };
        
        let mut encoder = G722Encoder::new(config).unwrap();
        assert!(encoder.reset().is_ok());
    }

    #[test]
    fn test_codec_type() {
        let config = CodecConfig {
            codec: CodecType::G722,
            sample_rate: 16000,
            channels: 1,
            bitrate: 64000,
            params: std::collections::HashMap::new(),
        };
        
        let encoder = G722Encoder::new(config).unwrap();
        assert_eq!(encoder.codec_type(), CodecType::G722);
    }

    #[test]
    fn test_adpcm_encoder_decoder() {
        let mut encoder = AdpcmEncoder::new();
        let mut decoder = AdpcmDecoder::new();
        
        let test_samples = vec![0, 1000, 2000, 1000, 0, -1000, -2000, -1000];
        
        for sample in test_samples {
            let encoded = encoder.encode(sample);
            let decoded = decoder.decode(encoded);
            
            // ADPCM is lossy, so we expect some error
            let error = (sample - decoded).abs();
            assert!(error < sample.abs() / 2 + 1000);
        }
    }
} 
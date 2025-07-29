//! Integration tests for G.729A codec with ITU-T reference tables

#[cfg(test)]
mod tests {
    use crate::codecs::g729a::{G729AEncoder, G729ADecoder, AudioFrame};
    use crate::codecs::g729a::types::Q15;
    
    #[test]
    fn test_codec_round_trip() {
        let mut encoder = G729AEncoder::new();
        let mut decoder = G729ADecoder::new();
        
        // Create a test frame with a simple sine wave
        let mut samples = [0i16; 80];
        for i in 0..80 {
            let phase = 2.0 * std::f32::consts::PI * i as f32 / 40.0;
            samples[i] = (0.3 * phase.sin() * 32767.0) as i16;
        }
        
        let input_frame = AudioFrame {
            samples,
            timestamp: 0,
        };
        
        // Create lookahead samples (zeros for simplicity)
        let lookahead = [0i16; 40];
        
        // Encode the frame
        let encoded = encoder.encode_frame_with_lookahead(&input_frame, &lookahead)
            .expect("Encoding failed");
        
        // Verify encoded frame is 10 bytes (80 bits)
        assert_eq!(encoded.len(), 10);
        
        // Decode the frame
        let decoded = decoder.decode_frame(&encoded)
            .expect("Decoding failed");
        
        // Check output frame size
        assert_eq!(decoded.samples.len(), 80);
        
        // The output won't be identical due to compression, but should have similar energy
        let input_energy: i64 = samples.iter().map(|&x| x as i64 * x as i64).sum();
        let output_energy: i64 = decoded.samples.iter().map(|&x| x as i64 * x as i64).sum();
        
        // Energy should be within reasonable bounds (50% to 150%)
        let energy_ratio = output_energy as f64 / input_energy as f64;
        assert!(energy_ratio > 0.5 && energy_ratio < 1.5,
            "Energy ratio {} is out of bounds", energy_ratio);
    }
    
    #[test]
    fn test_silence_encoding() {
        let mut encoder = G729AEncoder::new();
        let mut decoder = G729ADecoder::new();
        
        // Create a silent frame
        let input_frame = AudioFrame {
            samples: [0i16; 80],
            timestamp: 0,
        };
        
        let lookahead = [0i16; 40];
        
        // Encode
        let encoded = encoder.encode_frame_with_lookahead(&input_frame, &lookahead)
            .expect("Encoding failed");
        
        // Decode
        let decoded = decoder.decode_frame(&encoded)
            .expect("Decoding failed");
        
        // Output should be mostly silence (allowing for some codec noise)
        let max_sample = decoded.samples.iter().map(|&x| x.saturating_abs()).max().unwrap();
        assert!(max_sample < 1000, "Silent input produced loud output: {}", max_sample);
    }
    
    #[test]
    fn test_multiple_frames() {
        let mut encoder = G729AEncoder::new();
        let mut decoder = G729ADecoder::new();
        
        // Process multiple frames to test state continuity
        for frame_idx in 0..10 {
            let mut samples = [0i16; 80];
            
            // Different frequency for each frame
            let freq = 440.0 + (frame_idx as f32 * 50.0);
            for i in 0..80 {
                let phase = 2.0 * std::f32::consts::PI * freq * i as f32 / 8000.0;
                samples[i] = (0.2 * phase.sin() * 32767.0) as i16;
            }
            
            let input_frame = AudioFrame {
                samples,
                timestamp: frame_idx as u64 * 80,
            };
            
            let lookahead = [0i16; 40];
            
            // Encode and decode
            let encoded = encoder.encode_frame_with_lookahead(&input_frame, &lookahead)
                .expect("Encoding failed");
            let decoded = decoder.decode_frame(&encoded)
                .expect("Decoding failed");
            
            // Basic sanity check
            assert_eq!(decoded.samples.len(), 80);
        }
    }
    
    #[test]
    fn test_error_concealment() {
        let mut encoder = G729AEncoder::new();
        let mut decoder = G729ADecoder::new();
        
        // First, encode and decode a normal frame
        let mut samples = [0i16; 80];
        for i in 0..80 {
            samples[i] = ((i as f32 / 80.0 - 0.5) * 16384.0) as i16;
        }
        
        let input_frame = AudioFrame {
            samples,
            timestamp: 0,
        };
        
        let lookahead = [0i16; 40];
        let encoded = encoder.encode_frame_with_lookahead(&input_frame, &lookahead)
            .expect("Encoding failed");
        let _ = decoder.decode_frame(&encoded).expect("Decoding failed");
        
        // Now test error concealment
        let concealed = decoder.decode_frame_with_concealment(None)
            .expect("Concealment failed");
        
        // Should produce some output
        assert_eq!(concealed.samples.len(), 80);
        
        // Energy should be attenuated but not zero
        let energy: i64 = concealed.samples.iter().map(|&x| x as i64 * x as i64).sum();
        assert!(energy > 0, "Concealed frame has zero energy");
    }
    
    #[test]
    fn test_codec_reset() {
        let mut encoder = G729AEncoder::new();
        let mut decoder = G729ADecoder::new();
        
        // Process a frame
        let input_frame = AudioFrame {
            samples: [1000i16; 80],
            timestamp: 0,
        };
        let lookahead = [0i16; 40];
        
        let encoded = encoder.encode_frame_with_lookahead(&input_frame, &lookahead)
            .expect("Encoding failed");
        let _ = decoder.decode_frame(&encoded).expect("Decoding failed");
        
        // Reset
        encoder.reset();
        decoder.reset();
        
        // Process silence - should work correctly after reset
        let silent_frame = AudioFrame {
            samples: [0i16; 80],
            timestamp: 80,
        };
        
        let encoded = encoder.encode_frame_with_lookahead(&silent_frame, &lookahead)
            .expect("Encoding after reset failed");
        let decoded = decoder.decode_frame(&encoded)
            .expect("Decoding after reset failed");
        
        // Should produce low energy output
        let max_sample = decoded.samples.iter().map(|&x| x.abs()).max().unwrap();
        assert!(max_sample < 2000, "Reset didn't clear state properly");
    }
} 
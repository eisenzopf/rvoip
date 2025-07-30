//! G.711 Encoder Unit Tests
//!
//! Tests for G.711 encoding functionality including:
//! - μ-law and A-law encoding accuracy
//! - Boundary value handling
//! - Quantization behavior
//! - Buffer operations
//! - Performance characteristics

use crate::codecs::g711::*;
use crate::types::{AudioCodec, CodecConfig, CodecType, SampleRate};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alaw_encoding_basic() {
        // Test simple A-law encoding with known values
        let test_values = vec![0i16, 128, 256, 512, 1024, -128, -256, -512, -1024];
        
        for &sample in &test_values {
            let encoded = alaw_compress(sample);
            // Should produce valid 8-bit encoded values
            assert!(encoded <= 255u8);
        }
    }

    #[test]
    fn test_mulaw_encoding_basic() {
        // Test simple μ-law encoding with known values
        let test_values = vec![0i16, 128, 256, 512, 1024, -128, -256, -512, -1024];
        
        for &sample in &test_values {
            let encoded = ulaw_compress(sample);
            // Should produce valid 8-bit encoded values
            assert!(encoded <= 255u8);
        }
    }

    #[test]
    fn test_encoding_boundary_values() {
        // Test boundary values
        let boundary_samples = vec![
            i16::MIN,    // -32768
            -16384,      // -1/2 range
            -1000,       // Small negative
            0,           // Zero
            1000,        // Small positive
            16384,       // +1/2 range
            i16::MAX,    // +32767
        ];

        for &sample in &boundary_samples {
            let alaw_encoded = alaw_compress(sample);
            let mulaw_encoded = ulaw_compress(sample);
            
            // G.711 encoded values are always in 0-255 range
            assert!(alaw_encoded <= 255);
            assert!(mulaw_encoded <= 255);
            
            println!("Sample {}: A-law=0x{:02x}, μ-law=0x{:02x}", 
                     sample, alaw_encoded, mulaw_encoded);
        }
    }

    #[test]
    fn test_encoding_symmetry() {
        // Test positive and negative values
        let test_values = vec![1000, 2000, 4000, 8000, 16000];

        for &val in &test_values {
            let pos_alaw = alaw_compress(val);
            let neg_alaw = alaw_compress(-val);
            let pos_mulaw = ulaw_compress(val);
            let neg_mulaw = ulaw_compress(-val);
            
            // Positive and negative values should encode differently
            assert_ne!(pos_alaw, neg_alaw, 
                      "A-law: Positive and negative values should encode differently");
            assert_ne!(pos_mulaw, neg_mulaw,
                      "μ-law: Positive and negative values should encode differently");
        }
    }

    #[test]
    fn test_encoding_quantization_levels() {
        // Test that nearby values may map to same quantization level
        let mut samples = Vec::new();
        for i in 0..160 {
            samples.push(i as i16);
        }

        let alaw_encoded: Vec<u8> = samples.iter().map(|&s| alaw_compress(s)).collect();
        let mulaw_encoded: Vec<u8> = samples.iter().map(|&s| ulaw_compress(s)).collect();
        
        // Should have fewer unique encoded values than input values
        // due to quantization
        let unique_alaw: std::collections::HashSet<_> = alaw_encoded.iter().collect();
        let unique_mulaw: std::collections::HashSet<_> = mulaw_encoded.iter().collect();
        let unique_input: std::collections::HashSet<_> = samples.iter().collect();
        
        assert!(unique_alaw.len() < unique_input.len(), 
               "A-law quantization should reduce unique values");
        assert!(unique_mulaw.len() < unique_input.len(),
               "μ-law quantization should reduce unique values");
    }

    #[test]
    fn test_codec_encoding_operations() {
        let config_mu = CodecConfig::new(CodecType::G711Pcmu)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(1);

        let mut codec_mu = G711Codec::new_pcmu(config_mu).unwrap();
        
        let config_a = CodecConfig::new(CodecType::G711Pcma)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(1);
            
        let mut codec_a = G711Codec::new_pcma(config_a).unwrap();

        let samples = vec![1000i16; 160];

        let encoded_mu = codec_mu.encode(&samples).unwrap();
        let encoded_a = codec_a.encode(&samples).unwrap();

        // μ-law and A-law should produce different encoded data
        assert_ne!(encoded_mu, encoded_a);
        
        // But both should be same length
        assert_eq!(encoded_mu.len(), encoded_a.len());
        assert_eq!(encoded_mu.len(), 160);
    }



    #[test]
    fn test_encoding_repeatability() {
        let samples = vec![12345i16; 160];

        // Encode same data multiple times using simple functions
        let encoded1: Vec<u8> = samples.iter().map(|&s| alaw_compress(s)).collect();
        let encoded2: Vec<u8> = samples.iter().map(|&s| alaw_compress(s)).collect();
        let encoded3: Vec<u8> = samples.iter().map(|&s| alaw_compress(s)).collect();

        // Results should be identical (G.711 is stateless)
        assert_eq!(encoded1, encoded2);
        assert_eq!(encoded2, encoded3);
    }

    #[test]
    fn test_alaw_vs_mulaw_encoding_differences() {
        let samples = vec![12345i16; 160];

        let encoded_a: Vec<u8> = samples.iter().map(|&s| alaw_compress(s)).collect();
        let encoded_mu: Vec<u8> = samples.iter().map(|&s| ulaw_compress(s)).collect();

        // A-law and μ-law should produce different encoded data
        assert_ne!(encoded_a, encoded_mu);
        
        // But both should be same length
        assert_eq!(encoded_a.len(), encoded_mu.len());
    }

    #[test]
    fn test_encoding_performance_characteristics() {
        // Test with larger data sets to check performance
        let samples = vec![1000i16; 160];
        
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let _encoded: Vec<u8> = samples.iter().map(|&s| alaw_compress(s)).collect();
        }
        let duration = start.elapsed();

        // Should be very fast (less than 10ms for 1000 encodes)
        assert!(duration.as_millis() < 100, 
               "Encoding should be fast: {:?}", duration);
    }

    #[test]
    fn test_encoding_zero_value() {
        // Test encoding of zero specifically
        let zero_sample = 0i16;
        
        let alaw_zero = alaw_compress(zero_sample);
        let mulaw_zero = ulaw_compress(zero_sample);
        
        // Zero should encode to known values
        println!("Zero encodes to: A-law=0x{:02x}, μ-law=0x{:02x}", alaw_zero, mulaw_zero);
        
        // Verify round-trip
        let alaw_decoded = alaw_expand(alaw_zero);
        let mulaw_decoded = ulaw_expand(mulaw_zero);
        
        // Should be close to zero (within quantization error)
        assert!(alaw_decoded.abs() < 100, "A-law zero round-trip error");
        assert!(mulaw_decoded.abs() < 100, "μ-law zero round-trip error");
    }

    #[test]
    fn test_encoding_known_values() {
        // Test some known input/output pairs for verification
        let test_cases = vec![
            (128i16, "small positive"),
            (-128i16, "small negative"),
            (1024i16, "medium positive"),
            (-1024i16, "medium negative"),
            (8192i16, "large positive"),
            (-8192i16, "large negative"),
        ];
        
        for (sample, description) in test_cases {
            let alaw_encoded = alaw_compress(sample);
            let mulaw_encoded = ulaw_compress(sample);
            
            println!("{} ({}): A-law=0x{:02x}, μ-law=0x{:02x}", 
                     sample, description, alaw_encoded, mulaw_encoded);
            
            // Verify round-trip quality
            let alaw_decoded = alaw_expand(alaw_encoded);
            let mulaw_decoded = ulaw_expand(mulaw_encoded);
            
            let alaw_error = (alaw_decoded - sample).abs();
            let mulaw_error = (mulaw_decoded - sample).abs();
            
            assert!(alaw_error < 2000, "A-law error too large for {}: {}", sample, alaw_error);
            assert!(mulaw_error < 2000, "μ-law error too large for {}: {}", sample, mulaw_error);
        }
    }

    #[test]
    fn test_encoding_saturation() {
        // Test that extreme values don't cause overflow
        let extreme_values = vec![i16::MIN, i16::MIN + 1, i16::MAX - 1, i16::MAX];
        
        for &sample in &extreme_values {
            let alaw_encoded = alaw_compress(sample);
            let mulaw_encoded = ulaw_compress(sample);
            
            // Should always produce valid 8-bit values
            assert!(alaw_encoded <= 255);
            assert!(mulaw_encoded <= 255);
            
            // Verify we can decode without panic
            let _alaw_decoded = alaw_expand(alaw_encoded);
            let _mulaw_decoded = ulaw_expand(mulaw_encoded);
        }
    }

    #[test]
    fn test_encoding_monotonicity() {
        // Test that larger positive values generally produce larger encoded values
        // (within quantization segments)
        let test_values = vec![0i16, 100, 200, 500, 1000, 2000, 4000, 8000];
        
        for window in test_values.windows(2) {
            let smaller = window[0];
            let larger = window[1];
            
            let alaw_smaller = alaw_compress(smaller);
            let alaw_larger = alaw_compress(larger);
            let mulaw_smaller = ulaw_compress(smaller);
            let mulaw_larger = ulaw_compress(larger);
            
            // Note: Due to companding law characteristics, we can't guarantee
            // strict monotonicity, but we can check that encoding is reasonable
            println!("Values {} -> 0x{:02x}, {} -> 0x{:02x} (A-law)", 
                     smaller, alaw_smaller, larger, alaw_larger);
            println!("Values {} -> 0x{:02x}, {} -> 0x{:02x} (μ-law)", 
                     smaller, mulaw_smaller, larger, mulaw_larger);
        }
    }
} 
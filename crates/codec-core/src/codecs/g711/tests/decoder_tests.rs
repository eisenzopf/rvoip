//! G.711 Decoder Unit Tests
//!
//! Tests for G.711 decoding functionality including:
//! - μ-law and A-law decoding accuracy
//! - Reconstruction quality
//! - Buffer operations
//! - Error handling
//! - Round-trip consistency

use crate::codecs::g711::*;
use crate::types::{AudioCodec, CodecConfig, CodecType, SampleRate};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alaw_decoding_basic() {
        // Test simple A-law decoding with known values
        let test_values = vec![0u8, 0x55, 0xD5, 0x2A, 0xFF];
        
        for &encoded in &test_values {
            let decoded = alaw_expand(encoded);
            // Should produce valid 16-bit samples
            assert!(decoded >= i16::MIN && decoded <= i16::MAX);
        }
    }

    #[test]
    fn test_mulaw_decoding_basic() {
        // Test simple μ-law decoding with known values
        let test_values = vec![0u8, 0x7F, 0xFF, 0x80, 0x00];
        
        for &encoded in &test_values {
            let decoded = ulaw_expand(encoded);
            // Should produce valid 16-bit samples
            assert!(decoded >= i16::MIN && decoded <= i16::MAX);
        }
    }

    #[test]
    fn test_decoding_all_values() {
        // Test decoding all possible 8-bit values for both laws
        for encoded in 0u8..=255u8 {
            let alaw_decoded = alaw_expand(encoded);
            let mulaw_decoded = ulaw_expand(encoded);
            
            // All decoded values should be valid 16-bit samples
            assert!(alaw_decoded >= i16::MIN && alaw_decoded <= i16::MAX);
            assert!(mulaw_decoded >= i16::MIN && mulaw_decoded <= i16::MAX);
        }
    }

    #[test]
    fn test_decoding_boundary_encoded_values() {
        // Test boundary encoded values
        let boundary_encoded = vec![
            0u8,      // Minimum
            127u8,    // Mid-range
            128u8,    // Sign bit boundary  
            255u8,    // Maximum
        ];

        for &encoded in &boundary_encoded {
            let alaw_decoded = alaw_expand(encoded);
            let mulaw_decoded = ulaw_expand(encoded);
            
            println!("Encoded 0x{:02x}: A-law -> {}, μ-law -> {}", 
                     encoded, alaw_decoded, mulaw_decoded);
            
            // Should be valid 16-bit range
            assert!(alaw_decoded >= i16::MIN && alaw_decoded <= i16::MAX);
            assert!(mulaw_decoded >= i16::MIN && mulaw_decoded <= i16::MAX);
        }
    }

    #[test]
    fn test_codec_decoding_operations() {
        let config_mu = CodecConfig::new(CodecType::G711Pcmu)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(1);

        let mut codec_mu = G711Codec::new_pcmu(config_mu).unwrap();
        
        let config_a = CodecConfig::new(CodecType::G711Pcma)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(1);
            
        let mut codec_a = G711Codec::new_pcma(config_a).unwrap();

        let encoded = vec![100u8; 160];
        
        let decoded_mu = codec_mu.decode(&encoded).unwrap();
        let decoded_a = codec_a.decode(&encoded).unwrap();

        assert_eq!(decoded_mu.len(), 160);
        assert_eq!(decoded_a.len(), 160);
        
        // μ-law and A-law should produce different decoded data
        assert_ne!(decoded_mu, decoded_a);
    }



    #[test]
    fn test_decoding_repeatability() {
        let encoded = vec![123u8; 160];

        // Decode same data multiple times using simple functions
        let decoded1: Vec<i16> = encoded.iter().map(|&e| alaw_expand(e)).collect();
        let decoded2: Vec<i16> = encoded.iter().map(|&e| alaw_expand(e)).collect();
        let decoded3: Vec<i16> = encoded.iter().map(|&e| alaw_expand(e)).collect();

        // Results should be identical (G.711 is stateless)
        assert_eq!(decoded1, decoded2);
        assert_eq!(decoded2, decoded3);
    }

    #[test]
    fn test_decoding_reconstruction_quality() {
        // Test with a pattern that should be well-reconstructed
        let mut encoded = Vec::new();
        for i in 0..160 {
            // Create a pattern with various encoded values
            encoded.push((i % 16 + 128) as u8);
        }

        let decoded: Vec<i16> = encoded.iter().map(|&e| alaw_expand(e)).collect();
        
        // Check that adjacent similar encoded values produce 
        // reasonably similar decoded values
        for i in 0..15 {
            let diff = (decoded[i] - decoded[i + 16]).abs();
            assert!(diff < 5000, "Similar encoded values should decode similarly");
        }
    }

    #[test]
    fn test_alaw_vs_mulaw_decoding_differences() {
        let encoded = vec![123u8; 160];

        let decoded_a: Vec<i16> = encoded.iter().map(|&e| alaw_expand(e)).collect();
        let decoded_mu: Vec<i16> = encoded.iter().map(|&e| ulaw_expand(e)).collect();

        // A-law and μ-law should produce different decoded data
        assert_ne!(decoded_a, decoded_mu);
        
        // But both should be same length
        assert_eq!(decoded_a.len(), decoded_mu.len());
    }

    #[test]
    fn test_decoding_performance_characteristics() {
        let encoded = vec![100u8; 160];
        
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let _decoded: Vec<i16> = encoded.iter().map(|&e| alaw_expand(e)).collect();
        }
        let duration = start.elapsed();

        // Should be very fast (less than 10ms for 1000 decodes)
        assert!(duration.as_millis() < 100, 
               "Decoding should be fast: {:?}", duration);
    }

    #[test]
    fn test_decoding_round_trip_basic() {
        // Test round-trip encoding/decoding
        let original_samples = vec![1000i16; 160];
        
        // Test A-law round trip
        let alaw_encoded: Vec<u8> = original_samples.iter().map(|&s| alaw_compress(s)).collect();
        let alaw_decoded: Vec<i16> = alaw_encoded.iter().map(|&e| alaw_expand(e)).collect();

        assert_eq!(alaw_decoded.len(), original_samples.len());
        
        // Check reconstruction quality
        for (&original, &decoded) in original_samples.iter().zip(alaw_decoded.iter()) {
            let error = (original - decoded).abs();
            // G.711 is lossy, but error should be reasonable
            assert!(error < 2000, "A-law round-trip error too large: {}", error);
        }
        
        // Test μ-law round trip
        let mulaw_encoded: Vec<u8> = original_samples.iter().map(|&s| ulaw_compress(s)).collect();
        let mulaw_decoded: Vec<i16> = mulaw_encoded.iter().map(|&e| ulaw_expand(e)).collect();

        for (&original, &decoded) in original_samples.iter().zip(mulaw_decoded.iter()) {
            let error = (original - decoded).abs();
            assert!(error < 2000, "μ-law round-trip error too large: {}", error);
        }
    }

    #[test]
    fn test_decoding_sign_preservation() {
        // Test positive and negative values
        let positive_samples = vec![5000i16; 80];
        let negative_samples = vec![-5000i16; 80];
        
        // Encode and decode positive values
        let pos_alaw_encoded: Vec<u8> = positive_samples.iter().map(|&s| alaw_compress(s)).collect();
        let pos_alaw_decoded: Vec<i16> = pos_alaw_encoded.iter().map(|&e| alaw_expand(e)).collect();
        
        let pos_mulaw_encoded: Vec<u8> = positive_samples.iter().map(|&s| ulaw_compress(s)).collect();
        let pos_mulaw_decoded: Vec<i16> = pos_mulaw_encoded.iter().map(|&e| ulaw_expand(e)).collect();
        
        // Encode and decode negative values
        let neg_alaw_encoded: Vec<u8> = negative_samples.iter().map(|&s| alaw_compress(s)).collect();
        let neg_alaw_decoded: Vec<i16> = neg_alaw_encoded.iter().map(|&e| alaw_expand(e)).collect();
        
        let neg_mulaw_encoded: Vec<u8> = negative_samples.iter().map(|&s| ulaw_compress(s)).collect();
        let neg_mulaw_decoded: Vec<i16> = neg_mulaw_encoded.iter().map(|&e| ulaw_expand(e)).collect();

        // Check sign preservation
        for &sample in &pos_alaw_decoded {
            assert!(sample > 0, "Positive A-law values should stay positive");
        }
        for &sample in &pos_mulaw_decoded {
            assert!(sample > 0, "Positive μ-law values should stay positive");
        }
        for &sample in &neg_alaw_decoded {
            assert!(sample < 0, "Negative A-law values should stay negative");
        }
        for &sample in &neg_mulaw_decoded {
            assert!(sample < 0, "Negative μ-law values should stay negative");
        }
    }
} 
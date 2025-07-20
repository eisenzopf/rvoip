//! G.729BA Test Data Parsing Utilities
//!
//! This module provides utilities for parsing ITU-T G.729BA test vector files.
//! G.729BA test files include both G.729A (reduced complexity) and G.729B (VAD/DTX/CNG)
//! test vectors with various frame types and silence handling scenarios.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// G.729BA Test Vector Types
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum G729BATestType {
    /// G.729A + G.729B combined tests (tstseq*a.*)
    AnnexBA,
    /// G.729B only tests (tstseq*.*)
    AnnexB,
}

/// G.729BA Frame Types for VAD/DTX testing
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum G729BAFrameType {
    /// Active speech frame (80 bits)
    Speech,
    /// Silence Insertion Descriptor frame (SID, 15 bits)
    SID,
    /// Silence frame (no transmission)
    NoTransmission,
}

/// Parse 16-bit PCM samples from G.729BA test input files (*.bin)
/// 
/// Format: 16-bit little-endian signed integers (Intel PC format)
/// 
/// # Arguments
/// * `filename` - Test vector filename (e.g., "tstseq1.bin")
/// 
/// # Returns
/// Vector of 16-bit PCM samples
pub fn parse_g729ba_pcm_samples(filename: &str) -> io::Result<Vec<i16>> {
    let path = get_g729ba_test_data_path(filename)?;
    let data = fs::read(&path)?;
    
    if data.len() % 2 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("File {} has odd number of bytes, expected 16-bit samples", filename)
        ));
    }
    
    let mut samples = Vec::with_capacity(data.len() / 2);
    for chunk in data.chunks_exact(2) {
        // Intel (PC) format = little-endian
        let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
        samples.push(sample);
    }
    
    Ok(samples)
}

/// Parse G.729BA encoded bitstream from test files (*.bit)
/// 
/// Format: Variable-length frames depending on frame type:
/// - Speech frames: 80 bits (10 bytes)
/// - SID frames: 15 bits (2 bytes, some padding)
/// - No transmission: 0 bits
/// 
/// # Arguments
/// * `filename` - Bitstream filename (e.g., "tstseq1.bit" or "tstseq1a.bit")
/// 
/// # Returns
/// Raw bitstream data
pub fn parse_g729ba_bitstream(filename: &str) -> io::Result<Vec<u8>> {
    let path = get_g729ba_test_data_path(filename)?;
    let data = fs::read(&path)?;
    Ok(data)
}

/// Parse G.729BA decoder output samples from test files (*.out)
/// 
/// Format: 16-bit little-endian signed integers (Intel PC format)
/// 
/// # Arguments
/// * `filename` - Output filename (e.g., "tstseq1.out" or "tstseq1a.out")
/// 
/// # Returns
/// Vector of 16-bit PCM samples
pub fn parse_g729ba_output_samples(filename: &str) -> io::Result<Vec<i16>> {
    // Same format as input samples
    parse_g729ba_pcm_samples(filename)
}

/// Get the full path to a G.729BA test data file
/// 
/// # Arguments
/// * `filename` - Test data filename
/// 
/// # Returns
/// Full path to the test data file
pub fn get_g729ba_test_data_path(filename: &str) -> io::Result<PathBuf> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let test_data_path = Path::new(manifest_dir)
        .join("src")
        .join("codecs")
        .join("g729ba")
        .join("tests")
        .join("test_data")
        .join(filename);
    
    if !test_data_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Test data file not found: {}", test_data_path.display())
        ));
    }
    
    Ok(test_data_path)
}

/// Analyze G.729BA bitstream to identify frame types
/// 
/// This function examines the bitstream to determine the distribution of
/// speech frames, SID frames, and no-transmission periods.
/// 
/// # Arguments
/// * `bitstream` - Raw bitstream data
/// 
/// # Returns
/// Vector of frame types
pub fn analyze_g729ba_frame_types(bitstream: &[u8]) -> Vec<G729BAFrameType> {
    let mut frame_types = Vec::new();
    let mut pos = 0;
    
    while pos < bitstream.len() {
        // For simplicity, assume standard frame patterns
        // In a full implementation, this would parse the actual bitstream format
        
        if pos + 10 <= bitstream.len() {
            // Check if this looks like a speech frame (80 bits = 10 bytes)
            let frame_data = &bitstream[pos..pos + 10];
            
            // Simple heuristic: if frame has non-zero content, it's likely speech
            if frame_data.iter().any(|&b| b != 0) {
                frame_types.push(G729BAFrameType::Speech);
                pos += 10;
            } else {
                // Could be SID or no transmission
                frame_types.push(G729BAFrameType::SID);
                pos += 2; // SID frames are shorter
            }
        } else {
            // Remaining data too short for speech frame
            frame_types.push(G729BAFrameType::SID);
            break;
        }
    }
    
    frame_types
}

/// Calculate similarity between two bitstreams
/// 
/// # Arguments
/// * `expected` - Expected bitstream
/// * `actual` - Actual generated bitstream
/// 
/// # Returns
/// Similarity percentage (0.0 to 100.0)
pub fn calculate_bitstream_similarity(expected: &[u8], actual: &[u8]) -> f64 {
    if expected.is_empty() && actual.is_empty() {
        return 100.0;
    }
    
    if expected.is_empty() || actual.is_empty() {
        return 0.0;
    }
    
    let min_len = expected.len().min(actual.len());
    let mut matches = 0;
    
    for i in 0..min_len {
        if expected[i] == actual[i] {
            matches += 1;
        }
    }
    
    // Factor in length difference
    let length_penalty = (expected.len() as f64 - actual.len() as f64).abs() / expected.len() as f64;
    let base_similarity = (matches as f64) / (expected.len() as f64);
    let similarity = base_similarity * (1.0 - length_penalty);
    
    (similarity * 100.0).max(0.0).min(100.0)
}

/// Calculate signal similarity between two PCM sample arrays
/// 
/// # Arguments
/// * `expected` - Expected PCM samples
/// * `actual` - Actual generated PCM samples
/// 
/// # Returns
/// Similarity percentage (0.0 to 100.0)
pub fn calculate_signal_similarity(expected: &[i16], actual: &[i16]) -> f64 {
    if expected.is_empty() && actual.is_empty() {
        return 100.0;
    }
    
    if expected.is_empty() || actual.is_empty() {
        return 0.0;
    }
    
    let min_len = expected.len().min(actual.len());
    let mut total_error = 0.0;
    let mut max_magnitude: f64 = 0.0;
    
    for i in 0..min_len {
        let error = (expected[i] as f64 - actual[i] as f64).abs();
        total_error += error;
        max_magnitude = max_magnitude.max(expected[i].abs() as f64);
    }
    
    if max_magnitude == 0.0 {
        return 100.0; // Both signals are silent
    }
    
    let mse = total_error / min_len as f64;
    let normalized_error = mse / max_magnitude;
    let similarity = (1.0 - normalized_error.min(1.0)) * 100.0;
    
    similarity.max(0.0).min(100.0)
}

/// G.729BA Test Vector Information
#[derive(Debug)]
pub struct G729BATestVector {
    /// Input PCM file (*.bin) - empty for decoder-only tests
    pub input_file: &'static str,
    /// Encoded bitstream file (*.bit)
    pub bitstream_file: &'static str,
    /// Expected decoder output file (*.out)
    pub output_file: &'static str,
    /// Test vector type (AnnexBA or AnnexB)
    pub test_type: G729BATestType,
    /// Human-readable description of the test
    pub description: &'static str,
}

/// Get all available G.729BA test vectors
pub fn get_g729ba_test_vectors() -> Vec<G729BATestVector> {
    vec![
        // G.729A + G.729B (Annex BA) test vectors
        G729BATestVector {
            input_file: "tstseq1.bin",
            bitstream_file: "tstseq1a.bit",
            output_file: "tstseq1a.out",
            test_type: G729BATestType::AnnexBA,
            description: "Speech sequence 1 with VAD/DTX",
        },
        G729BATestVector {
            input_file: "tstseq2.bin",
            bitstream_file: "tstseq2a.bit",
            output_file: "tstseq2a.out",
            test_type: G729BATestType::AnnexBA,
            description: "Speech sequence 2 with silence periods",
        },
        G729BATestVector {
            input_file: "tstseq3.bin",
            bitstream_file: "tstseq3a.bit",
            output_file: "tstseq3a.out",
            test_type: G729BATestType::AnnexBA,
            description: "Mixed speech and silence sequence",
        },
        G729BATestVector {
            input_file: "tstseq4.bin",
            bitstream_file: "tstseq4a.bit",
            output_file: "tstseq4a.out",
            test_type: G729BATestType::AnnexBA,
            description: "Voice activity detection test",
        },
        
        // G.729B only test vectors
        G729BATestVector {
            input_file: "tstseq1.bin",
            bitstream_file: "tstseq1.bit",
            output_file: "tstseq1.out",
            test_type: G729BATestType::AnnexB,
            description: "Core G.729 + VAD/DTX sequence 1",
        },
        G729BATestVector {
            input_file: "tstseq2.bin",
            bitstream_file: "tstseq2.bit",
            output_file: "tstseq2.out",
            test_type: G729BATestType::AnnexB,
            description: "Core G.729 + VAD/DTX sequence 2",
        },
        G729BATestVector {
            input_file: "tstseq3.bin",
            bitstream_file: "tstseq3.bit",
            output_file: "tstseq3.out",
            test_type: G729BATestType::AnnexB,
            description: "Core G.729 + VAD/DTX sequence 3",
        },
        G729BATestVector {
            input_file: "tstseq4.bin",
            bitstream_file: "tstseq4.bit",
            output_file: "tstseq4.out",
            test_type: G729BATestType::AnnexB,
            description: "Core G.729 + VAD/DTX sequence 4",
        },
        
        // Decoder-only test vectors (no input, just bitstream->output)
        G729BATestVector {
            input_file: "",
            bitstream_file: "tstseq5.bit",
            output_file: "tstseq5.out",
            test_type: G729BATestType::AnnexB,
            description: "Decoder test sequence 5",
        },
        G729BATestVector {
            input_file: "",
            bitstream_file: "tstseq6.bit",
            output_file: "tstseq6.out",
            test_type: G729BATestType::AnnexB,
            description: "Decoder test sequence 6",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_test_vectors() {
        let vectors = get_g729ba_test_vectors();
        assert!(!vectors.is_empty());
        
        // Check that we have both AnnexBA and AnnexB test types
        let has_ba = vectors.iter().any(|v| v.test_type == G729BATestType::AnnexBA);
        let has_b = vectors.iter().any(|v| v.test_type == G729BATestType::AnnexB);
        assert!(has_ba, "Should have G.729BA test vectors");
        assert!(has_b, "Should have G.729B test vectors");
    }

    #[test]
    fn test_similarity_calculation() {
        // Test identical data
        let data1 = vec![1, 2, 3, 4, 5];
        let data2 = vec![1, 2, 3, 4, 5];
        assert_eq!(calculate_bitstream_similarity(&data1, &data2), 100.0);
        
        // Test completely different data
        let data3 = vec![1, 2, 3, 4, 5];
        let data4 = vec![6, 7, 8, 9, 10];
        assert_eq!(calculate_bitstream_similarity(&data3, &data4), 0.0);
        
        // Test partial match
        let data5 = vec![1, 2, 3, 4, 5];
        let data6 = vec![1, 2, 0, 4, 5];
        let similarity = calculate_bitstream_similarity(&data5, &data6);
        assert!(similarity > 50.0 && similarity < 100.0);
    }

    #[test]
    fn test_frame_type_analysis() {
        // Test with some sample data
        let bitstream = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A];
        let frame_types = analyze_g729ba_frame_types(&bitstream);
        assert!(!frame_types.is_empty());
    }
} 
//! G.729A Test Utilities
//!
//! This module provides utilities for parsing ITU-T G.729A test vector files.
//! All test files use 16-bit Intel (PC) format for PCM samples and bitstream data.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use crate::codecs::g729a::types::*;

/// G.729A Test Vector Types
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum G729ATestType {
    /// General speech test
    Speech,
    /// Pitch analysis test
    Pitch,
    /// LSP quantization test
    LSP,
    /// Fixed codebook test
    Fixed,
    /// Algorithm test
    Algorithm,
    /// Taming procedure test
    Tame,
    /// Overflow handling test
    Overflow,
    /// Frame erasure test
    Erasure,
    /// Parity test
    Parity,
    /// General test
    Test,
}

/// G.729A Test Vector Information
#[derive(Debug)]
pub struct G729ATestVector {
    /// Input PCM file (*.in) - may be empty for decoder-only tests
    pub input_file: &'static str,
    /// Encoded bitstream file (*.bit)
    pub bitstream_file: &'static str,
    /// Expected decoder output file (*.pst)
    pub output_file: &'static str,
    /// Test vector type
    pub test_type: G729ATestType,
    /// Human-readable description of the test
    pub description: &'static str,
}

/// Parse 16-bit PCM samples from G.729A test input files (*.in)
/// 
/// Format: 16-bit little-endian signed integers (Intel PC format)
/// 
/// # Arguments
/// * `filename` - Test vector filename (e.g., "SPEECH.IN")
/// 
/// # Returns
/// Vector of 16-bit PCM samples
pub fn parse_g729a_pcm_samples(filename: &str) -> io::Result<Vec<i16>> {
    let path = get_g729a_test_data_path(filename)?;
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

/// Parse G.729A encoded bitstream from test files (*.bit)
/// 
/// Format: Raw bitstream data, 80 bits (10 bytes) per frame
/// 
/// # Arguments
/// * `filename` - Bitstream filename (e.g., "SPEECH.BIT")
/// 
/// # Returns
/// Raw bitstream data
pub fn parse_g729a_bitstream(filename: &str) -> io::Result<Vec<u8>> {
    let path = get_g729a_test_data_path(filename)?;
    let data = fs::read(&path)?;
    Ok(data)
}

/// Parse G.729A decoder output samples from test files (*.pst)
/// 
/// Format: 16-bit little-endian signed integers (Intel PC format)
/// 
/// # Arguments
/// * `filename` - Output filename (e.g., "SPEECH.PST")
/// 
/// # Returns
/// Vector of 16-bit PCM samples
pub fn parse_g729a_output_samples(filename: &str) -> io::Result<Vec<i16>> {
    // Same format as input samples
    parse_g729a_pcm_samples(filename)
}

/// Get the full path to a G.729A test data file
/// 
/// # Arguments
/// * `filename` - Test data filename
/// 
/// # Returns
/// Full path to the test data file
pub fn get_g729a_test_data_path(filename: &str) -> io::Result<PathBuf> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let test_data_path = Path::new(manifest_dir)
        .join("src")
        .join("codecs")
        .join("g729a")
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

/// Calculate bitstream similarity between two bit streams
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

/// Get all available G.729A test vectors
pub fn get_g729a_test_vectors() -> Vec<G729ATestVector> {
    vec![
        G729ATestVector {
            input_file: "SPEECH.IN",
            bitstream_file: "SPEECH.BIT",
            output_file: "SPEECH.PST",
            test_type: G729ATestType::Speech,
            description: "General speech coding test sequence",
        },
        G729ATestVector {
            input_file: "PITCH.IN",
            bitstream_file: "PITCH.BIT",
            output_file: "PITCH.PST",
            test_type: G729ATestType::Pitch,
            description: "Pitch analysis and synthesis test",
        },
        G729ATestVector {
            input_file: "LSP.IN",
            bitstream_file: "LSP.BIT",
            output_file: "LSP.PST",
            test_type: G729ATestType::LSP,
            description: "Line Spectral Pair quantization test",
        },
        G729ATestVector {
            input_file: "FIXED.IN",
            bitstream_file: "FIXED.BIT",
            output_file: "FIXED.PST",
            test_type: G729ATestType::Fixed,
            description: "Fixed codebook ACELP test",
        },
        G729ATestVector {
            input_file: "ALGTHM.IN",
            bitstream_file: "ALGTHM.BIT",
            output_file: "ALGTHM.PST",
            test_type: G729ATestType::Algorithm,
            description: "Algorithm conditional parts test",
        },
        G729ATestVector {
            input_file: "TAME.IN",
            bitstream_file: "TAME.BIT",
            output_file: "TAME.PST",
            test_type: G729ATestType::Tame,
            description: "Taming procedure test",
        },
        G729ATestVector {
            input_file: "",
            bitstream_file: "OVERFLOW.BIT",
            output_file: "OVERFLOW.PST",
            test_type: G729ATestType::Overflow,
            description: "Overflow handling test (decoder only)",
        },
        G729ATestVector {
            input_file: "",
            bitstream_file: "ERASURE.BIT",
            output_file: "ERASURE.PST",
            test_type: G729ATestType::Erasure,
            description: "Frame erasure concealment test (decoder only)",
        },
        G729ATestVector {
            input_file: "",
            bitstream_file: "PARITY.BIT",
            output_file: "PARITY.PST",
            test_type: G729ATestType::Parity,
            description: "Parity check test (decoder only)",
        },
        G729ATestVector {
            input_file: "TEST.IN",
            bitstream_file: "TEST.BIT",
            output_file: "TEST.pst",
            test_type: G729ATestType::Test,
            description: "General functionality test",
        },
    ]
}

/// Frame analysis helper for test verification
pub fn analyze_frames(samples: &[i16]) -> FrameAnalysis {
    let frame_count = samples.len() / L_FRAME;
    let total_energy: f64 = samples.iter().map(|&s| (s as f64).powi(2)).sum();
    let average_energy = if samples.is_empty() { 0.0 } else { total_energy / samples.len() as f64 };
    
    let mut silent_frames = 0;
    let mut active_frames = 0;
    
    for frame_idx in 0..frame_count {
        let start = frame_idx * L_FRAME;
        let end = start + L_FRAME;
        let frame_energy: f64 = samples[start..end].iter().map(|&s| (s as f64).powi(2)).sum();
        
        if frame_energy < 1000.0 { // Threshold for silence
            silent_frames += 1;
        } else {
            active_frames += 1;
        }
    }
    
    FrameAnalysis {
        frame_count,
        silent_frames,
        active_frames,
        average_energy,
        total_energy,
    }
}

/// Frame analysis results
#[derive(Debug)]
pub struct FrameAnalysis {
    /// Total number of frames
    pub frame_count: usize,
    /// Number of silent frames
    pub silent_frames: usize,
    /// Number of active speech frames
    pub active_frames: usize,
    /// Average signal energy
    pub average_energy: f64,
    /// Total signal energy
    pub total_energy: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_test_vectors() {
        let vectors = get_g729a_test_vectors();
        assert!(!vectors.is_empty());
        
        // Check that we have various test types
        let has_speech = vectors.iter().any(|v| v.test_type == G729ATestType::Speech);
        let has_pitch = vectors.iter().any(|v| v.test_type == G729ATestType::Pitch);
        let has_lsp = vectors.iter().any(|v| v.test_type == G729ATestType::LSP);
        
        assert!(has_speech, "Should have speech test vectors");
        assert!(has_pitch, "Should have pitch test vectors");
        assert!(has_lsp, "Should have LSP test vectors");
    }

    #[test]
    fn test_similarity_calculation() {
        // Test identical signals
        let signal1 = vec![1000i16, 2000, 3000, 4000, 5000];
        let signal2 = vec![1000i16, 2000, 3000, 4000, 5000];
        assert_eq!(calculate_signal_similarity(&signal1, &signal2), 100.0);
        
        // Test completely different signals
        let signal3 = vec![1000i16, 2000, 3000, 4000, 5000];
        let signal4 = vec![-1000i16, -2000, -3000, -4000, -5000];
        let similarity = calculate_signal_similarity(&signal3, &signal4);
        assert!(similarity < 50.0);
        
        // Test partial similarity
        let signal5 = vec![1000i16, 2000, 3000, 4000, 5000];
        let signal6 = vec![1000i16, 2000, 0, 4000, 5000];
        let similarity = calculate_signal_similarity(&signal5, &signal6);
        assert!(similarity > 50.0 && similarity < 100.0);
    }

    #[test]
    fn test_frame_analysis() {
        // Create test signal with mix of silence and speech
        let mut test_signal = vec![0i16; L_FRAME * 4]; // 4 frames
        
        // Frame 1: Silent
        // (already zeros)
        
        // Frame 2: Active
        for i in L_FRAME..2*L_FRAME {
            test_signal[i] = (i % 100) as i16 * 100;
        }
        
        // Frame 3: Silent
        // (already zeros)
        
        // Frame 4: Active
        for i in 3*L_FRAME..4*L_FRAME {
            test_signal[i] = (i % 50) as i16 * 200;
        }
        
        let analysis = analyze_frames(&test_signal);
        
        assert_eq!(analysis.frame_count, 4);
        assert_eq!(analysis.silent_frames, 2);
        assert_eq!(analysis.active_frames, 2);
        assert!(analysis.average_energy > 0.0);
    }
} 
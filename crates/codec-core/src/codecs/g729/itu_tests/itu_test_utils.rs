//! ITU-T G.729 Test Data Parsing Utilities (Simplified)
//!
//! This module provides utilities for parsing ITU-T G.729 test vector files.
//! The test files use 16-bit Intel (PC) format for both PCM samples and bitstream data.
//!
//! File formats:
//! - *.in files: 16-bit PCM input samples  
//! - *.bit files: Encoded bitstream data
//! - *.pst/*.out files: Expected decoder output samples

#[allow(missing_docs)]

use std::fs;
use std::io;
use std::path::Path;
use super::super::src::encoder::G729Variant;

/// Parse 16-bit PCM samples from G.729 test input files (*.in)
/// 
/// Format: 16-bit little-endian signed integers (Intel PC format)
/// 
/// # Arguments
/// * `filename` - Test vector filename relative to test_data directory
/// 
/// # Returns
/// Vector of 16-bit PCM samples
pub fn parse_g729_pcm_samples(filename: &str) -> io::Result<Vec<i16>> {
    let path = get_test_data_path(filename)?;
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

/// Parse G.729 encoded bitstream from test files (*.bit)
/// 
/// Format: Raw bitstream data, typically 80 bits (10 bytes) per frame
/// 
/// # Arguments
/// * `filename` - Test vector filename relative to test_data directory
/// 
/// # Returns
/// Vector of bytes representing the encoded bitstream
pub fn parse_g729_bitstream(filename: &str) -> io::Result<Vec<u8>> {
    let path = get_test_data_path(filename)?;
    fs::read(&path)
}

/// Parse G.729 reference output from .pst/.out files
/// 
/// Same format as input files - 16-bit little-endian samples
pub fn parse_g729_reference_output(filename: &str) -> io::Result<Vec<i16>> {
    parse_g729_pcm_samples(filename)
}

/// Parse 16-bit samples from raw byte data
pub fn parse_16bit_samples(data: &[u8]) -> io::Result<Vec<i16>> {
    if data.len() % 2 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Data has odd number of bytes, expected 16-bit samples"
        ));
    }
    
    let mut samples = Vec::with_capacity(data.len() / 2);
    for chunk in data.chunks_exact(2) {
        let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
        samples.push(sample);
    }
    
    Ok(samples)
}

/// Get test data path for specific G.729 variant
pub fn get_variant_test_data_path(variant: G729Variant, filename: &str) -> io::Result<std::path::PathBuf> {
    let variant_dir = match variant {
        G729Variant::Core => "g729",
        G729Variant::AnnexA => "g729AnnexA", 
        G729Variant::AnnexB => "g729AnnexB",
    };
    
    let search_paths = [
        format!("src/codecs/g729/itu_tests/test_data/{}/{}", variant_dir, filename),
        format!("T-REC-G.729-201206/Software/G729_Release3/{}/test_vectors/{}", variant_dir, filename),
        format!("{}/{}", variant_dir, filename),
    ];
    
    for path_str in &search_paths {
        let path = std::path::Path::new(path_str);
        if path.exists() {
            return Ok(path.to_path_buf());
        }
    }
    
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("Test data file not found for {:?}: {}", variant, filename)
    ))
}

/// Get the full path to a test data file
/// 
/// Searches in multiple possible locations:
/// 1. src/codecs/g729/itu_tests/test_data/g729/
/// 2. T-REC-G.729-201206/Software/G729_Release3/g729/test_vectors/
/// 3. Current directory
/// 
/// # Arguments
/// * `filename` - Test vector filename
/// 
/// # Returns
/// Full path to the test file
fn get_test_data_path(filename: &str) -> io::Result<std::path::PathBuf> {
    // Try multiple possible locations for test data
    let search_paths = [
        format!("src/codecs/g729/itu_tests/test_data/g729/{}", filename),
        format!("T-REC-G.729-201206/Software/G729_Release3/g729/test_vectors/{}", filename),
        filename.to_string(),
    ];
    
    for path_str in &search_paths {
        let path = std::path::Path::new(path_str);
        if path.exists() {
            return Ok(path.to_path_buf());
        }
    }
    
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("Test data file not found: {}", filename)
    ))
}

/// Calculate similarity between two signal sequences
/// 
/// Uses normalized correlation coefficient
/// 
/// # Arguments
/// * `signal1` - First signal
/// * `signal2` - Second signal
/// 
/// # Returns
/// Similarity score from 0.0 (no similarity) to 1.0 (identical)
pub fn calculate_signal_similarity(signal1: &[i16], signal2: &[i16]) -> f64 {
    if signal1.is_empty() || signal2.is_empty() {
        return 0.0;
    }
    
    let len = signal1.len().min(signal2.len());
    if len == 0 {
        return 0.0;
    }
    
    // Calculate means
    let mean1 = signal1[..len].iter().map(|&x| x as f64).sum::<f64>() / len as f64;
    let mean2 = signal2[..len].iter().map(|&x| x as f64).sum::<f64>() / len as f64;
    
    // Calculate correlation coefficient
    let mut numerator = 0.0;
    let mut sum_sq1 = 0.0;
    let mut sum_sq2 = 0.0;
    
    for i in 0..len {
        let x1 = signal1[i] as f64 - mean1;
        let x2 = signal2[i] as f64 - mean2;
        
        numerator += x1 * x2;
        sum_sq1 += x1 * x1;
        sum_sq2 += x2 * x2;
    }
    
    let denominator = (sum_sq1 * sum_sq2).sqrt();
    if denominator == 0.0 {
        return if numerator == 0.0 { 1.0 } else { 0.0 };
    }
    
    (numerator / denominator).abs()
}

/// Alias for calculate_signal_similarity for compatibility
pub fn calculate_sample_similarity(signal1: &[i16], signal2: &[i16]) -> f64 {
    calculate_signal_similarity(signal1, signal2)
}

/// Calculate similarity between bitstreams
pub fn calculate_bitstream_similarity(stream1: &[u8], stream2: &[u8]) -> f64 {
    if stream1.is_empty() || stream2.is_empty() {
        return 0.0;
    }
    
    let len = stream1.len().min(stream2.len());
    if len == 0 {
        return 0.0;
    }
    
    let mut matches = 0;
    for i in 0..len {
        if stream1[i] == stream2[i] {
            matches += 1;
        }
    }
    
    matches as f64 / len as f64
}

/// Calculate signal-to-noise ratio between reference and test signals
/// 
/// # Arguments
/// * `reference` - Reference signal
/// * `test` - Test signal
/// 
/// # Returns
/// SNR in decibels
pub fn calculate_snr(reference: &[i16], test: &[i16]) -> f32 {
    if reference.is_empty() || test.is_empty() {
        return 0.0;
    }
    
    let min_len = reference.len().min(test.len());
    let ref_sig = &reference[..min_len];
    let test_sig = &test[..min_len];
    
    // Calculate signal power
    let signal_power: f64 = ref_sig.iter().map(|&x| (x as f64).powi(2)).sum();
    
    // Calculate noise power (difference between signals)
    let noise_power: f64 = ref_sig.iter().zip(test_sig.iter())
        .map(|(&r, &t)| ((r - t) as f64).powi(2))
        .sum();
    
    if noise_power > 0.0 && signal_power > 0.0 {
        10.0 * (signal_power / noise_power).log10() as f32
    } else {
        100.0 // Very high SNR if no noise
    }
}

/// Compliance test results aggregator
#[derive(Debug, Default)]
pub struct ComplianceResults {
    /// Test suite results
    pub suites: Vec<(String, TestSuiteResult)>,
}

/// Test suite result
#[derive(Debug)]
pub struct TestSuiteResult {
    /// Test name
    pub name: String,
    /// Number of tests passed
    pub passed: usize,
    /// Total number of tests
    pub total: usize,
    /// Overall compliance percentage
    pub compliance: f64,
    /// Individual test results
    pub tests: Vec<TestResult>,
}

/// Individual test result
#[derive(Debug)]
pub struct TestResult {
    /// Test name
    pub name: String,
    /// Test passed
    pub passed: bool,
    /// Similarity score (0.0 to 1.0)
    pub similarity: f64,
    /// Additional details
    pub details: String,
}

impl ComplianceResults {
    /// Create new compliance results aggregator
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Add test suite result
    pub fn add_suite(&mut self, name: String, result: TestSuiteResult) {
        self.suites.push((name, result));
    }
    
    /// Calculate overall compliance percentage
    pub fn overall_compliance(&self) -> f64 {
        if self.suites.is_empty() {
            return 0.0;
        }
        
        let total_compliance: f64 = self.suites.iter().map(|(_, result)| result.compliance).sum();
        total_compliance / self.suites.len() as f64
    }
    
    /// Print comprehensive compliance summary
    pub fn print_summary(&self) {
        println!("\n🎯 G.729 ITU-T COMPLIANCE TEST SUMMARY");
        println!("=====================================");
        
        for (suite_name, result) in &self.suites {
            println!("\n📊 {}: {:.1}% ({}/{} tests passed)", 
                     suite_name, result.compliance * 100.0, result.passed, result.total);
            
            // Show failed tests
            for test in &result.tests {
                if !test.passed {
                    println!("  ❌ {}: {:.1}% - {}", test.name, test.similarity * 100.0, test.details);
                }
            }
        }
        
        println!("\n🎉 OVERALL COMPLIANCE: {:.1}%", self.overall_compliance() * 100.0);
        
        let compliance = self.overall_compliance();
        if compliance > 0.9 {
            println!("✅ EXCELLENT - Production ready ITU-T G.729 implementation");
        } else if compliance > 0.8 {
            println!("🟡 GOOD - Minor optimizations needed for full compliance");
        } else if compliance > 0.6 {
            println!("🟠 NEEDS WORK - Significant improvements required");
        } else {
            println!("❌ NEEDS WORK - Significant compliance issues");
        }
    }
}

impl TestSuiteResult {
    /// Create new test suite result
    pub fn new(name: String) -> Self {
        Self {
            name,
            passed: 0,
            total: 0,
            compliance: 0.0,
            tests: Vec::new(),
        }
    }
    
    /// Add individual test result
    pub fn add_test(&mut self, test: TestResult) {
        if test.passed {
            self.passed += 1;
        }
        self.total += 1;
        self.tests.push(test);
        
        // Recalculate compliance
        self.compliance = self.passed as f64 / self.total as f64;
    }
}

// Quality evaluation data structures and utilities

/// ACELP quality metrics aggregator
#[derive(Debug, Default)]
pub struct AcelpQualityMetrics {
    results: Vec<(String, AcelpResult)>,
}

/// ACELP quality result for a test signal
#[derive(Debug)]
pub struct AcelpResult {
    pub position_diversity: f64,
    pub track_utilization: f64,
    pub clustering_penalty: f64,
}

/// Pulse position statistics tracker
#[derive(Debug)]
pub struct PulsePositionStats {
    position_counts: [usize; 40], // Count usage of each position 0-39
    total_frames: usize,
}

impl Default for PulsePositionStats {
    fn default() -> Self {
        Self {
            position_counts: [0; 40],
            total_frames: 0,
        }
    }
}

/// Track usage statistics tracker
#[derive(Debug, Default)]
pub struct TrackUsageStats {
    track_counts: [usize; 4], // Count usage of each track 0-3
    total_frames: usize,
}

impl AcelpQualityMetrics {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn add_result(&mut self, name: String, result: AcelpResult) {
        self.results.push((name, result));
    }
    
    pub fn calculate_overall_score(&self) -> f64 {
        if self.results.is_empty() {
            return 0.0;
        }
        
        let avg_diversity: f64 = self.results.iter().map(|(_, r)| r.position_diversity).sum::<f64>() / self.results.len() as f64;
        let avg_utilization: f64 = self.results.iter().map(|(_, r)| r.track_utilization).sum::<f64>() / self.results.len() as f64;
        let avg_clustering: f64 = self.results.iter().map(|(_, r)| 1.0 - r.clustering_penalty).sum::<f64>() / self.results.len() as f64;
        
        (avg_diversity + avg_utilization + avg_clustering) / 3.0
    }
}

impl PulsePositionStats {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn add_frame(&mut self, positions: &[usize; 4]) {
        for &pos in positions {
            if pos < 40 {
                self.position_counts[pos] += 1;
            }
        }
        self.total_frames += 1;
    }
    
    pub fn calculate_diversity(&self) -> f64 {
        if self.total_frames == 0 {
            return 0.0;
        }
        
        // Calculate entropy-based diversity
        let mut entropy = 0.0;
        let total_pulses = self.total_frames * 4;
        
        if total_pulses > 0 {
            for &count in &self.position_counts {
                if count > 0 {
                    let p = count as f64 / total_pulses as f64;
                    entropy -= p * p.log2();
                }
            }
            
            // Normalize to 0-1 range (max entropy for 40 positions)
            entropy / (40_f64.log2())
        } else {
            0.0
        }
    }
    
    pub fn calculate_clustering_penalty(&self) -> f64 {
        if self.total_frames == 0 {
            return 0.0;
        }
        
        // Penalty for clustering all pulses at position 0
        let zero_position_ratio = self.position_counts[0] as f64 / (self.total_frames * 4) as f64;
        zero_position_ratio // Higher penalty for more clustering at position 0
    }
}

impl TrackUsageStats {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn add_frame(&mut self, positions: &[usize; 4]) {
        for &pos in positions {
            if pos < 40 {
                let track = pos % 5; // G.729 track assignment
                if track < 4 {
                    self.track_counts[track] += 1;
                }
            }
        }
        self.total_frames += 1;
    }
    
    pub fn calculate_utilization(&self) -> f64 {
        if self.total_frames == 0 {
            return 0.0;
        }
        
        // Ideal is equal distribution across 4 tracks
        let total_pulses = self.total_frames * 4;
        let ideal_per_track = total_pulses as f64 / 4.0;
        
        if ideal_per_track > 0.0 {
            let variance: f64 = self.track_counts.iter()
                .map(|&count| (count as f64 - ideal_per_track).powi(2))
                .sum::<f64>() / 4.0;
            
            let max_variance = ideal_per_track.powi(2);
            1.0 - (variance / max_variance).min(1.0)
        } else {
            0.0
        }
    }
}

/// Count unique tracks used by pulse positions
pub fn count_unique_tracks(positions: &[usize; 4]) -> usize {
    let mut tracks_used = [false; 4];
    for &pos in positions {
        if pos < 40 {
            let track = pos % 5; // G.729 track assignment
            if track < 4 {
                tracks_used[track] = true;
            }
        }
    }
    tracks_used.iter().filter(|&&used| used).count()
}

/// Generate test signals for quality evaluation
pub fn generate_test_signals() -> Vec<(String, Vec<i16>)> {
    let mut signals = Vec::new();
    
    // Silence signal
    signals.push(("Silence".to_string(), vec![0i16; 320])); // 4 frames
    
    // Sine wave signal
    let mut sine_wave = Vec::with_capacity(320);
    for i in 0..320 {
        let sample = (1000.0 * (2.0 * std::f32::consts::PI * i as f32 / 40.0).sin()) as i16;
        sine_wave.push(sample);
    }
    signals.push(("Sine Wave".to_string(), sine_wave));
    
    // White noise signal
    let mut white_noise = Vec::with_capacity(320);
    let mut seed = 12345i32;
    for _i in 0..320 {
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        let sample = (seed >> 16) as i16;
        white_noise.push(sample);
    }
    signals.push(("White Noise".to_string(), white_noise));
    
    // Impulse train signal
    let mut impulse_train = vec![0i16; 320];
    for i in (0..320).step_by(40) {
        impulse_train[i] = 8000;
    }
    signals.push(("Impulse Train".to_string(), impulse_train));
    
    signals
}

/// Generate test LSP vectors across the valid range
pub fn generate_test_lsp_vectors() -> Vec<(String, [i16; 10])> {
    let mut vectors = Vec::new();
    
    // Low frequency emphasis
    vectors.push(("Low Frequency".to_string(), [
        1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000
    ]));
    
    // High frequency emphasis
    vectors.push(("High Frequency".to_string(), [
        20000, 21000, 22000, 23000, 24000, 25000, 26000, 27000, 28000, 29000
    ]));
    
    // Uniform distribution
    vectors.push(("Uniform".to_string(), [
        3000, 6000, 9000, 12000, 15000, 18000, 21000, 24000, 27000, 30000
    ]));
    
    // Formant-like structure
    vectors.push(("Formant-like".to_string(), [
        2000, 4000, 6000, 8000, 12000, 16000, 20000, 24000, 28000, 30000
    ]));
    
    vectors
}

/// Generate pitch test signals with known characteristics
pub fn generate_pitch_test_signals() -> Vec<(String, Vec<i16>, f32)> {
    let mut signals = Vec::new();
    
    // 40-sample pitch period
    let mut pitch_40 = Vec::with_capacity(320);
    for i in 0..320 {
        let sample = (2000.0 * (2.0 * std::f32::consts::PI * i as f32 / 40.0).sin()) as i16;
        pitch_40.push(sample);
    }
    signals.push(("Pitch 40".to_string(), pitch_40, 40.0));
    
    // 80-sample pitch period
    let mut pitch_80 = Vec::with_capacity(320);
    for i in 0..320 {
        let sample = (2000.0 * (2.0 * std::f32::consts::PI * i as f32 / 80.0).sin()) as i16;
        pitch_80.push(sample);
    }
    signals.push(("Pitch 80".to_string(), pitch_80, 80.0));
    
    // 120-sample pitch period
    let mut pitch_120 = Vec::with_capacity(480);
    for i in 0..480 {
        let sample = (2000.0 * (2.0 * std::f32::consts::PI * i as f32 / 120.0).sin()) as i16;
        pitch_120.push(sample);
    }
    signals.push(("Pitch 120".to_string(), pitch_120, 120.0));
    
    signals
}

// Additional quality metric structures (LSP, Pitch, Synthesis, ITU)

/// LSP quality metrics aggregator
#[derive(Debug, Default)]
pub struct LspQualityMetrics {
    results: Vec<(String, LspResult)>,
}

/// LSP quality result
#[derive(Debug)]
pub struct LspResult {
    pub spectral_distortion: f32,
    pub lsf_distortion: f32,
    pub ordering_violations: usize,
    pub stability_margin: f32,
}

/// Pitch quality metrics aggregator
#[derive(Debug, Default)]
pub struct PitchQualityMetrics {
    results: Vec<(String, PitchResult)>,
}

/// Pitch quality result
#[derive(Debug)]
pub struct PitchResult {
    pub pitch_error: f32,
    pub correlation_quality: f32,
    pub voicing_accuracy: f32,
    pub stability_score: f32,
}

/// Synthesis quality metrics aggregator
#[derive(Debug, Default)]
pub struct SynthesisQualityMetrics {
    results: Vec<(String, SynthesisResult)>,
}

/// Synthesis quality result
#[derive(Debug)]
pub struct SynthesisResult {
    pub energy_preservation: f32,
    pub overall_snr: f32,
    pub dynamic_range: f32,
    pub silence_preservation: f32,
}

/// ITU compliance metrics aggregator
#[derive(Debug, Default)]
pub struct ItuComplianceMetrics {
    results: Vec<(String, ItuResult)>,
}

/// ITU compliance result
#[derive(Debug)]
pub struct ItuResult {
    pub similarity: f64,
    pub snr: f32,
    pub correlation: f64,
    pub rms_error: f32,
}

// Implementations for quality metrics

impl LspQualityMetrics {
    pub fn new() -> Self { Self::default() }
    
    pub fn add_result(&mut self, name: String, result: LspResult) {
        self.results.push((name, result));
    }
    
    pub fn calculate_overall_score(&self) -> f64 {
        if self.results.is_empty() { return 0.0; }
        
        let mut total_score = 0.0;
        for (_, result) in &self.results {
            // Weight different metrics
            let distortion_score = (20.0 - result.spectral_distortion.min(20.0)) / 20.0;
            let ordering_score = if result.ordering_violations == 0 { 1.0 } else { 0.0 };
            let stability_score = result.stability_margin.min(1.0) as f64;
            
            total_score += (distortion_score as f64 + ordering_score + stability_score) / 3.0;
        }
        
        total_score / self.results.len() as f64
    }
}

impl PitchQualityMetrics {
    pub fn new() -> Self { Self::default() }
    
    pub fn add_result(&mut self, name: String, result: PitchResult) {
        self.results.push((name, result));
    }
    
    pub fn calculate_overall_score(&self) -> f64 {
        if self.results.is_empty() { return 0.0; }
        
        let mut total_score = 0.0;
        for (_, result) in &self.results {
            let error_score = (20.0 - result.pitch_error.min(20.0)) / 20.0;
            let correlation_score = result.correlation_quality as f64;
            let voicing_score = result.voicing_accuracy as f64;
            let stability_score = result.stability_score as f64;
            
            total_score += (error_score as f64 + correlation_score + voicing_score + stability_score) / 4.0;
        }
        
        total_score / self.results.len() as f64
    }
}

impl SynthesisQualityMetrics {
    pub fn new() -> Self { Self::default() }
    
    pub fn add_result(&mut self, name: String, result: SynthesisResult) {
        self.results.push((name, result));
    }
    
    pub fn calculate_overall_score(&self) -> f64 {
        if self.results.is_empty() { return 0.0; }
        
        let mut total_score = 0.0;
        for (_, result) in &self.results {
            let energy_score = result.energy_preservation as f64;
            let snr_score = (result.overall_snr.max(0.0).min(30.0) / 30.0) as f64;
            let range_score = (result.dynamic_range.max(0.0).min(60.0) / 60.0) as f64;
            let silence_score = result.silence_preservation as f64;
            
            total_score += (energy_score + snr_score + range_score + silence_score) / 4.0;
        }
        
        total_score / self.results.len() as f64
    }
}

impl ItuComplianceMetrics {
    pub fn new() -> Self { Self::default() }
    
    pub fn add_result(&mut self, name: String, result: ItuResult) {
        self.results.push((name, result));
    }
    
    pub fn has_results(&self) -> bool {
        !self.results.is_empty()
    }
    
    pub fn calculate_overall_score(&self) -> f64 {
        if self.results.is_empty() { return 0.0; }
        
        let mut total_score = 0.0;
        for (_, result) in &self.results {
            let similarity_score = result.similarity;
            let snr_score = (result.snr.max(0.0).min(30.0) / 30.0) as f64;
            let correlation_score = result.correlation;
            
            total_score += (similarity_score + snr_score + correlation_score) / 3.0;
        }
        
        total_score / self.results.len() as f64
    }
}

// Quality calculation utility functions

/// Calculate spectral distortion between LSP vectors
pub fn calculate_spectral_distortion(lsp1: &[i16], lsp2: &[i16]) -> f32 {
    let mut total_distortion = 0.0;
    for i in 0..lsp1.len().min(lsp2.len()) {
        let diff = (lsp1[i] - lsp2[i]) as f32;
        total_distortion += diff * diff;
    }
    (total_distortion / lsp1.len() as f32).sqrt() / 1000.0 // Normalize
}

/// Calculate LSF distortion
pub fn calculate_lsf_distortion(lsp1: &[i16], lsp2: &[i16]) -> f32 {
    calculate_spectral_distortion(lsp1, lsp2) // Simplified
}

/// Check LSP ordering violations
pub fn check_lsp_ordering(lsp: &[i16]) -> usize {
    let mut violations = 0;
    for i in 1..lsp.len() {
        if lsp[i] <= lsp[i-1] {
            violations += 1;
        }
    }
    violations
}

/// Calculate stability margin for LSPs
pub fn calculate_stability_margin(lsp: &[i16]) -> f32 {
    let mut min_gap = i16::MAX;
    for i in 1..lsp.len() {
        let gap = lsp[i] - lsp[i-1];
        min_gap = min_gap.min(gap);
    }
    (min_gap as f32 / 1000.0).max(0.0) // Normalize
}

/// Evaluate pitch correlation quality
pub fn evaluate_pitch_correlation(_analyzer: &crate::codecs::g729::src::pitch::PitchAnalyzer, _signal: &[i16], _pitch: i16) -> f32 {
    0.7 // Placeholder - would implement actual correlation analysis
}

/// Evaluate voicing decision accuracy
pub fn evaluate_voicing_decision(_signal: &[i16], _pitch: i16) -> f32 {
    0.8 // Placeholder - would implement voicing analysis
}

/// Evaluate pitch stability
pub fn evaluate_pitch_stability(_analyzer: &crate::codecs::g729::src::pitch::PitchAnalyzer, _signal: &[i16]) -> f32 {
    0.75 // Placeholder - would implement stability analysis
}

/// Calculate energy preservation between signals
pub fn calculate_energy_preservation(input: &[i16], output: &[i16]) -> f32 {
    let input_energy: f64 = input.iter().map(|&x| (x as f64).powi(2)).sum();
    let output_energy: f64 = output.iter().map(|&x| (x as f64).powi(2)).sum();
    
    if input_energy > 0.0 {
        (output_energy / input_energy).min(1.0) as f32
    } else {
        if output_energy == 0.0 { 1.0 } else { 0.0 }
    }
}

/// Calculate spectral distortion between frames
pub fn calculate_spectral_distortion_frame(_input: &[i16], _output: &[i16]) -> f32 {
    2.5 // Placeholder - would implement FFT-based spectral analysis
}

/// Calculate dynamic range of signal
pub fn calculate_dynamic_range(signal: &[i16]) -> f32 {
    if signal.is_empty() { return 0.0; }
    
    let max_val = signal.iter().map(|&x| x.abs()).max().unwrap_or(0) as f32;
    let min_val = signal.iter().map(|&x| x.abs()).filter(|&x| x > 0).min().unwrap_or(1) as f32;
    
    if min_val > 0.0 {
        20.0 * (max_val / min_val).log10()
    } else {
        60.0 // Assume good dynamic range
    }
}

/// Evaluate silence preservation
pub fn evaluate_silence_preservation(input: &[i16], output: &[i16]) -> f32 {
    let input_silent: usize = input.iter().filter(|&&x| x.abs() < 100).count();
    let output_silent: usize = output.iter().filter(|&&x| x.abs() < 100).count();
    
    if input_silent > 0 {
        (output_silent.min(input_silent) as f32) / (input_silent as f32)
    } else {
        1.0 // No silence to preserve
    }
}

/// Calculate correlation between signals
pub fn calculate_correlation(signal1: &[i16], signal2: &[i16]) -> f64 {
    calculate_signal_similarity(signal1, signal2) // Reuse similarity function
}

/// Calculate RMS error between signals
pub fn calculate_rms_error(reference: &[i16], test: &[i16]) -> f32 {
    if reference.is_empty() || test.is_empty() { return 100.0; }
    
    let min_len = reference.len().min(test.len());
    let sum_sq_error: f64 = reference[..min_len].iter()
        .zip(test[..min_len].iter())
        .map(|(&r, &t)| ((r - t) as f64).powi(2))
        .sum();
    
    (sum_sq_error / min_len as f64).sqrt() as f32
} 
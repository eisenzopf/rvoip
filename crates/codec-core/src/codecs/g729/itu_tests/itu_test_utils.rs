//! ITU-T G.729 Test Data Parsing Utilities (Simplified)
//!
//! This module provides utilities for parsing ITU-T G.729 test vector files.
//! The test files use 16-bit Intel (PC) format for both PCM samples and bitstream data.
//!
//! File formats:
//! - *.in files: 16-bit PCM input samples  
//! - *.bit files: Encoded bitstream data
//! - *.pst/*.out files: Expected decoder output samples

use std::fs;
use std::io;
use std::path::Path;

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
/// Vector of bytes containing the bitstream
pub fn parse_g729_bitstream(filename: &str) -> io::Result<Vec<u8>> {
    let path = get_test_data_path(filename)?;
    let data = fs::read(&path)?;
    Ok(data)
}

/// Parse expected decoder output from test files (*.pst, *.out)
/// 
/// Format: 16-bit little-endian signed integers (Intel PC format)
/// 
/// # Arguments  
/// * `filename` - Test vector filename relative to test_data directory
/// 
/// # Returns
/// Vector of 16-bit PCM samples
pub fn parse_g729_reference_output(filename: &str) -> io::Result<Vec<i16>> {
    // Same format as input PCM samples
    parse_g729_pcm_samples(filename)
}

/// Get the full path to a test data file
/// 
/// Searches in the G.729 test data directory
fn get_test_data_path(filename: &str) -> io::Result<std::path::PathBuf> {
    let base_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/codecs/g729/itu_tests/test_data");
    
    // Try different variant directories
    let search_paths = [
        base_path.join("g729").join(filename),
        base_path.join("g729AnnexA").join(filename), 
        base_path.join("g729AnnexB").join(filename),
        base_path.join(filename), // Try root level
    ];
    
    for path in &search_paths {
        if path.exists() {
            return Ok(path.clone());
        }
    }
    
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("Test file '{}' not found in any G.729 test data directory", filename)
    ))
}

/// Calculate similarity between two bitstreams
/// 
/// # Arguments
/// * `expected` - Expected bitstream data
/// * `actual` - Actual bitstream data from our encoder
/// 
/// # Returns
/// Similarity ratio from 0.0 to 1.0
pub fn calculate_bitstream_similarity(expected: &[u8], actual: &[u8]) -> f64 {
    if expected.is_empty() && actual.is_empty() {
        return 1.0;
    }
    
    if expected.is_empty() || actual.is_empty() {
        return 0.0;
    }
    
    let min_len = expected.len().min(actual.len());
    let max_len = expected.len().max(actual.len());
    
    let mut matching_bytes = 0;
    for i in 0..min_len {
        if expected[i] == actual[i] {
            matching_bytes += 1;
        }
    }
    
    // Account for length differences
    matching_bytes as f64 / max_len as f64
}

/// Calculate similarity between two PCM sample arrays
/// 
/// Uses normalized correlation to account for minor amplitude differences
/// that don't affect perceptual quality.
/// 
/// # Arguments
/// * `expected` - Expected PCM samples
/// * `actual` - Actual PCM samples from our decoder
/// 
/// # Returns
/// Similarity ratio from 0.0 to 1.0
pub fn calculate_sample_similarity(expected: &[i16], actual: &[i16]) -> f64 {
    if expected.is_empty() && actual.is_empty() {
        return 1.0;
    }
    
    if expected.is_empty() || actual.is_empty() {
        return 0.0;
    }
    
    let min_len = expected.len().min(actual.len());
    
    // Calculate normalized correlation
    let mut sum_xy = 0i64;
    let mut sum_xx = 0i64;
    let mut sum_yy = 0i64;
    
    for i in 0..min_len {
        let x = expected[i] as i64;
        let y = actual[i] as i64;
        
        sum_xy += x * y;
        sum_xx += x * x;
        sum_yy += y * y;
    }
    
    if sum_xx == 0 && sum_yy == 0 {
        return 1.0; // Both signals are zero
    }
    
    if sum_xx == 0 || sum_yy == 0 {
        return 0.0; // One signal is zero, other is not
    }
    
    let correlation = sum_xy as f64 / ((sum_xx as f64) * (sum_yy as f64)).sqrt();
    correlation.max(0.0).min(1.0)
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
        
        let overall = self.overall_compliance();
        println!("\n🎉 OVERALL COMPLIANCE: {:.1}%", overall * 100.0);
        
        if overall >= 0.95 {
            println!("✅ EXCELLENT - Ready for production use!");
        } else if overall >= 0.85 {
            println!("✅ GOOD - Minor issues may need attention");
        } else if overall >= 0.75 {
            println!("⚠️  ACCEPTABLE - Several issues need attention");
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
    
    /// Add test result
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bitstream_similarity() {
        let a = vec![0x12u8, 0x34, 0x56, 0x78];
        let b = vec![0x12u8, 0x34, 0x56, 0x78];
        assert_eq!(calculate_bitstream_similarity(&a, &b), 1.0);
        
        let c = vec![0x12u8, 0x34, 0x99, 0x78];
        assert_eq!(calculate_bitstream_similarity(&a, &c), 0.75);
        
        assert_eq!(calculate_bitstream_similarity(&[], &[]), 1.0);
        assert_eq!(calculate_bitstream_similarity(&[], &[1, 2, 3]), 0.0);
    }

    #[test]
    fn test_sample_similarity() {
        let a = vec![1000i16, 2000, 3000, 4000];
        let b = vec![1000i16, 2000, 3000, 4000];
        assert_eq!(calculate_sample_similarity(&a, &b), 1.0);
        
        let c = vec![1001i16, 2001, 3001, 4001];
        let similarity = calculate_sample_similarity(&a, &c);
        assert!(similarity > 0.99, "Should have high similarity for close samples");
        
        assert_eq!(calculate_sample_similarity(&[], &[]), 1.0);
        assert_eq!(calculate_sample_similarity(&[], &[1, 2, 3]), 0.0);
    }

    #[test]
    fn test_compliance_results() {
        let mut results = ComplianceResults::new();
        
        let mut suite1 = TestSuiteResult::new("Test Suite 1".to_string());
        suite1.add_test(TestResult {
            name: "Test 1".to_string(),
            passed: true,
            similarity: 0.95,
            details: "Good".to_string(),
        });
        suite1.add_test(TestResult {
            name: "Test 2".to_string(),
            passed: false,
            similarity: 0.70,
            details: "Failed".to_string(),
        });
        
        results.add_suite("Suite 1".to_string(), suite1);
        
        assert_eq!(results.overall_compliance(), 0.5); // 1 passed out of 2
    }
} 
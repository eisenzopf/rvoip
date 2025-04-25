// RFC Compliance Torture Tests - Based on RFC 4475

use std::fs;
use std::path::Path;
use std::env;
use rvoip_sip_core::parse_message;
use rvoip_sip_core::error::Error as SipError;

/// Structure for tracking test results
struct TestResults {
    successes: Vec<String>,
    failures: Vec<(String, SipError, String)>,
}

impl TestResults {
    fn new() -> Self {
        Self {
            successes: Vec::new(),
            failures: Vec::new(),
        }
    }

    fn add_success(&mut self, filename: String) {
        self.successes.push(filename);
    }

    fn add_failure(&mut self, filename: String, error: SipError, content: String) {
        self.failures.push((filename, error, content));
    }

    fn report_failures(&self, expected_to_fail: bool) {
        if self.failures.is_empty() {
            return;
        }

        let action = if expected_to_fail { "succeed" } else { "fail" };
        
        eprintln!("\n{} files unexpectedly {}ed parsing:", 
                 self.failures.len(), action);
        
        for (i, (file, error, content)) in self.failures.iter().enumerate() {
            eprintln!("\n{}. File '{}' - {}", i + 1, file, if expected_to_fail { "parsed successfully (expected to fail)" } else { "failed to parse" });
            if !expected_to_fail {
                eprintln!("   Error: {}", error);
            }
            eprintln!("   --- Message Content ---");
            eprintln!("   {}", content);
            eprintln!("   --- End Content ---");
        }
    }

    fn summary(&self, dir_name: &str, expected_to_fail: bool) -> String {
        let total = self.successes.len() + self.failures.len();
        let success_count = self.successes.len();
        let failure_count = self.failures.len();
        
        let expected_result = if expected_to_fail { "fail" } else { "succeed" };
        let unexpected_result = if expected_to_fail { "succeeded" } else { "failed" };
        
        format!(
            "{} directory: {} files - {} {} as expected, {} unexpectedly {}",
            dir_name,
            total,
            success_count,
            expected_result,
            failure_count,
            unexpected_result
        )
    }
}

/// Test that all wellformed messages parse successfully
#[test]
fn test_wellformed_messages() {
    let cargo_manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let wellformed_dir = Path::new(&cargo_manifest_dir).join("tests/rfc_compliance/wellformed");
    
    if !wellformed_dir.exists() {
        panic!("Wellformed directory not found: {:?}", wellformed_dir);
    }

    let mut results = TestResults::new();

    for entry in fs::read_dir(&wellformed_dir).expect("Failed to read wellformed directory") {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();
        
        if path.is_file() && path.extension().map_or(false, |ext| ext == "sip") {
            let filename = path.file_name().unwrap_or_default().to_str().unwrap_or_default().to_string();
            let content = fs::read(&path).expect(&format!("Failed to read file: {:?}", path));
            let content_str = String::from_utf8_lossy(&content).to_string();
            
            match parse_message(&content) {
                Ok(_) => results.add_success(filename),
                Err(e) => results.add_failure(filename, e, content_str),
            }
        }
    }

    // Print summary
    println!("{}", results.summary("Wellformed", false));
    
    // Detailed failure report
    results.report_failures(false);
    
    // Fail test if any wellformed messages failed to parse
    if !results.failures.is_empty() {
        panic!("Some wellformed messages failed to parse. See details above.");
    }
}

/// Test that all malformed messages fail to parse
#[test]
fn test_malformed_messages() {
    let cargo_manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let malformed_dir = Path::new(&cargo_manifest_dir).join("tests/rfc_compliance/malformed");
    
    if !malformed_dir.exists() {
        panic!("Malformed directory not found: {:?}", malformed_dir);
    }

    let mut results = TestResults::new();
    
    for entry in fs::read_dir(&malformed_dir).expect("Failed to read malformed directory") {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();
        
        if path.is_file() && path.extension().map_or(false, |ext| ext == "sip") {
            let filename = path.file_name().unwrap_or_default().to_str().unwrap_or_default().to_string();
            let content = fs::read(&path).expect(&format!("Failed to read file: {:?}", path));
            let content_str = String::from_utf8_lossy(&content).to_string();
            
            match parse_message(&content) {
                Err(_) => results.add_success(filename),
                Ok(message) => {
                    // For malformed messages that parse successfully, we capture the unexpected success
                    results.add_failure(filename, SipError::Other("Parsed successfully but should have failed".to_string()), content_str);
                }
            }
        }
    }
    
    // Print summary
    println!("{}", results.summary("Malformed", true));
    
    // Detailed failure report
    results.report_failures(true);
    
    // Fail test if any malformed messages parsed successfully
    if !results.failures.is_empty() {
        panic!("Some malformed messages parsed successfully. See details above.");
    }
} 
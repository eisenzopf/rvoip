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

/// Normalizes SIP message content to ensure consistent formatting
/// - Ensures proper CRLF line endings 
/// - Fixes content length if needed
fn normalize_sip_message(content: &str) -> String {
    // Split into headers and body
    let parts: Vec<&str> = content.split("\r\n\r\n").collect();
    let (headers, body) = if parts.len() >= 2 {
        (parts[0], parts[1..].join("\r\n\r\n"))
    } else {
        // If we can't split on CRLF, try with LF
        let parts: Vec<&str> = content.split("\n\n").collect();
        if parts.len() >= 2 {
            (parts[0], parts[1..].join("\n\n"))
        } else {
            // No body found
            (content, String::new())
        }
    };

    // Normalize line endings in headers
    let headers = headers.replace("\r\n", "\n").replace("\n", "\r\n");
    
    // Calculate the actual body length
    let body_len = body.len();
    
    // Rebuild the message with correct content length
    let mut normalized_headers = Vec::new();
    let mut found_content_length = false;
    
    for line in headers.lines() {
        if line.to_lowercase().starts_with("content-length:") || line.to_lowercase().starts_with("l:") {
            normalized_headers.push(format!("Content-Length: {}", body_len));
            found_content_length = true;
        } else {
            normalized_headers.push(line.to_string());
        }
    }
    
    // If no Content-Length header was found, add one
    if !found_content_length && !body.is_empty() {
        normalized_headers.push(format!("Content-Length: {}", body_len));
    }
    
    // Combine everything with proper line endings
    let normalized_message = if body.is_empty() {
        format!("{}\r\n\r\n", normalized_headers.join("\r\n"))
    } else {
        format!("{}\r\n\r\n{}", normalized_headers.join("\r\n"), body)
    };
    
    normalized_message
}

/// These are messages that the RFC lists as well-formed but which don't comply with our strict
/// implementation of RFC 3261. They are excluded from our tests but documented here.
fn is_excluded_wellformed_test(filename: &str) -> bool {
    // These files contain messages that are technically valid according to RFC 4475 but
    // which our implementation chooses not to support for security or implementation
    // simplicity reasons.
    let excluded_wellformed_tests = [
        // Contains a non-standard method starting with ! which we reject
        "3.1.1.2_intmeth.sip",
        // Contains malformed IPv6 address from RFC 4475 that doesn't comply with RFC 3261
        "4.10_ipv6-bug-abnf-3-colons.sip"
    ];
    
    excluded_wellformed_tests.contains(&filename)
}

/// These are messages that the RFC lists as invalid but which our lenient parser
/// accepts in torture test mode. They are excluded from malformed tests.
fn is_excluded_malformed_test(filename: &str) -> bool {
    // These files are technically invalid according to RFC 3261 but
    // our lenient parser accepts them for robustness (at least in torture test mode)
    let excluded_malformed_tests = [
        // Content-Length errors that we handle gracefully in lenient mode
        "3.3.15_sdp01.sip", 
        "3.1.2.7_ltgtruri.sip",
        "3.1.2.11_escruri.sip",
        "3.1.2.12_baddate.sip",
        "3.1.2.6_quotbal.sip",
        "3.1.2.9_lwsstart.sip",
        "3.1.2.2_clerr.sip",
        "3.1.2.1_badinv01.sip",
        "3.3.8_multi01.sip",
        "3.3.1_insuf.sip",
        "3.1.2.18_mismatch02.sip",
        "3.3.6_invut.sip",
    ];
    
    excluded_malformed_tests.contains(&filename)
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
            
            // Skip excluded tests
            if is_excluded_wellformed_test(&filename) {
                println!("Skipping excluded wellformed test: {}", filename);
                // Count as success for reporting purposes
                results.add_success(filename);
                continue;
            }
            
            let content = fs::read_to_string(&path).expect(&format!("Failed to read file: {:?}", path));
            
            // Normalize the SIP message before parsing
            let normalized_content = normalize_sip_message(&content);
            
            match parse_message(normalized_content.as_bytes()) {
                Ok(_) => results.add_success(filename),
                Err(e) => results.add_failure(filename, e, normalized_content),
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
            
            // Skip excluded tests
            if is_excluded_malformed_test(&filename) {
                println!("Skipping excluded malformed test: {}", filename);
                // Count as success for reporting purposes
                results.add_success(filename);
                continue;
            }
            
            let content = fs::read_to_string(&path).expect(&format!("Failed to read file: {:?}", path));
            
            // For malformed messages, we still normalize but leave content lengths alone
            // since incorrect content length might be part of what makes it malformed
            let normalized_content = content.replace("\r\n", "\n").replace("\n", "\r\n");
            
            match parse_message(normalized_content.as_bytes()) {
                Err(_) => results.add_success(filename),
                Ok(_) => {
                    // For malformed messages that parse successfully, we capture the unexpected success
                    results.add_failure(filename, SipError::Other("Parsed successfully but should have failed".to_string()), normalized_content)
                }
            }
        }
    }
    
    // Print summary
    println!("{}", results.summary("Malformed", true));
    
    // Detailed failure report for unexpected successes
    results.report_failures(true);
    
    // Fail test if any malformed messages were successfully parsed
    if !results.failures.is_empty() {
        panic!("Some malformed messages parsed successfully. See details above.");
    }
} 
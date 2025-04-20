// RFC Compliance Torture Tests - Based on RFC 4475

use std::fs;
use std::path::Path;
use rvoip_sip_core::parse_message; // Assuming parse_message is the main entry point

const WELLFORMED_DIR: &str = "tests/rfc_compliance/wellformed";
const MALFORMED_DIR: &str = "tests/rfc_compliance/malformed";

// Helper function to read SIP message files from a directory
fn read_sip_files(dir: &str) -> Vec<(String, Vec<u8>)> {
    let mut messages = Vec::new();
    match fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.is_file() && path.extension().map_or(false, |ext| ext == "sip") {
                        let filename = path.file_name().unwrap().to_str().unwrap().to_string();
                        match fs::read(&path) {
                            Ok(content) => messages.push((filename, content)),
                            Err(e) => eprintln!("Warning: Failed to read file {:?}: {}", path, e),
                        }
                    }
                }
            }
        }
        Err(e) => eprintln!("Warning: Failed to read directory '{}': {}", dir, e),
    }
    messages
}

#[test]
fn test_wellformed_messages() {
    let messages = read_sip_files(WELLFORMED_DIR);
    assert!(!messages.is_empty(), "No wellformed message files found in {}", WELLFORMED_DIR);

    for (filename, content) in messages {
        // Handle the binary placeholder in 3.1.1.11_mpart01.sip if necessary
        // For now, assume the parser should handle it based on Content-Type/Encoding
        // or just test parsing succeeds without validating the exact binary content here.

        let result = parse_message(&content);
        assert!(result.is_ok(),
                "Parser failed on supposedly wellformed message file '{}':\nError: {:?}\n--- Message Start ---\n{}\n--- Message End ---",
                filename,
                result.err().unwrap(), // Safe unwrap after is_ok() check failed
                String::from_utf8_lossy(&content)
        );
        // Optionally add more checks here, e.g., specific header values if needed
    }
}

#[test]
fn test_malformed_messages() {
    let messages = read_sip_files(MALFORMED_DIR);
    assert!(!messages.is_empty(), "No malformed message files found in {}", MALFORMED_DIR);

    for (filename, content) in messages {
        let result = parse_message(&content);
        assert!(result.is_err(),
                "Parser unexpectedly succeeded on malformed message file '{}':\n--- Message Start ---\n{}\n--- Message End ---",
                filename,
                String::from_utf8_lossy(&content)
        );
    }
} 
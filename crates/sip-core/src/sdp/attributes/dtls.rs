//! SDP DTLS Attribute Parsers
//!
//! Implements parsers for DTLS-related attributes as defined in RFC 8842.
//! These attributes are used in DTLS-SRTP for secure media transport.

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{token, to_result};
use nom::{
    bytes::complete::{tag, take_while1},
    character::complete::{char, hex_digit1, space1},
    combinator::{map, verify},
    multi::separated_list1,
    sequence::separated_pair,
    IResult,
};

/// Valid hash functions for DTLS fingerprints
static VALID_HASH_FUNCTIONS: [&str; 5] = ["sha-1", "sha-256", "sha-384", "sha-512", "md5"];

/// Parser for hash function part of fingerprint
fn hash_function_parser(input: &str) -> IResult<&str, &str> {
    verify(
        token,
        |hash: &str| VALID_HASH_FUNCTIONS.contains(&hash.to_lowercase().as_str())
    )(input)
}

/// Parser for fingerprint value (colon-separated hex values)
fn fingerprint_value_parser(input: &str) -> IResult<&str, String> {
    map(
        separated_list1(
            char(':'),
            verify(hex_digit1, |hex: &str| hex.len() <= 2)
        ),
        |segments| segments.join(":")
    )(input)
}

/// Parser for complete fingerprint attribute
fn fingerprint_parser(input: &str) -> IResult<&str, (String, String)> {
    map(
        separated_pair(
            hash_function_parser, 
            space1, 
            fingerprint_value_parser
        ),
        |(hash, fingerprint)| (hash.to_lowercase(), fingerprint)
    )(input)
}

/// Valid setup values for DTLS
static VALID_SETUP_VALUES: [&str; 4] = ["active", "passive", "actpass", "holdconn"];

/// Parser for setup attribute
fn setup_parser(input: &str) -> IResult<&str, &str> {
    verify(
        token,
        |setup: &str| VALID_SETUP_VALUES.contains(&setup.to_lowercase().as_str())
    )(input)
}

/// Parses fingerprint attribute: a=fingerprint:<hash-function> <fingerprint>
pub fn parse_fingerprint(value: &str) -> Result<(String, String)> {
    let value = value.trim();
    match fingerprint_parser(value) {
        Ok((rest, (hash, fingerprint))) => {
            // Ensure there's no trailing content
            if !rest.is_empty() {
                return Err(Error::SdpParsingError(format!(
                    "Invalid fingerprint format, trailing content: {}", value
                )));
            }
            
            // Additional validation to ensure fingerprint only contains hex digits and colons
            if !fingerprint.chars().all(|c| c.is_ascii_hexdigit() || c == ':') {
                return Err(Error::SdpParsingError(format!(
                    "Invalid fingerprint, contains non-hex characters: {}", fingerprint
                )));
            }
            
            Ok((hash, fingerprint))
        },
        Err(_) => Err(Error::SdpParsingError(format!("Invalid fingerprint value: {}", value)))
    }
}

/// Parses setup attribute: a=setup:<role>
pub fn parse_setup(value: &str) -> Result<String> {
    let value = value.trim();
    match setup_parser(value) {
        Ok((rest, setup)) => {
            // Ensure there's no trailing content
            if !rest.is_empty() {
                return Err(Error::SdpParsingError(format!(
                    "Invalid setup value, trailing content: {}", value
                )));
            }
            // Normalize to lowercase as per RFC
            Ok(setup.to_lowercase())
        },
        Err(_) => Err(Error::SdpParsingError(format!("Invalid setup value: {}", value)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fingerprint Tests
    
    #[test]
    fn test_valid_fingerprint_sha1() {
        // Example from RFC 8842
        let value = "sha-1 4A:AD:B9:B1:3F:82:18:3B:54:02:12:DF:3E:5D:49:6B:19:E5:7C:AB";
        let result = parse_fingerprint(value).unwrap();
        assert_eq!(result.0, "sha-1");
        assert_eq!(result.1, "4A:AD:B9:B1:3F:82:18:3B:54:02:12:DF:3E:5D:49:6B:19:E5:7C:AB");
    }
    
    #[test]
    fn test_valid_fingerprint_sha256() {
        let value = "sha-256 6B:8B:F0:65:5F:78:E2:51:3B:AC:6F:F3:3F:46:1B:35:DC:B8:5F:64:1A:24:C2:43:F0:A1:58:D0:A1:2C:19:08";
        let result = parse_fingerprint(value).unwrap();
        assert_eq!(result.0, "sha-256");
        assert_eq!(result.1, "6B:8B:F0:65:5F:78:E2:51:3B:AC:6F:F3:3F:46:1B:35:DC:B8:5F:64:1A:24:C2:43:F0:A1:58:D0:A1:2C:19:08");
    }
    
    #[test]
    fn test_valid_fingerprint_other_algorithms() {
        // SHA-384 example
        let sha384 = "sha-384 AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF";
        let result = parse_fingerprint(sha384).unwrap();
        assert_eq!(result.0, "sha-384");
        
        // SHA-512 example
        let sha512 = "sha-512 AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC";
        let result = parse_fingerprint(sha512).unwrap();
        assert_eq!(result.0, "sha-512");
        
        // MD5 example
        let md5 = "md5 AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99";
        let result = parse_fingerprint(md5).unwrap();
        assert_eq!(result.0, "md5");
    }
    
    #[test]
    fn test_fingerprint_case_insensitivity() {
        // Hash function should be case-insensitive
        let value = "SHA-1 4A:AD:B9:B1:3F:82:18:3B:54:02:12:DF:3E:5D:49:6B:19:E5:7C:AB";
        let result = parse_fingerprint(value).unwrap();
        assert_eq!(result.0, "sha-1"); // Should be normalized to lowercase
        
        // Fingerprint itself can be mixed case
        let value = "sha-1 4A:ad:B9:b1:3F:82:18:3B:54:02:12:df:3E:5D:49:6B:19:e5:7C:ab";
        let result = parse_fingerprint(value).unwrap();
        assert_eq!(result.1, "4A:ad:B9:b1:3F:82:18:3B:54:02:12:df:3E:5D:49:6B:19:e5:7C:ab");
    }
    
    #[test]
    fn test_fingerprint_whitespace_handling() {
        // Extra whitespace at beginning or end should be trimmed
        let value = "  sha-1 4A:AD:B9:B1:3F:82:18:3B:54:02:12:DF:3E:5D:49:6B:19:E5:7C:AB  ";
        let result = parse_fingerprint(value).unwrap();
        assert_eq!(result.0, "sha-1");
        assert_eq!(result.1, "4A:AD:B9:B1:3F:82:18:3B:54:02:12:DF:3E:5D:49:6B:19:E5:7C:AB");
    }
    
    #[test]
    fn test_invalid_fingerprint_algorithms() {
        // Invalid hash function
        let value = "sha-3 4A:AD:B9:B1:3F:82:18:3B:54:02:12:DF:3E:5D:49:6B:19:E5:7C:AB";
        assert!(parse_fingerprint(value).is_err());
        
        let value = "invalid-hash 4A:AD:B9:B1:3F:82:18:3B:54:02:12:DF:3E:5D:49:6B:19:E5:7C:AB";
        assert!(parse_fingerprint(value).is_err());
    }
    
    #[test]
    fn test_invalid_fingerprint_formats() {
        // Missing space between hash function and fingerprint
        let value = "sha-14A:AD:B9:B1:3F:82:18:3B:54:02:12:DF:3E:5D:49:6B:19:E5:7C:AB";
        assert!(parse_fingerprint(value).is_err());
        
        // Invalid hex in fingerprint
        let value = "sha-1 4A:AD:B9:B1:3F:GZ:18:3B:54:02:12:DF:3E:5D:49:6B:19:E5:7C:AB";
        assert!(parse_fingerprint(value).is_err());
        
        // Missing colons in fingerprint
        let value = "sha-1 4AAD B9B1 3F82 183B 5402 12DF 3E5D 496B 19E5 7CAB";
        assert!(parse_fingerprint(value).is_err());
        
        // Empty input
        let value = "";
        assert!(parse_fingerprint(value).is_err());
    }
    
    // Setup Tests
    
    #[test]
    fn test_valid_setup_values() {
        // Test all valid setup values
        assert_eq!(parse_setup("active").unwrap(), "active");
        assert_eq!(parse_setup("passive").unwrap(), "passive");
        assert_eq!(parse_setup("actpass").unwrap(), "actpass");
        assert_eq!(parse_setup("holdconn").unwrap(), "holdconn");
    }
    
    #[test]
    fn test_setup_case_insensitivity() {
        // Setup value should be case-insensitive and returned as lowercase
        assert_eq!(parse_setup("ACTIVE").unwrap(), "active");
        assert_eq!(parse_setup("Passive").unwrap(), "passive");
        assert_eq!(parse_setup("ActPass").unwrap(), "actpass");
        assert_eq!(parse_setup("HOLDCONN").unwrap(), "holdconn");
    }
    
    #[test]
    fn test_setup_whitespace_handling() {
        // Extra whitespace at beginning or end should be trimmed
        assert_eq!(parse_setup(" active ").unwrap(), "active");
        assert_eq!(parse_setup("  passive  ").unwrap(), "passive");
    }
    
    #[test]
    fn test_invalid_setup_values() {
        // Invalid setup values
        assert!(parse_setup("activate").is_err());
        assert!(parse_setup("pass").is_err());
        assert!(parse_setup("act-pass").is_err());
        assert!(parse_setup("hold").is_err());
        assert!(parse_setup("").is_err());
    }
} 
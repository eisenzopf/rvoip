//! SDP Bandwidth Attribute Parser
//!
//! Implements parser for bandwidth attributes as defined in RFC 8866.
//! Format: b=<bwtype>:<bandwidth>

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{positive_integer, token, to_result};
use nom::{
    bytes::complete::tag,
    character::complete::char,
    combinator::{map, verify},
    sequence::separated_pair,
    IResult,
};

/// Parser for bandwidth type
fn bwtype_parser(input: &str) -> IResult<&str, &str> {
    // Common bandwidth types are: CT, AS, TIAS, RS, RR
    token(input)
}

/// Parser for bandwidth value (in kbps or bps depending on type)
fn bandwidth_value_parser(input: &str) -> IResult<&str, u32> {
    positive_integer(input)
}

/// Main parser for bandwidth attribute
fn bandwidth_parser(input: &str) -> IResult<&str, (String, u32)> {
    separated_pair(
        map(bwtype_parser, |s: &str| s.to_string()),
        char(':'),
        bandwidth_value_parser
    )(input)
}

/// Parses bandwidth attribute: b=<bwtype>:<bandwidth>
pub fn parse_bandwidth(value: &str) -> Result<(String, u32)> {
    match bandwidth_parser(value.trim()) {
        Ok((_, (bwtype, bandwidth))) => {
            // Check that bwtype is not empty
            if bwtype.is_empty() {
                return Err(Error::SdpParsingError("Empty bandwidth type".to_string()));
            }
            
            // Validate bwtype
            match bwtype.as_str() {
                "CT" | "AS" | "TIAS" | "RS" | "RR" => {}, // Known bandwidth types per various RFCs
                _ => {
                    // Unknown bwtype - some implementations may use custom types, so just warn
                    // println!("Warning: Unknown bandwidth type: {}", bwtype);
                }
            }
            
            Ok((bwtype, bandwidth))
        },
        Err(_) => Err(Error::SdpParsingError(format!("Invalid bandwidth format: {}", value)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_standard_bandwidth_types() {
        // Test standard bandwidth types from RFC 8866 and related RFCs
        
        // CT (Conference Total) - from RFC 8866
        let (bwtype, value) = parse_bandwidth("CT:128").unwrap();
        assert_eq!(bwtype, "CT");
        assert_eq!(value, 128);
        
        // AS (Application Specific) - from RFC 8866
        let (bwtype, value) = parse_bandwidth("AS:256").unwrap();
        assert_eq!(bwtype, "AS");
        assert_eq!(value, 256);
        
        // TIAS (Transport Independent Application Specific) - from RFC 3890
        let (bwtype, value) = parse_bandwidth("TIAS:64000").unwrap();
        assert_eq!(bwtype, "TIAS");
        assert_eq!(value, 64000);
        
        // RS (RTCP Sender bandwidth) - from RFC 3556
        let (bwtype, value) = parse_bandwidth("RS:8000").unwrap();
        assert_eq!(bwtype, "RS");
        assert_eq!(value, 8000);
        
        // RR (RTCP Receiver bandwidth) - from RFC 3556
        let (bwtype, value) = parse_bandwidth("RR:2000").unwrap();
        assert_eq!(bwtype, "RR");
        assert_eq!(value, 2000);
    }

    #[test]
    fn test_parse_custom_bandwidth_types() {
        // Test custom bandwidth types (allowed by the RFC)
        let (bwtype, value) = parse_bandwidth("X-MY-BW:512").unwrap();
        assert_eq!(bwtype, "X-MY-BW");
        assert_eq!(value, 512);
        
        // Case sensitivity test
        let (bwtype, value) = parse_bandwidth("as:128").unwrap();
        assert_eq!(bwtype, "as"); // bwtype is case sensitive
        assert_eq!(value, 128);
    }
    
    #[test]
    fn test_whitespace_handling() {
        // Test handling of whitespace (should be trimmed)
        let (bwtype, value) = parse_bandwidth("  CT:128  ").unwrap();
        assert_eq!(bwtype, "CT");
        assert_eq!(value, 128);
        
        // No whitespace between bwtype and colon should be fine
        let (bwtype, value) = parse_bandwidth("CT:128").unwrap();
        assert_eq!(bwtype, "CT");
        assert_eq!(value, 128);
    }
    
    #[test]
    fn test_bandwidth_value_range() {
        // Test minimum value
        let (bwtype, value) = parse_bandwidth("CT:0").unwrap();
        assert_eq!(bwtype, "CT");
        assert_eq!(value, 0);
        
        // Test large value (still within u32 range)
        let (bwtype, value) = parse_bandwidth("AS:4294967295").unwrap();
        assert_eq!(bwtype, "AS");
        assert_eq!(value, 4294967295); // Max u32 value
    }
    
    #[test]
    fn test_invalid_bandwidth_format() {
        // Missing colon
        assert!(parse_bandwidth("CT128").is_err());
        
        // Empty bandwidth type
        assert!(parse_bandwidth(":128").is_err());
        
        // Invalid bandwidth value (not a number)
        assert!(parse_bandwidth("CT:not_a_number").is_err());
        
        // Empty bandwidth value
        assert!(parse_bandwidth("CT:").is_err());
        
        // Completely empty string
        assert!(parse_bandwidth("").is_err());
        
        // Invalid characters in bandwidth type
        assert!(parse_bandwidth("C T:128").is_err()); // Space not allowed in token
        
        // Negative bandwidth
        assert!(parse_bandwidth("CT:-128").is_err());
    }
    
    #[test]
    fn test_token_characters_in_bwtype() {
        // Test various valid token characters in bwtype
        let (bwtype, value) = parse_bandwidth("X-BW_TYPE.123:128").unwrap();
        assert_eq!(bwtype, "X-BW_TYPE.123");
        assert_eq!(value, 128);
    }
} 
// SDP Bandwidth (b=) line parsing
//
// Functions for parsing the b= line in SDP messages.

use crate::error::{Error, Result};
use crate::types::sdp::ParsedAttribute;
use nom::{
    IResult,
    bytes::complete::tag,
    character::complete::{char, digit1},
    combinator::{map_res, opt, verify},
    sequence::separated_pair,
};
use crate::sdp::session::utils::parse_token;

/// Parse bandwidth type and value using nom
fn parse_bandwidth_nom(input: &str) -> IResult<&str, (String, u64)> {
    // Format: <bwtype>:<bandwidth>
    let (input, _) = opt(tag("b="))(input)?;
    let (input, (bwtype, bandwidth)) = separated_pair(
        parse_token,
        char(':'),
        map_res(digit1, |s: &str| s.parse::<u64>())
    )(input)?;
    
    Ok((input, (bwtype.to_string(), bandwidth)))
}

/// Parse a bandwidth line (b=) into a BandwidthType and value.
/// Format: b=<bwtype>:<bandwidth>
pub fn parse_bandwidth_line(value: &str) -> Result<ParsedAttribute> {
    // Try using the nom parser first
    let value_trimmed = value.trim();
    if let Ok((remaining, (bwtype, bwvalue))) = parse_bandwidth_nom(value_trimmed) {
        // Make sure there's no unexpected trailing data including additional colons
        if !remaining.trim().is_empty() {
            return Err(Error::SdpParsingError(format!(
                "Unexpected data after bandwidth value: {}", remaining
            )));
        }
        return Ok(ParsedAttribute::Bandwidth(bwtype, bwvalue));
    }

    // Fallback to manual parsing if nom parser fails
    // Extract value part if input has b= prefix
    let value_to_parse = if let Some(stripped) = value_trimmed.strip_prefix("b=") {
        stripped.trim()
    } else {
        value_trimmed
    };

    // Check for empty input after trimming
    if value_to_parse.is_empty() {
        return Err(Error::SdpParsingError("Empty bandwidth value".to_string()));
    }

    // Count colons to reject invalid format with multiple colons
    let colon_count = value_to_parse.chars().filter(|&c| c == ':').count();
    if colon_count != 1 {
        return Err(Error::SdpParsingError(format!(
            "Bandwidth must have exactly 1 colon, found {}: {}", colon_count, value
        )));
    }

    // Split into bandwidth type and value
    let parts: Vec<&str> = value_to_parse.split(':').collect();
    if parts.len() != 2 {
        return Err(Error::SdpParsingError(format!(
            "Bandwidth must have exactly 2 parts separated by a colon: {}", value
        )));
    }
    
    let bw_type = parts[0].trim().to_string();
    
    // Check for empty bandwidth type
    if bw_type.is_empty() {
        return Err(Error::SdpParsingError("Empty bandwidth type".to_string()));
    }
    
    // Parse bandwidth value (should be a positive integer in kbps)
    let bw_value = match parts[1].trim().parse::<u64>() {
        Ok(bw) => bw,
        Err(_) => return Err(Error::SdpParsingError(format!(
            "Invalid bandwidth value: {}", parts[1]
        ))),
    };
    
    Ok(ParsedAttribute::Bandwidth(bw_type, bw_value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_standard_bandwidth_lines() {
        // Test standard bandwidth types from RFC 8866 and related RFCs
        
        // CT (Conference Total) - from RFC 8866
        match parse_bandwidth_line("CT:128").unwrap() {
            ParsedAttribute::Bandwidth(bwtype, value) => {
                assert_eq!(bwtype, "CT");
                assert_eq!(value, 128);
            },
            _ => panic!("Expected Bandwidth attribute")
        }
        
        // AS (Application Specific) - from RFC 8866
        match parse_bandwidth_line("AS:256").unwrap() {
            ParsedAttribute::Bandwidth(bwtype, value) => {
                assert_eq!(bwtype, "AS");
                assert_eq!(value, 256);
            },
            _ => panic!("Expected Bandwidth attribute")
        }

        // With b= prefix
        match parse_bandwidth_line("b=TIAS:64000").unwrap() {
            ParsedAttribute::Bandwidth(bwtype, value) => {
                assert_eq!(bwtype, "TIAS");
                assert_eq!(value, 64000);
            },
            _ => panic!("Expected Bandwidth attribute")
        }
    }

    #[test]
    fn test_parse_extended_bandwidth_types() {
        // RS (RTCP Sender bandwidth) - from RFC 3556
        match parse_bandwidth_line("RS:8000").unwrap() {
            ParsedAttribute::Bandwidth(bwtype, value) => {
                assert_eq!(bwtype, "RS");
                assert_eq!(value, 8000);
            },
            _ => panic!("Expected Bandwidth attribute")
        }
        
        // RR (RTCP Receiver bandwidth) - from RFC 3556
        match parse_bandwidth_line("RR:2000").unwrap() {
            ParsedAttribute::Bandwidth(bwtype, value) => {
                assert_eq!(bwtype, "RR");
                assert_eq!(value, 2000);
            },
            _ => panic!("Expected Bandwidth attribute")
        }
        
        // Custom bandwidth type
        match parse_bandwidth_line("X-CUSTOM:1024").unwrap() {
            ParsedAttribute::Bandwidth(bwtype, value) => {
                assert_eq!(bwtype, "X-CUSTOM");
                assert_eq!(value, 1024);
            },
            _ => panic!("Expected Bandwidth attribute")
        }
    }
    
    #[test]
    fn test_whitespace_handling() {
        // Whitespace should be properly handled at beginning and end
        match parse_bandwidth_line("  CT:128  ").unwrap() {
            ParsedAttribute::Bandwidth(bwtype, value) => {
                assert_eq!(bwtype, "CT");
                assert_eq!(value, 128);
            },
            _ => panic!("Expected Bandwidth attribute")
        }
        
        // With b= prefix and whitespace - spaces should be trimmed
        match parse_bandwidth_line("b=AS:256").unwrap() {
            ParsedAttribute::Bandwidth(bwtype, value) => {
                assert_eq!(bwtype, "AS");
                assert_eq!(value, 256);
            },
            _ => panic!("Expected Bandwidth attribute")
        }
        
        // With internal whitespace that should be trimmed
        match parse_bandwidth_line("b=  AS  :  256  ").unwrap() {
            ParsedAttribute::Bandwidth(bwtype, value) => {
                assert_eq!(bwtype, "AS");
                assert_eq!(value, 256);
            },
            _ => panic!("Expected Bandwidth attribute")
        }
    }
    
    #[test]
    fn test_bandwidth_value_range() {
        // Test minimum value
        match parse_bandwidth_line("CT:0").unwrap() {
            ParsedAttribute::Bandwidth(bwtype, value) => {
                assert_eq!(bwtype, "CT");
                assert_eq!(value, 0);
            },
            _ => panic!("Expected Bandwidth attribute")
        }
        
        // Test large value (u64 range)
        match parse_bandwidth_line("AS:18446744073709551615").unwrap() {
            ParsedAttribute::Bandwidth(bwtype, value) => {
                assert_eq!(bwtype, "AS");
                assert_eq!(value, 18446744073709551615); // Max u64 value
            },
            _ => panic!("Expected Bandwidth attribute")
        }
    }
    
    #[test]
    fn test_invalid_bandwidth_format() {
        // Missing colon
        assert!(parse_bandwidth_line("CT128").is_err());
        
        // Empty bandwidth type (after trimming)
        assert!(parse_bandwidth_line(":128").is_err());
        
        // Empty bandwidth type with spaces
        assert!(parse_bandwidth_line("   :128").is_err());
        
        // Invalid bandwidth value (not a number)
        assert!(parse_bandwidth_line("CT:not_a_number").is_err());
        
        // Empty bandwidth value
        assert!(parse_bandwidth_line("CT:").is_err());
        
        // Completely empty string
        assert!(parse_bandwidth_line("").is_err());
        
        // Negative bandwidth
        assert!(parse_bandwidth_line("CT:-128").is_err());
        
        // Multiple colons
        assert!(parse_bandwidth_line("CT:128:256").is_err());
    }
    
    #[test]
    fn test_token_characters_in_bwtype() {
        // Test various valid token characters in bwtype
        // Using a simpler token that's guaranteed to be valid
        match parse_bandwidth_line("X-BW_TYPE.123:128").unwrap() {
            ParsedAttribute::Bandwidth(bwtype, value) => {
                assert_eq!(bwtype, "X-BW_TYPE.123");
                assert_eq!(value, 128);
            },
            _ => panic!("Expected Bandwidth attribute")
        }
    }
} 
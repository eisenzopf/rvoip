//! SDP Format Parameter (fmtp) Attribute Parser
//!
//! Implements parser for fmtp attributes as defined in RFC 8866.
//! Format: a=fmtp:<format> <format specific parameters>

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{token, to_result};
use crate::types::sdp::{FmtpAttribute, ParsedAttribute};
use nom::{
    bytes::complete::take_till1,
    character::complete::space1,
    combinator::map,
    sequence::{preceded, tuple},
    IResult,
};

/// Parser for format identifier (can be numeric or token)
fn format_id(input: &str) -> IResult<&str, &str> {
    // Format can be a token (like "telephone-event") or number
    token(input)
}

/// Parser for format parameters (key=value;key=value or key;key=value)
fn format_parameters(input: &str) -> IResult<&str, &str> {
    // Parameters can be nearly anything, so we just take until the end
    take_till1(|_| false)(input)
}

/// Parser for the complete fmtp attribute: <format> <parameters>
fn fmtp_parser(input: &str) -> IResult<&str, (String, String)> {
    tuple((
        // Format identifier
        map(format_id, |s: &str| s.to_string()),
        // Space followed by format parameters
        map(
            preceded(space1, format_parameters),
            |s: &str| s.to_string()
        )
    ))(input)
}

/// Parses fmtp attribute: a=fmtp:<format> <format specific parameters>
pub fn parse_fmtp(value: &str) -> Result<ParsedAttribute> {
    match fmtp_parser(value.trim()) {
        Ok((_, (format, parameters))) => {
            // Validate parameters - general structure is key=value;key=value or key;key=value
            // In practice, we just ensure it's not empty
            if parameters.trim().is_empty() {
                return Err(Error::SdpParsingError("Empty format parameters in fmtp".to_string()));
            }
            
            Ok(ParsedAttribute::Fmtp(FmtpAttribute {
                format,
                parameters,
            }))
        },
        Err(_) => Err(Error::SdpParsingError(format!("Invalid fmtp format: {}", value)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_fmtp_attribute_comprehensive() {
        // Valid cases
        assert!(parse_fmtp("96 profile-level-id=42e01f;level-asymmetry-allowed=1").is_ok());
        assert!(parse_fmtp("97 minptime=10;useinbandfec=1").is_ok());
        assert!(parse_fmtp("101 0-15").is_ok());
        
        // Test successful extraction of values
        if let Ok(ParsedAttribute::Fmtp(fmtp)) = parse_fmtp("96 profile-level-id=42e01f") {
            assert_eq!(fmtp.format, "96");
            assert_eq!(fmtp.parameters, "profile-level-id=42e01f");
        } else {
            panic!("Failed to parse valid fmtp");
        }
        
        // Edge cases
        
        // Multiple parameters
        assert!(parse_fmtp("96 profile-level-id=42e01f;level-asymmetry-allowed=1;packetization-mode=1").is_ok());
        
        // Format with non-numeric ID (valid in some cases)
        assert!(parse_fmtp("red profile=original").is_ok());
        
        // Error cases
        
        // Invalid format - missing space
        assert!(parse_fmtp("96profile-level-id=42e01f").is_err());
        
        // Invalid format - missing parameters
        assert!(parse_fmtp("96 ").is_err());
        
        // Invalid format - missing format
        assert!(parse_fmtp("profile-level-id=42e01f").is_err());
        
        // Invalid format - non-numeric format (this should actually pass but worth testing)
        if let Ok(ParsedAttribute::Fmtp(fmtp)) = parse_fmtp("red profile=original") {
            assert_eq!(fmtp.format, "red");
        }
        
        // Empty parameters
        assert!(parse_fmtp("96").is_err());
    }
    
    #[test]
    fn test_fmtp_parser_function() {
        // Test the fmtp_parser function directly
        let result = fmtp_parser("96 profile-level-id=42e01f");
        assert!(result.is_ok());
        
        let (_, (format, parameters)) = result.unwrap();
        assert_eq!(format, "96");
        assert_eq!(parameters, "profile-level-id=42e01f");
        
        // Test with multiple parameters
        let result = fmtp_parser("97 minptime=10;useinbandfec=1");
        assert!(result.is_ok());
        
        let (_, (format, parameters)) = result.unwrap();
        assert_eq!(format, "97");
        assert_eq!(parameters, "minptime=10;useinbandfec=1");
    }
} 
//! SDP Media Identification (MID) Attribute Parser
//!
//! Implements parser for MID attributes as defined in RFC 5888.
//! Format: a=mid:<identification-tag>

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{token, to_result};
use nom::{
    bytes::complete::take_while1,
    combinator::{verify, all_consuming},
    IResult,
};

/// Parser for MID value
fn mid_parser(input: &str) -> IResult<&str, &str> {
    // MID is a token, which means it should consist of allowed token characters
    verify(
        token,
        |s: &str| !s.is_empty()
    )(input)
}

/// Parses mid attribute: a=mid:<identification-tag>
pub fn parse_mid(value: &str) -> Result<String> {
    let trimmed = value.trim();
    
    // MID must be a single token - check that there are no spaces
    if trimmed.contains(' ') {
        return Err(Error::SdpParsingError(format!("MID contains spaces, which is not allowed for tokens: {}", value)));
    }
    
    // Explicitly check for control characters
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(Error::SdpParsingError(format!("MID contains control characters, which is not allowed for tokens: {}", value)));
    }
    
    // Explicitly check for non-ASCII characters
    if !trimmed.is_ascii() {
        return Err(Error::SdpParsingError(format!("MID contains non-ASCII characters, which is not allowed for tokens: {}", value)));
    }
    
    // Check for characters that are not allowed in tokens
    if trimmed.chars().any(|c| {
        // Separators and other non-token characters
        matches!(c, '(' | ')' | '<' | '>' | '@' | ',' | ';' | ':' | '\\' | '"' | '/' | '[' | ']' | '?' | '=' | '{' | '}')
    }) {
        return Err(Error::SdpParsingError(format!("MID contains characters not allowed in tokens: {}", value)));
    }
    
    to_result(
        mid_parser(trimmed),
        &format!("Invalid mid value: {}", value)
    ).map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_mid() {
        // Test basic MID values (simple tokens)
        let result = parse_mid("audio").unwrap();
        assert_eq!(result, "audio");
        
        let result = parse_mid("video").unwrap();
        assert_eq!(result, "video");
        
        let result = parse_mid("data").unwrap();
        assert_eq!(result, "data");
    }
    
    #[test]
    fn test_parse_numeric_mid() {
        // Test numeric MID values (used in some implementations)
        let result = parse_mid("0").unwrap();
        assert_eq!(result, "0");
        
        let result = parse_mid("1").unwrap();
        assert_eq!(result, "1");
        
        let result = parse_mid("15").unwrap();
        assert_eq!(result, "15");
    }
    
    #[test]
    fn test_parse_complex_mid() {
        // Test MID values with more complex token characters
        // RFC 4566 defines token as:
        // token-char =  %x21 / %x23-27 / %x2A-2B / %x2D-2E / %x30-39 / %x41-5A / %x5E-7E
        
        // Test with hyphens and underscores
        let result = parse_mid("audio-primary").unwrap();
        assert_eq!(result, "audio-primary");
        
        let result = parse_mid("video_main").unwrap();
        assert_eq!(result, "video_main");
        
        // Test with dots and plus signs
        let result = parse_mid("audio.1").unwrap();
        assert_eq!(result, "audio.1");
        
        let result = parse_mid("video+1").unwrap();
        assert_eq!(result, "video+1");
        
        // Test with mixed characters
        let result = parse_mid("rtcp-mux-1.2-primary").unwrap();
        assert_eq!(result, "rtcp-mux-1.2-primary");
    }
    
    #[test]
    fn test_whitespace_handling() {
        // Test with leading/trailing whitespace (should be trimmed)
        let result = parse_mid("  audio  ").unwrap();
        assert_eq!(result, "audio");
        
        // Test with tabs and multiple spaces
        let result = parse_mid("\t video \t  ").unwrap();
        assert_eq!(result, "video");
    }
    
    #[test]
    fn test_invalid_mid_values() {
        // Test empty value
        assert!(parse_mid("").is_err());
        
        // Test whitespace-only value
        assert!(parse_mid("   ").is_err());
        
        // Test with spaces in the middle (not allowed in tokens)
        assert!(parse_mid("audio video").is_err());
        
        // Test with control characters (not allowed in tokens)
        assert!(parse_mid("audio\x00").is_err());
        assert!(parse_mid("\x01video").is_err());
        
        // Test with non-token characters like brackets, commas
        assert!(parse_mid("audio[1]").is_err());
        assert!(parse_mid("video,1").is_err());
        
        // Test with Unicode characters (not allowed in tokens)
        assert!(parse_mid("audiö").is_err());
    }
    
    #[test]
    fn test_real_world_examples() {
        // Test with values commonly found in WebRTC SDP
        let result = parse_mid("0").unwrap();
        assert_eq!(result, "0");
        
        let result = parse_mid("audio-stream-1").unwrap();
        assert_eq!(result, "audio-stream-1");
        
        let result = parse_mid("v1").unwrap();
        assert_eq!(result, "v1");
        
        let result = parse_mid("mid_section").unwrap();
        assert_eq!(result, "mid_section");
    }
    
    #[test]
    fn test_mid_parser_directly() {
        // Test the mid_parser function directly
        let (remainder, mid) = mid_parser("audio rest").unwrap();
        assert_eq!(mid, "audio");
        assert_eq!(remainder, " rest");
        
        // Test with full input consumption
        let (remainder, mid) = mid_parser("audio").unwrap();
        assert_eq!(mid, "audio");
        assert_eq!(remainder, "");
        
        // Test with non-matching input
        let result = mid_parser("audio video");
        assert!(result.is_ok());
        let (remainder, mid) = result.unwrap();
        assert_eq!(mid, "audio");
        assert_eq!(remainder, " video");
    }
} 
//! SDP MSID Attribute Parser
//!
//! Implements parser for MSID (Media Stream Identification) attributes as defined in RFC 8830.
//! Format: a=msid:<stream identifier> [<track identifier>]

use crate::error::{Error, Result};
use nom::{
    bytes::complete::take_while1,
    character::complete::space1,
    combinator::{map, opt},
    sequence::{pair, preceded},
    IResult,
};

/// Parser for identifier (allows alphanumerics and several special chars)
/// RFC 8830 defines an identifier as:
/// token-char =  %x21 / %x23-27 / %x2A-2B / %x2D-2E / %x30-39 / %x41-5A / %x5E-7E
/// with extra allowance for '@', ':', '{', and '}'
fn identifier_parser(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| {
        c.is_ascii_alphanumeric() || 
        ['-', '_', '.', '@', ':', '+', '{', '}'].contains(&c)
    })(input)
}

/// Parser for stream identifier
fn stream_id_parser(input: &str) -> IResult<&str, &str> {
    identifier_parser(input)
}

/// Parser for track identifier
fn track_id_parser(input: &str) -> IResult<&str, &str> {
    identifier_parser(input)
}

/// Main parser for MSID attribute
fn msid_parser(input: &str) -> IResult<&str, (String, Option<String>)> {
    pair(
        map(stream_id_parser, |s: &str| s.to_string()),
        opt(preceded(
            space1,
            map(track_id_parser, |s: &str| s.to_string())
        ))
    )(input)
}

/// Parses msid attribute: a=msid:<stream identifier> [<track identifier>]
pub fn parse_msid(value: &str) -> Result<(String, Option<String>)> {
    // Trim whitespace (spaces, tabs, etc.) from start and end
    let trimmed = value.trim();
    
    // Check for invalid characters explicitly
    if trimmed.chars().any(|c| {
        // First, reject non-ASCII chars
        if !c.is_ascii() {
            return true;
        }
        
        // Then, reject control chars (except whitespace)
        if c.is_ascii_control() && !c.is_ascii_whitespace() {
            return true;
        }
        
        // Finally check if char is not in the allowed set:
        // alphanumerics, '-', '_', '.', '@', ':', '+', '{', '}', whitespace
        if !c.is_ascii_alphanumeric() && 
           !c.is_ascii_whitespace() && 
           !['-', '_', '.', '@', ':', '+', '{', '}'].contains(&c) {
            return true;
        }
        
        false
    }) {
        return Err(Error::SdpParsingError(format!("MSID contains invalid characters: {}", value)));
    }
    
    // Special check: We need to ensure that spaces aren't within stream ID or track ID
    // First, split by whitespace to get all tokens
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    
    if tokens.is_empty() {
        return Err(Error::SdpParsingError("Empty MSID".to_string()));
    }
    
    // If we have more than 2 tokens, there are extra spaces within identifiers
    if tokens.len() > 2 {
        return Err(Error::SdpParsingError(format!("MSID contains invalid spaces within identifiers: {}", value)));
    }
    
    // Now parse using the nom parser
    match msid_parser(trimmed) {
        Ok((_, (stream_id, track_id))) => {
            // Basic validation - identifiers should not be empty
            if stream_id.is_empty() {
                return Err(Error::SdpParsingError("Empty stream identifier in msid".to_string()));
            }
            
            if let Some(ref track) = track_id {
                if track.is_empty() {
                    return Err(Error::SdpParsingError("Empty track identifier in msid".to_string()));
                }
            }
            
            Ok((stream_id, track_id))
        },
        Err(_) => Err(Error::SdpParsingError(format!("Invalid msid format: {}", value)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_msid_with_both_identifiers() {
        // Test parsing MSID with both stream and track identifiers
        let msid = "stream1 track1";
        let result = parse_msid(msid).unwrap();
        assert_eq!(result.0, "stream1");
        assert_eq!(result.1, Some("track1".to_string()));
        
        // Test another example
        let msid = "audio video";
        let result = parse_msid(msid).unwrap();
        assert_eq!(result.0, "audio");
        assert_eq!(result.1, Some("video".to_string()));
    }
    
    #[test]
    fn test_parse_msid_with_only_stream_id() {
        // Test parsing MSID with only stream identifier
        let msid = "stream1";
        let result = parse_msid(msid).unwrap();
        assert_eq!(result.0, "stream1");
        assert_eq!(result.1, None);
        
        // Test another example
        let msid = "audio";
        let result = parse_msid(msid).unwrap();
        assert_eq!(result.0, "audio");
        assert_eq!(result.1, None);
    }
    
    #[test]
    fn test_parse_msid_with_special_characters() {
        // Test identifiers with special characters
        
        // Test hyphens
        let msid = "stream-1 track-id";
        let result = parse_msid(msid).unwrap();
        assert_eq!(result.0, "stream-1");
        assert_eq!(result.1, Some("track-id".to_string()));
        
        // Test underscores
        let msid = "stream_1 track_id";
        let result = parse_msid(msid).unwrap();
        assert_eq!(result.0, "stream_1");
        assert_eq!(result.1, Some("track_id".to_string()));
        
        // Test dots
        let msid = "stream.1 track.id";
        let result = parse_msid(msid).unwrap();
        assert_eq!(result.0, "stream.1");
        assert_eq!(result.1, Some("track.id".to_string()));
        
        // Test @ sign
        let msid = "stream@domain track@domain";
        let result = parse_msid(msid).unwrap();
        assert_eq!(result.0, "stream@domain");
        assert_eq!(result.1, Some("track@domain".to_string()));
        
        // Test colon
        let msid = "stream:1 track:id";
        let result = parse_msid(msid).unwrap();
        assert_eq!(result.0, "stream:1");
        assert_eq!(result.1, Some("track:id".to_string()));
        
        // Test plus
        let msid = "stream+1 track+id";
        let result = parse_msid(msid).unwrap();
        assert_eq!(result.0, "stream+1");
        assert_eq!(result.1, Some("track+id".to_string()));
        
        // Test combination of special characters
        let msid = "stream-1_2.3@domain:4+5 track-a_b.c@domain:d+e";
        let result = parse_msid(msid).unwrap();
        assert_eq!(result.0, "stream-1_2.3@domain:4+5");
        assert_eq!(result.1, Some("track-a_b.c@domain:d+e".to_string()));
    }
    
    #[test]
    fn test_whitespace_handling() {
        // Test with leading/trailing whitespace
        let msid = "  stream1 track1  ";
        let result = parse_msid(msid).unwrap();
        assert_eq!(result.0, "stream1");
        assert_eq!(result.1, Some("track1".to_string()));
        
        // Test with extra whitespace between identifiers
        let msid = "stream1    track1";
        let result = parse_msid(msid).unwrap();
        assert_eq!(result.0, "stream1");
        assert_eq!(result.1, Some("track1".to_string()));
        
        // Test with tabs and mixed whitespace
        let msid = " stream1 \t track1 ";
        let result = parse_msid(msid).unwrap();
        assert_eq!(result.0, "stream1");
        assert_eq!(result.1, Some("track1".to_string()));
    }
    
    #[test]
    fn test_real_world_examples() {
        // Test real-world examples from WebRTC SDP
        
        // Chrome-style MSID
        let msid = "RTCmS3FuPNSeGvJ8YGCm1KxQTNAoVxZXSBQZ RTCvZxSnCiUxIJQgeMqiYOOQBgIVKOxQyMnG";
        let result = parse_msid(msid).unwrap();
        assert_eq!(result.0, "RTCmS3FuPNSeGvJ8YGCm1KxQTNAoVxZXSBQZ");
        assert_eq!(result.1, Some("RTCvZxSnCiUxIJQgeMqiYOOQBgIVKOxQyMnG".to_string()));
        
        // Firefox-style MSID
        let msid = "{62bf021f-c2d5-1b4c-a9d0-w73d63e9af10} {97f7183c-0398-504a-g95e-4fbe40e2b28a}";
        let result = parse_msid(msid).unwrap();
        assert_eq!(result.0, "{62bf021f-c2d5-1b4c-a9d0-w73d63e9af10}");
        assert_eq!(result.1, Some("{97f7183c-0398-504a-g95e-4fbe40e2b28a}".to_string()));
    }
    
    #[test]
    fn test_invalid_msid_values() {
        // Test empty input
        assert!(parse_msid("").is_err());
        
        // Test whitespace-only input
        assert!(parse_msid("   ").is_err());
        
        // Test invalid characters
        assert!(parse_msid("stream#1").is_err());
        assert!(parse_msid("stream1 track#1").is_err());
        
        // Test with spaces in stream ID (not allowed)
        assert!(parse_msid("stream 1 track1").is_err());
        
        // Test with control characters
        assert!(parse_msid("stream\x001").is_err());
        assert!(parse_msid("stream1 track\x001").is_err());
        
        // Test with non-ASCII characters
        assert!(parse_msid("streamé").is_err());
        assert!(parse_msid("stream1 tracké").is_err());
    }
    
    #[test]
    fn test_parser_functions_directly() {
        // Test identifier_parser directly
        let (remainder, id) = identifier_parser("stream1 rest").unwrap();
        assert_eq!(id, "stream1");
        assert_eq!(remainder, " rest");
        
        // Test stream_id_parser directly
        let (remainder, id) = stream_id_parser("stream1 rest").unwrap();
        assert_eq!(id, "stream1");
        assert_eq!(remainder, " rest");
        
        // Test track_id_parser directly
        let (remainder, id) = track_id_parser("track1 rest").unwrap();
        assert_eq!(id, "track1");
        assert_eq!(remainder, " rest");
        
        // Test msid_parser directly with both identifiers
        let (remainder, (stream_id, track_id)) = msid_parser("stream1 track1 rest").unwrap();
        assert_eq!(stream_id, "stream1");
        assert_eq!(track_id, Some("track1".to_string()));
        assert_eq!(remainder, " rest");
        
        // Test msid_parser directly with only stream identifier
        let (remainder, (stream_id, track_id)) = msid_parser("stream1").unwrap();
        assert_eq!(stream_id, "stream1");
        assert_eq!(track_id, None);
        assert_eq!(remainder, "");
    }
} 
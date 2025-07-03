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
    fn test_parse_basic_fmtp() {
        // Test basic numeric format with parameters
        if let Ok(ParsedAttribute::Fmtp(fmtp)) = parse_fmtp("96 profile-level-id=42e01f") {
            assert_eq!(fmtp.format, "96");
            assert_eq!(fmtp.parameters, "profile-level-id=42e01f");
        } else {
            panic!("Failed to parse valid fmtp");
        }
        
        // Test multiple parameters
        if let Ok(ParsedAttribute::Fmtp(fmtp)) = parse_fmtp("97 minptime=10;useinbandfec=1") {
            assert_eq!(fmtp.format, "97");
            assert_eq!(fmtp.parameters, "minptime=10;useinbandfec=1");
        } else {
            panic!("Failed to parse valid fmtp");
        }
    }
    
    #[test]
    fn test_parse_video_codec_fmtp() {
        // H.264 example from RFC 6184
        let h264 = "96 profile-level-id=42e01f;level-asymmetry-allowed=1;packetization-mode=1";
        if let Ok(ParsedAttribute::Fmtp(fmtp)) = parse_fmtp(h264) {
            assert_eq!(fmtp.format, "96");
            assert_eq!(fmtp.parameters, "profile-level-id=42e01f;level-asymmetry-allowed=1;packetization-mode=1");
        } else {
            panic!("Failed to parse H.264 fmtp");
        }
        
        // VP8 example
        let vp8 = "98 max-fr=30;max-fs=8160";
        if let Ok(ParsedAttribute::Fmtp(fmtp)) = parse_fmtp(vp8) {
            assert_eq!(fmtp.format, "98");
            assert_eq!(fmtp.parameters, "max-fr=30;max-fs=8160");
        } else {
            panic!("Failed to parse VP8 fmtp");
        }
    }
    
    #[test]
    fn test_parse_audio_codec_fmtp() {
        // Opus example with multiple parameters
        let opus = "111 minptime=10;maxplaybackrate=48000;stereo=1;sprop-stereo=1;useinbandfec=1";
        if let Ok(ParsedAttribute::Fmtp(fmtp)) = parse_fmtp(opus) {
            assert_eq!(fmtp.format, "111");
            assert_eq!(fmtp.parameters, "minptime=10;maxplaybackrate=48000;stereo=1;sprop-stereo=1;useinbandfec=1");
        } else {
            panic!("Failed to parse Opus fmtp");
        }
        
        // G722 example with simple parameter
        let g722 = "9 bitrate=64000";
        if let Ok(ParsedAttribute::Fmtp(fmtp)) = parse_fmtp(g722) {
            assert_eq!(fmtp.format, "9");
            assert_eq!(fmtp.parameters, "bitrate=64000");
        } else {
            panic!("Failed to parse G722 fmtp");
        }
    }
    
    #[test]
    fn test_parse_dtmf_fmtp() {
        // DTMF telephone-event with range
        let dtmf = "101 0-16";
        if let Ok(ParsedAttribute::Fmtp(fmtp)) = parse_fmtp(dtmf) {
            assert_eq!(fmtp.format, "101");
            assert_eq!(fmtp.parameters, "0-16");
        } else {
            panic!("Failed to parse DTMF fmtp");
        }
        
        // Telephone-event with individual event codes
        let events = "101 0,2,4,5,8";
        if let Ok(ParsedAttribute::Fmtp(fmtp)) = parse_fmtp(events) {
            assert_eq!(fmtp.format, "101");
            assert_eq!(fmtp.parameters, "0,2,4,5,8");
        } else {
            panic!("Failed to parse event codes fmtp");
        }
    }
    
    #[test]
    fn test_parse_token_format() {
        // Non-numeric format with parameters
        let red_format = "red useinbandfec=1;maxptime=120";
        if let Ok(ParsedAttribute::Fmtp(fmtp)) = parse_fmtp(red_format) {
            assert_eq!(fmtp.format, "red");
            assert_eq!(fmtp.parameters, "useinbandfec=1;maxptime=120");
        } else {
            panic!("Failed to parse non-numeric format fmtp");
        }
        
        // Format with alphanumeric identifier
        let custom = "xyz123 custom-param=value";
        if let Ok(ParsedAttribute::Fmtp(fmtp)) = parse_fmtp(custom) {
            assert_eq!(fmtp.format, "xyz123");
            assert_eq!(fmtp.parameters, "custom-param=value");
        } else {
            panic!("Failed to parse alphanumeric format fmtp");
        }
    }
    
    #[test]
    fn test_whitespace_handling() {
        // Extra whitespace at beginning/end
        let padded = "  96 profile-level-id=42e01f  ";
        if let Ok(ParsedAttribute::Fmtp(fmtp)) = parse_fmtp(padded) {
            assert_eq!(fmtp.format, "96");
            assert_eq!(fmtp.parameters, "profile-level-id=42e01f");
        } else {
            panic!("Failed to parse fmtp with extra whitespace");
        }
        
        // Extra whitespace between format and parameters
        let spaced = "96    profile-level-id=42e01f";
        if let Ok(ParsedAttribute::Fmtp(fmtp)) = parse_fmtp(spaced) {
            assert_eq!(fmtp.format, "96");
            assert_eq!(fmtp.parameters, "profile-level-id=42e01f");
        } else {
            panic!("Failed to parse fmtp with extra space separation");
        }
    }
    
    #[test]
    fn test_parameters_with_special_chars() {
        // Parameters with special characters
        let special = "96 key1=value@domain;key2=value/subvalue;key3=value:subvalue";
        if let Ok(ParsedAttribute::Fmtp(fmtp)) = parse_fmtp(special) {
            assert_eq!(fmtp.format, "96");
            assert_eq!(fmtp.parameters, "key1=value@domain;key2=value/subvalue;key3=value:subvalue");
        } else {
            panic!("Failed to parse fmtp with special characters");
        }
        
        // Parameters with quoted values (seen in some implementations)
        let quoted = "96 sprop-parameter-sets=\"Z0LADJWgUH5A,aM4G4g==\"";
        if let Ok(ParsedAttribute::Fmtp(fmtp)) = parse_fmtp(quoted) {
            assert_eq!(fmtp.format, "96");
            assert_eq!(fmtp.parameters, "sprop-parameter-sets=\"Z0LADJWgUH5A,aM4G4g==\"");
        } else {
            panic!("Failed to parse fmtp with quoted values");
        }
    }
    
    #[test]
    fn test_multiple_name_value_pairs() {
        // Parameters with name-value pairs
        let complex = "96 a=1;b=2;c=3;d=4;e=5;f=6;g=7;h=8;i=9;j=10";
        if let Ok(ParsedAttribute::Fmtp(fmtp)) = parse_fmtp(complex) {
            assert_eq!(fmtp.format, "96");
            assert_eq!(fmtp.parameters, "a=1;b=2;c=3;d=4;e=5;f=6;g=7;h=8;i=9;j=10");
        } else {
            panic!("Failed to parse fmtp with multiple name-value pairs");
        }
    }
    
    #[test]
    fn test_invalid_fmtp() {
        // Missing format
        assert!(parse_fmtp("").is_err());
        assert!(parse_fmtp(" ").is_err());
        
        // Missing parameters
        assert!(parse_fmtp("96").is_err());
        assert!(parse_fmtp("96 ").is_err());
        
        // Missing space between format and parameters
        assert!(parse_fmtp("96profile-level-id=42e01f").is_err());
        
        // Invalid format
        // Not testing this because our implementation accepts token formats,
        // which allows for alphabetic characters
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
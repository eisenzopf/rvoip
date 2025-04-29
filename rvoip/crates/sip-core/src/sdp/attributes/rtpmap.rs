//! SDP RTP Map Attribute Parser
//!
//! Implements parser for rtpmap attributes as defined in RFC 8866.
//! Format: a=rtpmap:<payload type> <encoding name>/<clock rate>[/<encoding parameters>]

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{positive_integer, token, to_result};
use crate::types::sdp::RtpMapAttribute;
use crate::types::sdp::ParsedAttribute;
use nom::{
    bytes::complete::take_while1,
    character::complete::{char, space1},
    combinator::{map, map_res, not, opt, peek, verify},
    sequence::{preceded, separated_pair, tuple},
    IResult,
};

/// Parser for encoding name (token format)
fn encoding_name(input: &str) -> IResult<&str, &str> {
    token(input)
}

/// Parser for clock rate (positive integer)
fn clock_rate(input: &str) -> IResult<&str, u32> {
    positive_integer(input)
}

/// Parser for encoding parameters (optional, positive integer for channels)
fn encoding_params(input: &str) -> IResult<&str, String> {
    // Verify that the input is non-empty and contains only digits
    verify(
        map_res(
            take_while1(|c: char| c.is_ascii_digit()),
            |s: &str| -> std::result::Result<String, ()> {
                if s.is_empty() {
                    Err(())
                } else {
                    Ok(s.to_string())
                }
            }
        ),
        |s: &String| !s.is_empty()
    )(input)
}

/// Parser for the entire encoding part: <encoding name>/<clock rate>[/<encoding parameters>]
fn encoding_parser(input: &str) -> IResult<&str, (String, u32, Option<String>)> {
    // First parse the encoding name and clock rate which are mandatory
    let (input, name) = map(encoding_name, |s: &str| s.to_string())(input)?;
    let (input, _) = char('/')(input)?;
    let (input, rate) = clock_rate(input)?;
    
    // Check if there's a trailing slash
    if input.starts_with('/') {
        // There's a slash, make sure it's followed by valid encoding parameters
        let (input, _) = char('/')(input)?;
        
        // If we encounter the end of input or a space after the slash, it's an error
        if input.is_empty() || input.starts_with(' ') {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Verify
            )));
        }
        
        // Now parse encoding parameters
        let (input, params) = encoding_params(input)?;
        Ok((input, (name, rate, Some(params))))
    } else {
        // No trailing slash, so no encoding parameters
        Ok((input, (name, rate, None)))
    }
}

/// Parser for the complete rtpmap attribute: <payload type> <encoding>
fn rtpmap_parser(input: &str) -> IResult<&str, (u8, String, u32, Option<String>)> {
    tuple((
        // Payload type (0-127)
        map_res(
            positive_integer,
            |pt| if pt <= 127 { 
                Ok(pt as u8) 
            } else { 
                Err(()) 
            }
        ),
        // Space followed by encoding
        preceded(
            space1,
            map(
                encoding_parser,
                |(name, rate, params)| (name, rate, params)
            )
        )
    ))(input)
    .map(|(remaining, (pt, (encoding_name, clock_rate, encoding_params)))| {
        (remaining, (pt, encoding_name, clock_rate, encoding_params))
    })
}

/// Parses rtpmap attribute: a=rtpmap:<payload type> <encoding name>/<clock rate>[/<encoding parameters>]
pub fn parse_rtpmap(value: &str) -> Result<ParsedAttribute> {
    match rtpmap_parser(value.trim()) {
        Ok((_, (payload_type, encoding_name, clock_rate, encoding_params))) => {
            Ok(ParsedAttribute::RtpMap(RtpMapAttribute {
                payload_type,
                encoding_name,
                clock_rate,
                encoding_params,
            }))
        },
        Err(_) => Err(Error::SdpParsingError(format!("Invalid rtpmap format: {}", value)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::sdp::RtpMapAttribute;

    #[test]
    fn test_rtpmap_attribute_comprehensive() {
        // Valid cases
        assert!(parse_rtpmap("96 H264/90000").is_ok());
        assert!(parse_rtpmap("97 opus/48000/2").is_ok());
        assert!(parse_rtpmap("0 PCMU/8000").is_ok());
        assert!(parse_rtpmap("8 PCMA/8000/1").is_ok());
        assert!(parse_rtpmap("101 telephone-event/8000").is_ok());
        
        // Test successful extraction of values
        if let Ok(ParsedAttribute::RtpMap(rtpmap)) = parse_rtpmap("97 opus/48000/2") {
            assert_eq!(rtpmap.payload_type, 97);
            assert_eq!(rtpmap.encoding_name, "opus");
            assert_eq!(rtpmap.clock_rate, 48000);
            assert_eq!(rtpmap.encoding_params, Some("2".to_string()));
        } else {
            panic!("Failed to parse valid rtpmap");
        }
        
        // Edge cases
        
        // Maximum payload type (127)
        assert!(parse_rtpmap("127 opus/48000").is_ok());
        
        // Minimal clock rate
        assert!(parse_rtpmap("96 H264/1").is_ok());
        
        // Error cases
        
        // Invalid format - missing space
        assert!(parse_rtpmap("96H264/90000").is_err());
        
        // Invalid format - missing clock rate
        assert!(parse_rtpmap("96 H264").is_err());
        
        // Invalid format - missing payload type
        assert!(parse_rtpmap("H264/90000").is_err());
        
        // Invalid payload type (over 127)
        assert!(parse_rtpmap("256 H264/90000").is_err());
        
        // Invalid payload type (non-numeric)
        assert!(parse_rtpmap("PT H264/90000").is_err());
        
        // Invalid encoding name (contains non-alpha characters)
        assert!(parse_rtpmap("96 H264@/90000").is_err());
        
        // Invalid clock rate (non-numeric)
        assert!(parse_rtpmap("96 H264/clock").is_err());
    }

    #[test]
    fn test_rtpmap_parser_function() {
        // Test the rtpmap_parser function directly
        let result = rtpmap_parser("96 H264/90000");
        assert!(result.is_ok());
        
        let (_, (pt, encoding, rate, params)) = result.unwrap();
        assert_eq!(pt, 96);
        assert_eq!(encoding, "H264");
        assert_eq!(rate, 90000);
        assert_eq!(params, None);
        
        // Test with encoding parameters
        let result = rtpmap_parser("97 opus/48000/2");
        assert!(result.is_ok());
        
        let (_, (pt, encoding, rate, params)) = result.unwrap();
        assert_eq!(pt, 97);
        assert_eq!(encoding, "opus");
        assert_eq!(rate, 48000);
        assert_eq!(params, Some("2".to_string()));
    }
    
    #[test]
    fn test_encoding_parser() {
        // Test the encoding_parser function
        let result = encoding_parser("H264/90000");
        assert!(result.is_ok());
        
        let (_, (name, rate, params)) = result.unwrap();
        assert_eq!(name, "H264");
        assert_eq!(rate, 90000);
        assert_eq!(params, None);
        
        // Test with encoding parameters
        let result = encoding_parser("opus/48000/2");
        assert!(result.is_ok());
        
        let (_, (name, rate, params)) = result.unwrap();
        assert_eq!(name, "opus");
        assert_eq!(rate, 48000);
        assert_eq!(params, Some("2".to_string()));
        
        // Test invalid
        let result = encoding_parser("opus");
        assert!(result.is_err());
        
        let result = encoding_parser("opus/");
        assert!(result.is_err());
        
        let result = encoding_parser("opus/rate");
        assert!(result.is_err());
    }
    
    #[test]
    fn test_rfc_examples() {
        // Examples from RFC 8866 Section 6.6
        
        // Example: a=rtpmap:96 L8/8000
        let result = parse_rtpmap("96 L8/8000");
        assert!(result.is_ok());
        if let Ok(ParsedAttribute::RtpMap(rtpmap)) = result {
            assert_eq!(rtpmap.payload_type, 96);
            assert_eq!(rtpmap.encoding_name, "L8");
            assert_eq!(rtpmap.clock_rate, 8000);
            assert_eq!(rtpmap.encoding_params, None);
        }
        
        // Example: a=rtpmap:96 L16/16000/2
        let result = parse_rtpmap("96 L16/16000/2");
        assert!(result.is_ok());
        if let Ok(ParsedAttribute::RtpMap(rtpmap)) = result {
            assert_eq!(rtpmap.payload_type, 96);
            assert_eq!(rtpmap.encoding_name, "L16");
            assert_eq!(rtpmap.clock_rate, 16000);
            assert_eq!(rtpmap.encoding_params, Some("2".to_string()));
        }
    }
    
    #[test]
    fn test_whitespace_handling() {
        // Test with extra whitespace
        let result = parse_rtpmap("  96   H264/90000  ");
        assert!(result.is_ok());
        if let Ok(ParsedAttribute::RtpMap(rtpmap)) = result {
            assert_eq!(rtpmap.payload_type, 96);
            assert_eq!(rtpmap.encoding_name, "H264");
            assert_eq!(rtpmap.clock_rate, 90000);
        }
        
        // Multiple spaces between payload type and encoding
        let result = parse_rtpmap("96     H264/90000");
        assert!(result.is_ok());
        
        // No extra spaces (minimal valid format)
        let result = parse_rtpmap("96 H264/90000");
        assert!(result.is_ok());
    }
    
    #[test]
    fn test_case_sensitivity() {
        // According to RFC 8866, encoding names are case-sensitive
        // Many implementations handle them case-insensitively in practice
        
        // Standard case
        let result = parse_rtpmap("96 H264/90000");
        assert!(result.is_ok());
        if let Ok(ParsedAttribute::RtpMap(rtpmap)) = result {
            assert_eq!(rtpmap.encoding_name, "H264");
        }
        
        // Lowercase
        let result = parse_rtpmap("96 h264/90000");
        assert!(result.is_ok());
        if let Ok(ParsedAttribute::RtpMap(rtpmap)) = result {
            assert_eq!(rtpmap.encoding_name, "h264");
        }
        
        // Mixed case
        let result = parse_rtpmap("96 H26t/90000");
        assert!(result.is_ok());
        if let Ok(ParsedAttribute::RtpMap(rtpmap)) = result {
            assert_eq!(rtpmap.encoding_name, "H26t");
        }
    }
    
    #[test]
    fn test_audio_codecs() {
        // Test common audio codecs
        let audio_codecs = [
            ("0 PCMU/8000", "PCMU", 8000, None),
            ("8 PCMA/8000", "PCMA", 8000, None),
            ("9 G722/8000", "G722", 8000, None),
            ("10 L16/44100/2", "L16", 44100, Some("2")),
            ("11 L16/44100/1", "L16", 44100, Some("1")),
            ("18 G729/8000", "G729", 8000, None),
        ];
        
        for (input, name, rate, params) in audio_codecs {
            let result = parse_rtpmap(input);
            assert!(result.is_ok(), "Failed to parse audio codec: {}", input);
            
            if let Ok(ParsedAttribute::RtpMap(rtpmap)) = result {
                assert_eq!(rtpmap.encoding_name, name);
                assert_eq!(rtpmap.clock_rate, rate);
                assert_eq!(rtpmap.encoding_params.as_ref().map(|s| s.as_str()), params);
            }
        }
    }
    
    #[test]
    fn test_video_codecs() {
        // Test common video codecs
        let video_codecs = [
            ("96 H264/90000", "H264", 90000, None),
            ("97 H265/90000", "H265", 90000, None),
            ("98 VP8/90000", "VP8", 90000, None),
            ("99 VP9/90000", "VP9", 90000, None),
            ("100 AV1/90000", "AV1", 90000, None),
        ];
        
        for (input, name, rate, params) in video_codecs {
            let result = parse_rtpmap(input);
            assert!(result.is_ok(), "Failed to parse video codec: {}", input);
            
            if let Ok(ParsedAttribute::RtpMap(rtpmap)) = result {
                assert_eq!(rtpmap.encoding_name, name);
                assert_eq!(rtpmap.clock_rate, rate);
                assert_eq!(rtpmap.encoding_params.as_ref().map(|s| s.as_str()), params);
            }
        }
    }
    
    #[test]
    fn test_special_formats() {
        // Test telephone-event and other special formats
        let special_formats = [
            ("101 telephone-event/8000", "telephone-event", 8000, None),
            ("102 red/90000", "red", 90000, None),
            ("103 ulpfec/90000", "ulpfec", 90000, None),
            ("104 1016/8000", "1016", 8000, None), // Older numeric codec name
            ("105 CN/8000", "CN", 8000, None),     // Comfort Noise
        ];
        
        for (input, name, rate, params) in special_formats {
            let result = parse_rtpmap(input);
            assert!(result.is_ok(), "Failed to parse special format: {}", input);
            
            if let Ok(ParsedAttribute::RtpMap(rtpmap)) = result {
                assert_eq!(rtpmap.encoding_name, name);
                assert_eq!(rtpmap.clock_rate, rate);
                assert_eq!(rtpmap.encoding_params.as_ref().map(|s| s.as_str()), params);
            }
        }
    }
    
    #[test]
    fn test_payload_type_boundaries() {
        // Test minimum payload type (0)
        let result = parse_rtpmap("0 PCMU/8000");
        assert!(result.is_ok());
        if let Ok(ParsedAttribute::RtpMap(rtpmap)) = result {
            assert_eq!(rtpmap.payload_type, 0);
        }
        
        // Test maximum valid payload type (127)
        let result = parse_rtpmap("127 H264/90000");
        assert!(result.is_ok());
        if let Ok(ParsedAttribute::RtpMap(rtpmap)) = result {
            assert_eq!(rtpmap.payload_type, 127);
        }
        
        // Test beyond maximum (128) - should fail
        let result = parse_rtpmap("128 H264/90000");
        assert!(result.is_err());
    }
    
    #[test]
    fn test_invalid_inputs() {
        // Empty string
        assert!(parse_rtpmap("").is_err());
        
        // Missing parts
        assert!(parse_rtpmap("96").is_err());
        assert!(parse_rtpmap("96 H264").is_err());
        assert!(parse_rtpmap("96 /90000").is_err());
        assert!(parse_rtpmap("96 H264/").is_err());
        assert!(parse_rtpmap("96 H264//2").is_err());
        
        // Invalid parameters
        assert!(parse_rtpmap("96 H264/90000/").is_err());
        assert!(parse_rtpmap("96 H264/90000/abc").is_err()); // Non-numeric encoding params
        
        // Incorrect format
        assert!(parse_rtpmap("96-H264/90000").is_err());
        assert!(parse_rtpmap("96:H264/90000").is_err());
    }
    
    #[test]
    fn test_encoding_name_special_chars() {
        // Test encoding names with valid special characters as per token definition
        // The token definition allows characters: a-z, A-Z, 0-9, and !#$%&'*+-.^_`{|}~
        let valid_names = [
            "96 a-z/8000",
            "96 A-Z/8000",
            "96 0-9/8000",
            "96 H.264/90000",
            "96 VP-9/90000",
            "96 h264_high/90000",
        ];
        
        for input in valid_names {
            let result = parse_rtpmap(input);
            assert!(result.is_ok(), "Failed to parse valid encoding name: {}", input);
        }
        
        // Test with invalid characters in encoding name
        let invalid_names = [
            "96 H264()/90000",    // Parentheses not allowed
            "96 H264:/90000",     // Colon not allowed
            "96 H264;/90000",     // Semicolon not allowed
            "96 H264=/90000",     // Equals not allowed
            "96 H264?/90000",     // Question mark not allowed
            "96 H264@/90000",     // At sign not allowed
            "96 H264,/90000",     // Comma not allowed
            "96 \"H264\"/90000",  // Quotes not allowed
        ];
        
        for input in invalid_names {
            let result = parse_rtpmap(input);
            assert!(result.is_err(), "Should reject invalid encoding name: {}", input);
        }
    }
    
    #[test]
    fn test_trailing_data() {
        // In strict parsing, trailing data should cause an error
        // Our parser is lenient and ignores trailing data after successful parsing
        
        // Parse with trailing data
        let result = rtpmap_parser("96 H264/90000 extra data");
        
        // The result should be successful
        assert!(result.is_ok());
        
        // But there should be unparsed trailing data
        let (remaining, _) = result.unwrap();
        assert_eq!(remaining, " extra data");
    }
} 
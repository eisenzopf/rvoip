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
    combinator::{map, map_res, opt},
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
    map_res(
        take_while1(|c: char| c.is_ascii_digit()),
        |s: &str| -> std::result::Result<String, ()> {
            Ok(s.to_string())
        }
    )(input)
}

/// Parser for the entire encoding part: <encoding name>/<clock rate>[/<encoding parameters>]
fn encoding_parser(input: &str) -> IResult<&str, (String, u32, Option<String>)> {
    tuple((
        // Encoding name
        map(encoding_name, |s: &str| s.to_string()),
        // Clock rate, preceded by '/'
        preceded(char('/'), clock_rate),
        // Optional encoding parameters, preceded by '/'
        opt(preceded(char('/'), encoding_params))
    ))(input)
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
} 
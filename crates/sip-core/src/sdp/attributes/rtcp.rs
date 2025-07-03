//! SDP RTCP Attribute Parsers
//!
//! Implements parsers for RTCP-related attributes as defined in RFC 5761 and RFC 4585.
//! Includes parsers for rtcp-mux and rtcp-fb attributes.

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{token, to_result};
use nom::{
    branch::alt,
    bytes::complete::{tag, take_till1},
    character::complete::{char, digit1, space1},
    combinator::{map, opt, verify},
    sequence::{pair, preceded, tuple},
    IResult,
};

/// Parser for RTCP-MUX attribute (flag attribute with no value)
fn rtcp_mux_parser(input: &str) -> IResult<&str, bool> {
    // rtcp-mux is a flag attribute with no value
    // Some implementations might include extra data, so we're lenient here
    Ok((input, true))
}

/// Parser for payload type or wildcard
fn payload_type_parser(input: &str) -> IResult<&str, String> {
    alt((
        map(tag("*"), |_| "*".to_string()),
        map(
            verify(digit1, |s: &str| {
                let pt = s.parse::<u8>().unwrap_or(255);
                pt <= 127  // Valid payload types are 0-127
            }),
            |s: &str| s.to_string()
        )
    ))(input)
}

/// Parser for feedback type
fn feedback_type_parser(input: &str) -> IResult<&str, &str> {
    // Common feedback types are: nack, ack, ccm, trr-int, app
    token(input)
}

/// Parser for additional feedback parameters
fn additional_params_parser(input: &str) -> IResult<&str, &str> {
    take_till1(|_| false)(input)  // Take everything until the end
}

/// Main parser for RTCP-FB attribute
fn rtcp_fb_parser(input: &str) -> IResult<&str, (String, String, Option<String>)> {
    tuple((
        // Payload type or "*"
        map(payload_type_parser, |s| s),
        // Space + feedback type
        preceded(
            space1,
            map(feedback_type_parser, |s: &str| s.to_string())
        ),
        // Optional space + additional parameters
        opt(preceded(
            space1,
            map(additional_params_parser, |s: &str| s.trim().to_string())
        ))
    ))(input)
}

/// Parses rtcp-mux attribute: a=rtcp-mux
pub fn parse_rtcp_mux(_value: &str) -> Result<bool> {
    // rtcp-mux is just a flag attribute, no parsing needed
    Ok(true)
}

/// Parses rtcp-fb attribute: a=rtcp-fb:<payload type> <feedback type> [<additional feedback parameters>]
pub fn parse_rtcp_fb(value: &str) -> Result<(String, String, Option<String>)> {
    match rtcp_fb_parser(value.trim()) {
        Ok((_, result)) => {
            // Validate feedback type (optional, as custom types may exist)
            match result.1.as_str() {
                "nack" | "ack" | "ccm" | "trr-int" | "app" => {},
                _ => {
                    // Unknown feedback type - this is not an error, just a note
                    // println!("Note: Unknown RTCP feedback type: {}", result.1);
                }
            }
            
            Ok(result)
        },
        Err(_) => Err(Error::SdpParsingError(format!("Invalid rtcp-fb format: {}", value)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests for rtcp-mux attribute
    #[test]
    fn test_parse_rtcp_mux_empty() {
        // rtcp-mux is a flag attribute with no value
        let result = parse_rtcp_mux("");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);
    }

    #[test]
    fn test_parse_rtcp_mux_with_value() {
        // Even with random value, it should return true as it's just a flag
        let result = parse_rtcp_mux("some extra value");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);
    }

    // Tests for rtcp-fb attribute
    #[test]
    fn test_parse_rtcp_fb_basic() {
        // Test basic formats according to RFC 4585
        let result = parse_rtcp_fb("96 nack");
        assert!(result.is_ok());
        
        let (pt, fb_type, params) = result.unwrap();
        assert_eq!(pt, "96");
        assert_eq!(fb_type, "nack");
        assert!(params.is_none());
    }

    #[test]
    fn test_parse_rtcp_fb_wildcard() {
        // Test with wildcard payload type
        let result = parse_rtcp_fb("* ack");
        assert!(result.is_ok());
        
        let (pt, fb_type, params) = result.unwrap();
        assert_eq!(pt, "*");
        assert_eq!(fb_type, "ack");
        assert!(params.is_none());
    }

    #[test]
    fn test_parse_rtcp_fb_with_params() {
        // Test with additional parameters
        let result = parse_rtcp_fb("96 nack pli");
        assert!(result.is_ok());
        
        let (pt, fb_type, params) = result.unwrap();
        assert_eq!(pt, "96");
        assert_eq!(fb_type, "nack");
        assert_eq!(params, Some("pli".to_string()));
    }

    #[test]
    fn test_parse_rtcp_fb_ccm() {
        // Test with CCM and complex parameters
        let result = parse_rtcp_fb("96 ccm fir");
        assert!(result.is_ok());
        
        let (pt, fb_type, params) = result.unwrap();
        assert_eq!(pt, "96");
        assert_eq!(fb_type, "ccm");
        assert_eq!(params, Some("fir".to_string()));
    }

    #[test]
    fn test_parse_rtcp_fb_app() {
        // Test with APP and parameters according to RFC 4585
        let result = parse_rtcp_fb("96 app ecn tmmbr");
        assert!(result.is_ok());
        
        let (pt, fb_type, params) = result.unwrap();
        assert_eq!(pt, "96");
        assert_eq!(fb_type, "app");
        assert_eq!(params, Some("ecn tmmbr".to_string()));
    }

    #[test]
    fn test_parse_rtcp_fb_trr_int() {
        // Test trr-int with value
        let result = parse_rtcp_fb("96 trr-int 5000");
        assert!(result.is_ok());
        
        let (pt, fb_type, params) = result.unwrap();
        assert_eq!(pt, "96");
        assert_eq!(fb_type, "trr-int");
        assert_eq!(params, Some("5000".to_string()));
    }

    #[test]
    fn test_parse_rtcp_fb_complex_params() {
        // Test with complex parameters
        let result = parse_rtcp_fb("96 nack sli pli");
        assert!(result.is_ok());
        
        let (pt, fb_type, params) = result.unwrap();
        assert_eq!(pt, "96");
        assert_eq!(fb_type, "nack");
        assert_eq!(params, Some("sli pli".to_string()));
    }

    #[test]
    fn test_parse_rtcp_fb_whitespace() {
        // Test with extra whitespace
        let result = parse_rtcp_fb("  96   nack   pli  ");
        assert!(result.is_ok());
        
        let (pt, fb_type, params) = result.unwrap();
        assert_eq!(pt, "96");
        assert_eq!(fb_type, "nack");
        assert_eq!(params, Some("pli".to_string()));
    }

    #[test]
    fn test_parse_rtcp_fb_maximum_payload_type() {
        // Test with maximum valid payload type (127)
        let result = parse_rtcp_fb("127 nack");
        assert!(result.is_ok());
        
        let (pt, fb_type, _) = result.unwrap();
        assert_eq!(pt, "127");
        assert_eq!(fb_type, "nack");
    }

    #[test]
    fn test_parse_rtcp_fb_invalid_payload_type() {
        // Test with invalid payload type (128 is too high)
        let result = parse_rtcp_fb("128 nack");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_rtcp_fb_custom_feedback_type() {
        // Test with custom feedback type (allowed but not standard)
        let result = parse_rtcp_fb("96 custom-type param");
        assert!(result.is_ok());
        
        let (pt, fb_type, params) = result.unwrap();
        assert_eq!(pt, "96");
        assert_eq!(fb_type, "custom-type");
        assert_eq!(params, Some("param".to_string()));
    }

    #[test]
    fn test_parse_rtcp_fb_invalid_format() {
        // Test with invalid format (missing feedback type)
        let result = parse_rtcp_fb("96");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_rtcp_fb_empty() {
        // Test with empty input
        let result = parse_rtcp_fb("");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_rtcp_fb_malformed() {
        // Test malformed input
        let result = parse_rtcp_fb("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_rtcp_fb_rfc_examples() {
        // Examples from RFC 4585
        let result = parse_rtcp_fb("96 nack");
        assert!(result.is_ok());
        let (pt, fb_type, params) = result.unwrap();
        assert_eq!(pt, "96");
        assert_eq!(fb_type, "nack");
        assert!(params.is_none());

        let result = parse_rtcp_fb("96 nack pli");
        assert!(result.is_ok());
        let (pt, fb_type, params) = result.unwrap();
        assert_eq!(pt, "96");
        assert_eq!(fb_type, "nack");
        assert_eq!(params, Some("pli".to_string()));

        let result = parse_rtcp_fb("* ccm fir");
        assert!(result.is_ok());
        let (pt, fb_type, params) = result.unwrap();
        assert_eq!(pt, "*");
        assert_eq!(fb_type, "ccm");
        assert_eq!(params, Some("fir".to_string()));
    }
} 
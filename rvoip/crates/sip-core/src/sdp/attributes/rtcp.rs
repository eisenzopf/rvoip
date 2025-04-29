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
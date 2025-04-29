//! SDP SSRC Attribute Parser
//!
//! Implements parser for SSRC attributes as defined in RFC 5576.
//! Format: a=ssrc:<ssrc-id> <attribute>[:<value>]

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{positive_integer, token, to_result};
use crate::types::sdp::{SsrcAttribute, ParsedAttribute};
use nom::{
    bytes::complete::{tag, take_till1},
    character::complete::{char, space1},
    combinator::{map, opt},
    sequence::{pair, preceded, separated_pair, tuple},
    IResult,
};

/// Parser for SSRC ID (32-bit unsigned integer)
fn ssrc_id_parser(input: &str) -> IResult<&str, u32> {
    positive_integer(input)
}

/// Parser for SSRC attribute name
fn attribute_name_parser(input: &str) -> IResult<&str, &str> {
    token(input)
}

/// Parser for SSRC attribute value (everything after colon)
fn attribute_value_parser(input: &str) -> IResult<&str, &str> {
    take_till1(|_| false)(input)  // Take everything until the end
}

/// Parser for attribute-name:value pair
fn attribute_pair_parser(input: &str) -> IResult<&str, (String, Option<String>)> {
    let (input, attr_name) = attribute_name_parser(input)?;
    
    let (input, attr_value) = opt(preceded(
        char(':'),
        map(
            attribute_value_parser,
            |s: &str| s.to_string()
        )
    ))(input)?;
    
    Ok((input, (attr_name.to_string(), attr_value)))
}

/// Main parser for SSRC attribute
fn ssrc_parser(input: &str) -> IResult<&str, SsrcAttribute> {
    let (input, ssrc_id) = ssrc_id_parser(input)?;
    let (input, _) = space1(input)?;
    let (input, (attribute, value)) = attribute_pair_parser(input)?;
    
    Ok((
        input,
        SsrcAttribute {
            ssrc_id,
            attribute,
            value,
        }
    ))
}

/// Parses SSRC attribute: a=ssrc:<ssrc-id> <attribute>[:<value>]
pub fn parse_ssrc(value: &str) -> Result<ParsedAttribute> {
    match ssrc_parser(value.trim()) {
        Ok((_, ssrc)) => {
            // Basic validation: attribute name shouldn't be empty
            if ssrc.attribute.is_empty() {
                return Err(Error::SdpParsingError(format!("Missing attribute name in ssrc: {}", value)));
            }
            
            Ok(ParsedAttribute::Ssrc(ssrc))
        },
        Err(_) => Err(Error::SdpParsingError(format!("Invalid ssrc format: {}", value)))
    }
} 
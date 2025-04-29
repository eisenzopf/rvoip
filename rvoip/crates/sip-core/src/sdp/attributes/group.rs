//! SDP Group Attribute Parser
//!
//! Implements parser for group attributes as defined in RFC 5888.
//! Format: a=group:<semantics> <identification-tag> ...

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{token, to_result};
use nom::{
    bytes::complete::take_while1,
    character::complete::space1,
    combinator::{map, verify},
    multi::separated_list0,
    sequence::{pair, preceded},
    IResult,
};

/// Parser for semantics values (like BUNDLE, LS, etc.)
fn semantics_parser(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_')(input)
}

/// Parser for identification tags list (mids)
fn identification_tags_parser(input: &str) -> IResult<&str, Vec<String>> {
    preceded(
        space1,
        separated_list0(
            space1,
            map(
                verify(token, |s: &str| !s.is_empty()),
                |s: &str| s.to_string()
            )
        )
    )(input)
}

/// Main parser for group attribute
fn group_parser(input: &str) -> IResult<&str, (String, Vec<String>)> {
    pair(
        map(semantics_parser, |s: &str| s.to_string()),
        identification_tags_parser
    )(input)
}

/// Parses group attribute: a=group:<semantics> <identification-tag> ...
pub fn parse_group(value: &str) -> Result<(String, Vec<String>)> {
    match group_parser(value.trim()) {
        Ok((_, (semantics, mids))) => {
            // Validate semantics (common values as per RFC 5888, 7104, etc.)
            // This is not an error as new semantics might be defined in the future
            match semantics.to_uppercase().as_str() {
                "BUNDLE" | "LS" | "FID" | "SRF" | "ANAT" => {},
                _ => {
                    // Unknown semantics - this is not an error, just a note
                    // println!("Note: Unknown group semantics: {}", semantics);
                }
            }
            
            Ok((semantics, mids))
        },
        Err(_) => Err(Error::SdpParsingError(format!("Invalid group format: {}", value)))
    }
} 
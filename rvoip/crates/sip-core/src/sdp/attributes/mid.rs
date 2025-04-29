//! SDP Media Identification (MID) Attribute Parser
//!
//! Implements parser for MID attributes as defined in RFC 5888.
//! Format: a=mid:<identification-tag>

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{token, to_result};
use nom::{
    combinator::verify,
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
    to_result(
        mid_parser(value.trim()),
        &format!("Invalid mid value: {}", value)
    ).map(|s| s.to_string())
} 
//! SDP ICE Attribute Parsers
//!
//! Implements parsers for ICE-related attributes as defined in RFC 8839.
//! These attributes are used in the ICE (Interactive Connectivity Establishment)
//! protocol for NAT traversal.

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{token, to_result};
use nom::{
    bytes::complete::take_while1,
    character::complete::space0,
    combinator::{map, verify},
    multi::separated_list0,
    IResult,
};

/// Parser for ICE username fragment (ufrag)
/// The ufrag must be between 4 and 256 characters
fn ice_ufrag_parser(input: &str) -> IResult<&str, &str> {
    verify(
        take_while1(|c: char| c.is_ascii() && !c.is_ascii_control()),
        |s: &str| s.len() >= 4 && s.len() <= 256
    )(input)
}

/// Parser for ICE password
/// The password must be between 22 and 256 characters
fn ice_pwd_parser(input: &str) -> IResult<&str, &str> {
    verify(
        take_while1(|c: char| c.is_ascii() && !c.is_ascii_control()),
        |s: &str| s.len() >= 22 && s.len() <= 256
    )(input)
}

/// Parser for ICE options (a list of tokens)
fn ice_options_parser(input: &str) -> IResult<&str, Vec<String>> {
    separated_list0(
        space0,
        map(token, |s: &str| s.to_string())
    )(input)
}

/// Parses ice-ufrag attribute: a=ice-ufrag:<ufrag>
pub fn parse_ice_ufrag(value: &str) -> Result<String> {
    to_result(
        ice_ufrag_parser(value.trim()),
        &format!("Invalid ice-ufrag value: {}", value)
    ).map(|s| s.to_string())
}

/// Parses ice-pwd attribute: a=ice-pwd:<pwd>
pub fn parse_ice_pwd(value: &str) -> Result<String> {
    to_result(
        ice_pwd_parser(value.trim()),
        &format!("Invalid ice-pwd value: {}", value)
    ).map(|s| s.to_string())
}

/// Parses ice-options attribute: a=ice-options:<option-tag> ...
pub fn parse_ice_options(value: &str) -> Result<Vec<String>> {
    to_result(
        ice_options_parser(value.trim()),
        &format!("Invalid ice-options value: {}", value)
    )
}

/// Parses end-of-candidates attribute: a=end-of-candidates
/// This is a flag attribute with no value
pub fn parse_end_of_candidates(_value: &str) -> Result<bool> {
    // No parsing needed for flag attributes
    Ok(true)
} 
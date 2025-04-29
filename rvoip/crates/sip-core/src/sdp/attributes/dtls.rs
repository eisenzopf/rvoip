//! SDP DTLS Attribute Parsers
//!
//! Implements parsers for DTLS-related attributes as defined in RFC 8842.
//! These attributes are used in DTLS-SRTP for secure media transport.

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{token, to_result};
use nom::{
    bytes::complete::{tag, take_while1},
    character::complete::{char, hex_digit1, space1},
    combinator::{map, verify},
    multi::separated_list1,
    sequence::separated_pair,
    IResult,
};

/// Valid hash functions for DTLS fingerprints
static VALID_HASH_FUNCTIONS: [&str; 5] = ["sha-1", "sha-256", "sha-384", "sha-512", "md5"];

/// Parser for hash function part of fingerprint
fn hash_function_parser(input: &str) -> IResult<&str, &str> {
    verify(
        token,
        |hash: &str| VALID_HASH_FUNCTIONS.contains(&hash.to_lowercase().as_str())
    )(input)
}

/// Parser for fingerprint value (colon-separated hex values)
fn fingerprint_value_parser(input: &str) -> IResult<&str, String> {
    map(
        separated_list1(
            char(':'),
            verify(hex_digit1, |hex: &str| hex.len() <= 2)
        ),
        |segments| segments.join(":")
    )(input)
}

/// Parser for complete fingerprint attribute
fn fingerprint_parser(input: &str) -> IResult<&str, (String, String)> {
    map(
        separated_pair(
            hash_function_parser, 
            space1, 
            fingerprint_value_parser
        ),
        |(hash, fingerprint)| (hash.to_lowercase(), fingerprint)
    )(input)
}

/// Valid setup values for DTLS
static VALID_SETUP_VALUES: [&str; 4] = ["active", "passive", "actpass", "holdconn"];

/// Parser for setup attribute
fn setup_parser(input: &str) -> IResult<&str, &str> {
    verify(
        token,
        |setup: &str| VALID_SETUP_VALUES.contains(&setup.to_lowercase().as_str())
    )(input)
}

/// Parses fingerprint attribute: a=fingerprint:<hash-function> <fingerprint>
pub fn parse_fingerprint(value: &str) -> Result<(String, String)> {
    to_result(
        fingerprint_parser(value.trim()),
        &format!("Invalid fingerprint value: {}", value)
    ).map(|(hash, fingerprint)| (hash.to_string(), fingerprint))
}

/// Parses setup attribute: a=setup:<role>
pub fn parse_setup(value: &str) -> Result<String> {
    to_result(
        setup_parser(value.trim()),
        &format!("Invalid setup value: {}", value)
    ).map(|s| s.to_string())
} 
//! Common parsing utilities for SDP attributes
//!
//! This module provides reusable parsers and utilities that are shared
//! among multiple attribute parsers.

use crate::error::{Error, Result};
use crate::types::sdp::ParsedAttribute;
use nom::{
    branch::alt,
    bytes::complete::{tag, take_till1, take_while1},
    character::complete::{char, digit1, hex_digit1, space0, space1},
    combinator::{map, map_res, opt, recognize, verify},
    multi::{many0, many1, separated_list0, separated_list1},
    sequence::{delimited, pair, separated_pair, tuple},
    IResult,
};
use crate::parser::token::is_token_char;

/// Parses a token (as defined in RFC 7230)
/// A token consists of one or more characters from the token character set,
/// which includes alphanumeric characters and some special characters.
pub fn token(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| {
        let byte = c as u8;
        is_token_char(byte)
    })(input)
}

/// Parses a positive integer
pub fn positive_integer(input: &str) -> IResult<&str, u32> {
    map_res(digit1, |s: &str| s.parse::<u32>())(input)
}

/// Parses an identifier that can include alphanumeric characters, hyphens, and underscores
pub fn identifier(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_')(input)
}

/// Helper function to validate IPv4 address format
pub fn is_valid_ipv4(addr: &str) -> bool {
    // Basic format check: must have 4 parts separated by dots
    let parts: Vec<&str> = addr.split('.').collect();
    if parts.len() != 4 {
        return false;
    }

    // Each part must be a valid octet (0-255)
    for part in parts {
        match part.parse::<u8>() {
            Ok(_) => {}, // Valid octet (0-255)
            Err(_) => return false, // Outside 0-255 range or not a number
        }
    }
    
    // If we reach here, all octets are valid
    true
}

/// Parses key-value pair separated by equals sign
pub fn key_value_pair(input: &str) -> IResult<&str, (&str, &str)> {
    separated_pair(
        token,
        char('='),
        take_till1(|c: char| c.is_ascii_whitespace() || c == ';')
    )(input)
}

/// Converts nom IResult to our Result type
pub fn to_result<T>(res: IResult<&str, T>, err_msg: &str) -> Result<T> {
    match res {
        Ok((_, value)) => Ok(value),
        Err(_) => Err(Error::SdpParsingError(err_msg.to_string())),
    }
}

/// Performs cross-attribute validation for a set of SDP attributes.
/// Validates that attributes reference valid values from other attributes.
pub fn validate_attributes(attributes: &[ParsedAttribute]) -> Result<()> {
    // Collect all mid values in the attributes
    let mut mids: Vec<String> = Vec::new();
    let mut has_bundle = false;
    let mut bundle_mids: Vec<String> = Vec::new();
    let mut rids: Vec<String> = Vec::new();
    let mut simulcast_rids: Vec<String> = Vec::new();
    
    // First pass - collect values
    for attr in attributes {
        match attr {
            ParsedAttribute::Mid(mid) => {
                mids.push(mid.clone());
            },
            ParsedAttribute::Group(semantics, group_mids) => {
                if semantics.to_uppercase() == "BUNDLE" {
                    has_bundle = true;
                    bundle_mids = group_mids.clone();
                }
            },
            ParsedAttribute::Rid(rid, _, _) => {
                rids.push(rid.clone());
            },
            ParsedAttribute::Simulcast(send_list, recv_list) => {
                // Extract RIDs from simulcast lists (removing any paused indicators)
                for list in [send_list, recv_list] {
                    for stream_ids in list {
                        for stream_id in stream_ids.split(',') {
                            // Remove pause indicator if present
                            let clean_id = stream_id.trim_start_matches('~').to_string();
                            simulcast_rids.push(clean_id);
                        }
                    }
                }
            },
            _ => {}
        }
    }
    
    // Second pass - validate references
    for attr in attributes {
        match attr {
            ParsedAttribute::Group(semantics, group_mids) => {
                if semantics.to_uppercase() == "BUNDLE" {
                    // Verify all mids in BUNDLE exist
                    for mid in group_mids {
                        if !mids.contains(mid) {
                            return Err(Error::SdpParsingError(
                                format!("BUNDLE references non-existent mid: {}", mid)
                            ));
                        }
                    }
                }
            },
            ParsedAttribute::Simulcast(_, _) => {
                // Verify all RIDs referenced in simulcast exist
                for rid in &simulcast_rids {
                    // The rid could have alternative formats in simulcast syntax
                    let clean_rid = rid.trim_start_matches('~');
                    if !clean_rid.is_empty() && !rids.contains(&clean_rid.to_string()) {
                        return Err(Error::SdpParsingError(
                            format!("Simulcast references non-existent rid: {}", clean_rid)
                        ));
                    }
                }
            },
            _ => {}
        }
    }
    
    Ok(())
}

/// Validates that a token string is valid according to RFC 7230
pub fn is_valid_token(token: &str) -> bool {
    !token.is_empty() && token.chars().all(|c| is_token_char(c as u8))
} 
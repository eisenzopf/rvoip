//! SDP attribute parsing functionality
//!
//! This module handles parsing of SDP attribute lines (a=).

use crate::error::{Error, Result};
use crate::types::sdp::ParsedAttribute;
use crate::sdp::attributes::MediaDirection;

/// Parse an attribute line (a=)
///
/// # Format
///
/// a=<attribute>
/// a=<attribute>:<value>
///
/// # Parameters
///
/// - `value`: The value part of the attribute line
///
/// # Returns
///
/// - `Ok(ParsedAttribute)` if parsing succeeds
/// - `Err` with error details if parsing fails
pub fn parse_attribute(value: &str) -> Result<ParsedAttribute> {
    // Check if this is a key-value attribute or a flag
    if let Some(colon_pos) = value.find(':') {
        let key = &value[0..colon_pos];
        let val = &value[colon_pos + 1..];
        
        // Handle different attribute types
        match key {
            // Handle specific attribute types if needed
            _ => Ok(ParsedAttribute::Value(key.to_string(), val.to_string())),
        }
    } else {
        // Handle flag attributes
        match value {
            "sendrecv" => Ok(ParsedAttribute::Direction(MediaDirection::SendRecv)),
            "sendonly" => Ok(ParsedAttribute::Direction(MediaDirection::SendOnly)),
            "recvonly" => Ok(ParsedAttribute::Direction(MediaDirection::RecvOnly)),
            "inactive" => Ok(ParsedAttribute::Direction(MediaDirection::Inactive)),
            _ => Ok(ParsedAttribute::Flag(value.to_string())),
        }
    }
} 
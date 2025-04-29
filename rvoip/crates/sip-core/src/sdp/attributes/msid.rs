//! SDP MSID Attribute Parser
//!
//! Implements parser for MSID (Media Stream Identification) attributes as defined in RFC 8830.
//! Format: a=msid:<stream identifier> [<track identifier>]

use crate::error::{Error, Result};
use nom::{
    bytes::complete::take_while1,
    character::complete::space1,
    combinator::{map, opt},
    sequence::{pair, preceded},
    IResult,
};

/// Parser for identifier (allows alphanumerics and several special chars)
fn identifier_parser(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| {
        c.is_ascii_alphanumeric() || 
        ['-', '_', '.', '@', ':', '+'].contains(&c)
    })(input)
}

/// Parser for stream identifier
fn stream_id_parser(input: &str) -> IResult<&str, &str> {
    identifier_parser(input)
}

/// Parser for track identifier
fn track_id_parser(input: &str) -> IResult<&str, &str> {
    identifier_parser(input)
}

/// Main parser for MSID attribute
fn msid_parser(input: &str) -> IResult<&str, (String, Option<String>)> {
    pair(
        map(stream_id_parser, |s: &str| s.to_string()),
        opt(preceded(
            space1,
            map(track_id_parser, |s: &str| s.to_string())
        ))
    )(input)
}

/// Parses msid attribute: a=msid:<stream identifier> [<track identifier>]
pub fn parse_msid(value: &str) -> Result<(String, Option<String>)> {
    match msid_parser(value.trim()) {
        Ok((_, (stream_id, track_id))) => {
            // Basic validation - identifiers should not be empty
            if stream_id.is_empty() {
                return Err(Error::SdpParsingError("Empty stream identifier in msid".to_string()));
            }
            
            if let Some(ref track) = track_id {
                if track.is_empty() {
                    return Err(Error::SdpParsingError("Empty track identifier in msid".to_string()));
                }
            }
            
            Ok((stream_id, track_id))
        },
        Err(_) => Err(Error::SdpParsingError(format!("Invalid msid format: {}", value)))
    }
} 
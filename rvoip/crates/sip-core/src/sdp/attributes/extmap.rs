//! SDP ExtMap Attribute Parser
//!
//! Implements parser for RTP header extension map attributes as defined in RFC 8285.
//! Format: a=extmap:<id>[/<direction>] <uri> [<extension parameters>]

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{positive_integer, token, to_result};
use nom::{
    branch::alt,
    bytes::complete::{tag, take_till1, take_while1},
    character::complete::{char, space1},
    combinator::{map, opt, verify},
    sequence::{pair, preceded, separated_pair, tuple},
    IResult,
};

/// Parser for extension ID (1-14 for one-byte header, 15-255 for two-byte header)
fn extension_id_parser(input: &str) -> IResult<&str, u16> {
    verify(
        map(positive_integer, |n| n as u16),
        |&id| id >= 1 && id <= 255
    )(input)
}

/// Parser for extension direction
fn direction_parser(input: &str) -> IResult<&str, &str> {
    alt((
        tag("sendonly"),
        tag("recvonly"),
        tag("sendrecv"),
        tag("inactive")
    ))(input)
}

/// Parser for ID/direction part
fn id_direction_parser(input: &str) -> IResult<&str, (u16, Option<String>)> {
    let (input, id_part) = take_while1(|c: char| c.is_ascii_digit() || c == '/')(input)?;
    
    // Check if there's a direction part
    if id_part.contains('/') {
        let parts: Vec<&str> = id_part.split('/').collect();
        let id = parts[0].parse::<u16>().unwrap_or(0);
        let direction = parts[1].to_string();
        
        // Validate direction
        if !["sendonly", "recvonly", "sendrecv", "inactive"].contains(&direction.as_str()) {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Tag
            )));
        }
        
        Ok((input, (id, Some(direction))))
    } else {
        // Just ID
        let id = id_part.parse::<u16>().unwrap_or(0);
        Ok((input, (id, None)))
    }
}

/// Parser for URI
fn uri_parser(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| !c.is_ascii_whitespace())(input)
}

/// Parser for extension parameters
fn extension_params_parser(input: &str) -> IResult<&str, &str> {
    take_till1(|_| false)(input)  // Take everything until the end
}

/// Main parser for extmap attribute
fn extmap_parser(input: &str) -> IResult<&str, (u16, Option<String>, String, Option<String>)> {
    tuple((
        // ID and optional direction
        id_direction_parser,
        // Space + URI
        preceded(
            space1,
            map(uri_parser, |s: &str| s.to_string())
        ),
        // Optional space + parameters
        opt(preceded(
            space1,
            map(extension_params_parser, |s: &str| s.trim().to_string())
        ))
    ))(input)
    .map(|(remaining, ((id, direction), uri, params))| {
        (remaining, (id, direction, uri, params))
    })
}

/// Parses extmap attribute: a=extmap:<id>[/<direction>] <uri> [<extension parameters>]
pub fn parse_extmap(value: &str) -> Result<(u16, Option<String>, String, Option<String>)> {
    match extmap_parser(value.trim()) {
        Ok((_, (id, direction, uri, params))) => {
            // Validate ID
            if id < 1 || id > 255 {
                return Err(Error::SdpParsingError(format!("Extmap id out of range (1-255): {}", id)));
            }
            
            // Validate direction if present
            if let Some(ref dir) = direction {
                if !["sendonly", "recvonly", "sendrecv", "inactive"].contains(&dir.as_str()) {
                    return Err(Error::SdpParsingError(format!("Invalid extmap direction: {}", dir)));
                }
            }
            
            // Basic URI validation - should start with urn: or http:
            if !uri.starts_with("urn:") && !uri.starts_with("http:") {
                return Err(Error::SdpParsingError(format!("Invalid extmap URI: {}", uri)));
            }
            
            Ok((id, direction, uri, params))
        },
        Err(_) => Err(Error::SdpParsingError(format!("Invalid extmap format: {}", value)))
    }
} 
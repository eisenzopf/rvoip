// SDP Origin (o=) line parsing
//
// Functions for parsing the o= line in SDP messages.

use crate::error::{Error, Result};
use crate::types::sdp::Origin;
use crate::sdp::session::validation::is_valid_hostname;
use nom::{
    IResult,
    bytes::complete::{tag, take_till, take_while},
    character::complete::{digit1, space1},
    combinator::{map, opt},
    sequence::tuple,
    branch::alt,
};

/// Use nom to parse the origin line
pub fn parse_origin_nom(input: &str) -> IResult<&str, Origin> {
    // Format: o=<username> <sess-id> <sess-version> <nettype> <addrtype> <unicast-address>
    let (input, _) = opt(tag("o="))(input)?;
    let (input, (username, _, sess_id, _, sess_version, _, net_type, _, addr_type, _, addr)) = 
        tuple((
            take_till(|c| c == ' '),
            space1,
            digit1,
            space1,
            digit1,
            space1,
            tag("IN"),
            space1,
            alt((tag("IP4"), tag("IP6"))),
            space1,
            take_while(|c: char| c != '\r' && c != '\n')
        ))(input)?;
    
    Ok((
        input,
        Origin {
            username: username.to_string(),
            sess_id: sess_id.to_string(),
            sess_version: sess_version.to_string(),
            net_type: net_type.to_string(),
            addr_type: addr_type.to_string(),
            unicast_address: addr.to_string(),
        }
    ))
}

/// Parses a session origin line (o=) into an Origin struct.
/// Format: o=<username> <sess-id> <sess-version> <nettype> <addrtype> <unicast-address>
pub fn parse_origin_line(value: &str) -> Result<Origin> {
    // Try using the nom parser first
    if let Ok((_, origin)) = parse_origin_nom(value) {
        return Ok(origin);
    }
    
    // Fallback to manual parsing if nom parser fails
    // Extract value part if input has o= prefix
    let value_to_parse = if value.starts_with("o=") {
        &value[2..]
    } else {
        value
    };

    let parts: Vec<&str> = value_to_parse.split_whitespace().collect();
    if parts.len() != 6 {
        return Err(Error::SdpParsingError(format!("Invalid o= line format: {}", value)));
    }
    
    let username = parts[0];
    let session_id = parts[1];
    let session_version = parts[2];
    let net_type = parts[3];
    let addr_type = parts[4];
    let addr = parts[5];
    
    // Validate parts
    if net_type != "IN" {
        return Err(Error::SdpParsingError(format!("Unsupported network type: {}", net_type)));
    }
    
    if addr_type != "IP4" && addr_type != "IP6" {
        return Err(Error::SdpParsingError(format!("Unsupported address type: {}", addr_type)));
    }
    
    // Construct result
    Ok(Origin {
        username: username.to_string(),
        sess_id: session_id.to_string(),
        sess_version: session_version.to_string(),
        net_type: net_type.to_string(),
        addr_type: addr_type.to_string(),
        unicast_address: addr.to_string(),
    })
} 
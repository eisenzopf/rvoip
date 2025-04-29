//! SDP ICE Candidate Attribute Parser
//!
//! Implements parser for ICE candidate attributes as defined in RFC 8839.
//! Format: a=candidate:<foundation> <component-id> <transport> <priority> <conn-addr> <port> typ <cand-type> [raddr <raddr>] [rport <rport>] *(extensions)

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{positive_integer, token, to_result};
use crate::types::sdp::{CandidateAttribute, ParsedAttribute};
use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    character::complete::{char, digit1, space1},
    combinator::{map, map_res, opt, value, verify},
    multi::{many0, many_till},
    sequence::{delimited, pair, preceded, tuple},
    IResult,
};
use std::net::IpAddr;

/// Validates an IPv4 address
fn is_valid_ipv4(addr: &str) -> bool {
    if let Ok(ip) = addr.parse::<IpAddr>() {
        ip.is_ipv4()
    } else {
        // Basic format check: must have 4 parts separated by dots
        let parts: Vec<&str> = addr.split('.').collect();
        if parts.len() != 4 {
            return false;
        }

        // Each part must be a valid octet (0-255)
        parts.iter().all(|part| {
            part.parse::<u8>().is_ok()
        })
    }
}

/// Validates an IPv6 address
fn is_valid_ipv6(addr: &str) -> bool {
    if let Ok(ip) = addr.parse::<IpAddr>() {
        ip.is_ipv6()
    } else {
        // Basic validation: IPv6 has colons
        addr.contains(':') && addr.split(':').count() <= 8
    }
}

/// Validates a hostname
fn is_valid_hostname(hostname: &str) -> bool {
    // Simple validation rules for hostnames
    // - Should not be empty
    // - Should only contain letters, numbers, dots and hyphens
    // - Should not start or end with dot or hyphen
    // - Should not have consecutive dots
    
    if hostname.is_empty() {
        return false;
    }
    
    if hostname.starts_with('.') || hostname.ends_with('.') || 
       hostname.starts_with('-') || hostname.ends_with('-') {
        return false;
    }
    
    if hostname.contains("..") {
        return false;
    }
    
    hostname.chars().all(|c| {
        c.is_ascii_alphanumeric() || c == '.' || c == '-'
    })
}

/// Parser for foundation (token in the ICE spec)
fn foundation_parser(input: &str) -> IResult<&str, &str> {
    token(input)
}

/// Parser for component ID (1-256)
fn component_id_parser(input: &str) -> IResult<&str, u32> {
    verify(
        positive_integer,
        |&id| id >= 1 && id <= 256
    )(input)
}

/// Parser for transport protocol (UDP, TCP)
fn transport_parser(input: &str) -> IResult<&str, &str> {
    alt((
        tag("UDP"),
        tag("TCP"),
        tag("udp"),
        tag("tcp")
    ))(input)
}

/// Parser for priority (0-2^31-1)
fn priority_parser(input: &str) -> IResult<&str, u32> {
    positive_integer(input)
}

/// Parser for connection address (IP or hostname)
fn connection_address_parser(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| {
        c.is_ascii_alphanumeric() || c == '.' || c == ':' || c == '-'
    })(input)
}

/// Parser for port (0-65535)
fn port_parser(input: &str) -> IResult<&str, u16> {
    map_res(
        digit1,
        |s: &str| s.parse::<u16>()
    )(input)
}

/// Parser for candidate type
fn candidate_type_parser(input: &str) -> IResult<&str, &str> {
    preceded(
        pair(tag("typ"), space1),
        alt((
            tag("host"),
            tag("srflx"),
            tag("prflx"),
            tag("relay")
        ))
    )(input)
}

/// Parser for related address field
fn related_address_parser(input: &str) -> IResult<&str, &str> {
    preceded(
        pair(tag("raddr"), space1),
        connection_address_parser
    )(input)
}

/// Parser for related port field
fn related_port_parser(input: &str) -> IResult<&str, u16> {
    preceded(
        pair(tag("rport"), space1),
        port_parser
    )(input)
}

/// Parser for extension (key-value or flag)
fn extension_parser(input: &str) -> IResult<&str, (String, Option<String>)> {
    let (input, key) = token(input)?;
    
    let (input, value) = opt(preceded(
        space1,
        map(
            token,
            |s: &str| s.to_string()
        )
    ))(input)?;
    
    // If the next token would be a keyword, then treat this as a flag
    Ok((input, (key.to_string(), value)))
}

/// Main parser for ICE candidate
fn candidate_parser(input: &str) -> IResult<&str, CandidateAttribute> {
    // Parse fixed fields
    let (input, foundation) = foundation_parser(input)?;
    let (input, _) = space1(input)?;
    let (input, component_id) = component_id_parser(input)?;
    let (input, _) = space1(input)?;
    let (input, transport) = transport_parser(input)?;
    let (input, _) = space1(input)?;
    let (input, priority) = priority_parser(input)?;
    let (input, _) = space1(input)?;
    let (input, connection_address) = connection_address_parser(input)?;
    let (input, _) = space1(input)?;
    let (input, port) = port_parser(input)?;
    let (input, _) = space1(input)?;
    let (input, candidate_type) = candidate_type_parser(input)?;
    
    // Parse optional fields
    let (input, _) = opt(space1)(input)?;
    
    let mut related_address = None;
    let mut related_port = None;
    let mut extensions = Vec::new();
    let mut remaining = input;
    
    // Parse remaining fields
    while let Ok((input, _)) = space1::<_, nom::error::Error<_>>(remaining) {
        remaining = input;
        
        // Try to parse raddr
        if let Ok((input, addr)) = related_address_parser(remaining) {
            related_address = Some(addr.to_string());
            remaining = input;
            continue;
        }
        
        // Try to parse rport
        if let Ok((input, port)) = related_port_parser(remaining) {
            related_port = Some(port);
            remaining = input;
            continue;
        }
        
        // Try to parse extension
        if let Ok((input, (key, value))) = extension_parser(remaining) {
            extensions.push((key, value));
            remaining = input;
            continue;
        }
        
        // If we get here, we couldn't parse anything
        break;
    }
    
    Ok((
        remaining,
        CandidateAttribute {
            foundation: foundation.to_string(),
            component_id,
            transport: transport.to_string(),
            priority,
            connection_address: connection_address.to_string(),
            port,
            candidate_type: candidate_type.to_string(),
            related_address,
            related_port,
            extensions,
        }
    ))
}

/// Parses candidate attribute based on RFC 8839
pub fn parse_candidate(value: &str) -> Result<ParsedAttribute> {
    match candidate_parser(value.trim()) {
        Ok((_, candidate)) => {
            // Validate connection address
            let connection_address = &candidate.connection_address;
            if !is_valid_ipv4(connection_address) && 
               !is_valid_ipv6(connection_address) && 
               !is_valid_hostname(connection_address) {
                return Err(Error::SdpParsingError(format!(
                    "Invalid connection address in candidate: {}", connection_address
                )));
            }
            
            // Validate related address if present
            if let Some(ref addr) = candidate.related_address {
                if !is_valid_ipv4(addr) && 
                   !is_valid_ipv6(addr) && 
                   !is_valid_hostname(addr) {
                    return Err(Error::SdpParsingError(format!(
                        "Invalid related address in candidate: {}", addr
                    )));
                }
            }
            
            // If related address is present, related port should also be present
            if candidate.related_address.is_some() && candidate.related_port.is_none() {
                return Err(Error::SdpParsingError(
                    "Related address is present but related port is missing".to_string()
                ));
            }
            
            Ok(ParsedAttribute::Candidate(candidate))
        },
        Err(_) => Err(Error::SdpParsingError(format!("Invalid candidate attribute: {}", value)))
    }
} 
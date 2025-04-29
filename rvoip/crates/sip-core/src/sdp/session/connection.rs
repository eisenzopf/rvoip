// SDP Connection (c=) line parsing
//
// Functions for parsing the c= line in SDP messages.

use crate::error::{Error, Result};
use crate::types::sdp::ConnectionData;
use crate::sdp::session::validation::{is_valid_hostname, is_valid_ipv4_or_hostname, is_valid_ipv6_or_hostname};
use nom::{
    IResult,
    bytes::complete::{tag, take_while},
    character::complete::{char, digit1, space1},
    combinator::{map, map_res, opt, recognize},
    multi::separated_list1,
    sequence::{preceded, tuple},
    branch::alt,
};
use std::net::{Ipv4Addr, Ipv6Addr};

/// Parse an IPv4 address using nom
fn parse_ipv4_address(input: &str) -> IResult<&str, &str> {
    // We need to create a new parser function for each octet
    let parse_octet1 = map_res(digit1, |s: &str| s.parse::<u8>());
    let parse_octet2 = map_res(digit1, |s: &str| s.parse::<u8>());
    let parse_octet3 = map_res(digit1, |s: &str| s.parse::<u8>());
    let parse_octet4 = map_res(digit1, |s: &str| s.parse::<u8>());
    
    recognize(
        tuple((
            parse_octet1, char('.'),
            parse_octet2, char('.'),
            parse_octet3, char('.'),
            parse_octet4
        ))
    )(input)
}

/// Parse an IPv6 address using nom
fn parse_ipv6_address(input: &str) -> IResult<&str, &str> {
    // Simplified IPv6 parser - for a more complete one, you'd need to handle all cases
    // including compressed notation (::)
    let hex_segment = take_while(|c: char| c.is_ascii_hexdigit());
    
    let segments = separated_list1(
        char(':'),
        hex_segment
    );
    
    // Match either a full IPv6 address or one with :: compression
    recognize(segments)(input)
}

/// Parse network type (IN for Internet)
fn parse_network_type(input: &str) -> IResult<&str, &str> {
    tag("IN")(input)
}

/// Parse address type (IP4 or IP6)
fn parse_address_type(input: &str) -> IResult<&str, &str> {
    alt((tag("IP4"), tag("IP6")))(input)
}

/// Parse a connection address with optional TTL and multicast count
fn parse_connection_address_nom<'a>(input: &'a str, addr_type: &'a str) -> IResult<&'a str, (String, Option<u8>, Option<u32>)> {
    if addr_type == "IP4" {
        // IPv4 address with optional TTL and multicast count
        // Format: <base-multicast-address>[/<ttl>][/<number-of-addresses>]
        let (input, ip) = parse_ipv4_address(input)?;
        
        // Optional TTL
        let (input, ttl) = opt(preceded(
            char('/'),
            map_res(digit1, |s: &str| s.parse::<u8>())
        ))(input)?;
        
        // Optional multicast count
        let (input, count) = opt(preceded(
            char('/'),
            map_res(digit1, |s: &str| s.parse::<u32>())
        ))(input)?;
        
        Ok((input, (ip.to_string(), ttl, count)))
    } else {
        // IPv6 address - no TTL/multicast in specification
        let (input, ip) = parse_ipv6_address(input)?;
        Ok((input, (ip.to_string(), None, None)))
    }
}

/// Use nom to parse a connection line
fn parse_connection_nom(input: &str) -> IResult<&str, ConnectionData> {
    // Format: c=<nettype> <addrtype> <connection-address>
    let (input, _) = opt(tag("c="))(input)?;
    let (input, (net_type, _, addr_type, _, addr_with_ttl)) = 
        tuple((
            parse_network_type,
            space1,
            parse_address_type,
            space1,
            take_while(|c: char| c != '\r' && c != '\n')
        ))(input)?;
    
    // Parse address with potential TTL/multicast parameters
    let (_, (address, ttl, count)) = 
        parse_connection_address_nom(addr_with_ttl, addr_type)?;
    
    Ok((
        input,
        ConnectionData {
            net_type: net_type.to_string(),
            addr_type: addr_type.to_string(),
            connection_address: address,
            ttl,
            multicast_count: count,
        }
    ))
}

/// Parses a connection data line (c=) into a ConnectionData struct.
/// Format: c=<nettype> <addrtype> <connection-address>
pub fn parse_connection_line(value: &str) -> Result<ConnectionData> {
    // Try using the nom parser first
    if let Ok((_, conn_data)) = parse_connection_nom(value) {
        return Ok(conn_data);
    }
    
    // Fallback to manual parsing if nom parser fails
    // Extract value part if input has c= prefix
    let value_to_parse = if value.starts_with("c=") {
        &value[2..]
    } else {
        value
    };

    let parts: Vec<&str> = value_to_parse.split_whitespace().collect();
    if parts.len() != 3 {
        return Err(Error::SdpParsingError(format!(
            "Connection data must have exactly 3 parts: {}", value
        )));
    }
    
    // Parse network type (should be "IN" for Internet)
    let net_type = parts[0].to_string();
    if net_type != "IN" {
        return Err(Error::SdpParsingError(format!(
            "Invalid network type: {}", net_type
        )));
    }
    
    // Parse address type (should be "IP4" or "IP6")
    let addr_type = parts[1].to_string();
    if addr_type != "IP4" && addr_type != "IP6" {
        return Err(Error::SdpParsingError(format!(
            "Invalid address type: {}", addr_type
        )));
    }
    
    // Parse connection address (can be IP, FQDN, or multicast address with TTL and count)
    let addr_part = parts[2];
    let (address, ttl, multicast_count) = if addr_part.contains('/') {
        let addr_parts: Vec<&str> = addr_part.split('/').collect();
        
        if addr_parts.len() > 3 {
            return Err(Error::SdpParsingError(format!(
                "Invalid connection address format: {}", addr_part
            )));
        }
        
        let base_addr = addr_parts[0].to_string();
        
        // If using IP4 multicast, the second part is TTL
        let ttl = if addr_type == "IP4" && addr_parts.len() > 1 {
            match addr_parts[1].parse::<u8>() {
                Ok(t) => Some(t),
                Err(_) => return Err(Error::SdpParsingError(format!(
                    "Invalid TTL value: {}", addr_parts[1]
                ))),
            }
        } else {
            None
        };
        
        // The last part (if present) is the multicast count
        let count = if addr_parts.len() == 3 || (addr_type == "IP6" && addr_parts.len() == 2) {
            let count_index = if addr_parts.len() == 3 { 2 } else { 1 };
            match addr_parts[count_index].parse::<u32>() {
                Ok(c) => Some(c),
                Err(_) => return Err(Error::SdpParsingError(format!(
                    "Invalid multicast count: {}", addr_parts[count_index]
                ))),
            }
        } else {
            None
        };
        
        (base_addr, ttl, count)
    } else {
        (addr_part.to_string(), None, None)
    };
    
    // Validate that the address is correct for the address type
    if addr_type == "IP4" {
        if let Ok(ip) = address.parse::<Ipv4Addr>() {
            // If TTL is provided, ensure it's a multicast address
            if ttl.is_some() && !ip.is_multicast() {
                return Err(Error::SdpParsingError(format!(
                    "TTL provided for non-multicast address: {}", address
                )));
            }
        } else if !is_valid_hostname(&address) {
            return Err(Error::SdpParsingError(format!(
                "Invalid IPv4 address or hostname: {}", address
            )));
        }
    } else if addr_type == "IP6" {
        if let Ok(ip) = address.parse::<Ipv6Addr>() {
            // If multicast count is provided, ensure it's a multicast address
            if multicast_count.is_some() && !ip.is_multicast() {
                return Err(Error::SdpParsingError(format!(
                    "Multicast count provided for non-multicast address: {}", address
                )));
            }
        } else if !is_valid_hostname(&address) {
            return Err(Error::SdpParsingError(format!(
                "Invalid IPv6 address or hostname: {}", address
            )));
        }
    }
    
    // Create the connection data
    Ok(ConnectionData {
        net_type: net_type.to_string(),
        addr_type: addr_type.to_string(),
        connection_address: address.to_string(),
        ttl,
        multicast_count: multicast_count,
    })
} 
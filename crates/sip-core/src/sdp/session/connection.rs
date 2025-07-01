// SDP Connection (c=) line parsing
//
// Functions for parsing the c= line in SDP messages.

use crate::error::{Error, Result};
use crate::types::sdp::ConnectionData;
use crate::sdp::attributes::common::{is_valid_hostname, is_valid_ipv4, is_valid_ipv6};
use nom::{
    IResult,
    bytes::complete::{tag, take_till, take_while},
    character::complete::{char, digit1, space1},
    combinator::{map, map_res, opt},
    sequence::{preceded, tuple},
    branch::alt,
};
use std::net::{Ipv4Addr, Ipv6Addr};

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
        
        // First, parse the base address up to a possible '/'. The standard library will validate it.
        let (input, ip_str) = take_till(|c| c == '/' || c == ' ' || c == '\r' || c == '\n')(input)?;
        
        // Try to parse the IP address to validate it - treat invalid IPv4 as an error
        if !is_valid_ipv4(ip_str) {
            // Check if it's a valid hostname instead
            if !is_valid_hostname(ip_str) {
                return Err(nom::Err::Error(nom::error::Error::new(
                    ip_str,
                    nom::error::ErrorKind::Tag
                )));
            }
        }
        
        // Count the remaining '/' - should be 0, 1, or 2
        let slash_count = input.chars().filter(|&c| c == '/').count();
        if slash_count > 2 {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::TooLarge
            )));
        }
        
        // Helper function to parse a numeric value and fail on non-digits
        fn parse_numeric<T: std::str::FromStr>(input: &str) -> IResult<&str, T> {
            let (input, digits) = digit1(input)?;
            match digits.parse::<T>() {
                Ok(val) => Ok((input, val)),
                Err(_) => Err(nom::Err::Error(nom::error::Error::new(
                    input,
                    nom::error::ErrorKind::Digit
                ))),
            }
        }
        
        // Optional TTL - if we have a slash but not digits after, it's an error
        let (input, ttl) = if let Some(pos) = input.find('/') {
            let input_after_slash = &input[pos+1..];
            if input_after_slash.is_empty() || !input_after_slash.chars().next().unwrap().is_ascii_digit() {
                return Err(nom::Err::Error(nom::error::Error::new(
                    input,
                    nom::error::ErrorKind::Digit
                )));
            }
            let (input, _) = tag("/")(input)?;
            let (input, ttl) = parse_numeric::<u8>(input)?;
            (input, Some(ttl))
        } else {
            (input, None)
        };
        
        // Optional multicast count - if we have a slash but not digits after, it's an error
        let (input, count) = if let Some(pos) = input.find('/') {
            let input_after_slash = &input[pos+1..];
            if input_after_slash.is_empty() || !input_after_slash.chars().next().unwrap().is_ascii_digit() {
                return Err(nom::Err::Error(nom::error::Error::new(
                    input,
                    nom::error::ErrorKind::Digit
                )));
            }
            let (input, _) = tag("/")(input)?;
            let (input, count) = parse_numeric::<u32>(input)?;
            (input, Some(count))
        } else {
            (input, None)
        };
        
        Ok((input, (ip_str.to_string(), ttl, count)))
    } else {
        // IPv6 address with optional multicast count (no TTL)
        
        // First, parse the base address up to a possible '/'. The standard library will validate it.
        let (input, ip_str) = take_till(|c| c == '/' || c == ' ' || c == '\r' || c == '\n')(input)?;
        
        // Try to parse the IP address to validate it - treat invalid IPv6 as an error
        if !is_valid_ipv6(ip_str) {
            // Check if it's a valid hostname instead
            if !is_valid_hostname(ip_str) {
                return Err(nom::Err::Error(nom::error::Error::new(
                    ip_str,
                    nom::error::ErrorKind::Tag
                )));
            }
        }
        
        // Count the remaining '/' - should be 0 or 1 for IPv6
        let slash_count = input.chars().filter(|&c| c == '/').count();
        if slash_count > 1 {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::TooLarge
            )));
        }
        
        // Helper function to parse a numeric value and fail on non-digits
        fn parse_numeric<T: std::str::FromStr>(input: &str) -> IResult<&str, T> {
            let (input, digits) = digit1(input)?;
            match digits.parse::<T>() {
                Ok(val) => Ok((input, val)),
                Err(_) => Err(nom::Err::Error(nom::error::Error::new(
                    input,
                    nom::error::ErrorKind::Digit
                ))),
            }
        }
        
        // Optional multicast count - if we have a slash but not digits after, it's an error
        let (input, count) = if let Some(pos) = input.find('/') {
            let input_after_slash = &input[pos+1..];
            if input_after_slash.is_empty() || !input_after_slash.chars().next().unwrap().is_ascii_digit() {
                return Err(nom::Err::Error(nom::error::Error::new(
                    input,
                    nom::error::ErrorKind::Digit
                )));
            }
            let (input, _) = tag("/")(input)?;
            let (input, count) = parse_numeric::<u32>(input)?;
            (input, Some(count))
        } else {
            (input, None)
        };
        
        Ok((input, (ip_str.to_string(), None, count)))
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
        // Validate the parsed data
        validate_connection_data(&conn_data)?;
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
        
        // Validate the base address based on address type
        if addr_type == "IP4" {
            if !is_valid_ipv4(&base_addr) {
                // Additional check for IP-looking hostnames that are invalid IPv4 (like "192.168.1")
                if base_addr.chars().all(|c| c.is_ascii_digit() || c == '.') && 
                   base_addr.contains('.') &&
                   base_addr.split('.').count() < 4 {
                    return Err(Error::SdpParsingError(format!(
                        "Invalid IPv4 address (too few segments): {}", base_addr
                    )));
                }
                
                if !is_valid_hostname(&base_addr) {
                    return Err(Error::SdpParsingError(format!(
                        "Invalid IPv4 address or hostname: {}", base_addr
                    )));
                }
            }
        } else if addr_type == "IP6" {
            if !is_valid_ipv6(&base_addr) {
                if !is_valid_hostname(&base_addr) {
                    return Err(Error::SdpParsingError(format!(
                        "Invalid IPv6 address or hostname: {}", base_addr
                    )));
                }
            }
        }
        
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
        // Validate the address based on address type
        if addr_type == "IP4" {
            // This case specifically tests "192.168.1" which should not be a valid IPv4 address
            if !is_valid_ipv4(addr_part) {
                // Additional check for IP-looking hostnames that are invalid IPv4 (like "192.168.1")
                if addr_part.chars().all(|c| c.is_ascii_digit() || c == '.') && 
                   addr_part.contains('.') &&
                   addr_part.split('.').count() < 4 {
                    return Err(Error::SdpParsingError(format!(
                        "Invalid IPv4 address (too few segments): {}", addr_part
                    )));
                }
                
                if !is_valid_hostname(addr_part) {
                    return Err(Error::SdpParsingError(format!(
                        "Invalid IPv4 address or hostname: {}", addr_part
                    )));
                }
            }
        } else if addr_type == "IP6" {
            if !is_valid_ipv6(addr_part) {
                if !is_valid_hostname(addr_part) {
                    return Err(Error::SdpParsingError(format!(
                        "Invalid IPv6 address or hostname: {}", addr_part
                    )));
                }
            }
        }
        
        (addr_part.to_string(), None, None)
    };
    
    let conn_data = ConnectionData {
        net_type: net_type.to_string(),
        addr_type: addr_type.to_string(),
        connection_address: address.to_string(),
        ttl,
        multicast_count,
    };
    
    // Validate the parsed data
    validate_connection_data(&conn_data)?;
    
    // Create the connection data
    Ok(conn_data)
}

/// Validate connection data according to RFC 4566 rules
fn validate_connection_data(conn_data: &ConnectionData) -> Result<()> {
    // Validate that the address is correct for the address type
    let addr = &conn_data.connection_address;
    let addr_type = &conn_data.addr_type;
    
    if addr_type == "IP4" {
        // For IP4, reject any address that looks like a partial IPv4 address
        let looks_like_partial_ipv4 = addr.chars().all(|c| c.is_ascii_digit() || c == '.') && 
                                     addr.contains('.') &&
                                     addr.split('.').count() < 4;
                                     
        if looks_like_partial_ipv4 {
            return Err(Error::SdpParsingError(format!(
                "Invalid IPv4 address (incomplete format): {}", addr
            )));
        }
        
        // Direct parsing to validate IPv4 address
        if is_valid_ipv4(addr) {
            // If TTL is provided, ensure it's a multicast address
            if conn_data.ttl.is_some() {
                let ip = addr.parse::<Ipv4Addr>().unwrap(); // Safe because we validated above
                if !ip.is_multicast() {
                    return Err(Error::SdpParsingError(format!(
                        "TTL provided for non-multicast address: {}", addr
                    )));
                }
            }
            
            // If multicast count is provided, ensure it's a multicast address
            if conn_data.multicast_count.is_some() {
                let ip = addr.parse::<Ipv4Addr>().unwrap(); // Safe because we validated above
                if !ip.is_multicast() {
                    return Err(Error::SdpParsingError(format!(
                        "Multicast count provided for non-multicast address: {}", addr
                    )));
                }
            }
        } else {
            // If it's not a valid IPv4 address, check if it's a valid hostname
            if !is_valid_hostname(addr) {
                return Err(Error::SdpParsingError(format!(
                    "Invalid IPv4 address or hostname: {}", addr
                )));
            }
        }
    } else if addr_type == "IP6" {
        // Direct parsing to validate IPv6 address
        if is_valid_ipv6(addr) {
            // TTL is not used for IPv6
            if conn_data.ttl.is_some() {
                return Err(Error::SdpParsingError(format!(
                    "TTL not allowed for IPv6 address: {}", addr
                )));
            }
            
            // If multicast count is provided, ensure it's a multicast address
            if conn_data.multicast_count.is_some() {
                let ip = addr.parse::<Ipv6Addr>().unwrap(); // Safe because we validated above
                if !ip.is_multicast() {
                    return Err(Error::SdpParsingError(format!(
                        "Multicast count provided for non-multicast address: {}", addr
                    )));
                }
            }
        } else {
            // If it's not a valid IPv6 address, check if it's a valid hostname
            if !is_valid_hostname(addr) {
                return Err(Error::SdpParsingError(format!(
                    "Invalid IPv6 address or hostname: {}", addr
                )));
            }
        }
    }
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_basic_ipv4_connection() {
        // Basic IPv4 connection data according to RFC 4566
        let conn_line = "IN IP4 192.168.1.1";
        let result = parse_connection_line(conn_line);
        assert!(result.is_ok(), "Failed to parse basic IPv4 connection");
        
        let conn = result.unwrap();
        assert_eq!(conn.net_type, "IN", "Incorrect network type");
        assert_eq!(conn.addr_type, "IP4", "Incorrect address type");
        assert_eq!(conn.connection_address, "192.168.1.1", "Incorrect connection address");
        assert_eq!(conn.ttl, None, "TTL should be None for unicast");
        assert_eq!(conn.multicast_count, None, "Multicast count should be None for unicast");
    }
    
    #[test]
    fn test_parse_with_c_prefix() {
        // Connection data with c= prefix
        let conn_line = "c=IN IP4 192.168.1.1";
        let result = parse_connection_line(conn_line);
        assert!(result.is_ok(), "Failed to parse connection with c= prefix");
        
        let conn = result.unwrap();
        assert_eq!(conn.connection_address, "192.168.1.1", "Incorrect connection address");
    }
    
    #[test]
    fn test_parse_ipv4_multicast() {
        // IPv4 multicast with TTL (time-to-live)
        let conn_line = "IN IP4 224.2.36.42/127";
        let result = parse_connection_line(conn_line);
        assert!(result.is_ok(), "Failed to parse IPv4 multicast with TTL");
        
        let conn = result.unwrap();
        assert_eq!(conn.connection_address, "224.2.36.42", "Incorrect multicast address");
        assert_eq!(conn.ttl, Some(127), "Incorrect TTL value");
        assert_eq!(conn.multicast_count, None, "Multicast count should be None");
    }
    
    #[test]
    fn test_parse_ipv4_multicast_with_count() {
        // IPv4 multicast with TTL and count
        let conn_line = "IN IP4 224.2.36.42/127/3";
        let result = parse_connection_line(conn_line);
        assert!(result.is_ok(), "Failed to parse IPv4 multicast with TTL and count");
        
        let conn = result.unwrap();
        assert_eq!(conn.connection_address, "224.2.36.42", "Incorrect multicast address");
        assert_eq!(conn.ttl, Some(127), "Incorrect TTL value");
        assert_eq!(conn.multicast_count, Some(3), "Incorrect multicast count");
    }
    
    #[test]
    fn test_parse_basic_ipv6_connection() {
        // Basic IPv6 connection data
        let conn_line = "IN IP6 2001:db8::1";
        let result = parse_connection_line(conn_line);
        assert!(result.is_ok(), "Failed to parse basic IPv6 connection");
        
        let conn = result.unwrap();
        assert_eq!(conn.net_type, "IN", "Incorrect network type");
        assert_eq!(conn.addr_type, "IP6", "Incorrect address type");
        assert_eq!(conn.connection_address, "2001:db8::1", "Incorrect connection address");
        assert_eq!(conn.ttl, None, "TTL should be None for IPv6");
        assert_eq!(conn.multicast_count, None, "Multicast count should be None");
    }
    
    #[test]
    fn test_parse_ipv6_multicast() {
        // IPv6 multicast with count (RFC 4566 Section 5.7)
        let conn_line = "IN IP6 FF15::101/3";
        let result = parse_connection_line(conn_line);
        assert!(result.is_ok(), "Failed to parse IPv6 multicast with count");
        
        let conn = result.unwrap();
        assert_eq!(conn.connection_address, "FF15::101", "Incorrect IPv6 multicast address");
        // Note: For IPv6, the second part is interpreted as count, not TTL
        assert_eq!(conn.ttl, None, "TTL should be None for IPv6");
        assert_eq!(conn.multicast_count, Some(3), "Incorrect multicast count");
    }
    
    #[test]
    fn test_parse_hostname_connection() {
        // Connection data with hostname
        let conn_line = "IN IP4 example.com";
        let result = parse_connection_line(conn_line);
        assert!(result.is_ok(), "Failed to parse connection with hostname");
        
        let conn = result.unwrap();
        assert_eq!(conn.connection_address, "example.com", "Incorrect hostname");
    }
    
    #[test]
    fn test_parse_fqdn_connection() {
        // Connection data with FQDN
        let conn_line = "IN IP4 server.example.com";
        let result = parse_connection_line(conn_line);
        assert!(result.is_ok(), "Failed to parse connection with FQDN");
        
        let conn = result.unwrap();
        assert_eq!(conn.connection_address, "server.example.com", "Incorrect FQDN");
    }
    
    #[test]
    fn test_invalid_network_type() {
        // Invalid network type (not "IN")
        let conn_line = "NET IP4 192.168.1.1";
        let result = parse_connection_line(conn_line);
        assert!(result.is_err(), "Should reject invalid network type");
    }
    
    #[test]
    fn test_invalid_address_type() {
        // Invalid address type (not "IP4" or "IP6")
        let conn_line = "IN IPX 192.168.1.1";
        let result = parse_connection_line(conn_line);
        assert!(result.is_err(), "Should reject invalid address type");
    }
    
    #[test]
    fn test_invalid_ipv4_format() {
        // Invalid IPv4 format
        let conn_line = "IN IP4 192.168.1";
        let result = parse_connection_line(conn_line);
        assert!(result.is_err(), "Should reject invalid IPv4 format");
    }
    
    #[test]
    fn test_invalid_ipv6_format() {
        // Invalid IPv6 format
        let conn_line = "IN IP6 2001:db8:::1";
        let result = parse_connection_line(conn_line);
        assert!(result.is_err(), "Should reject invalid IPv6 format");
    }
    
    #[test]
    fn test_ttl_with_non_multicast() {
        // TTL with non-multicast IPv4 (should be rejected)
        let conn_line = "IN IP4 192.168.1.1/127";
        let result = parse_connection_line(conn_line);
        assert!(result.is_err(), "Should reject TTL with non-multicast address");
    }
    
    #[test]
    fn test_invalid_ttl_format() {
        // Invalid TTL format
        let conn_line = "IN IP4 224.2.36.42/abc";
        let result = parse_connection_line(conn_line);
        assert!(result.is_err(), "Should reject invalid TTL format");
    }
    
    #[test]
    fn test_invalid_multicast_count_format() {
        // Invalid multicast count format
        let conn_line = "IN IP4 224.2.36.42/127/abc";
        let result = parse_connection_line(conn_line);
        assert!(result.is_err(), "Should reject invalid multicast count format");
    }
    
    #[test]
    fn test_too_many_parts() {
        // Too many parts in connection address
        let conn_line = "IN IP4 224.2.36.42/127/3/4";
        let result = parse_connection_line(conn_line);
        assert!(result.is_err(), "Should reject too many parts in connection address");
    }
    
    #[test]
    fn test_too_few_parts() {
        // Too few parts in connection line
        let conn_line = "IN IP4";
        let result = parse_connection_line(conn_line);
        assert!(result.is_err(), "Should reject connection data with too few parts");
    }
    
    #[test]
    fn test_whitespace_handling() {
        // Extra whitespace in connection line
        let conn_line = "  IN   IP4   192.168.1.1  ";
        let result = parse_connection_line(conn_line);
        assert!(result.is_ok(), "Failed to parse connection with extra whitespace");
        
        let conn = result.unwrap();
        assert_eq!(conn.connection_address, "192.168.1.1", "Incorrect connection address");
    }
    
    #[test]
    fn test_parse_rfc4566_examples() {
        // Examples from RFC 4566
        let examples = [
            "c=IN IP4 224.2.36.42/127",              // Example with TTL
            "c=IN IP4 224.2.1.1/127/3",              // Example with TTL and count
            "c=IN IP6 FF15::101/3",                  // IPv6 multicast
        ];
        
        for example in examples.iter() {
            let result = parse_connection_line(example);
            assert!(result.is_ok(), "Failed to parse RFC example: {}", example);
        }
    }
} 
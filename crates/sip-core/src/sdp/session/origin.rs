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
    
    // Parse each field with strict whitespace between them
    let (remainder, (username, _, sess_id, _, sess_version, _, net_type, _, addr_type, _, addr)) = 
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
            take_till(|c| c == ' ' || c == '\r' || c == '\n')
        ))(input)?;
    
    // Check that there's no extra content (extra fields would be separated by spaces)
    let remainder_trimmed = remainder.trim();
    if !remainder_trimmed.is_empty() && remainder_trimmed.starts_with(' ') {
        return Err(nom::Err::Error(nom::error::Error::new(
            remainder,
            nom::error::ErrorKind::TooLarge
        )));
    }
    
    Ok((
        remainder,
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
    if let Ok((remainder, origin)) = parse_origin_nom(value) {
        // Check that there's no extra content after the parsed origin
        if !remainder.trim().is_empty() {
            return Err(Error::SdpParsingError(format!("Invalid o= line format (extra content): {}", value)));
        }
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
    
    // Validate session ID is numeric
    if session_id.parse::<u64>().is_err() {
        return Err(Error::SdpParsingError(format!("Session ID must be numeric: {}", session_id)));
    }
    
    // Validate session version is numeric
    if session_version.parse::<u64>().is_err() {
        return Err(Error::SdpParsingError(format!("Session version must be numeric: {}", session_version)));
    }
    
    // Validate network type
    if net_type != "IN" {
        return Err(Error::SdpParsingError(format!("Unsupported network type: {}", net_type)));
    }
    
    // Validate address type
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

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_basic_origin() {
        // Basic valid origin line according to RFC 4566
        let origin_line = "jdoe 2890844526 2890842807 IN IP4 10.47.16.5";
        let result = parse_origin_line(origin_line);
        assert!(result.is_ok(), "Failed to parse basic origin line");
        
        let origin = result.unwrap();
        assert_eq!(origin.username, "jdoe", "Incorrect username");
        assert_eq!(origin.sess_id, "2890844526", "Incorrect session ID");
        assert_eq!(origin.sess_version, "2890842807", "Incorrect session version");
        assert_eq!(origin.net_type, "IN", "Incorrect network type");
        assert_eq!(origin.addr_type, "IP4", "Incorrect address type");
        assert_eq!(origin.unicast_address, "10.47.16.5", "Incorrect unicast address");
    }
    
    #[test]
    fn test_parse_with_o_prefix() {
        // Origin line with o= prefix
        let origin_line = "o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5";
        let result = parse_origin_line(origin_line);
        assert!(result.is_ok(), "Failed to parse origin with o= prefix");
        
        let origin = result.unwrap();
        assert_eq!(origin.username, "jdoe", "Incorrect username");
        assert_eq!(origin.unicast_address, "10.47.16.5", "Incorrect unicast address");
    }
    
    #[test]
    fn test_parse_with_hyphen_username() {
        // RFC 4566: "The <username> is the user's login on the originating host"
        // If the originating host does not support the concept of user IDs, 
        // the <username> should be "-".
        let origin_line = "- 2890844526 2890842807 IN IP4 10.47.16.5";
        let result = parse_origin_line(origin_line);
        assert!(result.is_ok(), "Failed to parse origin with hyphen username");
        
        let origin = result.unwrap();
        assert_eq!(origin.username, "-", "Incorrect username");
    }
    
    #[test]
    fn test_parse_ipv6_address() {
        // Origin line with IPv6 address
        let origin_line = "jdoe 2890844526 2890842807 IN IP6 2001:db8::1";
        let result = parse_origin_line(origin_line);
        assert!(result.is_ok(), "Failed to parse origin with IPv6 address");
        
        let origin = result.unwrap();
        assert_eq!(origin.addr_type, "IP6", "Incorrect address type");
        assert_eq!(origin.unicast_address, "2001:db8::1", "Incorrect unicast address");
    }
    
    #[test]
    fn test_parse_hostname() {
        // Origin line with hostname
        let origin_line = "jdoe 2890844526 2890842807 IN IP4 example.com";
        let result = parse_origin_line(origin_line);
        assert!(result.is_ok(), "Failed to parse origin with hostname");
        
        let origin = result.unwrap();
        assert_eq!(origin.unicast_address, "example.com", "Incorrect unicast address");
    }
    
    #[test]
    fn test_parse_numeric_username() {
        // Username can be numeric according to RFC
        let origin_line = "1234 2890844526 2890842807 IN IP4 10.47.16.5";
        let result = parse_origin_line(origin_line);
        assert!(result.is_ok(), "Failed to parse origin with numeric username");
        
        let origin = result.unwrap();
        assert_eq!(origin.username, "1234", "Incorrect username");
    }
    
    #[test]
    fn test_parse_large_session_values() {
        // Large numeric values for session ID and version
        let origin_line = "jdoe 9223372036854775807 9223372036854775807 IN IP4 10.47.16.5";
        let result = parse_origin_line(origin_line);
        assert!(result.is_ok(), "Failed to parse origin with large session values");
        
        let origin = result.unwrap();
        assert_eq!(origin.sess_id, "9223372036854775807", "Incorrect session ID");
        assert_eq!(origin.sess_version, "9223372036854775807", "Incorrect session version");
    }
    
    #[test]
    fn test_invalid_network_type() {
        // Invalid network type (not "IN")
        let origin_line = "jdoe 2890844526 2890842807 NET IP4 10.47.16.5";
        let result = parse_origin_line(origin_line);
        assert!(result.is_err(), "Should reject invalid network type");
        
        if let Err(e) = result {
            let error_message = format!("{:?}", e);
            assert!(error_message.contains("Unsupported network type"), 
                    "Error message should mention unsupported network type");
        }
    }
    
    #[test]
    fn test_invalid_address_type() {
        // Invalid address type (not "IP4" or "IP6")
        let origin_line = "jdoe 2890844526 2890842807 IN IPX 10.47.16.5";
        let result = parse_origin_line(origin_line);
        assert!(result.is_err(), "Should reject invalid address type");
        
        if let Err(e) = result {
            let error_message = format!("{:?}", e);
            assert!(error_message.contains("Unsupported address type"), 
                    "Error message should mention unsupported address type");
        }
    }
    
    #[test]
    fn test_too_few_fields() {
        // Too few fields in origin line
        let origin_line = "jdoe 2890844526 2890842807 IN IP4";
        let result = parse_origin_line(origin_line);
        assert!(result.is_err(), "Should reject origin with too few fields");
        
        if let Err(e) = result {
            let error_message = format!("{:?}", e);
            assert!(error_message.contains("Invalid o= line format"), 
                    "Error message should mention invalid format");
        }
    }
    
    #[test]
    fn test_too_many_fields() {
        // Too many fields in origin line
        let origin_line = "jdoe 2890844526 2890842807 IN IP4 10.47.16.5 extra";
        let result = parse_origin_line(origin_line);
        assert!(result.is_err(), "Should reject origin with too many fields");
        
        if let Err(e) = result {
            let error_message = format!("{:?}", e);
            assert!(error_message.contains("Invalid o= line format"), 
                    "Error message should mention invalid format");
        }
    }
    
    #[test]
    fn test_non_numeric_session_id() {
        // Session ID must be numeric
        let origin_line = "jdoe sessionid 2890842807 IN IP4 10.47.16.5";
        let result = parse_origin_line(origin_line);
        assert!(result.is_err(), "Should reject non-numeric session ID");
    }
    
    #[test]
    fn test_non_numeric_session_version() {
        // Session version must be numeric
        let origin_line = "jdoe 2890844526 version IN IP4 10.47.16.5";
        let result = parse_origin_line(origin_line);
        assert!(result.is_err(), "Should reject non-numeric session version");
    }
    
    #[test]
    fn test_whitespace_handling() {
        // Extra whitespace in origin line
        let origin_line = "  jdoe   2890844526  2890842807   IN   IP4   10.47.16.5  ";
        let result = parse_origin_line(origin_line);
        assert!(result.is_ok(), "Failed to parse origin with extra whitespace");
        
        let origin = result.unwrap();
        assert_eq!(origin.username, "jdoe", "Incorrect username");
        assert_eq!(origin.unicast_address, "10.47.16.5", "Incorrect unicast address");
    }
    
    #[test]
    fn test_rfc_examples() {
        // Examples from RFC 4566
        let examples = [
            "o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5",
            "o=- 2890844526 2890842807 IN IP4 10.47.16.5"
        ];
        
        for example in examples.iter() {
            let result = parse_origin_line(example);
            assert!(result.is_ok(), "Failed to parse RFC example: {}", example);
        }
    }
} 
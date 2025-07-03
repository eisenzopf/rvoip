//! Session-level SDP parsing functionality
//!
//! This module handles parsing of session-level SDP elements according to RFC 8866, including:
//! - Origin (o=)
//! - Session Name (s=)
//! - Connection Data (c=)
//!
//! The Session Description Protocol (SDP) is a format for describing multimedia session parameters
//! for the purposes of session announcement, session invitation, and parameter negotiation.
//!
//! # SDP Example with Session-Level Elements
//! ```
//! # // We need to use the public API through parse_sdp instead of directly accessing session_parser
//! use bytes::Bytes;
//! use rvoip_sip_core::sdp::parser::parse_sdp;
//!
//! let sdp_str = "\
//! v=0
//! o=alice 123456 789 IN IP4 192.168.1.1
//! s=Example Session
//! c=IN IP4 224.2.36.42/127
//! t=0 0
//! ";
//!
//! let session = parse_sdp(&Bytes::from(sdp_str)).unwrap();
//! assert_eq!(session.origin.username, "alice");
//! assert_eq!(session.origin.unicast_address, "192.168.1.1");
//! 
//! // Check connection data
//! let connection = session.connection_info.unwrap();
//! assert_eq!(connection.connection_address, "224.2.36.42");
//! assert_eq!(connection.ttl, Some(127));
//! ```

use crate::error::{Error, Result};
use crate::types::sdp::{SdpSession, Origin, ConnectionData};
use super::validation::{validate_network_type, validate_address_type, is_valid_address};

/// Initialize a default SDP session
///
/// Creates an SDP session with default values according to the SDP specification.
/// The default session has an empty session name and default origin fields.
///
/// # Example
///
/// ```
/// # // Since this is a private module, we need to demonstrate this through the public API
/// use bytes::Bytes;
/// use rvoip_sip_core::sdp::parser::parse_sdp;
///
/// // Minimal valid SDP contains version, origin, session name, and timing
/// let minimal_sdp = "\
/// v=0
/// o=- 0 0 IN IP4 0.0.0.0
/// s=Session
/// t=0 0
/// ";
/// 
/// let session = parse_sdp(&Bytes::from(minimal_sdp)).unwrap();
///
/// // Check the default values from the SDP
/// assert_eq!(session.origin.username, "-");
/// assert_eq!(session.origin.sess_id, "0");
/// assert_eq!(session.origin.sess_version, "0");
/// assert_eq!(session.origin.unicast_address, "0.0.0.0");
/// 
/// // This will be different because we had to provide a non-empty session name
/// // while init_session_description() creates an empty session name by default
/// assert_eq!(session.session_name, "Session");
/// ```
///
/// # Returns
///
/// A new SDP session with default values
pub fn init_session_description() -> SdpSession {
    // Create default origin
    let origin = Origin {
        username: "-".to_string(),
        sess_id: "0".to_string(),
        sess_version: "0".to_string(),
        net_type: "IN".to_string(),
        addr_type: "IP4".to_string(),
        unicast_address: "0.0.0.0".to_string(),
    };
    
    // Create session with default values
    SdpSession::new(origin, "".to_string())
}

/// Parse an origin line (o=)
///
/// # Format
///
/// ```text
/// o=<username> <sess-id> <sess-version> <nettype> <addrtype> <unicast-address>
/// ```
///
/// The origin field identifies the session creator and a session identifier.
/// 
/// # Examples
///
/// ```
/// # // Demonstrate through the public API
/// use bytes::Bytes;
/// use rvoip_sip_core::sdp::parser::parse_sdp;
///
/// // Parse an SDP with just the required fields
/// let sdp_str = "\
/// v=0
/// o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5
/// s=Example
/// t=0 0
/// ";
///
/// let session = parse_sdp(&Bytes::from(sdp_str)).unwrap();
/// let origin = &session.origin;
///
/// // Check origin fields
/// assert_eq!(origin.username, "jdoe");
/// assert_eq!(origin.sess_id, "2890844526");
/// assert_eq!(origin.sess_version, "2890842807");
/// assert_eq!(origin.net_type, "IN");
/// assert_eq!(origin.addr_type, "IP4");
/// assert_eq!(origin.unicast_address, "10.47.16.5");
///
/// // IPv6 example
/// let sdp_with_ipv6 = "\
/// v=0
/// o=- 123456 789 IN IP6 2001:db8::1
/// s=Example
/// t=0 0
/// ";
///
/// let session = parse_sdp(&Bytes::from(sdp_with_ipv6)).unwrap();
/// assert_eq!(session.origin.addr_type, "IP6");
/// assert_eq!(session.origin.unicast_address, "2001:db8::1");
/// ```
///
/// # Parameters
///
/// - `value`: The value part of the origin line
///
/// # Returns
///
/// - `Ok(Origin)` if parsing succeeds
/// - `Err` with error details if parsing fails
///
/// # Errors
///
/// Returns an error if:
/// - The format doesn't match the required 6 fields
/// - Network type is not "IN"
/// - Address type is not "IP4" or "IP6"
/// - The address is invalid for the specified address type
pub fn parse_origin_line(value: &str) -> Result<Origin> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() != 6 {
        return Err(Error::SdpParsingError(format!("Invalid origin line format: {}", value)));
    }
    
    let username = parts[0].to_string();
    let sess_id = parts[1].to_string();
    let sess_version = parts[2].to_string();
    let net_type = parts[3].to_string();
    let addr_type = parts[4].to_string();
    let unicast_address = parts[5].to_string();
    
    // Validate the components
    validate_network_type(&net_type)?;
    validate_address_type(&addr_type)?;
    
    if !is_valid_address(&unicast_address, &addr_type) {
        return Err(Error::SdpParsingError(format!("Invalid address: {}", unicast_address)));
    }
    
    Ok(Origin {
        username,
        sess_id,
        sess_version,
        net_type,
        addr_type,
        unicast_address,
    })
}

/// Parse a connection line (c=)
///
/// # Format
///
/// ```text
/// c=<nettype> <addrtype> <connection-address>
/// ```
///
/// The connection field contains connection data for the session or for specific media.
/// 
/// For IP4 multicast, the connection address may include a TTL value and a multicast count:
/// ```text
/// c=IN IP4 224.2.36.42/127/3
/// ```
///
/// For IP6 multicast, the connection address may include a scope value:
/// ```text
/// c=IN IP6 FF15::101/3
/// ```
///
/// # Examples
///
/// ```
/// # // Demonstrate through the public API
/// use bytes::Bytes;
/// use rvoip_sip_core::sdp::parser::parse_sdp;
///
/// // SDP with IPv4 unicast connection
/// let sdp_str = "\
/// v=0
/// o=- 123 456 IN IP4 0.0.0.0
/// s=Example
/// c=IN IP4 192.168.1.1
/// t=0 0
/// ";
///
/// let session = parse_sdp(&Bytes::from(sdp_str)).unwrap();
/// let conn = session.connection_info.unwrap();
/// assert_eq!(conn.net_type, "IN");
/// assert_eq!(conn.addr_type, "IP4");
/// assert_eq!(conn.connection_address, "192.168.1.1");
/// assert_eq!(conn.ttl, None);
/// assert_eq!(conn.multicast_count, None);
///
/// // SDP with IPv4 multicast connection including TTL and count
/// let multicast_sdp = "\
/// v=0
/// o=- 123 456 IN IP4 0.0.0.0
/// s=Example
/// c=IN IP4 224.2.36.42/127/3
/// t=0 0
/// ";
///
/// let session = parse_sdp(&Bytes::from(multicast_sdp)).unwrap();
/// let conn = session.connection_info.unwrap();
/// assert_eq!(conn.connection_address, "224.2.36.42");
/// assert_eq!(conn.ttl, Some(127));
/// assert_eq!(conn.multicast_count, Some(3));
/// ```
///
/// # Parameters
///
/// - `value`: The value part of the connection line
///
/// # Returns
///
/// - `Ok(ConnectionData)` if parsing succeeds
/// - `Err` with error details if parsing fails
///
/// # Errors
///
/// Returns an error if:
/// - The format doesn't match the required 3 fields
/// - Network type is not "IN"
/// - Address type is not "IP4" or "IP6"
/// - The address is invalid for the specified address type
/// - TTL or multicast count values are not valid numbers
pub fn parse_connection_line(value: &str) -> Result<ConnectionData> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() != 3 {
        return Err(Error::SdpParsingError(format!("Invalid connection line format: {}", value)));
    }
    
    let net_type = parts[0].to_string();
    let addr_type = parts[1].to_string();
    let connection_address = parts[2].to_string();
    
    // Validate components
    validate_network_type(&net_type)?;
    validate_address_type(&addr_type)?;
    
    // Parse address, TTL, and multicast count
    let mut ttl = None;
    let mut multicast_count = None;
    
    let addr_parts: Vec<&str> = connection_address.split('/').collect();
    let base_addr = addr_parts[0];
    
    if !is_valid_address(base_addr, &addr_type) {
        return Err(Error::SdpParsingError(format!("Invalid address: {}", base_addr)));
    }
    
    // Parse TTL and multicast count if present
    if addr_parts.len() > 1 {
        match addr_parts[1].parse::<u8>() {
            Ok(val) => ttl = Some(val),
            Err(_) => return Err(Error::SdpParsingError(format!("Invalid TTL: {}", addr_parts[1]))),
        }
    }
    
    if addr_parts.len() > 2 {
        match addr_parts[2].parse::<u32>() {
            Ok(val) => multicast_count = Some(val),
            Err(_) => return Err(Error::SdpParsingError(format!("Invalid multicast count: {}", addr_parts[2]))),
        }
    }
    
    Ok(ConnectionData {
        net_type,
        addr_type,
        connection_address: base_addr.to_string(),
        ttl,
        multicast_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_session_description() {
        let session = init_session_description();
        
        // Check default origin values
        assert_eq!(session.origin.username, "-");
        assert_eq!(session.origin.sess_id, "0");
        assert_eq!(session.origin.sess_version, "0");
        assert_eq!(session.origin.net_type, "IN");
        assert_eq!(session.origin.addr_type, "IP4");
        assert_eq!(session.origin.unicast_address, "0.0.0.0");
        
        // Check other default session values
        assert_eq!(session.session_name, "");
        assert!(session.time_descriptions.is_empty());
        assert!(session.media_descriptions.is_empty());
        assert!(session.connection_info.is_none());
        assert!(session.direction.is_none());
    }

    #[test]
    fn test_parse_origin_line_valid() {
        // Standard example from RFC 8866
        let result = parse_origin_line("jdoe 2890844526 2890842807 IN IP4 10.47.16.5");
        assert!(result.is_ok());
        
        let origin = result.unwrap();
        assert_eq!(origin.username, "jdoe");
        assert_eq!(origin.sess_id, "2890844526");
        assert_eq!(origin.sess_version, "2890842807");
        assert_eq!(origin.net_type, "IN");
        assert_eq!(origin.addr_type, "IP4");
        assert_eq!(origin.unicast_address, "10.47.16.5");
        
        // Test with IPv6 address
        let result = parse_origin_line("- 123456 789 IN IP6 2001:db8::1");
        assert!(result.is_ok());
        
        let origin = result.unwrap();
        assert_eq!(origin.username, "-");
        assert_eq!(origin.sess_id, "123456");
        assert_eq!(origin.sess_version, "789");
        assert_eq!(origin.net_type, "IN");
        assert_eq!(origin.addr_type, "IP6");
        assert_eq!(origin.unicast_address, "2001:db8::1");
        
        // Test with hostname
        let result = parse_origin_line("user 123 456 IN IP4 example.com");
        assert!(result.is_ok());
        
        let origin = result.unwrap();
        assert_eq!(origin.username, "user");
        assert_eq!(origin.unicast_address, "example.com");
    }

    #[test]
    fn test_parse_origin_line_invalid() {
        // Too few parts - SDP should enforce the exact format for o= line
        let result = parse_origin_line("jdoe 2890844526 2890842807 IN IP4");
        assert!(result.is_err());
        
        // Too many parts - SDP should not allow extra fields
        let result = parse_origin_line("jdoe 2890844526 2890842807 IN IP4 10.47.16.5 extrapart");
        assert!(result.is_err());
        
        // Invalid network type - RFC 8866 only defines "IN" for Internet
        let result = parse_origin_line("jdoe 2890844526 2890842807 INVALID IP4 10.47.16.5");
        assert!(result.is_err());
        
        // Invalid address type - RFC 8866 only defines "IP4" and "IP6"
        let result = parse_origin_line("jdoe 2890844526 2890842807 IN IPX 10.47.16.5");
        assert!(result.is_err());
        
        // Incomplete IPv4 address - Important to validate as partial addresses could cause issues
        let result = parse_origin_line("jdoe 2890844526 2890842807 IN IP4 192.168.1");
        assert!(result.is_err());
        
        // Invalid IPv6 address format - Invalid characters in address
        let result = parse_origin_line("jdoe 2890844526 2890842807 IN IP6 2001:zzzz::1");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_connection_line_valid() {
        // Standard IPv4 unicast
        let result = parse_connection_line("IN IP4 192.168.1.1");
        assert!(result.is_ok());
        
        let conn = result.unwrap();
        assert_eq!(conn.net_type, "IN");
        assert_eq!(conn.addr_type, "IP4");
        assert_eq!(conn.connection_address, "192.168.1.1");
        assert_eq!(conn.ttl, None);
        assert_eq!(conn.multicast_count, None);
        
        // IPv4 multicast with TTL
        let result = parse_connection_line("IN IP4 224.2.36.42/127");
        assert!(result.is_ok());
        
        let conn = result.unwrap();
        assert_eq!(conn.connection_address, "224.2.36.42");
        assert_eq!(conn.ttl, Some(127));
        assert_eq!(conn.multicast_count, None);
        
        // IPv4 multicast with TTL and multicast count
        let result = parse_connection_line("IN IP4 224.2.36.42/127/3");
        assert!(result.is_ok());
        
        let conn = result.unwrap();
        assert_eq!(conn.connection_address, "224.2.36.42");
        assert_eq!(conn.ttl, Some(127));
        assert_eq!(conn.multicast_count, Some(3));
        
        // IPv6 unicast
        let result = parse_connection_line("IN IP6 2001:db8::1");
        assert!(result.is_ok());
        
        let conn = result.unwrap();
        assert_eq!(conn.net_type, "IN");
        assert_eq!(conn.addr_type, "IP6");
        assert_eq!(conn.connection_address, "2001:db8::1");
        assert_eq!(conn.ttl, None);
        assert_eq!(conn.multicast_count, None);
        
        // IPv6 multicast with scope
        let result = parse_connection_line("IN IP6 FF15::101/3");
        assert!(result.is_ok());
        
        let conn = result.unwrap();
        assert_eq!(conn.connection_address, "FF15::101");
        assert_eq!(conn.ttl, Some(3));  // For IPv6, this is actually the scope ID
        assert_eq!(conn.multicast_count, None);
    }

    #[test]
    fn test_parse_connection_line_invalid() {
        // Too few parts - Must have exactly nettype, addrtype, and connection-address
        let result = parse_connection_line("IN IP4");
        assert!(result.is_err());
        
        // Too many parts - SDP does not allow additional unrecognized fields
        let result = parse_connection_line("IN IP4 192.168.1.1 extrapart");
        assert!(result.is_err());
        
        // Invalid network type - Only "IN" is defined in RFC 8866
        let result = parse_connection_line("INVALID IP4 192.168.1.1");
        assert!(result.is_err());
        
        // Invalid address type - Only "IP4" and "IP6" are allowed
        let result = parse_connection_line("IN IPX 192.168.1.1");
        assert!(result.is_err());
        
        // Incomplete IPv4 address - Should reject partial addresses
        let result = parse_connection_line("IN IP4 192.168.1");
        assert!(result.is_err());
        
        // Invalid TTL format - TTL must be a number (typically 0-255)
        let result = parse_connection_line("IN IP4 224.2.36.42/abc");
        assert!(result.is_err());
        
        // Invalid multicast count format - Must be a positive integer
        let result = parse_connection_line("IN IP4 224.2.36.42/127/xyz");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_connection_line_edge_cases() {
        // Minimum TTL
        let result = parse_connection_line("IN IP4 224.2.36.42/1");
        assert!(result.is_ok());
        let conn = result.unwrap();
        assert_eq!(conn.ttl, Some(1));
        
        // Maximum TTL
        let result = parse_connection_line("IN IP4 224.2.36.42/255");
        assert!(result.is_ok());
        let conn = result.unwrap();
        assert_eq!(conn.ttl, Some(255));
        
        // Hostname instead of IP
        let result = parse_connection_line("IN IP4 example.com");
        assert!(result.is_ok());
        let conn = result.unwrap();
        assert_eq!(conn.connection_address, "example.com");
    }
} 
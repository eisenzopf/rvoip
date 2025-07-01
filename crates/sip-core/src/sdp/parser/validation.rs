//! SDP validation functionality 
//!
//! This module provides validation functions for SDP messages according to RFC 8866.
//! 
//! It includes validators for:
//! - Network types (e.g., "IN" for Internet)
//! - Address types (e.g., "IP4", "IP6")
//! - IPv4 and IPv6 addresses and hostnames
//! - Complete SDP session validation
//!
//! The validation functions follow the requirements specified in RFC 8866:
//! [Session Description Protocol](https://datatracker.ietf.org/doc/html/rfc8866).

use crate::error::{Error, Result};
use crate::types::sdp::SdpSession;
use crate::sdp::session::validation as session_validation;

// Re-export validation functions from session module
pub use crate::sdp::session::validation::{
    is_valid_hostname,
    is_valid_username,
    is_valid_ipv4_or_hostname,
    is_valid_ipv6_or_hostname,
};

/// Validate that a network type is valid according to RFC 8866
///
/// RFC 8866 only defines "IN" (Internet) as a valid network type. Other network
/// types are not currently supported in the SDP specification.
///
/// # Parameters
///
/// - `net_type`: The network type string to validate
///
/// # Returns
///
/// - `Ok(())` if valid
/// - `Err` with an `SdpParsingError` if invalid
///
/// # Example
///
/// ```
/// use rvoip_sip_core::sdp::parser::validation::validate_network_type;
/// 
/// // Valid network type
/// assert!(validate_network_type("IN").is_ok());
/// 
/// // Invalid network types
/// assert!(validate_network_type("NET").is_err());
/// assert!(validate_network_type("in").is_err()); // Case-sensitive
/// ```
pub fn validate_network_type(net_type: &str) -> Result<()> {
    // RFC 8866 only defines "IN" for Internet
    if net_type != "IN" {
        return Err(Error::SdpParsingError(format!("Invalid network type: {}", net_type)));
    }
    Ok(())
}

/// Validate that an address type is valid according to RFC 8866
///
/// RFC 8866 defines "IP4" and "IP6" as valid address types for IPv4 and IPv6 addresses
/// respectively.
///
/// # Parameters
///
/// - `addr_type`: The address type string to validate
///
/// # Returns
///
/// - `Ok(())` if valid
/// - `Err` with an `SdpParsingError` if invalid
///
/// # Example
///
/// ```
/// use rvoip_sip_core::sdp::parser::validation::validate_address_type;
/// 
/// // Valid address types
/// assert!(validate_address_type("IP4").is_ok());
/// assert!(validate_address_type("IP6").is_ok());
/// 
/// // Invalid address types
/// assert!(validate_address_type("ip4").is_err()); // Case-sensitive
/// assert!(validate_address_type("IPV4").is_err());
/// ```
pub fn validate_address_type(addr_type: &str) -> Result<()> {
    // RFC 8866 defines "IP4" and "IP6"
    match addr_type {
        "IP4" | "IP6" => Ok(()),
        _ => Err(Error::SdpParsingError(format!("Invalid address type: {}", addr_type))),
    }
}

/// Helper function to check if a string is a valid IPv4 address
///
/// Uses Rust's standard library to validate that the address is a correctly formatted
/// IPv4 address with values in the proper ranges.
///
/// # Parameters
///
/// - `addr`: The string to check
///
/// # Returns
///
/// - `true` if the string is a valid IPv4 address
/// - `false` otherwise
///
/// # Example
///
/// ```
/// use rvoip_sip_core::sdp::parser::validation::is_valid_ipv4;
/// 
/// // Valid IPv4 addresses
/// assert!(is_valid_ipv4("192.168.1.1"));
/// assert!(is_valid_ipv4("0.0.0.0"));
/// assert!(is_valid_ipv4("255.255.255.255"));
/// 
/// // Invalid IPv4 addresses
/// assert!(!is_valid_ipv4("256.0.0.1")); // Invalid range
/// assert!(!is_valid_ipv4("192.168.1")); // Incomplete
/// assert!(!is_valid_ipv4("192.168.1.1.5")); // Too many segments
/// ```
pub fn is_valid_ipv4(addr: &str) -> bool {
    // Use standard library's Ipv4Addr parsing which properly validates range limits
    addr.parse::<std::net::Ipv4Addr>().is_ok()
}

/// Helper function to check if a string is a valid IPv6 address
///
/// Validates both standard and RFC-compliant IPv6 addresses (with brackets).
/// Returns false for empty strings or malformed IPv6 addresses.
///
/// # Parameters
///
/// - `addr`: The string to check
///
/// # Returns
///
/// - `true` if the string is a valid IPv6 address
/// - `false` otherwise
///
/// # Example
///
/// ```
/// use rvoip_sip_core::sdp::parser::validation::is_valid_ipv6;
/// 
/// // Valid IPv6 addresses
/// assert!(is_valid_ipv6("2001:db8::1"));
/// assert!(is_valid_ipv6("::1")); // Localhost
/// assert!(is_valid_ipv6("::")); // Unspecified
/// 
/// // Valid with brackets (RFC format)
/// assert!(is_valid_ipv6("[2001:db8::1]"));
/// 
/// // Invalid IPv6 addresses
/// assert!(!is_valid_ipv6("2001:db8")); // Incomplete
/// assert!(!is_valid_ipv6("2001:zz::1")); // Invalid characters
/// assert!(!is_valid_ipv6("[2001:db8::1")); // Unclosed bracket
/// ```
pub fn is_valid_ipv6(addr: &str) -> bool {
    // Handle RFC format with brackets
    let addr = if addr.starts_with('[') && addr.ends_with(']') {
        &addr[1..addr.len()-1]
    } else {
        addr
    };
    
    addr.parse::<std::net::Ipv6Addr>().is_ok()
}

/// Helper function to validate an address based on its type
///
/// Validates that an address string is appropriate for the specified address type
/// according to RFC 8866. For IP4, validates IPv4 addresses and hostnames. For IP6,
/// validates IPv6 addresses and hostnames.
///
/// Handles multicast addresses with TTL or count specifications (e.g., "224.0.0.1/127").
/// Also properly validates addresses with special formats for each type.
///
/// # Parameters
///
/// - `addr`: The address string to validate
/// - `addr_type`: The address type ("IP4" or "IP6")
///
/// # Returns
///
/// - `true` if the address is valid for the given address type
/// - `false` otherwise
///
/// # Example
///
/// ```
/// use rvoip_sip_core::sdp::parser::validation::{is_valid_address, is_valid_hostname};
/// 
/// // Valid IP4 addresses
/// assert!(is_valid_address("192.168.1.1", "IP4"));
/// assert!(is_valid_address("224.0.0.1/127", "IP4")); // Multicast with TTL
/// assert!(is_valid_address("example.com", "IP4")); // Hostname
/// 
/// // Invalid IP4 addresses
/// assert!(!is_valid_address("256.0.0.1", "IP4")); // Invalid range
/// assert!(!is_valid_address("192.168.1", "IP4")); // Incomplete
/// 
/// // Valid IP6 addresses
/// assert!(is_valid_address("2001:db8::1", "IP6"));
/// assert!(is_valid_address("[2001:db8::1]", "IP6")); // With brackets
/// assert!(is_valid_address("example.com", "IP6")); // Hostname
/// 
/// // Invalid IP6 addresses
/// assert!(!is_valid_address("192.168.1.1", "IP6")); // IPv4 address with IP6 type
/// ```
pub fn is_valid_address(addr: &str, addr_type: &str) -> bool {
    match addr_type {
        "IP4" => {
            // Check if it's a multicast address with TTL/count specification
            if addr.contains('/') {
                let parts: Vec<&str> = addr.split('/').collect();
                if parts.len() <= 2 {
                    // Just validate the IP portion
                    let ip_part = parts[0];
                    return is_valid_ipv4(ip_part) || is_valid_hostname(ip_part);
                }
                return false;
            }
            
            // Check for incomplete IPv4-like addresses (contains dots and only digits)
            // These should not be treated as hostnames
            if addr.contains('.') {
                let segments = addr.split('.').count();
                let has_only_dots_and_digits = addr.chars().all(|c| c.is_ascii_digit() || c == '.');
                
                // If it looks like an IPv4 address but doesn't have 4 segments, reject it
                if has_only_dots_and_digits && segments != 4 {
                    return false;
                }
            }
            
            // Special check for invalid IP address values like "256.0.0.1"
            if addr.split('.').count() == 4 {
                let parts: Vec<&str> = addr.split('.').collect();
                for part in parts {
                    if let Ok(num) = part.parse::<u32>() {
                        if num > 255 {
                            return false;
                        }
                    }
                }
            }
            
            // For regular addresses, use either IPv4 validation or hostname validation
            is_valid_ipv4(addr) || is_valid_hostname(addr)
        },
        "IP6" => {
            // Check if it's a multicast address with count specification
            if addr.contains('/') {
                let parts: Vec<&str> = addr.split('/').collect();
                if parts.len() <= 2 {
                    // Just validate the IP portion
                    let ip_part = if parts[0].starts_with('[') && parts[0].ends_with(']') {
                        &parts[0][1..parts[0].len()-1]
                    } else {
                        parts[0]
                    };
                    return is_valid_ipv6(ip_part) || is_valid_hostname(ip_part);
                }
                return false;
            }
            
            // Handle bracketed IPv6
            let addr_to_check = if addr.starts_with('[') && addr.ends_with(']') {
                &addr[1..addr.len()-1]
            } else {
                addr
            };
            
            // Only reject dot-notation addresses that look like IPv4 but not hostnames
            if addr.contains('.') && !addr.contains(':') {
                // Check if it looks like an IPv4 address
                let has_only_dots_and_digits = addr.chars().all(|c| c.is_ascii_digit() || c == '.');
                // If it has dots and digits only but not IPv6 colons, it's probably an IPv4 address
                if has_only_dots_and_digits {
                    return false; // Reject IPv4-like addresses for IP6 type
                }
            }
            
            is_valid_ipv6(addr_to_check) || is_valid_hostname(addr_to_check)
        },
        _ => false,
    }
}

/// Validates a complete SDP session for compliance with RFC 8866
///
/// Performs thorough validation of an SDP session including:
/// - Version number (must be "0")
/// - Origin field validity
/// - Connection information
/// - Session name presence
/// - Time descriptions (at least one required)
/// - Media descriptions format validation
///
/// This function implements the requirements from RFC 8866 to ensure the SDP
/// session is well-formed and valid.
///
/// # Parameters
///
/// - `session`: The SDP session to validate
///
/// # Returns
///
/// - `Ok(())` if validation succeeds
/// - `Err` with an `SdpValidationError` detailing the specific validation failure
///
/// # Example
///
/// ```
/// use rvoip_sip_core::sdp::validate_sdp;
/// use rvoip_sip_core::types::sdp::{Origin, ConnectionData, MediaDescription, TimeDescription, SdpSession};
/// 
/// // Create a minimal valid SDP session
/// let origin = Origin {
///     username: "user".to_string(),
///     sess_id: "123".to_string(),
///     sess_version: "456".to_string(),
///     net_type: "IN".to_string(),
///     addr_type: "IP4".to_string(),
///     unicast_address: "192.168.1.1".to_string(),
/// };
/// 
/// let connection = ConnectionData {
///     net_type: "IN".to_string(),
///     addr_type: "IP4".to_string(),
///     connection_address: "224.0.0.1".to_string(),
///     ttl: Some(127),
///     multicast_count: None,
/// };
/// 
/// let time = TimeDescription {
///     start_time: "0".to_string(),
///     stop_time: "0".to_string(),
///     repeat_times: Vec::new(),
/// };
/// 
/// let media = MediaDescription {
///     media: "audio".to_string(),
///     port: 49170,
///     protocol: "RTP/AVP".to_string(),
///     formats: vec!["0".to_string()],
///     connection_info: None,
///     ptime: None,
///     direction: None,
///     generic_attributes: Vec::new(),
/// };
/// 
/// let mut session = SdpSession::new(origin, "Test Session".to_string());
/// session.version = "0".to_string();
/// session.connection_info = Some(connection);
/// session.time_descriptions = vec![time];
/// session.media_descriptions = vec![media];
/// 
/// // Should validate successfully
/// assert!(validate_sdp(&session).is_ok());
/// ```
pub fn validate_sdp(session: &SdpSession) -> Result<()> {
    // Check version
    if session.version != "0" {
        return Err(Error::SdpValidationError(format!("Invalid SDP version: {}", session.version)));
    }
    
    // Validate origin
    validate_network_type(&session.origin.net_type)?;
    validate_address_type(&session.origin.addr_type)?;
    
    if !is_valid_address(&session.origin.unicast_address, &session.origin.addr_type) {
        return Err(Error::SdpValidationError(format!("Invalid origin address: {}", session.origin.unicast_address)));
    }
    
    // Validate session name
    if session.session_name.is_empty() {
        return Err(Error::SdpValidationError("Empty session name".to_string()));
    }
    
    // Check for time descriptions
    if session.time_descriptions.is_empty() {
        return Err(Error::SdpValidationError("SDP must have at least one time description".to_string()));
    }
    
    // Validate connection data if present
    let session_has_connection = session.connection_info.is_some();
    if let Some(conn) = &session.connection_info {
        validate_network_type(&conn.net_type)?;
        validate_address_type(&conn.addr_type)?;
        
        if !is_valid_address(&conn.connection_address, &conn.addr_type) {
            return Err(Error::SdpValidationError(format!("Invalid connection address: {}", conn.connection_address)));
        }
    }
    
    // Validate media descriptions
    for media in &session.media_descriptions {
        // Validate media connection info if present
        if let Some(conn) = &media.connection_info {
            validate_network_type(&conn.net_type)?;
            validate_address_type(&conn.addr_type)?;
            
            if !is_valid_address(&conn.connection_address, &conn.addr_type) {
                return Err(Error::SdpValidationError(format!("Invalid media connection address: {}", conn.connection_address)));
            }
        } else if !session_has_connection {
            // If no session-level connection and no media-level connection, that's an error
            return Err(Error::SdpValidationError("Connection information must be present at session or media level".to_string()));
        }
        
        // Check that media section has at least one format
        if media.formats.is_empty() {
            return Err(Error::SdpValidationError(format!("Media section ({}) must have at least one format", media.media)));
        }
    }
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::sdp::{Origin, ConnectionData, MediaDescription, TimeDescription};

    #[test]
    fn test_validate_network_type() {
        // Valid network type according to RFC 8866
        assert!(validate_network_type("IN").is_ok());
        
        // Invalid network types
        assert!(validate_network_type("in").is_err()); // Case sensitive
        assert!(validate_network_type("NET").is_err());
        assert!(validate_network_type("").is_err());
        assert!(validate_network_type("Internet").is_err());
    }

    #[test]
    fn test_validate_address_type() {
        // Valid address types according to RFC 8866
        assert!(validate_address_type("IP4").is_ok());
        assert!(validate_address_type("IP6").is_ok());
        
        // Invalid address types
        assert!(validate_address_type("ip4").is_err()); // Case sensitive
        assert!(validate_address_type("ip6").is_err());
        assert!(validate_address_type("IPV4").is_err());
        assert!(validate_address_type("IPv6").is_err());
        assert!(validate_address_type("").is_err());
        assert!(validate_address_type("IP").is_err());
    }

    #[test]
    fn test_is_valid_ipv4() {
        // Valid IPv4 addresses
        assert!(is_valid_ipv4("192.168.1.1"));
        assert!(is_valid_ipv4("127.0.0.1"));
        assert!(is_valid_ipv4("0.0.0.0"));
        assert!(is_valid_ipv4("255.255.255.255"));
        assert!(is_valid_ipv4("224.0.0.1")); // Multicast
        
        // Invalid IPv4 addresses
        assert!(!is_valid_ipv4("192.168.1")); // Incomplete
        assert!(!is_valid_ipv4("192.168.1."));
        assert!(!is_valid_ipv4("192.168"));
        assert!(!is_valid_ipv4("256.0.0.1")); // Out of range
        assert!(!is_valid_ipv4("192.168.1.300"));
        assert!(!is_valid_ipv4("192.168.1.1.5")); // Too many segments
        assert!(!is_valid_ipv4("192.168.1.a")); // Non-numeric
        assert!(!is_valid_ipv4("")); // Empty
    }

    #[test]
    fn test_is_valid_ipv6() {
        // Valid IPv6 addresses
        assert!(is_valid_ipv6("2001:db8::1"));
        assert!(is_valid_ipv6("::1")); // Localhost
        assert!(is_valid_ipv6("::")); // Unspecified address
        assert!(is_valid_ipv6("fe80::1234:5678:abcd:ef12")); // Link-local
        assert!(is_valid_ipv6("ff02::1")); // Multicast
        assert!(is_valid_ipv6("2001:0db8:85a3:0000:0000:8a2e:0370:7334")); // Full form
        assert!(is_valid_ipv6("2001:db8:85a3::8a2e:370:7334")); // Compressed form
        
        // Valid IPv6 addresses with brackets (RFC format)
        assert!(is_valid_ipv6("[2001:db8::1]"));
        assert!(is_valid_ipv6("[::1]"));
        
        // Invalid IPv6 addresses
        assert!(!is_valid_ipv6("2001:db8:")); // Incomplete
        assert!(!is_valid_ipv6("2001:db8:85a3::8a2e:370:7334:1:2")); // Too many segments
        assert!(!is_valid_ipv6("2001:db8:85a3::8a2e:370g:7334")); // Invalid characters
        assert!(!is_valid_ipv6("")); // Empty
        assert!(!is_valid_ipv6("[")); // Malformed brackets
        assert!(!is_valid_ipv6("]"));
        assert!(!is_valid_ipv6("[2001:db8::1")); // Unclosed bracket
        assert!(!is_valid_ipv6("2001:db8::1]")); // Unopened bracket
    }

    #[test]
    fn test_is_valid_address_ipv4() {
        // Valid IPv4 addresses
        assert!(is_valid_address("192.168.1.1", "IP4"));
        assert!(is_valid_address("0.0.0.0", "IP4"));
        assert!(is_valid_address("224.0.0.1", "IP4")); // Multicast

        // Valid IPv4 multicast with TTL
        assert!(is_valid_address("224.0.0.1/127", "IP4"));
        
        // Valid hostname
        assert!(is_valid_address("example.com", "IP4"));
        
        // Invalid IPv4 addresses
        assert!(!is_valid_address("192.168.1", "IP4")); // Incomplete
        assert!(!is_valid_address("192.168", "IP4"));
        assert!(!is_valid_address("256.0.0.1", "IP4")); // Out of range
        
        // Invalid IPv4 multicast with too many parts
        assert!(!is_valid_address("224.0.0.1/127/3/4", "IP4"));
        
        // Invalid for IP4 type
        assert!(!is_valid_address("2001:db8::1", "IP4")); // IPv6 address with IP4 type
    }
    
    #[test]
    fn test_is_valid_address_ipv6() {
        // Valid IPv6 addresses
        assert!(is_valid_address("2001:db8::1", "IP6"));
        assert!(is_valid_address("::1", "IP6"));
        assert!(is_valid_address("ff02::1", "IP6")); // Multicast
        
        // Valid IPv6 with brackets
        assert!(is_valid_address("[2001:db8::1]", "IP6"));
        
        // Valid IPv6 multicast with scope
        assert!(is_valid_address("ff02::1/5", "IP6"));
        assert!(is_valid_address("[ff02::1]/5", "IP6"));
        
        // Valid hostname
        assert!(is_valid_address("example.com", "IP6"));
        
        // Invalid IPv6 addresses
        assert!(!is_valid_address("2001:db8:", "IP6")); // Incomplete
        assert!(!is_valid_address("2001:db8:85a3::8a2e:370g:7334", "IP6")); // Invalid chars
        
        // Invalid IPv6 multicast with too many parts
        assert!(!is_valid_address("ff02::1/5/4/3", "IP6"));
        
        // Invalid for IP6 type
        assert!(!is_valid_address("192.168.1.1", "IP6")); // IPv4 address with IP6 type 
    }
    
    #[test]
    fn test_is_valid_address_invalid_type() {
        // Invalid address type
        assert!(!is_valid_address("192.168.1.1", "IP5"));
        assert!(!is_valid_address("2001:db8::1", "IPV6"));
        assert!(!is_valid_address("example.com", ""));
    }

    #[test]
    fn test_validate_sdp_valid() {
        // Create a minimal valid SDP session
        let origin = Origin {
            username: "user".to_string(),
            sess_id: "123".to_string(),
            sess_version: "456".to_string(),
            net_type: "IN".to_string(),
            addr_type: "IP4".to_string(),
            unicast_address: "192.168.1.1".to_string(),
        };
        
        let connection = ConnectionData {
            net_type: "IN".to_string(),
            addr_type: "IP4".to_string(),
            connection_address: "224.0.0.1".to_string(),
            ttl: Some(127),
            multicast_count: None,
        };
        
        let time = TimeDescription {
            start_time: "0".to_string(),
            stop_time: "0".to_string(),
            repeat_times: Vec::new(),
        };
        
        let media = MediaDescription {
            media: "audio".to_string(),
            port: 49170,
            protocol: "RTP/AVP".to_string(),
            formats: vec!["0".to_string()],
            connection_info: None,
            ptime: None,
            direction: None,
            generic_attributes: Vec::new(),
        };
        
        let mut session = SdpSession::new(origin, "Test Session".to_string());
        session.version = "0".to_string();
        session.connection_info = Some(connection);
        session.time_descriptions = vec![time];
        session.media_descriptions = vec![media];
        
        // Should validate successfully
        assert!(validate_sdp(&session).is_ok());
    }

    #[test]
    fn test_validate_sdp_invalid_version() {
        // Create a minimal SDP session with invalid version
        let origin = Origin {
            username: "user".to_string(),
            sess_id: "123".to_string(),
            sess_version: "456".to_string(),
            net_type: "IN".to_string(),
            addr_type: "IP4".to_string(),
            unicast_address: "192.168.1.1".to_string(),
        };
        
        let connection = ConnectionData {
            net_type: "IN".to_string(),
            addr_type: "IP4".to_string(),
            connection_address: "224.0.0.1".to_string(),
            ttl: Some(127),
            multicast_count: None,
        };
        
        let time = TimeDescription {
            start_time: "0".to_string(),
            stop_time: "0".to_string(),
            repeat_times: Vec::new(),
        };
        
        let media = MediaDescription {
            media: "audio".to_string(),
            port: 49170,
            protocol: "RTP/AVP".to_string(),
            formats: vec!["0".to_string()],
            connection_info: None,
            ptime: None,
            direction: None,
            generic_attributes: Vec::new(),
        };
        
        let mut session = SdpSession::new(origin, "Test Session".to_string());
        session.version = "1".to_string(); // Invalid version
        session.connection_info = Some(connection);
        session.time_descriptions = vec![time];
        session.media_descriptions = vec![media];
        
        // Should fail validation
        let result = validate_sdp(&session);
        assert!(result.is_err());
        match result {
            Err(Error::SdpValidationError(msg)) => {
                assert!(msg.contains("Invalid SDP version"));
            },
            _ => panic!("Expected SdpValidationError for invalid version"),
        }
    }

    #[test]
    fn test_validate_sdp_invalid_origin() {
        // Create a minimal SDP session with invalid origin
        let origin = Origin {
            username: "user".to_string(),
            sess_id: "123".to_string(),
            sess_version: "456".to_string(),
            net_type: "IN".to_string(),
            addr_type: "IP4".to_string(),
            unicast_address: "192.168.1".to_string(), // Invalid address
        };
        
        let connection = ConnectionData {
            net_type: "IN".to_string(),
            addr_type: "IP4".to_string(),
            connection_address: "224.0.0.1".to_string(),
            ttl: Some(127),
            multicast_count: None,
        };
        
        let time = TimeDescription {
            start_time: "0".to_string(),
            stop_time: "0".to_string(),
            repeat_times: Vec::new(),
        };
        
        let media = MediaDescription {
            media: "audio".to_string(),
            port: 49170,
            protocol: "RTP/AVP".to_string(),
            formats: vec!["0".to_string()],
            connection_info: None,
            ptime: None,
            direction: None,
            generic_attributes: Vec::new(),
        };
        
        let mut session = SdpSession::new(origin, "Test Session".to_string());
        session.version = "0".to_string();
        session.connection_info = Some(connection);
        session.time_descriptions = vec![time];
        session.media_descriptions = vec![media];
        
        // Should fail validation
        let result = validate_sdp(&session);
        assert!(result.is_err());
        match result {
            Err(Error::SdpValidationError(msg)) => {
                assert!(msg.contains("Invalid origin address"));
            },
            _ => panic!("Expected SdpValidationError for invalid origin address"),
        }
    }

    #[test]
    fn test_validate_sdp_empty_session_name() {
        // Create a minimal SDP session with empty session name
        let origin = Origin {
            username: "user".to_string(),
            sess_id: "123".to_string(),
            sess_version: "456".to_string(),
            net_type: "IN".to_string(),
            addr_type: "IP4".to_string(),
            unicast_address: "192.168.1.1".to_string(),
        };
        
        let connection = ConnectionData {
            net_type: "IN".to_string(),
            addr_type: "IP4".to_string(),
            connection_address: "224.0.0.1".to_string(),
            ttl: Some(127),
            multicast_count: None,
        };
        
        let time = TimeDescription {
            start_time: "0".to_string(),
            stop_time: "0".to_string(),
            repeat_times: Vec::new(),
        };
        
        let media = MediaDescription {
            media: "audio".to_string(),
            port: 49170,
            protocol: "RTP/AVP".to_string(),
            formats: vec!["0".to_string()],
            connection_info: None,
            ptime: None,
            direction: None,
            generic_attributes: Vec::new(),
        };
        
        let mut session = SdpSession::new(origin, "".to_string()); // Empty session name
        session.version = "0".to_string();
        session.connection_info = Some(connection);
        session.time_descriptions = vec![time];
        session.media_descriptions = vec![media];
        
        // Should fail validation
        let result = validate_sdp(&session);
        assert!(result.is_err());
        match result {
            Err(Error::SdpValidationError(msg)) => {
                assert_eq!("Empty session name", msg);
            },
            _ => panic!("Expected SdpValidationError for empty session name"),
        }
    }

    #[test]
    fn test_validate_sdp_no_time_description() {
        // Create a minimal SDP session with no time description
        let origin = Origin {
            username: "user".to_string(),
            sess_id: "123".to_string(),
            sess_version: "456".to_string(),
            net_type: "IN".to_string(),
            addr_type: "IP4".to_string(),
            unicast_address: "192.168.1.1".to_string(),
        };
        
        let connection = ConnectionData {
            net_type: "IN".to_string(),
            addr_type: "IP4".to_string(),
            connection_address: "224.0.0.1".to_string(),
            ttl: Some(127),
            multicast_count: None,
        };
        
        let media = MediaDescription {
            media: "audio".to_string(),
            port: 49170,
            protocol: "RTP/AVP".to_string(),
            formats: vec!["0".to_string()],
            connection_info: None,
            ptime: None,
            direction: None,
            generic_attributes: Vec::new(),
        };
        
        let mut session = SdpSession::new(origin, "Test Session".to_string());
        session.version = "0".to_string();
        session.connection_info = Some(connection);
        session.time_descriptions = vec![]; // No time description
        session.media_descriptions = vec![media];
        
        // Should fail validation
        let result = validate_sdp(&session);
        assert!(result.is_err());
        match result {
            Err(Error::SdpValidationError(msg)) => {
                assert!(msg.contains("SDP must have at least one time description"));
            },
            _ => panic!("Expected SdpValidationError for missing time description"),
        }
    }

    #[test]
    fn test_validate_sdp_invalid_connection() {
        // Create a minimal SDP session with invalid connection
        let origin = Origin {
            username: "user".to_string(),
            sess_id: "123".to_string(),
            sess_version: "456".to_string(),
            net_type: "IN".to_string(),
            addr_type: "IP4".to_string(),
            unicast_address: "192.168.1.1".to_string(),
        };
        
        let connection = ConnectionData {
            net_type: "IN".to_string(),
            addr_type: "IP4".to_string(),
            connection_address: "256.0.0.1".to_string(), // Invalid IP address
            ttl: Some(127),
            multicast_count: None,
        };
        
        let time = TimeDescription {
            start_time: "0".to_string(),
            stop_time: "0".to_string(),
            repeat_times: Vec::new(),
        };
        
        let media = MediaDescription {
            media: "audio".to_string(),
            port: 49170,
            protocol: "RTP/AVP".to_string(),
            formats: vec!["0".to_string()],
            connection_info: None,
            ptime: None,
            direction: None,
            generic_attributes: Vec::new(),
        };
        
        let mut session = SdpSession::new(origin, "Test Session".to_string());
        session.version = "0".to_string();
        session.connection_info = Some(connection);
        session.time_descriptions = vec![time];
        session.media_descriptions = vec![media];
        
        // Should fail validation
        let result = validate_sdp(&session);
        assert!(result.is_err());
        match result {
            Err(Error::SdpValidationError(msg)) => {
                assert!(msg.contains("Invalid connection address"));
            },
            _ => panic!("Expected SdpValidationError for invalid connection address"),
        }
    }

    #[test]
    fn test_validate_sdp_no_connection() {
        // Create a minimal SDP session with no connection info
        let origin = Origin {
            username: "user".to_string(),
            sess_id: "123".to_string(),
            sess_version: "456".to_string(),
            net_type: "IN".to_string(),
            addr_type: "IP4".to_string(),
            unicast_address: "192.168.1.1".to_string(),
        };
        
        let time = TimeDescription {
            start_time: "0".to_string(),
            stop_time: "0".to_string(),
            repeat_times: Vec::new(),
        };
        
        let media = MediaDescription {
            media: "audio".to_string(),
            port: 49170,
            protocol: "RTP/AVP".to_string(),
            formats: vec!["0".to_string()],
            connection_info: None, // No media-level connection
            ptime: None,
            direction: None,
            generic_attributes: Vec::new(),
        };
        
        let mut session = SdpSession::new(origin, "Test Session".to_string());
        session.version = "0".to_string();
        session.connection_info = None; // No session-level connection
        session.time_descriptions = vec![time];
        session.media_descriptions = vec![media];
        
        // Should fail validation
        let result = validate_sdp(&session);
        assert!(result.is_err());
        match result {
            Err(Error::SdpValidationError(msg)) => {
                assert!(msg.contains("Connection information must be present"));
            },
            _ => panic!("Expected SdpValidationError for missing connection information"),
        }
    }

    #[test]
    fn test_validate_sdp_media_with_no_formats() {
        // Create a minimal SDP session with media that has no formats
        let origin = Origin {
            username: "user".to_string(),
            sess_id: "123".to_string(),
            sess_version: "456".to_string(),
            net_type: "IN".to_string(),
            addr_type: "IP4".to_string(),
            unicast_address: "192.168.1.1".to_string(),
        };
        
        let connection = ConnectionData {
            net_type: "IN".to_string(),
            addr_type: "IP4".to_string(),
            connection_address: "224.0.0.1".to_string(),
            ttl: Some(127),
            multicast_count: None,
        };
        
        let time = TimeDescription {
            start_time: "0".to_string(),
            stop_time: "0".to_string(),
            repeat_times: Vec::new(),
        };
        
        let media = MediaDescription {
            media: "audio".to_string(),
            port: 49170,
            protocol: "RTP/AVP".to_string(),
            formats: vec![], // No formats
            connection_info: None,
            ptime: None,
            direction: None,
            generic_attributes: Vec::new(),
        };
        
        let mut session = SdpSession::new(origin, "Test Session".to_string());
        session.version = "0".to_string();
        session.connection_info = Some(connection);
        session.time_descriptions = vec![time];
        session.media_descriptions = vec![media];
        
        // Should fail validation
        let result = validate_sdp(&session);
        assert!(result.is_err());
        match result {
            Err(Error::SdpValidationError(msg)) => {
                assert!(msg.contains("Media section (audio) must have at least one format"));
            },
            _ => panic!("Expected SdpValidationError for media with no formats"),
        }
    }
} 
//! SDP validation functionality 
//!
//! This module provides validation functions for SDP messages.

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
/// # Parameters
///
/// - `net_type`: The network type string to validate
///
/// # Returns
///
/// - `Ok(())` if valid
/// - `Err` with an error message if invalid
pub fn validate_network_type(net_type: &str) -> Result<()> {
    // RFC 8866 only defines "IN" for Internet
    if net_type != "IN" {
        return Err(Error::SdpParsingError(format!("Invalid network type: {}", net_type)));
    }
    Ok(())
}

/// Validate that an address type is valid according to RFC 8866
///
/// # Parameters
///
/// - `addr_type`: The address type string to validate
///
/// # Returns
///
/// - `Ok(())` if valid
/// - `Err` with an error message if invalid
pub fn validate_address_type(addr_type: &str) -> Result<()> {
    // RFC 8866 defines "IP4" and "IP6"
    match addr_type {
        "IP4" | "IP6" => Ok(()),
        _ => Err(Error::SdpParsingError(format!("Invalid address type: {}", addr_type))),
    }
}

/// Helper function to check if a string is a valid IPv4 address
pub fn is_valid_ipv4(addr: &str) -> bool {
    addr.parse::<std::net::Ipv4Addr>().is_ok()
}

/// Helper function to check if a string is a valid IPv6 address
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
pub fn is_valid_address(addr: &str, addr_type: &str) -> bool {
    match addr_type {
        "IP4" => {
            // Check if it's a multicast address with TTL/count specification
            if addr.contains('/') {
                let parts: Vec<&str> = addr.split('/').collect();
                if parts.len() <= 2 {
                    // Just validate the IP portion
                    return is_valid_ipv4(parts[0]) || is_valid_hostname(parts[0]);
                }
                return false;
            }
            
            // Normal address
            is_valid_ipv4(addr) || session_validation::is_valid_hostname(addr)
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
            let addr = if addr.starts_with('[') && addr.ends_with(']') {
                &addr[1..addr.len()-1]
            } else {
                addr
            };
            
            is_valid_ipv6(addr) || session_validation::is_valid_hostname(addr)
        },
        _ => false,
    }
}

/// Validates a complete SDP session for compliance with RFC 8866
///
/// # Parameters
///
/// - `session`: The SDP session to validate
///
/// # Returns
///
/// - `Ok(())` if validation succeeds
/// - `Err` with validation errors if it fails
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
    
    // Validate connection data if present
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
        }
    }
    
    Ok(())
} 
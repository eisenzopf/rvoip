//! SDP validation functionality 
//!
//! This module provides validation functions for SDP messages.

use crate::error::{Error, Result};
use crate::types::sdp::SdpSession;

/// Helper function to check if a string is a valid hostname
pub fn is_valid_hostname(hostname: &str) -> bool {
    if hostname.is_empty() || hostname.len() > 255 {
        return false;
    }

    let labels: Vec<&str> = hostname.split('.').collect();
    
    if labels.is_empty() {
        return false;
    }
    
    for label in labels {
        // Each DNS label must be between 1 and 63 characters long
        if label.is_empty() || label.len() > 63 {
            return false;
        }
        
        // Labels must start and end with alphanumeric characters
        if !label.chars().next().unwrap().is_alphanumeric() 
           || !label.chars().last().unwrap().is_alphanumeric() {
            return false;
        }
        
        // Labels can contain alphanumeric characters and hyphens
        if !label.chars().all(|c| c.is_alphanumeric() || c == '-') {
            return false;
        }
    }
    
    true
}

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
    let segments: Vec<&str> = addr.split('.').collect();
    
    if segments.len() != 4 {
        return false;
    }
    
    for segment in segments {
        match segment.parse::<u8>() {
            Ok(_) => {}, // Valid octet
            Err(_) => return false,
        }
    }
    
    true
}

/// Helper function to check if a string is a valid IPv6 address
pub fn is_valid_ipv6(addr: &str) -> bool {
    // Basic validation for IPv6
    let addr = if addr.starts_with('[') && addr.ends_with(']') {
        &addr[1..addr.len()-1]
    } else {
        addr
    };
    
    // Check if there are too many double colons (can only have one)
    if addr.matches("::").count() > 1 {
        return false;
    }
    
    // Count segments - should be 8 or fewer with ::
    let segments: Vec<&str> = addr.split(':').collect();
    if segments.len() > 8 {
        return false;
    }
    
    // Check each segment is valid hex
    for segment in segments {
        if segment.is_empty() {
            continue; // This is part of a :: sequence
        }
        
        if segment.len() > 4 {
            return false;
        }
        
        if !segment.chars().all(|c| c.is_digit(16)) {
            return false;
        }
    }
    
    true
}

/// Helper function to validate an address based on its type
pub fn is_valid_address(addr: &str, addr_type: &str) -> bool {
    if addr_type == "IP4" {
        if addr.split('.').count() == 4 {
            return is_valid_ipv4(addr);
        } else {
            return is_valid_hostname(addr);
        }
    } else if addr_type == "IP6" {
        if addr.contains(':') {
            return is_valid_ipv6(addr);
        } else {
            return is_valid_hostname(addr);
        }
    }
    
    false
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
    
    // Additional validation would go here
    
    Ok(())
} 
//! Session-level SDP parsing functionality
//!
//! This module handles parsing of session-level SDP elements, including:
//! - Origin (o=)
//! - Session Name (s=)
//! - Connection Data (c=)

use crate::error::{Error, Result};
use crate::types::sdp::{SdpSession, Origin, ConnectionData};
use super::validation::{validate_network_type, validate_address_type, is_valid_address};

/// Initialize a default SDP session
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
/// o=<username> <sess-id> <sess-version> <nettype> <addrtype> <unicast-address>
///
/// # Parameters
///
/// - `value`: The value part of the origin line
///
/// # Returns
///
/// - `Ok(Origin)` if parsing succeeds
/// - `Err` with error details if parsing fails
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
/// c=<nettype> <addrtype> <connection-address>
///
/// # Parameters
///
/// - `value`: The value part of the connection line
///
/// # Returns
///
/// - `Ok(ConnectionData)` if parsing succeeds
/// - `Err` with error details if parsing fails
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
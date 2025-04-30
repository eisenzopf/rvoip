//! SDP media description parsing functionality
//!
//! This module handles parsing of SDP media description lines (m=).

use crate::error::{Error, Result};
use crate::types::sdp::MediaDescription;

/// Parse a media description line (m=)
///
/// # Format
///
/// m=<media> <port>[/<number of ports>] <proto> <fmt> [<fmt>...]
///
/// # Parameters
///
/// - `value`: The value part of the media line
///
/// # Returns
///
/// - `Ok(MediaDescription)` if parsing succeeds
/// - `Err` with error details if parsing fails
pub fn parse_media_description_line(value: &str) -> Result<MediaDescription> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() < 4 {
        return Err(Error::SdpParsingError(format!("Invalid media description format: {}", value)));
    }
    
    // Parse media type
    let media_type_str = parts[0];
    // Only validate if it's a standard media type, otherwise accept anything
    if !["audio", "video", "text", "application", "message"].contains(&media_type_str) {
        // You could add additional validation here
    }
    
    // Parse port and optional port count
    let port_parts: Vec<&str> = parts[1].split('/').collect();
    let port = match port_parts[0].parse::<u16>() {
        Ok(p) => p,
        Err(_) => return Err(Error::SdpParsingError(format!("Invalid port: {}", port_parts[0]))),
    };
    
    // Create the media description
    let mut md = MediaDescription::new(
        parts[0].to_string(),
        port,
        parts[2].to_string(),
        Vec::new(),
    );
    
    // Parse formats
    for i in 3..parts.len() {
        md.formats.push(parts[i].to_string());
    }
    
    // Return the parsed media description
    Ok(md)
} 
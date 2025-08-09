//! SDP utilities for hold/resume operations
//!
//! This module provides functions to manipulate SDP for implementing
//! hold and resume functionality according to RFC 3264.

use crate::errors::{SessionError, Result};
use tracing::{debug, info};

/// Media direction attribute for SDP
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaDirection {
    /// Can send and receive media (default)
    SendRecv,
    /// Can only send media (used for hold)
    SendOnly,
    /// Can only receive media (response to hold)
    RecvOnly,
    /// Cannot send or receive media
    Inactive,
}

impl MediaDirection {
    /// Convert to SDP attribute string
    pub fn to_sdp_attribute(&self) -> &'static str {
        match self {
            MediaDirection::SendRecv => "a=sendrecv",
            MediaDirection::SendOnly => "a=sendonly",
            MediaDirection::RecvOnly => "a=recvonly",
            MediaDirection::Inactive => "a=inactive",
        }
    }
    
    /// Parse from SDP attribute line
    pub fn from_sdp_line(line: &str) -> Option<Self> {
        let line = line.trim();
        if line == "a=sendrecv" || line.ends_with("sendrecv") {
            Some(MediaDirection::SendRecv)
        } else if line == "a=sendonly" || line.ends_with("sendonly") {
            Some(MediaDirection::SendOnly)
        } else if line == "a=recvonly" || line.ends_with("recvonly") {
            Some(MediaDirection::RecvOnly)
        } else if line == "a=inactive" || line.ends_with("inactive") {
            Some(MediaDirection::Inactive)
        } else {
            None
        }
    }
}

/// Generate hold SDP by modifying the current SDP
/// Sets all media streams to sendonly (we might send music-on-hold)
pub fn generate_hold_sdp(current_sdp: &str) -> Result<String> {
    info!("Generating hold SDP from current SDP");
    
    let mut result = Vec::new();
    let mut in_media_section = false;
    let mut media_direction_set = false;
    
    for line in current_sdp.lines() {
        let trimmed = line.trim();
        
        // Check if we're entering a media section
        if trimmed.starts_with("m=") {
            // If we were in a previous media section and didn't set direction, add it
            if in_media_section && !media_direction_set {
                result.push(MediaDirection::SendOnly.to_sdp_attribute());
            }
            
            in_media_section = true;
            media_direction_set = false;
            result.push(line);
        }
        // Skip existing direction attributes, we'll add our own
        else if MediaDirection::from_sdp_line(trimmed).is_some() {
            // Replace with sendonly
            result.push(MediaDirection::SendOnly.to_sdp_attribute());
            media_direction_set = true;
        }
        // Just pass through other lines
        else if in_media_section && trimmed.starts_with("c=") {
            result.push(line);
        }
        else {
            result.push(line);
        }
    }
    
    // Handle case where last media section didn't have direction set
    if in_media_section && !media_direction_set {
        result.push(MediaDirection::SendOnly.to_sdp_attribute());
    }
    
    let sdp = result.join("\r\n");
    debug!("Generated hold SDP with sendonly for all media streams");
    Ok(sdp)
}

/// Generate resume SDP by modifying the current SDP
/// Sets all media streams to sendrecv (normal bidirectional media)
pub fn generate_resume_sdp(current_sdp: &str) -> Result<String> {
    info!("Generating resume SDP from current SDP");
    
    let mut result = Vec::new();
    let mut in_media_section = false;
    let mut media_direction_set = false;
    
    for line in current_sdp.lines() {
        let trimmed = line.trim();
        
        // Check if we're entering a media section
        if trimmed.starts_with("m=") {
            // If we were in a previous media section and didn't set direction, add it
            if in_media_section && !media_direction_set {
                result.push(MediaDirection::SendRecv.to_sdp_attribute());
            }
            
            in_media_section = true;
            media_direction_set = false;
            result.push(line);
        }
        // Skip existing direction attributes, we'll add our own
        else if MediaDirection::from_sdp_line(trimmed).is_some() {
            // Replace with sendrecv
            result.push(MediaDirection::SendRecv.to_sdp_attribute());
            media_direction_set = true;
        }
        // Just pass through other lines
        else if in_media_section && trimmed.starts_with("c=") {
            result.push(line);
        }
        else {
            result.push(line);
        }
    }
    
    // Handle case where last media section didn't have direction set
    if in_media_section && !media_direction_set {
        result.push(MediaDirection::SendRecv.to_sdp_attribute());
    }
    
    let sdp = result.join("\r\n");
    debug!("Generated resume SDP with sendrecv for all media streams");
    Ok(sdp)
}

/// Parse media directions from SDP
/// Returns a vector of (media_index, direction) tuples
pub fn parse_media_directions(sdp: &str) -> Vec<(usize, MediaDirection)> {
    let mut directions = Vec::new();
    let mut current_media_index = None;
    let mut session_direction = None;
    
    for line in sdp.lines() {
        let trimmed = line.trim();
        
        // New media section
        if trimmed.starts_with("m=") {
            current_media_index = Some(directions.len());
            // Default to session direction or sendrecv
            directions.push((
                current_media_index.unwrap(),
                session_direction.unwrap_or(MediaDirection::SendRecv)
            ));
        }
        // Direction attribute
        else if let Some(dir) = MediaDirection::from_sdp_line(trimmed) {
            if let Some(idx) = current_media_index {
                // Media-level attribute
                directions[idx] = (idx, dir);
            } else {
                // Session-level attribute
                session_direction = Some(dir);
                // Update any media sections that don't have explicit direction
                for i in 0..directions.len() {
                    if directions[i].1 == MediaDirection::SendRecv {
                        directions[i].1 = dir;
                    }
                }
            }
        }
    }
    
    directions
}

/// Validate hold response according to RFC 3264
/// Returns true if the response is valid for the offer
pub fn validate_hold_response(offer_dir: MediaDirection, answer_dir: MediaDirection) -> bool {
    match (offer_dir, answer_dir) {
        // When we offer sendonly (hold), remote must answer recvonly or inactive
        (MediaDirection::SendOnly, MediaDirection::RecvOnly) => true,
        (MediaDirection::SendOnly, MediaDirection::Inactive) => true,
        // When we offer sendrecv (resume), remote can answer with any direction
        (MediaDirection::SendRecv, _) => true,
        // Inactive must be answered with inactive
        (MediaDirection::Inactive, MediaDirection::Inactive) => true,
        // RecvOnly must be answered with sendonly or inactive
        (MediaDirection::RecvOnly, MediaDirection::SendOnly) => true,
        (MediaDirection::RecvOnly, MediaDirection::Inactive) => true,
        // All other combinations are invalid
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_media_direction_conversion() {
        assert_eq!(MediaDirection::SendOnly.to_sdp_attribute(), "a=sendonly");
        assert_eq!(MediaDirection::SendRecv.to_sdp_attribute(), "a=sendrecv");
        assert_eq!(MediaDirection::RecvOnly.to_sdp_attribute(), "a=recvonly");
        assert_eq!(MediaDirection::Inactive.to_sdp_attribute(), "a=inactive");
    }
    
    #[test]
    fn test_media_direction_parsing() {
        assert_eq!(MediaDirection::from_sdp_line("a=sendonly"), Some(MediaDirection::SendOnly));
        assert_eq!(MediaDirection::from_sdp_line("  a=sendrecv  "), Some(MediaDirection::SendRecv));
        assert_eq!(MediaDirection::from_sdp_line("a=recvonly"), Some(MediaDirection::RecvOnly));
        assert_eq!(MediaDirection::from_sdp_line("a=inactive"), Some(MediaDirection::Inactive));
        assert_eq!(MediaDirection::from_sdp_line("a=rtpmap:0 PCMU/8000"), None);
    }
    
    #[test]
    fn test_generate_hold_sdp() {
        let original = "v=0\r\n\
                       o=- 123 456 IN IP4 192.168.1.100\r\n\
                       s=-\r\n\
                       c=IN IP4 192.168.1.100\r\n\
                       t=0 0\r\n\
                       m=audio 5004 RTP/AVP 0\r\n\
                       c=IN IP4 192.168.1.100\r\n\
                       a=sendrecv\r\n\
                       a=rtpmap:0 PCMU/8000";
        
        let hold_sdp = generate_hold_sdp(original).unwrap();
        assert!(hold_sdp.contains("a=sendonly"));
        assert!(!hold_sdp.contains("a=sendrecv"));
    }
    
    #[test]
    fn test_generate_resume_sdp() {
        let hold = "v=0\r\n\
                   o=- 123 456 IN IP4 192.168.1.100\r\n\
                   s=-\r\n\
                   c=IN IP4 192.168.1.100\r\n\
                   t=0 0\r\n\
                   m=audio 5004 RTP/AVP 0\r\n\
                   c=IN IP4 192.168.1.100\r\n\
                   a=sendonly\r\n\
                   a=rtpmap:0 PCMU/8000";
        
        let resume_sdp = generate_resume_sdp(hold).unwrap();
        assert!(resume_sdp.contains("a=sendrecv"));
        assert!(!resume_sdp.contains("a=sendonly"));
    }
    
    #[test]
    fn test_validate_hold_response() {
        // Valid hold responses
        assert!(validate_hold_response(MediaDirection::SendOnly, MediaDirection::RecvOnly));
        assert!(validate_hold_response(MediaDirection::SendOnly, MediaDirection::Inactive));
        
        // Invalid hold responses
        assert!(!validate_hold_response(MediaDirection::SendOnly, MediaDirection::SendOnly));
        assert!(!validate_hold_response(MediaDirection::SendOnly, MediaDirection::SendRecv));
        
        // Valid resume responses (any direction is valid)
        assert!(validate_hold_response(MediaDirection::SendRecv, MediaDirection::SendRecv));
        assert!(validate_hold_response(MediaDirection::SendRecv, MediaDirection::SendOnly));
        assert!(validate_hold_response(MediaDirection::SendRecv, MediaDirection::RecvOnly));
        assert!(validate_hold_response(MediaDirection::SendRecv, MediaDirection::Inactive));
    }
    
    #[test]
    fn test_parse_media_directions() {
        let sdp = "v=0\r\n\
                  o=- 123 456 IN IP4 192.168.1.100\r\n\
                  s=-\r\n\
                  c=IN IP4 192.168.1.100\r\n\
                  t=0 0\r\n\
                  m=audio 5004 RTP/AVP 0\r\n\
                  a=sendonly\r\n\
                  m=video 5006 RTP/AVP 96\r\n\
                  a=recvonly";
        
        let directions = parse_media_directions(sdp);
        assert_eq!(directions.len(), 2);
        assert_eq!(directions[0], (0, MediaDirection::SendOnly));
        assert_eq!(directions[1], (1, MediaDirection::RecvOnly));
    }
}
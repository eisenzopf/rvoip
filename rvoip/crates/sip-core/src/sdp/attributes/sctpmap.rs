//! SDP SCTP Map Attribute Parser
//!
//! Implements parser for legacy SCTP Map attributes as defined in draft-ietf-mmusic-sctp-sdp.
//! These are now deprecated in favor of the sctp-port attribute in RFC 8841.
//! Format: a=sctpmap:<port> <app> <streams>

use crate::error::{Error, Result};
use crate::sdp::attributes::common::is_valid_token;

/// Parses sctpmap attribute: a=sctpmap:<port> <app> <streams>
/// Legacy attribute for SCTP in WebRTC data channels (obsolete by RFC 8841)
///
/// Example: a=sctpmap:5000 webrtc-datachannel 1024
pub fn parse_sctpmap(value: &str) -> Result<(u16, String, u32)> {
    // Example: a=sctpmap:5000 webrtc-datachannel 1024
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() < 3 {
        return Err(Error::SdpParsingError(format!("Invalid sctpmap format: {}", value)));
    }
    
    // Parse the SCTP port number
    let port = match parts[0].parse::<u16>() {
        Ok(p) => p,
        Err(_) => return Err(Error::SdpParsingError(format!("Invalid port in sctpmap: {}", parts[0])))
    };
    
    // The app name (typically 'webrtc-datachannel')
    let app = parts[1].to_string();
    if !is_valid_token(&app) {
        return Err(Error::SdpParsingError(format!("Invalid app name in sctpmap: {}", app)));
    }
    
    // The number of streams
    let streams = match parts[2].parse::<u32>() {
        Ok(s) => s,
        Err(_) => return Err(Error::SdpParsingError(format!("Invalid streams value in sctpmap: {}", parts[2])))
    };
    
    Ok((port, app, streams))
} 
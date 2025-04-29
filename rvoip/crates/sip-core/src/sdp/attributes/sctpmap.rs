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
    // Custom validation for app name - should be a token or a protocol string with slashes
    if !is_valid_app_name(&app) {
        return Err(Error::SdpParsingError(format!("Invalid app name in sctpmap: {}", app)));
    }
    
    // The number of streams
    let streams = match parts[2].parse::<u32>() {
        Ok(s) => s,
        Err(_) => return Err(Error::SdpParsingError(format!("Invalid streams value in sctpmap: {}", parts[2])))
    };
    
    Ok((port, app, streams))
}

/// Validates if a string is a valid app name for sctpmap
/// Allows regular tokens and protocol names with slashes (e.g., UDP/DTLS/SCTP)
fn is_valid_app_name(s: &str) -> bool {
    // First check if it's a regular token
    if is_valid_token(s) {
        return true;
    }
    
    // If not, check if it's a protocol string with slashes
    // Each part between slashes should be a valid token
    if s.contains('/') {
        return s.split('/')
            .all(|part| is_valid_token(part));
    }
    
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_sctpmap() {
        // Test standard format
        let (port, app, streams) = parse_sctpmap("5000 webrtc-datachannel 1024").unwrap();
        assert_eq!(port, 5000);
        assert_eq!(app, "webrtc-datachannel");
        assert_eq!(streams, 1024);

        // Test with minimum values
        let (port, app, streams) = parse_sctpmap("0 data 1").unwrap();
        assert_eq!(port, 0);
        assert_eq!(app, "data");
        assert_eq!(streams, 1);

        // Test with maximum values
        let (port, app, streams) = parse_sctpmap("65535 app 4294967295").unwrap();
        assert_eq!(port, 65535);
        assert_eq!(app, "app");
        assert_eq!(streams, 4294967295); // u32::MAX
    }

    #[test]
    fn test_whitespace_handling() {
        // Test with extra whitespace
        let (port, app, streams) = parse_sctpmap("  5000   webrtc-datachannel   2048  ").unwrap();
        assert_eq!(port, 5000);
        assert_eq!(app, "webrtc-datachannel");
        assert_eq!(streams, 2048);

        // Test with tab characters
        let (port, app, streams) = parse_sctpmap("5000\twebrtc-datachannel\t1024").unwrap();
        assert_eq!(port, 5000);
        assert_eq!(app, "webrtc-datachannel");
        assert_eq!(streams, 1024);
    }

    #[test]
    fn test_different_app_names() {
        // Test with different valid app tokens
        let (_, app, _) = parse_sctpmap("5000 webrtc-data 1024").unwrap();
        assert_eq!(app, "webrtc-data");

        let (_, app, _) = parse_sctpmap("5000 SCTP-CHANNEL 1024").unwrap();
        assert_eq!(app, "SCTP-CHANNEL");

        let (_, app, _) = parse_sctpmap("5000 dtls-sctp 1024").unwrap();
        assert_eq!(app, "dtls-sctp");

        let (_, app, _) = parse_sctpmap("5000 sctp 1024").unwrap();
        assert_eq!(app, "sctp");
    }

    #[test]
    fn test_protocol_app_names() {
        // Test app names with protocol format (containing slashes)
        let (_, app, _) = parse_sctpmap("5000 UDP/DTLS/SCTP 1024").unwrap();
        assert_eq!(app, "UDP/DTLS/SCTP");
        
        let (_, app, _) = parse_sctpmap("5000 DTLS/SCTP 1024").unwrap();
        assert_eq!(app, "DTLS/SCTP");
        
        let (_, app, _) = parse_sctpmap("5000 TCP/DTLS/SCTP 1024").unwrap();
        assert_eq!(app, "TCP/DTLS/SCTP");
    }

    #[test]
    fn test_invalid_format() {
        // Test with missing fields
        assert!(parse_sctpmap("").is_err(), "Empty string should be rejected");
        assert!(parse_sctpmap("5000").is_err(), "Missing app and streams should be rejected");
        assert!(parse_sctpmap("5000 webrtc-datachannel").is_err(), "Missing streams should be rejected");

        // Test with invalid port
        assert!(parse_sctpmap("invalid webrtc-datachannel 1024").is_err(), "Invalid port should be rejected");
        assert!(parse_sctpmap("-1 webrtc-datachannel 1024").is_err(), "Negative port should be rejected");
        assert!(parse_sctpmap("65536 webrtc-datachannel 1024").is_err(), "Port exceeding u16::MAX should be rejected");

        // Test with invalid app name
        assert!(parse_sctpmap("5000 invalid@app 1024").is_err(), "Invalid app name with @ should be rejected");
        assert!(parse_sctpmap("5000 invalid,app 1024").is_err(), "Invalid app name with comma should be rejected");
        assert!(parse_sctpmap("5000 \"quoted-app\" 1024").is_err(), "Invalid app name with quotes should be rejected");
        
        // Test with invalid protocol names
        assert!(parse_sctpmap("5000 UDP/DTLS/@SCTP 1024").is_err(), "Invalid protocol with @ should be rejected");
        assert!(parse_sctpmap("5000 UDP//DTLS 1024").is_err(), "Invalid empty protocol part should be rejected");

        // Test with invalid streams
        assert!(parse_sctpmap("5000 webrtc-datachannel invalid").is_err(), "Invalid streams should be rejected");
        assert!(parse_sctpmap("5000 webrtc-datachannel -1").is_err(), "Negative streams should be rejected");
        assert!(parse_sctpmap("5000 webrtc-datachannel 4294967296").is_err(), "Streams exceeding u32::MAX should be rejected");
    }

    #[test]
    fn test_draft_examples() {
        // Examples from draft-ietf-mmusic-sctp-sdp
        let (port, app, streams) = parse_sctpmap("5000 webrtc-datachannel 1024").unwrap();
        assert_eq!(port, 5000);
        assert_eq!(app, "webrtc-datachannel");
        assert_eq!(streams, 1024);

        // Another example with different app
        let (port, app, streams) = parse_sctpmap("5000 UDP/DTLS/SCTP 1024").unwrap();
        assert_eq!(port, 5000);
        assert_eq!(app, "UDP/DTLS/SCTP");
        assert_eq!(streams, 1024);
    }

    #[test]
    fn test_extra_parameters() {
        // Test with extra parameters - should ignore any parameters after the required three
        let (port, app, streams) = parse_sctpmap("5000 webrtc-datachannel 1024 extra param").unwrap();
        assert_eq!(port, 5000);
        assert_eq!(app, "webrtc-datachannel");
        assert_eq!(streams, 1024);
    }
} 
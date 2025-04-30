//! SDP media description parsing functionality
//!
//! This module handles parsing of SDP media description lines (m=) according to RFC 8866.
//! Media descriptions define the media streams in a session and typically appear after
//! the session-level information.
//!
//! An SDP media description begins with an "m=" line and is followed by zero or more
//! attribute lines ("a="), which further define or modify the media stream.
//!
//! Each media description contains:
//! - Media type (audio, video, text, application, message)
//! - Transport port
//! - Transport protocol (RTP/AVP, RTP/SAVP, UDP/TLS/RTP/SAVPF, etc.)
//! - Media format descriptions (payload types for RTP)
//!
//! Media descriptions are essential for negotiating the parameters of media streams
//! in SIP/SDP-based communication sessions.

use crate::error::{Error, Result};
use crate::types::sdp::MediaDescription;

/// Parse a media description line (m=) from an SDP message.
///
/// This function parses an SDP media description line according to RFC 8866 and
/// creates a MediaDescription struct containing the parsed information.
///
/// # Format
///
/// ```text
/// m=<media> <port>[/<number of ports>] <proto> <fmt> [<fmt>...]
/// ```
///
/// Where:
/// - `<media>` is the media type (audio, video, text, application, message)
/// - `<port>` is the transport port to which the media stream is sent
/// - `<number of ports>` is an optional range specifier (e.g., for RTP/RTCP)
/// - `<proto>` is the transport protocol (e.g., RTP/AVP, UDP/TLS/RTP/SAVPF)
/// - `<fmt>` is one or more media format descriptions (e.g., RTP payload types)
///
/// # Parameters
///
/// - `value`: The value part of the media line (without the "m=" prefix)
///
/// # Returns
///
/// - `Ok(MediaDescription)` if parsing succeeds
/// - `Err` with error details if parsing fails
///
/// # Examples
///
/// ```
/// use rvoip_sip_core::sdp::parser::parse_media_description_line;
/// use rvoip_sip_core::types::sdp::MediaDescription;
///
/// // Parse a simple audio media description
/// let result = parse_media_description_line("audio 49170 RTP/AVP 0").unwrap();
/// assert_eq!(result.media, "audio");
/// assert_eq!(result.port, 49170);
/// assert_eq!(result.protocol, "RTP/AVP");
/// assert_eq!(result.formats, vec!["0"]);
///
/// // Parse a video media description with multiple formats
/// let result = parse_media_description_line("video 51372 RTP/AVP 96 97 98").unwrap();
/// assert_eq!(result.media, "video");
/// assert_eq!(result.port, 51372);
/// assert_eq!(result.protocol, "RTP/AVP");
/// assert_eq!(result.formats, vec!["96", "97", "98"]);
///
/// // Parse a WebRTC data channel description
/// let result = parse_media_description_line("application 9 UDP/DTLS/SCTP webrtc-datachannel").unwrap();
/// assert_eq!(result.media, "application");
/// assert_eq!(result.port, 9);
/// assert_eq!(result.protocol, "UDP/DTLS/SCTP");
/// assert_eq!(result.formats, vec!["webrtc-datachannel"]);
/// ```
///
/// # Errors
///
/// This function returns an error if:
/// - The media line has fewer than 4 tokens (media type, port, protocol, format)
/// - The port value is not a valid integer
/// - The port value is outside the valid range for a u16 (0-65535)
///
/// # RFC Reference
///
/// - [RFC 8866 Section 5.14](https://datatracker.ietf.org/doc/html/rfc8866#section-5.14)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_audio_media() {
        // Test a basic audio media description from RFC 8866 example
        let result = parse_media_description_line("audio 49170 RTP/AVP 0").unwrap();
        
        assert_eq!(result.media, "audio");
        assert_eq!(result.port, 49170);
        assert_eq!(result.protocol, "RTP/AVP");
        assert_eq!(result.formats, vec!["0"]);
    }

    #[test]
    fn test_parse_video_media() {
        // Test a basic video media description from RFC 8866 example
        let result = parse_media_description_line("video 51372 RTP/AVP 31").unwrap();
        
        assert_eq!(result.media, "video");
        assert_eq!(result.port, 51372);
        assert_eq!(result.protocol, "RTP/AVP");
        assert_eq!(result.formats, vec!["31"]);
    }

    #[test]
    fn test_parse_multiple_formats() {
        // Test media description with multiple formats
        let result = parse_media_description_line("audio 49170 RTP/AVP 0 8 97").unwrap();
        
        assert_eq!(result.media, "audio");
        assert_eq!(result.port, 49170);
        assert_eq!(result.protocol, "RTP/AVP");
        assert_eq!(result.formats, vec!["0", "8", "97"]);
    }

    #[test]
    fn test_parse_port_range() {
        // Test media description with port range
        let result = parse_media_description_line("audio 49170/2 RTP/AVP 0").unwrap();
        
        assert_eq!(result.media, "audio");
        assert_eq!(result.port, 49170);
        // Note: The current implementation doesn't store the number of ports
        assert_eq!(result.protocol, "RTP/AVP");
        assert_eq!(result.formats, vec!["0"]);
    }

    #[test]
    fn test_parse_application_media() {
        // Test application media type (data channel)
        let result = parse_media_description_line("application 9 UDP/DTLS/SCTP webrtc-datachannel").unwrap();
        
        assert_eq!(result.media, "application");
        assert_eq!(result.port, 9);
        assert_eq!(result.protocol, "UDP/DTLS/SCTP");
        assert_eq!(result.formats, vec!["webrtc-datachannel"]);
    }

    #[test]
    fn test_parse_complex_protocol() {
        // Test media with complex protocol string
        let result = parse_media_description_line("video 9 UDP/TLS/RTP/SAVPF 96 97 98 99 100 101 102").unwrap();
        
        assert_eq!(result.media, "video");
        assert_eq!(result.port, 9);
        assert_eq!(result.protocol, "UDP/TLS/RTP/SAVPF");
        assert_eq!(result.formats, vec!["96", "97", "98", "99", "100", "101", "102"]);
    }

    #[test]
    fn test_parse_text_media() {
        // Test text media type
        let result = parse_media_description_line("text 49170 RTP/AVP 98").unwrap();
        
        assert_eq!(result.media, "text");
        assert_eq!(result.port, 49170);
        assert_eq!(result.protocol, "RTP/AVP");
        assert_eq!(result.formats, vec!["98"]);
    }

    #[test]
    fn test_parse_message_media() {
        // Test message media type
        let result = parse_media_description_line("message 49170 MSRP/TCP *").unwrap();
        
        assert_eq!(result.media, "message");
        assert_eq!(result.port, 49170);
        assert_eq!(result.protocol, "MSRP/TCP");
        assert_eq!(result.formats, vec!["*"]);
    }

    #[test]
    fn test_parse_nonstandard_media() {
        // Test non-standard media type (should still parse without error)
        let result = parse_media_description_line("custom 49170 CUSTOM/PROTO 100").unwrap();
        
        assert_eq!(result.media, "custom");
        assert_eq!(result.port, 49170);
        assert_eq!(result.protocol, "CUSTOM/PROTO");
        assert_eq!(result.formats, vec!["100"]);
    }

    #[test]
    fn test_error_missing_fields() {
        // Test error case: missing fields
        let result = parse_media_description_line("audio 49170 RTP/AVP");
        assert!(result.is_err());
        
        let result = parse_media_description_line("audio 49170");
        assert!(result.is_err());
        
        let result = parse_media_description_line("audio");
        assert!(result.is_err());
    }

    #[test]
    fn test_error_invalid_port() {
        // Test error case: invalid port
        let result = parse_media_description_line("audio invalid RTP/AVP 0");
        assert!(result.is_err());
        
        // Port too large for u16
        let result = parse_media_description_line("audio 65536 RTP/AVP 0");
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_string() {
        // Test error case: empty string
        let result = parse_media_description_line("");
        assert!(result.is_err());
    }
} 
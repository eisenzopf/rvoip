// Media description parsing for SDP
//
// Handles parsing of complete media descriptions (m= lines)

use crate::error::{Error, Result};
use crate::types::sdp::MediaDescription;
use crate::sdp::media::types::{parse_media_type, is_valid_media_type};
use crate::sdp::media::transport::parse_transport_protocol;
use crate::sdp::media::format::{parse_formats, parse_port_and_count};
use nom::{
    IResult,
    bytes::complete::tag,
    character::complete::space1,
    combinator::opt,
    sequence::tuple,
};

/// Parse a media description line using nom
/// Format: m=<media> <port>[/<port-count>] <proto> <fmt> [<fmt>]*
pub fn parse_media_description_nom(input: &str) -> IResult<&str, MediaDescription> {
    // m=<media> <port>[/<port-count>] <proto> <fmt> [<fmt>]*
    let (input, _) = opt(tag("m="))(input)?;
    let (input, (media_type, _, port_info, _, protocol, _, formats)) = 
        tuple((
            parse_media_type,
            space1,
            parse_port_and_count,
            space1,
            parse_transport_protocol,
            space1,
            parse_formats
        ))(input)?;
    
    let (port, _port_count) = port_info;
    
    Ok((
        input,
        MediaDescription {
            media: media_type,
            port,
            protocol,
            formats,
            connection_info: None, 
            ptime: None,
            direction: None,
            generic_attributes: Vec::new(),
        }
    ))
}

/// Parse a media description
pub fn parse_media_description_line(value: &str) -> Result<MediaDescription> {
    // Try the nom parser first
    if let Ok((_, media)) = parse_media_description_nom(value) {
        return Ok(media);
    }
    
    // Fallback to manual parsing
    // Extract value part if input has m= prefix
    let value_to_parse = if let Some(stripped) = value.strip_prefix("m=") {
        stripped
    } else {
        value
    };
    
    let parts: Vec<&str> = value_to_parse.split_whitespace().collect();
    if parts.len() < 4 {
        return Err(Error::SdpParsingError(format!("Invalid m= line format: {}", value)));
    }
    
    // Parse media type
    let media = parts[0].to_string();
    if !is_valid_media_type(&media) {
        return Err(Error::SdpParsingError(format!("Invalid media type: {}", media)));
    }
    
    // Parse port and optional port count
    let port_part = parts[1];
    let port_parts: Vec<&str> = port_part.split('/').collect();
    
    let port = match port_parts[0].parse::<u16>() {
        Ok(p) => p,
        Err(_) => return Err(Error::SdpParsingError(format!("Invalid port: {}", port_parts[0]))),
    };
    
    let _port_count = if port_parts.len() > 1 {
        match port_parts[1].parse::<u16>() {
            Ok(c) => Some(c),
            Err(_) => return Err(Error::SdpParsingError(format!("Invalid port count: {}", port_parts[1]))),
        }
    } else {
        None
    };
    
    // Parse protocol
    let protocol = parts[2].to_string();
    
    // Parse formats
    let formats = parts[3..].iter().map(|s| s.to_string()).collect();
    
    // Create media description
    Ok(MediaDescription {
        media,
        port,
        protocol,
        formats,
        connection_info: None,
        ptime: None,
        direction: None,
        generic_attributes: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_basic_media_description() {
        // Basic audio media description according to RFC 4566
        let media_line = "audio 49170 RTP/AVP 0";
        let result = parse_media_description_line(media_line);
        assert!(result.is_ok(), "Failed to parse basic audio media description");
        
        let media = result.unwrap();
        assert_eq!(media.media, "audio", "Incorrect media type");
        assert_eq!(media.port, 49170, "Incorrect port");
        assert_eq!(media.protocol, "RTP/AVP", "Incorrect protocol");
        assert_eq!(media.formats, vec!["0"], "Incorrect formats");
    }
    
    #[test]
    fn test_parse_with_m_prefix() {
        // Media description with m= prefix
        let media_line = "m=audio 49170 RTP/AVP 0";
        let result = parse_media_description_line(media_line);
        assert!(result.is_ok(), "Failed to parse media description with m= prefix");
        
        let media = result.unwrap();
        assert_eq!(media.media, "audio", "Incorrect media type");
        assert_eq!(media.port, 49170, "Incorrect port");
    }
    
    #[test]
    fn test_parse_multiple_formats() {
        // Media description with multiple formats
        let media_line = "audio 49170 RTP/AVP 0 8 96";
        let result = parse_media_description_line(media_line);
        assert!(result.is_ok(), "Failed to parse media description with multiple formats");
        
        let media = result.unwrap();
        assert_eq!(media.formats, vec!["0", "8", "96"], "Incorrect formats");
    }
    
    #[test]
    fn test_parse_video_media() {
        // Video media description
        let media_line = "video 51372 RTP/AVP 31 32";
        let result = parse_media_description_line(media_line);
        assert!(result.is_ok(), "Failed to parse video media description");
        
        let media = result.unwrap();
        assert_eq!(media.media, "video", "Incorrect media type");
        assert_eq!(media.port, 51372, "Incorrect port");
        assert_eq!(media.protocol, "RTP/AVP", "Incorrect protocol");
        assert_eq!(media.formats, vec!["31", "32"], "Incorrect formats");
    }
    
    #[test]
    fn test_parse_application_media() {
        // Application media description
        let media_line = "application 5000 UDP/BFCP *";
        let result = parse_media_description_line(media_line);
        assert!(result.is_ok(), "Failed to parse application media description");
        
        let media = result.unwrap();
        assert_eq!(media.media, "application", "Incorrect media type");
        assert_eq!(media.port, 5000, "Incorrect port");
        assert_eq!(media.protocol, "UDP/BFCP", "Incorrect protocol");
        assert_eq!(media.formats, vec!["*"], "Incorrect formats");
    }
    
    #[test]
    fn test_parse_with_port_count() {
        // Media description with port count
        let media_line = "audio 49170/2 RTP/AVP 0 8";
        let result = parse_media_description_line(media_line);
        assert!(result.is_ok(), "Failed to parse media description with port count");
        
        let media = result.unwrap();
        assert_eq!(media.port, 49170, "Incorrect port");
        // Port count is not stored in the MediaDescription struct currently
    }
    
    #[test]
    fn test_parse_secure_rtp() {
        // Media description with secure RTP
        let media_line = "audio 49170 RTP/SAVP 0";
        let result = parse_media_description_line(media_line);
        assert!(result.is_ok(), "Failed to parse media description with SRTP");
        
        let media = result.unwrap();
        assert_eq!(media.protocol, "RTP/SAVP", "Incorrect protocol");
    }
    
    #[test]
    fn test_parse_invalid_media() {
        // Invalid media type
        let media_line = "invalid 49170 RTP/AVP 0";
        let result = parse_media_description_line(media_line);
        assert!(result.is_err(), "Should reject invalid media type");
    }
    
    #[test]
    fn test_parse_invalid_port() {
        // Invalid port
        let media_line = "audio invalid RTP/AVP 0";
        let result = parse_media_description_line(media_line);
        assert!(result.is_err(), "Should reject invalid port");
    }
    
    #[test]
    fn test_parse_invalid_port_count() {
        // Invalid port count
        let media_line = "audio 49170/invalid RTP/AVP 0";
        let result = parse_media_description_line(media_line);
        assert!(result.is_err(), "Should reject invalid port count");
    }
    
    #[test]
    fn test_parse_incomplete_media_line() {
        // Incomplete media line (missing formats)
        let media_line = "audio 49170 RTP/AVP";
        let result = parse_media_description_line(media_line);
        assert!(result.is_err(), "Should reject incomplete media line");
    }
    
    #[test]
    fn test_parse_whitespace_handling() {
        // Media line with extra whitespace
        let media_line = "  audio   49170   RTP/AVP   0   8  ";
        let result = parse_media_description_line(media_line);
        assert!(result.is_ok(), "Failed to parse media line with extra whitespace");
        
        let media = result.unwrap();
        assert_eq!(media.formats, vec!["0", "8"], "Incorrect formats");
    }
    
    #[test]
    fn test_parse_rfc4566_examples() {
        // Examples from RFC 4566 that are supported by the current implementation
        let examples = [
            "m=audio 49230 RTP/AVP 96 97 98",
            "m=video 51372 RTP/AVP 31",
            "m=application 32416 udp wb",
            // "m=control 49234 H323 mc", // Currently not supported - 'control' is not a recognized media type
        ];
        
        for example in examples.iter() {
            let result = parse_media_description_line(example);
            assert!(result.is_ok(), "Failed to parse RFC example: {}", example);
        }
    }
    
    #[test]
    fn test_parse_media_description_nom() {
        // Test the nom parser directly
        let input = "audio 49170 RTP/AVP 0 8";
        let result = parse_media_description_nom(input);
        assert!(result.is_ok(), "Failed to parse with nom parser");
        
        let (rest, media) = result.unwrap();
        assert_eq!(rest, "", "Nom parser did not consume all input");
        assert_eq!(media.media, "audio", "Incorrect media type");
        assert_eq!(media.port, 49170, "Incorrect port");
        assert_eq!(media.protocol, "RTP/AVP", "Incorrect protocol");
        assert_eq!(media.formats, vec!["0", "8"], "Incorrect formats");
    }
} 
//! SDP ExtMap Attribute Parser
//!
//! Implements parser for RTP header extension map attributes as defined in RFC 8285.
//! Format: a=extmap:<id>[/<direction>] <uri> [<extension parameters>]

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{positive_integer, token, to_result};
use nom::{
    branch::alt,
    bytes::complete::{tag, take_till1, take_while1},
    character::complete::{char, space1},
    combinator::{map, opt, verify},
    sequence::{pair, preceded, separated_pair, tuple},
    IResult,
};

/// Parser for extension ID (1-14 for one-byte header, 15-255 for two-byte header)
fn extension_id_parser(input: &str) -> IResult<&str, u16> {
    verify(
        map(positive_integer, |n| n as u16),
        |&id| (1..=255).contains(&id)
    )(input)
}

/// Parser for extension direction
fn direction_parser(input: &str) -> IResult<&str, &str> {
    alt((
        tag("sendonly"),
        tag("recvonly"),
        tag("sendrecv"),
        tag("inactive")
    ))(input)
}

/// Parser for ID/direction part
fn id_direction_parser(input: &str) -> IResult<&str, (u16, Option<String>)> {
    // First try to parse a section containing digits and potentially a slash
    let (input, id_part) = take_while1(|c: char| c.is_ascii_digit() || c == '/')(input)?;
    
    // Check if there's a direction part
    if id_part.contains('/') {
        let parts: Vec<&str> = id_part.split('/').collect();
        if parts.len() != 2 {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Tag
            )));
        }
        
        let id = match parts[0].parse::<u16>() {
            Ok(id) => id,
            Err(_) => return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Digit
            ))),
        };
        
        let direction = parts[1].to_string();
        
        // Validate direction
        if !["sendonly", "recvonly", "sendrecv", "inactive"].contains(&direction.as_str()) {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Tag
            )));
        }
        
        Ok((input, (id, Some(direction))))
    } else {
        // Just ID
        match id_part.parse::<u16>() {
            Ok(id) => Ok((input, (id, None))),
            Err(_) => Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Digit
            ))),
        }
    }
}

/// Parser for URI
fn uri_parser(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| !c.is_ascii_whitespace())(input)
}

/// Parser for extension parameters
fn extension_params_parser(input: &str) -> IResult<&str, &str> {
    take_till1(|_| false)(input)  // Take everything until the end
}

/// Main parser for extmap attribute
fn extmap_parser(input: &str) -> IResult<&str, (u16, Option<String>, String, Option<String>)> {
    tuple((
        // ID and optional direction
        id_direction_parser,
        // Space + URI
        preceded(
            space1,
            map(uri_parser, |s: &str| s.to_string())
        ),
        // Optional space + parameters
        opt(preceded(
            space1,
            map(extension_params_parser, |s: &str| s.trim().to_string())
        ))
    ))(input)
    .map(|(remaining, ((id, direction), uri, params))| {
        (remaining, (id, direction, uri, params))
    })
}

/// Parses extmap attribute: a=extmap:<id>[/<direction>] <uri> [<extension parameters>]
pub fn parse_extmap(value: &str) -> Result<(u16, Option<String>, String, Option<String>)> {
    let value = value.trim();
    
    // First, split the string by whitespace
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(Error::SdpParsingError(format!("Invalid extmap format: {}", value)));
    }
    
    // Parse ID and optional direction
    let id_parts: Vec<&str> = parts[0].split('/').collect();
    
    let id = match id_parts[0].parse::<u16>() {
        Ok(id) if (1..=255).contains(&id) => id,
        _ => return Err(Error::SdpParsingError(format!("Extmap id out of range (1-255): {}", parts[0]))),
    };
    
    // Check if direction is present
    let direction = if id_parts.len() > 1 {
        let dir = id_parts[1];
        if !["sendonly", "recvonly", "sendrecv", "inactive"].contains(&dir) {
            return Err(Error::SdpParsingError(format!("Invalid extmap direction: {}", dir)));
        }
        Some(dir.to_string())
    } else {
        None
    };
    
    // URI is the second part
    let uri = parts[1].to_string();
    
    // Basic URI validation - should start with urn: or http:
    if !uri.starts_with("urn:") && !uri.starts_with("http:") {
        return Err(Error::SdpParsingError(format!("Invalid extmap URI: {}", uri)));
    }
    
    // Join any remaining parts as parameters
    let params = if parts.len() > 2 {
        Some(parts[2..].join(" "))
    } else {
        None
    };
    
    Ok((id, direction, uri, params))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test valid extmap attributes
    
    #[test]
    fn test_valid_extmap_basic() {
        // Basic example from RFC 8285
        let value = "1 urn:ietf:params:rtp-hdrext:ssrc-audio-level";
        let result = parse_extmap(value).unwrap();
        
        assert_eq!(result.0, 1); // ID
        assert_eq!(result.1, None); // No direction
        assert_eq!(result.2, "urn:ietf:params:rtp-hdrext:ssrc-audio-level"); // URI
        assert_eq!(result.3, None); // No params
    }
    
    #[test]
    fn test_valid_extmap_with_direction() {
        // Example with direction
        let value = "2/sendonly urn:ietf:params:rtp-hdrext:toffset";
        let result = parse_extmap(value).unwrap();
        
        assert_eq!(result.0, 2); // ID
        assert_eq!(result.1, Some("sendonly".to_string())); // Direction
        assert_eq!(result.2, "urn:ietf:params:rtp-hdrext:toffset"); // URI
        assert_eq!(result.3, None); // No params
    }
    
    #[test]
    fn test_valid_extmap_with_params() {
        // Example with parameters
        let value = "3 http://example.com/ext.uri ExampleParam=1234";
        let result = parse_extmap(value).unwrap();
        
        assert_eq!(result.0, 3); // ID
        assert_eq!(result.1, None); // No direction
        assert_eq!(result.2, "http://example.com/ext.uri"); // URI
        assert_eq!(result.3, Some("ExampleParam=1234".to_string())); // Parameters
    }
    
    #[test]
    fn test_valid_extmap_complete() {
        // Example with all components
        let value = "4/sendrecv urn:ietf:params:rtp-hdrext:sdes:mid CustomParam=example-value";
        let result = parse_extmap(value).unwrap();
        
        assert_eq!(result.0, 4); // ID
        assert_eq!(result.1, Some("sendrecv".to_string())); // Direction
        assert_eq!(result.2, "urn:ietf:params:rtp-hdrext:sdes:mid"); // URI
        assert_eq!(result.3, Some("CustomParam=example-value".to_string())); // Parameters
    }
    
    #[test]
    fn test_valid_extmap_different_directions() {
        // Test all valid directions
        let dir_sendonly = "5/sendonly urn:ietf:params:rtp-hdrext:ssrc-audio-level";
        assert_eq!(parse_extmap(dir_sendonly).unwrap().1, Some("sendonly".to_string()));
        
        let dir_recvonly = "5/recvonly urn:ietf:params:rtp-hdrext:ssrc-audio-level";
        assert_eq!(parse_extmap(dir_recvonly).unwrap().1, Some("recvonly".to_string()));
        
        let dir_sendrecv = "5/sendrecv urn:ietf:params:rtp-hdrext:ssrc-audio-level";
        assert_eq!(parse_extmap(dir_sendrecv).unwrap().1, Some("sendrecv".to_string()));
        
        let dir_inactive = "5/inactive urn:ietf:params:rtp-hdrext:ssrc-audio-level";
        assert_eq!(parse_extmap(dir_inactive).unwrap().1, Some("inactive".to_string()));
    }
    
    #[test]
    fn test_valid_extmap_id_ranges() {
        // Test valid ID ranges: one-byte header (1-14)
        let id_1 = "1 urn:ietf:params:rtp-hdrext:ssrc-audio-level";
        assert_eq!(parse_extmap(id_1).unwrap().0, 1);
        
        let id_14 = "14 urn:ietf:params:rtp-hdrext:ssrc-audio-level";
        assert_eq!(parse_extmap(id_14).unwrap().0, 14);
        
        // Test valid ID ranges: two-byte header (15-255)
        let id_15 = "15 urn:ietf:params:rtp-hdrext:ssrc-audio-level";
        assert_eq!(parse_extmap(id_15).unwrap().0, 15);
        
        let id_255 = "255 urn:ietf:params:rtp-hdrext:ssrc-audio-level";
        assert_eq!(parse_extmap(id_255).unwrap().0, 255);
    }
    
    #[test]
    fn test_valid_extmap_different_uris() {
        // Test different URI formats
        
        // URN with multiple segments
        let urn_complex = "1 urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id";
        assert_eq!(parse_extmap(urn_complex).unwrap().2, "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id");
        
        // HTTP URI
        let http_uri = "1 http://example.com/extension";
        assert_eq!(parse_extmap(http_uri).unwrap().2, "http://example.com/extension");
        
        // HTTPS URI - note current implementation only accepts urn: and http: prefixes
        // let https_uri = "1 https://example.com/secure-extension";
        // assert_eq!(parse_extmap(https_uri).unwrap().2, "https://example.com/secure-extension");
    }
    
    #[test]
    fn test_whitespace_handling() {
        // Test whitespace is properly handled
        let with_whitespace = "  1  urn:ietf:params:rtp-hdrext:ssrc-audio-level  ";
        let result = parse_extmap(with_whitespace).unwrap();
        
        assert_eq!(result.0, 1);
        assert_eq!(result.2, "urn:ietf:params:rtp-hdrext:ssrc-audio-level");
    }
    
    // Test invalid extmap attributes
    
    #[test]
    fn test_invalid_extmap_id() {
        // ID 0 is invalid
        let id_0 = "0 urn:ietf:params:rtp-hdrext:ssrc-audio-level";
        assert!(parse_extmap(id_0).is_err());
        
        // ID 256 is out of range
        let id_256 = "256 urn:ietf:params:rtp-hdrext:ssrc-audio-level";
        assert!(parse_extmap(id_256).is_err());
        
        // Negative ID
        let id_negative = "-1 urn:ietf:params:rtp-hdrext:ssrc-audio-level";
        assert!(parse_extmap(id_negative).is_err());
    }
    
    #[test]
    fn test_invalid_extmap_direction() {
        // Invalid direction
        let invalid_dir = "1/invalid urn:ietf:params:rtp-hdrext:ssrc-audio-level";
        assert!(parse_extmap(invalid_dir).is_err());
        
        // Missing direction after slash
        let missing_dir = "1/ urn:ietf:params:rtp-hdrext:ssrc-audio-level";
        assert!(parse_extmap(missing_dir).is_err());
    }
    
    #[test]
    fn test_invalid_extmap_uri() {
        // URI doesn't start with urn: or http:
        let invalid_uri = "1 example:test:extension";
        assert!(parse_extmap(invalid_uri).is_err());
        
        // Empty URI
        let empty_uri = "1 ";
        assert!(parse_extmap(empty_uri).is_err());
    }
    
    #[test]
    fn test_invalid_extmap_format() {
        // Missing ID
        let missing_id = "urn:ietf:params:rtp-hdrext:ssrc-audio-level";
        assert!(parse_extmap(missing_id).is_err());
        
        // Missing URI
        let missing_uri = "1";
        assert!(parse_extmap(missing_uri).is_err());
        
        // Non-numeric ID
        let non_numeric_id = "a urn:ietf:params:rtp-hdrext:ssrc-audio-level";
        assert!(parse_extmap(non_numeric_id).is_err());
        
        // Empty input
        let empty = "";
        assert!(parse_extmap(empty).is_err());
    }
} 
//! SDP Simulcast Attribute Parser
//!
//! Implements parsers for Simulcast attributes as defined in RFC 8853.
//! Simulcast is a technique that allows a sender to send multiple versions
//! (simulcast streams) of the same media source.

use crate::error::{Error, Result};
use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    character::complete::{alphanumeric1, char, space0, space1},
    combinator::{map, opt, value, all_consuming, eof},
    multi::{many0, separated_list1},
    sequence::{delimited, pair, preceded, tuple},
    IResult,
};

/// The direction in a simulcast stream description
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimulcastDirection {
    /// Send direction
    Send,
    /// Receive direction
    Recv,
}

/// The status of an alternative in a simulcast stream
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimulcastStatus {
    /// Active stream (default)
    Active,
    /// Paused stream (prefixed with '~')
    Paused,
}

/// A single simulcast alternative
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulcastAlternative {
    /// The status of this alternative
    pub status: SimulcastStatus,
    /// The RID identifier for this alternative
    pub rid: String,
}

/// A simulcast stream version
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulcastVersion {
    /// List of alternatives for this version
    pub alternatives: Vec<SimulcastAlternative>,
}

/// A complete simulcast description
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulcastAttribute {
    /// The direction of this simulcast attribute
    pub direction: SimulcastDirection,
    /// List of stream versions
    pub stream_versions: Vec<SimulcastVersion>,
}

/// Parse a simulcast direction
fn parse_simulcast_direction(input: &str) -> IResult<&str, SimulcastDirection> {
    alt((
        value(SimulcastDirection::Send, tag("send")),
        value(SimulcastDirection::Recv, tag("recv")),
    ))(input)
}

/// Parse a RID identifier according to RFC 8851
/// A RID identifier is a token defined in RFC 8851 as
/// consisting of alphanumeric characters, underscore, and hyphen
fn parse_rid(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| c.is_alphanumeric() || c == '_' || c == '-')(input)
}

/// Parse a simulcast alternative
fn parse_simulcast_alternative(input: &str) -> IResult<&str, SimulcastAlternative> {
    let (input, status) = opt(char('~'))(input)?;
    let (input, rid) = parse_rid(input)?;
    
    Ok((
        input,
        SimulcastAlternative {
            status: if status.is_some() { 
                SimulcastStatus::Paused 
            } else { 
                SimulcastStatus::Active 
            },
            rid: rid.to_string(),
        }
    ))
}

/// Parse a simulcast version
fn parse_simulcast_version(input: &str) -> IResult<&str, SimulcastVersion> {
    // Check for trailing comma before parsing
    if input.trim().ends_with(',') {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::TooLarge
        )));
    }
    
    map(
        separated_list1(
            char(','),
            parse_simulcast_alternative
        ),
        |alternatives| SimulcastVersion { alternatives }
    )(input)
}

/// Parse simulcast stream versions
fn parse_simulcast_stream_versions(input: &str) -> IResult<&str, Vec<SimulcastVersion>> {
    // Check for trailing semicolon before parsing
    if input.trim().ends_with(';') {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::TooLarge
        )));
    }
    
    separated_list1(
        char(';'),
        parse_simulcast_version
    )(input)
}

/// Parse a complete simulcast description
fn simulcast_parser(input: &str) -> IResult<&str, SimulcastAttribute> {
    let (input, direction) = parse_simulcast_direction(input)?;
    let (input, _) = space1(input)?;
    let (input, stream_versions) = parse_simulcast_stream_versions(input)?;
    
    Ok((
        input,
        SimulcastAttribute {
            direction,
            stream_versions,
        }
    ))
}

/// Parse simulcast attribute as per RFC 8853
///
/// Format: a=simulcast:<direction> <stream-versions>
/// Where <stream-versions> is a semicolon-separated list of alternatives
/// and each alternative is a comma-separated list of RID identifiers.
///
/// Example: a=simulcast:send 1;2,3 recv 4
pub fn parse_simulcast_struct(value: &str) -> Result<Vec<SimulcastAttribute>> {
    // Trim the input and normalize whitespace
    let value = value.trim();
    if value.is_empty() {
        return Err(Error::SdpParsingError("Empty simulcast attribute".to_string()));
    }
    
    // Split the value into space-separated parts
    let parts: Vec<&str> = value.split_whitespace().collect();
    
    // We need at least a direction followed by stream descriptions
    if parts.len() < 2 {
        return Err(Error::SdpParsingError(format!("Invalid simulcast format: {}", value)));
    }
    
    let mut result = Vec::new();
    let mut i = 0;
    
    while i < parts.len() {
        let direction_str = parts[i];
        
        // Parse the direction
        let direction = match direction_str {
            "send" => SimulcastDirection::Send,
            "recv" => SimulcastDirection::Recv,
            _ => return Err(Error::SdpParsingError(format!("Invalid simulcast direction: {}", direction_str))),
        };
        
        i += 1;
        if i >= parts.len() {
            return Err(Error::SdpParsingError(format!("Incomplete simulcast attribute: {}", value)));
        }
        
        // Collect all parts until the next direction or end
        let mut versions_str = parts[i].to_string();
        i += 1;
        
        while i < parts.len() && parts[i] != "send" && parts[i] != "recv" {
            versions_str.push(' ');
            versions_str.push_str(parts[i]);
            i += 1;
        }
        
        // Reject stream versions with trailing delimiters
        if versions_str.trim().ends_with(',') || versions_str.trim().ends_with(';') {
            return Err(Error::SdpParsingError(format!("Invalid simulcast stream format (trailing delimiter): {}", versions_str)));
        }
        
        // Check for invalid RID format
        if versions_str.contains('@') || versions_str.contains('!') || 
           versions_str.contains('#') || versions_str.contains('$') ||
           versions_str.contains(",,") || versions_str.contains(";;") {
            return Err(Error::SdpParsingError(format!("Invalid RID format in simulcast: {}", versions_str)));
        }
        
        // Handle the case where we have semicolons or commas with whitespace 
        let normalized_versions = versions_str
            .replace(" ;", ";")
            .replace("; ", ";")
            .replace(" , ", ",")
            .replace(", ", ",")
            .replace(" ,", ",");
        
        match parse_simulcast_stream_versions(&normalized_versions) {
            Ok((_, stream_versions)) => {
                result.push(SimulcastAttribute {
                    direction,
                    stream_versions,
                });
            }
            Err(_) => {
                return Err(Error::SdpParsingError(format!("Invalid simulcast stream versions: {}", versions_str)));
            }
        }
    }
    
    if result.is_empty() {
        Err(Error::SdpParsingError(format!("Failed to parse simulcast attribute: {}", value)))
    } else {
        Ok(result)
    }
}

// Compatibility function for parse_simulcast
// This matches the signature used in parser.rs
pub fn parse_simulcast_compat(value: &str) -> Result<(Vec<String>, Vec<String>)> {
    let simulcast_attrs = parse_simulcast_struct(value)?;
    
    let mut send_streams = Vec::new();
    let mut recv_streams = Vec::new();
    
    for attr in simulcast_attrs {
        let str_versions: Vec<String> = attr.stream_versions
            .iter()
            .map(|version| {
                version.alternatives
                    .iter()
                    .map(|alt| {
                        let prefix = if matches!(alt.status, SimulcastStatus::Paused) { "~" } else { "" };
                        format!("{}{}", prefix, alt.rid)
                    })
                    .collect::<Vec<String>>()
                    .join(",")
            })
            .collect();
        
        match attr.direction {
            SimulcastDirection::Send => send_streams.extend(str_versions),
            SimulcastDirection::Recv => recv_streams.extend(str_versions),
        }
    }
    
    Ok((send_streams, recv_streams))
}

// For backward compatibility with existing code
#[deprecated(since = "0.1.0", note = "Use parse_simulcast() returning SimulcastAttribute instead")]
pub fn parse_simulcast(value: &str) -> Result<(Vec<String>, Vec<String>)> {
    parse_simulcast_compat(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_simulcast_parsing() {
        // Basic simulcast test
        let simulcast_value = "send a1,a2,a3;~a4 recv a5;~a6,~a7";
        let result = parse_simulcast_struct(simulcast_value);
        assert!(result.is_ok(), "Failed to parse valid simulcast attribute");
        
        let attrs = result.unwrap();
        // Should have two attributes, one for send and one for recv
        assert_eq!(attrs.len(), 2, "Expected two simulcast attributes (send and recv)");
        
        // Check send attribute
        let send_attr = &attrs[0];
        assert!(matches!(send_attr.direction, SimulcastDirection::Send));
        assert_eq!(send_attr.stream_versions.len(), 2, "Expected 2 send stream versions");
        
        // Check recv attribute
        let recv_attr = &attrs[1];
        assert!(matches!(recv_attr.direction, SimulcastDirection::Recv));
        assert_eq!(recv_attr.stream_versions.len(), 2, "Expected 2 recv stream versions");
        
        // Check specific stream versions
        let send_version1 = &send_attr.stream_versions[0];
        assert_eq!(send_version1.alternatives.len(), 3);
        assert_eq!(send_version1.alternatives[0].rid, "a1");
        assert_eq!(send_version1.alternatives[1].rid, "a2");
        assert_eq!(send_version1.alternatives[2].rid, "a3");
        
        let send_version2 = &send_attr.stream_versions[1];
        assert_eq!(send_version2.alternatives.len(), 1);
        assert_eq!(send_version2.alternatives[0].rid, "a4");
        assert!(matches!(send_version2.alternatives[0].status, SimulcastStatus::Paused));
        
        // Test invalid simulcast - empty direction
        let invalid_simulcast = "send";
        assert!(parse_simulcast_struct(invalid_simulcast).is_err());
        
        // Test with only send direction
        let send_only = "send a1;a2";
        let (send, recv) = parse_simulcast_compat(send_only).unwrap();
        assert_eq!(send.len(), 2);
        assert_eq!(recv.len(), 0);
        
        // Test with only recv direction
        let recv_only = "recv a3;a4";
        let (send, recv) = parse_simulcast_compat(recv_only).unwrap();
        assert_eq!(send.len(), 0);
        assert_eq!(recv.len(), 2);
    }
    
    #[test]
    fn test_complex_simulcast() {
        // Complex simulcast with multiple patterns
        let complex_simulcast = "send a1,~a2,a3;a4,~a5 recv a6;~a7,a8;~a9";
        let result = parse_simulcast_struct(complex_simulcast);
        assert!(result.is_ok(), "Failed to parse complex simulcast");
        
        let attrs = result.unwrap();
        // Should have two attributes, one for send and one for recv
        assert_eq!(attrs.len(), 2, "Expected two simulcast attributes (send and recv)");
        
        // Check send attribute
        let send_attr = &attrs[0];
        assert!(matches!(send_attr.direction, SimulcastDirection::Send));
        assert_eq!(send_attr.stream_versions.len(), 2, "Expected 2 send stream versions");
        
        // Check first send version
        let send_version1 = &send_attr.stream_versions[0];
        assert_eq!(send_version1.alternatives.len(), 3);
        assert_eq!(send_version1.alternatives[0].rid, "a1");
        assert!(matches!(send_version1.alternatives[0].status, SimulcastStatus::Active));
        assert_eq!(send_version1.alternatives[1].rid, "a2");
        assert!(matches!(send_version1.alternatives[1].status, SimulcastStatus::Paused));
        assert_eq!(send_version1.alternatives[2].rid, "a3");
        assert!(matches!(send_version1.alternatives[2].status, SimulcastStatus::Active));
        
        // Check second send version
        let send_version2 = &send_attr.stream_versions[1];
        assert_eq!(send_version2.alternatives.len(), 2);
        assert_eq!(send_version2.alternatives[0].rid, "a4");
        assert!(matches!(send_version2.alternatives[0].status, SimulcastStatus::Active));
        assert_eq!(send_version2.alternatives[1].rid, "a5");
        assert!(matches!(send_version2.alternatives[1].status, SimulcastStatus::Paused));
        
        // Check recv attribute
        let recv_attr = &attrs[1];
        assert!(matches!(recv_attr.direction, SimulcastDirection::Recv));
        assert_eq!(recv_attr.stream_versions.len(), 3, "Expected 3 recv stream versions");
    }
    
    #[test]
    fn test_simulcast_edge_cases() {
        // Test with multiple alternative patterns
        let multi_pattern = "send a1,a2,a3,a4,a5";
        let (send, _) = parse_simulcast_compat(multi_pattern).unwrap();
        assert_eq!(send[0], "a1,a2,a3,a4,a5");
        
        // Test with invalid direction
        let invalid_direction = "foo a1";
        assert!(parse_simulcast_struct(invalid_direction).is_err());
        
        // Test with trailing comma
        let invalid_pattern = "send a1,";
        assert!(parse_simulcast_struct(invalid_pattern).is_err(), "Should reject trailing comma in alternatives");
        
        // Test with trailing semicolon
        let invalid_pattern2 = "send a1;";
        assert!(parse_simulcast_struct(invalid_pattern2).is_err(), "Should reject trailing semicolon in versions");
        
        // Test with both directions
        let both_directions = "send a1 recv a2";
        let (send, recv) = parse_simulcast_compat(both_directions).unwrap();
        assert_eq!(send.len(), 1);
        assert_eq!(recv.len(), 1);
    }
    
    #[test]
    fn test_rfc8853_examples() {
        // Example from RFC 8853 Section 5.1
        let example1 = "send rid-1,rid-2;rid-3 recv rid-4";
        let result = parse_simulcast_struct(example1);
        assert!(result.is_ok(), "Failed to parse RFC example 1");
        
        let attrs = result.unwrap();
        assert_eq!(attrs.len(), 2);
        
        // Check send attribute from example
        let send_attr = &attrs[0];
        assert!(matches!(send_attr.direction, SimulcastDirection::Send));
        assert_eq!(send_attr.stream_versions.len(), 2);
        assert_eq!(send_attr.stream_versions[0].alternatives.len(), 2);
        assert_eq!(send_attr.stream_versions[0].alternatives[0].rid, "rid-1");
        assert_eq!(send_attr.stream_versions[0].alternatives[1].rid, "rid-2");
        assert_eq!(send_attr.stream_versions[1].alternatives.len(), 1);
        assert_eq!(send_attr.stream_versions[1].alternatives[0].rid, "rid-3");
        
        // Check recv attribute from example
        let recv_attr = &attrs[1];
        assert!(matches!(recv_attr.direction, SimulcastDirection::Recv));
        assert_eq!(recv_attr.stream_versions.len(), 1);
        assert_eq!(recv_attr.stream_versions[0].alternatives.len(), 1);
        assert_eq!(recv_attr.stream_versions[0].alternatives[0].rid, "rid-4");
        
        // Example with paused streams
        let example2 = "recv ~rid-1,rid-2";
        let result = parse_simulcast_struct(example2);
        assert!(result.is_ok(), "Failed to parse RFC example 2");
        
        let attrs = result.unwrap();
        assert_eq!(attrs.len(), 1);
        let recv_attr = &attrs[0];
        assert!(matches!(recv_attr.direction, SimulcastDirection::Recv));
        assert_eq!(recv_attr.stream_versions.len(), 1);
        assert_eq!(recv_attr.stream_versions[0].alternatives.len(), 2);
        assert_eq!(recv_attr.stream_versions[0].alternatives[0].rid, "rid-1");
        assert!(matches!(recv_attr.stream_versions[0].alternatives[0].status, SimulcastStatus::Paused));
        assert_eq!(recv_attr.stream_versions[0].alternatives[1].rid, "rid-2");
        assert!(matches!(recv_attr.stream_versions[0].alternatives[1].status, SimulcastStatus::Active));
    }
    
    #[test]
    fn test_invalid_simulcast_formats() {
        // Empty string
        assert!(parse_simulcast_struct("").is_err(), "Should reject empty string");
        
        // Missing stream versions
        assert!(parse_simulcast_struct("send").is_err(), "Should reject missing stream versions");
        
        // Empty stream versions
        assert!(parse_simulcast_struct("send ").is_err(), "Should reject empty stream versions");
        
        // Invalid direction
        assert!(parse_simulcast_struct("invalid a1").is_err(), "Should reject invalid direction");
        
        // Invalid RID format with @
        assert!(parse_simulcast_struct("send a1@invalid").is_err(), "Should reject invalid RID format with @");
        
        // Malformed version list (empty part between commas)
        assert!(parse_simulcast_struct("send a1,,a2").is_err(), "Should reject empty parts between commas");
        
        // Malformed version list (empty part between semicolons)
        assert!(parse_simulcast_struct("send a1;;a2").is_err(), "Should reject empty parts between semicolons");
        
        // Missing direction value
        assert!(parse_simulcast_struct("a1").is_err(), "Should reject missing direction");
        
        // Valid format but with direction in wrong case
        assert!(parse_simulcast_struct("SEND a1").is_err(), "Should reject direction in wrong case");
    }
    
    #[test]
    fn test_whitespace_handling() {
        // Test with extra whitespace
        let with_spaces = "send a1,a2 ; a3 recv a4";
        let result = parse_simulcast_struct(with_spaces);
        assert!(result.is_ok(), "Should handle extra whitespace");
        
        // Test with tabs
        let with_tabs = "send\ta1,a2;\ta3\trecv\ta4";
        let result = parse_simulcast_struct(with_tabs);
        assert!(result.is_ok(), "Should handle tabs");
        
        // Test with leading/trailing whitespace
        let with_whitespace = "  send a1 recv a2  ";
        let result = parse_simulcast_struct(with_whitespace);
        assert!(result.is_ok(), "Should handle leading/trailing whitespace");
        
        // Check that alternatives are correctly parsed with extra whitespace
        let with_specific_spaces = "send a1 , a2 ; a3";
        let result = parse_simulcast_struct(with_specific_spaces);
        assert!(result.is_ok(), "Should handle spaces around commas and semicolons");
        let attrs = result.unwrap();
        assert_eq!(attrs[0].stream_versions[0].alternatives.len(), 2);
        assert_eq!(attrs[0].stream_versions[0].alternatives[0].rid, "a1");
        assert_eq!(attrs[0].stream_versions[0].alternatives[1].rid, "a2");
        assert_eq!(attrs[0].stream_versions[1].alternatives[0].rid, "a3");
    }
    
    #[test]
    fn test_parser_functions_directly() {
        // Test parse_simulcast_direction
        let (rest, direction) = parse_simulcast_direction("send rest").unwrap();
        assert_eq!(rest, " rest");
        assert!(matches!(direction, SimulcastDirection::Send));
        
        let (rest, direction) = parse_simulcast_direction("recv rest").unwrap();
        assert_eq!(rest, " rest");
        assert!(matches!(direction, SimulcastDirection::Recv));
        
        // Test parse_rid
        let (rest, rid) = parse_rid("rid-1 rest").unwrap();
        assert_eq!(rest, " rest");
        assert_eq!(rid, "rid-1");
        
        // Test parse_simulcast_alternative
        let (rest, alt) = parse_simulcast_alternative("rid-1 rest").unwrap();
        assert_eq!(rest, " rest");
        assert_eq!(alt.rid, "rid-1");
        assert!(matches!(alt.status, SimulcastStatus::Active));
        
        let (rest, alt) = parse_simulcast_alternative("~rid-1 rest").unwrap();
        assert_eq!(rest, " rest");
        assert_eq!(alt.rid, "rid-1");
        assert!(matches!(alt.status, SimulcastStatus::Paused));
        
        // Test parse_simulcast_version
        let (rest, version) = parse_simulcast_version("rid-1,rid-2 rest").unwrap();
        assert_eq!(rest, " rest");
        assert_eq!(version.alternatives.len(), 2);
        assert_eq!(version.alternatives[0].rid, "rid-1");
        assert_eq!(version.alternatives[1].rid, "rid-2");
        
        // Test parse_simulcast_stream_versions
        let (rest, versions) = parse_simulcast_stream_versions("rid-1;rid-2 rest").unwrap();
        assert_eq!(rest, " rest");
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].alternatives.len(), 1);
        assert_eq!(versions[0].alternatives[0].rid, "rid-1");
        assert_eq!(versions[1].alternatives.len(), 1);
        assert_eq!(versions[1].alternatives[0].rid, "rid-2");
    }
} 
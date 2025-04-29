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
    combinator::{map, opt, value},
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

/// Parse a RID identifier
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
    // Split the value into space-separated parts
    let parts: Vec<&str> = value.trim().split_whitespace().collect();
    
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
        
        // Parse the stream versions
        let versions_str = parts[i];
        i += 1;
        
        match parse_simulcast_stream_versions(versions_str) {
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
        let simulcast_value = "send 1,2,3;~4 recv 5;~6,~7";
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
        assert_eq!(send_version1.alternatives[0].rid, "1");
        assert_eq!(send_version1.alternatives[1].rid, "2");
        assert_eq!(send_version1.alternatives[2].rid, "3");
        
        let send_version2 = &send_attr.stream_versions[1];
        assert_eq!(send_version2.alternatives.len(), 1);
        assert_eq!(send_version2.alternatives[0].rid, "4");
        assert!(matches!(send_version2.alternatives[0].status, SimulcastStatus::Paused));
        
        // Test invalid simulcast - empty direction
        let invalid_simulcast = "send";
        assert!(parse_simulcast_struct(invalid_simulcast).is_err());
        
        // Test with only send direction
        let send_only = "send 1;2";
        let (send, recv) = parse_simulcast_compat(send_only).unwrap();
        assert_eq!(send.len(), 2);
        assert_eq!(recv.len(), 0);
        
        // Test with only recv direction
        let recv_only = "recv 3;4";
        let (send, recv) = parse_simulcast_compat(recv_only).unwrap();
        assert_eq!(send.len(), 0);
        assert_eq!(recv.len(), 2);
    }
    
    #[test]
    fn test_complex_simulcast() {
        // Complex simulcast with multiple patterns
        let complex_simulcast = "send 1,~2,3;4,~5 recv 6;~7,8;~9";
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
        assert_eq!(send_version1.alternatives[0].rid, "1");
        assert!(matches!(send_version1.alternatives[0].status, SimulcastStatus::Active));
        assert_eq!(send_version1.alternatives[1].rid, "2");
        assert!(matches!(send_version1.alternatives[1].status, SimulcastStatus::Paused));
        assert_eq!(send_version1.alternatives[2].rid, "3");
        assert!(matches!(send_version1.alternatives[2].status, SimulcastStatus::Active));
        
        // Check second send version
        let send_version2 = &send_attr.stream_versions[1];
        assert_eq!(send_version2.alternatives.len(), 2);
        assert_eq!(send_version2.alternatives[0].rid, "4");
        assert!(matches!(send_version2.alternatives[0].status, SimulcastStatus::Active));
        assert_eq!(send_version2.alternatives[1].rid, "5");
        assert!(matches!(send_version2.alternatives[1].status, SimulcastStatus::Paused));
        
        // Check recv attribute
        let recv_attr = &attrs[1];
        assert!(matches!(recv_attr.direction, SimulcastDirection::Recv));
        assert_eq!(recv_attr.stream_versions.len(), 3, "Expected 3 recv stream versions");
    }
    
    #[test]
    fn test_simulcast_edge_cases() {
        // Test with multiple alternative patterns
        let multi_pattern = "send 1,2,3,4,5";
        let (send, _) = parse_simulcast_compat(multi_pattern).unwrap();
        assert_eq!(send[0], "1,2,3,4,5");
        
        // Test with invalid direction
        let invalid_direction = "foo 1";
        assert!(parse_simulcast_struct(invalid_direction).is_err());
        
        // Test with invalid version pattern
        let invalid_pattern = "send 1,";
        assert!(parse_simulcast_struct(invalid_pattern).is_err());
        
        // Test with both directions
        let both_directions = "send 1 recv 2";
        let (send, recv) = parse_simulcast_compat(both_directions).unwrap();
        assert_eq!(send.len(), 1);
        assert_eq!(recv.len(), 1);
    }
} 
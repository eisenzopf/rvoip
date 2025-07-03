//! SDP Media Direction Attributes
//!
//! Implements parsers for media direction attributes (sendrecv, sendonly, recvonly, inactive)
//! as defined in RFC 8866.

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{to_result};
use nom::{
    branch::alt,
    bytes::complete::tag,
    combinator::map,
    IResult,
};
use serde::{Deserialize, Serialize};
use std::fmt;

/// SDP Media Direction attribute (e.g., sendrecv, sendonly)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaDirection {
    /// Endpoint can send and receive media
    SendRecv,
    /// Endpoint can only send media
    SendOnly,
    /// Endpoint can only receive media
    RecvOnly,
    /// Endpoint neither sends nor receives media
    Inactive,
}

// Display implementation for MediaDirection
impl fmt::Display for MediaDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaDirection::SendRecv => write!(f, "sendrecv"),
            MediaDirection::SendOnly => write!(f, "sendonly"),
            MediaDirection::RecvOnly => write!(f, "recvonly"),
            MediaDirection::Inactive => write!(f, "inactive"),
        }
    }
}

/// Parser for direction attribute values (empty values for flag attributes)
fn direction_parser(input: &str) -> IResult<&str, MediaDirection> {
    alt((
        map(tag("sendrecv"), |_| MediaDirection::SendRecv),
        map(tag("sendonly"), |_| MediaDirection::SendOnly),
        map(tag("recvonly"), |_| MediaDirection::RecvOnly),
        map(tag("inactive"), |_| MediaDirection::Inactive),
    ))(input)
}

/// Parses direction attributes (sendrecv, sendonly, recvonly, inactive)
pub fn parse_direction(value: &str) -> Result<MediaDirection> {
    to_result(
        direction_parser(value.trim()),
        &format!("Invalid direction attribute: {}", value)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_direction_attribute_comprehensive() {
        // Valid cases
        assert_eq!(parse_direction("sendrecv").unwrap(), MediaDirection::SendRecv);
        assert_eq!(parse_direction("sendonly").unwrap(), MediaDirection::SendOnly);
        assert_eq!(parse_direction("recvonly").unwrap(), MediaDirection::RecvOnly);
        assert_eq!(parse_direction("inactive").unwrap(), MediaDirection::Inactive);
        
        // Edge cases
        
        // Whitespace handling
        assert_eq!(parse_direction(" sendrecv ").unwrap(), MediaDirection::SendRecv);
        
        // Error cases
        
        // Invalid direction
        assert!(parse_direction("send").is_err());
        assert!(parse_direction("recv").is_err());
        assert!(parse_direction("sendrec").is_err());
        assert!(parse_direction("SENDRECV").is_err()); // Case sensitive
        assert!(parse_direction("").is_err());
    }
    
    #[test]
    fn test_media_direction_display() {
        assert_eq!(MediaDirection::SendRecv.to_string(), "sendrecv");
        assert_eq!(MediaDirection::SendOnly.to_string(), "sendonly");
        assert_eq!(MediaDirection::RecvOnly.to_string(), "recvonly");
        assert_eq!(MediaDirection::Inactive.to_string(), "inactive");
    }
    
    #[test]
    fn test_direction_parser_function() {
        // Test the direction_parser function directly
        let result = direction_parser("sendrecv");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().1, MediaDirection::SendRecv);
        
        let result = direction_parser("sendonly");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().1, MediaDirection::SendOnly);
        
        let result = direction_parser("recvonly");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().1, MediaDirection::RecvOnly);
        
        let result = direction_parser("inactive");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().1, MediaDirection::Inactive);
        
        // Invalid input
        let result = direction_parser("invalid");
        assert!(result.is_err());
    }
} 
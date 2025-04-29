// SDP session parsing utilities
//
// Common utilities and helper functions for session-level parsing.

use crate::types::sdp::{SdpSession, Origin, ParsedAttribute};
use nom::{
    IResult,
    bytes::complete::take_while1,
};

/// Parse a token (alphanumeric and certain safe symbols) using nom
pub fn parse_token(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| {
        c.is_ascii_alphanumeric() || 
        matches!(c, '!' | '#' | '$' | '%' | '&' | '\'' | '*' | '+' | '-' | '.' |
                     '^' | '_' | '`' | '{' | '|' | '}' | '~')
    })(input)
}

/// Initialize a new session description with mandatory fields
pub fn init_session_description() -> SdpSession {
    SdpSession {
        version: "0".to_string(),
        origin: Origin {
            username: "-".to_string(),
            sess_id: "0".to_string(),
            sess_version: "0".to_string(),
            net_type: "IN".to_string(),
            addr_type: "IP4".to_string(),
            unicast_address: "127.0.0.1".to_string(),
        },
        session_name: "-".to_string(),
        session_info: None,
        uri: None,
        email: None,
        phone: None,
        connection_info: None,
        time_descriptions: Vec::new(),
        media_descriptions: Vec::new(),
        direction: None,
        generic_attributes: Vec::new(),
    }
}

/// Checks if the given attribute is a session-level attribute
pub fn is_session_level_attribute(attribute: &str) -> bool {
    matches!(attribute,
        "ice-lite" | "ice-options" | "ice-pwd" | "ice-ufrag" | "fingerprint" | "setup" |
        "identity" | "group" | "msid-semantic" | "extmap-allow-mixed"
    )
} 
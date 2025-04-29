use crate::error::{Error, Result};
use nom::{
    bytes::complete::{tag, take_till1, take_while1},
    character::complete::{char, digit1, space0, space1},
    combinator::{map, map_res, opt},
    multi::separated_list1,
    sequence::{pair, tuple},
    IResult,
};
// Use the enum defined in types::sdp
// use crate::types::sdp::{RtpMapAttribute, FmtpAttribute, ParsedAttribute, CandidateAttribute, MediaDirection, SsrcAttribute};
// Import only the types needed from types::sdp, NOT MediaDirection
use crate::types::sdp::{RtpMapAttribute, FmtpAttribute, ParsedAttribute, CandidateAttribute, SsrcAttribute};
use serde::{Deserialize, Serialize};
use std::fmt; // Import fmt
use std::net::IpAddr;
use crate::parser::uri::{ipv4, ipv6, hostname}; // Import URI parsers

// --- Placeholder Attribute Structs (Consider moving to types/sdp_attributes.rs later) ---

// Remove these struct definitions as they should be defined in types/sdp.rs
/*
#[derive(Debug, Clone, PartialEq)]
pub struct RtpMapAttribute {
    pub payload_type: u8,
    pub encoding_name: String,
    pub clock_rate: u32,
    pub encoding_params: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FmtpAttribute {
    pub format: String,
    pub parameters: String, // Keep as raw string for now
}
*/

/// SDP Media Direction attribute (e.g., sendrecv, sendonly)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaDirection {
    SendRecv,
    SendOnly,
    RecvOnly,
    Inactive,
}

// Add Display implementation for MediaDirection
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

// Add MediaDirection to ParsedAttribute enum
// #[derive(Debug, Clone, PartialEq)]
// pub enum ParsedAttribute {
//    ... 
//    Direction(MediaDirection),
//    Ptime(u32),
//    ... 
//}

// Validation helper functions - similar to those in parser.rs but need to be accessible here too
/// Helper function to validate IPv4 addresses
fn is_valid_ipv4(addr: &str) -> bool {
    let parts: Vec<&str> = addr.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    
    parts.iter().all(|part| {
        if let Ok(num) = part.parse::<u8>() {
            true
        } else {
            false
        }
    })
}

/// Helper function to validate IPv6 addresses
fn is_valid_ipv6(addr: &str) -> bool {
    // Simplified IPv6 validation - just check for basic format
    addr.contains(':') && addr.split(':').count() <= 8
}

/// Helper function to validate hostnames
fn is_valid_hostname(hostname: &str) -> bool {
    // Simplified hostname validation
    // A hostname should contain only alphanumeric characters, hyphens, and dots
    // and should not start or end with a hyphen or dot
    if hostname.is_empty() || hostname.starts_with('.') || hostname.ends_with('.') ||
       hostname.starts_with('-') || hostname.ends_with('-') {
        return false;
    }
    
    hostname.chars().all(|c| {
        c.is_alphanumeric() || c == '-' || c == '.'
    })
}

/// Helper function to validate token format (per RFC 4566 ABNF)
fn is_valid_token(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| 
        c.is_ascii_alphanumeric() || 
        c == '-' || c == '.' || c == '!' || 
        c == '%' || c == '*' || c == '_' || 
        c == '+' || c == '`' || c == '\'' || 
        c == '~'
    )
}

// --- Parsing Functions --- 

/// Parses rtpmap attribute: a=rtpmap:<payload type> <encoding name>/<clock rate>[/<encoding parameters>]
pub fn parse_rtpmap(value: &str) -> Result<ParsedAttribute> {
    // Example: 96 H264/90000
    // Example: 0 PCMU/8000
    // Example: 8 PCMA/8000/1
    let parts: Vec<&str> = value.splitn(2, ' ').collect();
    if parts.len() != 2 {
        return Err(Error::SdpParsingError(format!("Invalid rtpmap format: {}", value)));
    }
    
    // Validate payload type: must be between 0-127 per RFC
    let payload_type = parts[0].parse::<u8>()
        .map_err(|_| Error::SdpParsingError(format!("Invalid payload type in rtpmap: {}", parts[0])))?;
    
    // Validate encoding format: <encoding name>/<clock rate>[/<encoding parameters>]
    let encoding_parts: Vec<&str> = parts[1].splitn(3, '/').collect();
    if encoding_parts.len() < 2 {
         return Err(Error::SdpParsingError(format!("Invalid encoding format in rtpmap: {}", parts[1])));
    }

    // Encoding name validation per RFC: should be token format (alpha-numeric + -)
    let encoding_name = encoding_parts[0].to_string();
    if !is_valid_token(&encoding_name) {
        return Err(Error::SdpParsingError(format!("Invalid encoding name in rtpmap (should be alpha-numeric): {}", encoding_name)));
    }
    
    // Clock rate validation: must be numeric
    let clock_rate = encoding_parts[1].parse::<u32>()
         .map_err(|_| Error::SdpParsingError(format!("Invalid clock rate in rtpmap: {}", encoding_parts[1])))?;
    
    // Optional encoding parameters (e.g., channels)
    let encoding_params = encoding_parts.get(2).map(|s| {
        let param = s.to_string();
        // Validate that parameter is numeric (for audio this is channels)
        if !param.chars().all(|c| c.is_ascii_digit()) {
            return Err(Error::SdpParsingError(format!("Invalid encoding parameters in rtpmap (should be numeric): {}", param)));
        }
        Ok(param)
    }).transpose()?;

    Ok(ParsedAttribute::RtpMap(RtpMapAttribute {
        payload_type,
        encoding_name,
        clock_rate,
        encoding_params,
    }))
}

/// Parses fmtp attribute: a=fmtp:<format> <format specific parameters>
pub fn parse_fmtp(value: &str) -> Result<ParsedAttribute> {
    // Example: 97 profile-level-id=42e01f;level-asymmetry-allowed=1;packetization-mode=1
    let parts: Vec<&str> = value.splitn(2, ' ').collect();
    if parts.len() != 2 {
        return Err(Error::SdpParsingError(format!("Invalid fmtp format: {}", value)));
    }

    // Validate format identifier (typically a payload type number)
    let format = parts[0].to_string();
    if !format.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::SdpParsingError(format!("Invalid format in fmtp (should be numeric): {}", format)));
    }
    
    // Validate parameters - general structure is key=value;key=value or key;key=value
    // In practice, we just ensure it's not empty
    let parameters = parts[1].to_string();
    if parameters.trim().is_empty() {
        return Err(Error::SdpParsingError("Empty format parameters in fmtp".to_string()));
    }

    Ok(ParsedAttribute::Fmtp(FmtpAttribute {
        format,
        parameters,
    }))
}

/// Parses ptime attribute: a=ptime:<packet time>
pub fn parse_ptime(value: &str) -> Result<u32> { // Return specific type
    value.trim().parse::<u32>()
         .map_err(|_| Error::SdpParsingError(format!("Invalid ptime value: {}", value)))
}

/// Parses direction attributes (sendrecv, sendonly, recvonly, inactive)
pub fn parse_direction(value: &str) -> Result<MediaDirection> { // Return specific type
    match value.trim() {
        "sendrecv" => Ok(MediaDirection::SendRecv),
        "sendonly" => Ok(MediaDirection::SendOnly),
        "recvonly" => Ok(MediaDirection::RecvOnly),
        "inactive" => Ok(MediaDirection::Inactive),
        _ => Err(Error::SdpParsingError(format!("Invalid direction attribute: {}", value)))
    }
}

/// Parses candidate attribute: a=candidate:<foundation> <component-id> <transport> <priority> <conn-addr> <port> typ <cand-type> [raddr <raddr>] [rport <rport>] *(extensions)
/// Based on RFC 8839 syntax
pub fn parse_candidate(value: &str) -> Result<ParsedAttribute> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    
    if parts.len() < 8 || parts[6] != "typ" {
        return Err(Error::SdpParsingError(format!("Invalid candidate format: not enough parts or missing 'typ' keyword: {}", value)));
    }
    
    let foundation = parts[0].to_string();
    let component_id = parts[1].parse::<u32>()
        .map_err(|_| Error::SdpParsingError(format!("Invalid component-id in candidate: {}", parts[1])))?;
    let transport = parts[2].to_string();
    let priority = parts[3].parse::<u32>()
        .map_err(|_| Error::SdpParsingError(format!("Invalid priority in candidate: {}", parts[3])))?;
    let connection_address = parts[4].to_string();
    
    // Validate connection address using helper functions
    let is_ipv4 = is_valid_ipv4(&connection_address);
    let is_ipv6 = !is_ipv4 && is_valid_ipv6(&connection_address);
    let is_hostname = !is_ipv4 && !is_ipv6 && is_valid_hostname(&connection_address);
    
    if !is_ipv4 && !is_ipv6 && !is_hostname {
        return Err(Error::SdpParsingError(format!("Invalid connection address in candidate: {}", connection_address)));
    }
    
    let port = parts[5].parse::<u16>()
        .map_err(|_| Error::SdpParsingError(format!("Invalid port in candidate: {}", parts[5])))?;
    // parts[6] is "typ"
    let candidate_type = parts[7].to_string();
    
    // Validate candidate type
    if !["host", "srflx", "prflx", "relay"].contains(&candidate_type.as_str()) {
        return Err(Error::SdpParsingError(format!("Invalid candidate type: {}", candidate_type)));
    }
    
    let mut current_index = 8;
    let mut related_address: Option<String> = None;
    let mut related_port: Option<u16> = None;
    let mut extensions: Vec<(String, Option<String>)> = Vec::new();

    while current_index < parts.len() {
        let key = parts[current_index];
        current_index += 1;
        
        match key {
            "raddr" => {
                if current_index < parts.len() {
                    let raddr = parts[current_index].to_string();
                    
                    // Validate raddr using helper functions
                    let is_ipv4 = is_valid_ipv4(&raddr);
                    let is_ipv6 = !is_ipv4 && is_valid_ipv6(&raddr);
                    let is_hostname = !is_ipv4 && !is_ipv6 && is_valid_hostname(&raddr);
                    
                    if !is_ipv4 && !is_ipv6 && !is_hostname {
                        return Err(Error::SdpParsingError(format!("Invalid related address (raddr) in candidate: {}", raddr)));
                    }
                    
                    related_address = Some(raddr);
                    current_index += 1;
                } else {
                    return Err(Error::SdpParsingError("Missing value for raddr in candidate".to_string()));
                }
            }
            "rport" => {
                if current_index < parts.len() {
                    related_port = parts[current_index].parse::<u16>().ok();
                    if related_port.is_none() {
                        return Err(Error::SdpParsingError(format!("Invalid value for rport in candidate: {}", parts[current_index])));
                    }
                    current_index += 1;
                } else {
                    return Err(Error::SdpParsingError("Missing value for rport in candidate".to_string()));
                }
            }
            // Handle other potential extensions (key-value or key-only)
            _ => {
                // Check if the next part exists and isn't another keyword
                if current_index < parts.len() && !["raddr", "rport", "typ", "tcptype", "generation", "network-id", "network-cost"].contains(&parts[current_index]) {
                    extensions.push((key.to_string(), Some(parts[current_index].to_string())));
                    current_index += 1;
                } else {
                    // Treat as a flag extension
                    extensions.push((key.to_string(), None));
                }
            }
        }
    }

    Ok(ParsedAttribute::Candidate(CandidateAttribute {
        foundation,
        component_id,
        transport,
        priority,
        connection_address,
        port,
        candidate_type,
        related_address,
        related_port,
        extensions,
    }))
}

/// Parses ssrc attribute: a=ssrc:<ssrc-id> <attribute>[:<value>]
pub fn parse_ssrc(value: &str) -> Result<ParsedAttribute> {
    // Example: 123456789 cname:user@example.com
    // Example: 987654321 msid:stream1 track1 (value can contain spaces)
    // Example: 111 mslabel:label1 
    
    let parts: Vec<&str> = value.splitn(2, ' ').collect();
    if parts.len() != 2 {
        return Err(Error::SdpParsingError(format!("Invalid ssrc format (missing space after ssrc-id): {}", value)));
    }
    
    let ssrc_id = parts[0].trim().parse::<u32>()
        .map_err(|_| Error::SdpParsingError(format!("Invalid SSRC ID in ssrc attribute: {}", parts[0])))?;
        
    let attr_part = parts[1].trim(); // The rest is the attribute + optional value
    let (attribute, ssrc_value) = match attr_part.split_once(':') {
        Some((attr, val)) => (attr.trim().to_string(), Some(val.trim().to_string())), 
        None => (attr_part.to_string(), None), // Treat as attribute name only if no colon
    };
    
    // Basic validation: attribute name shouldn't be empty
    if attribute.is_empty() {
         return Err(Error::SdpParsingError(format!("Missing attribute name in ssrc: {}", value)));
    }

    Ok(ParsedAttribute::Ssrc(SsrcAttribute {
        ssrc_id,
        attribute,
        value: ssrc_value,
    }))
}

// Add more attribute parsers as needed (e.g., candidate, ssrc, etc.) 
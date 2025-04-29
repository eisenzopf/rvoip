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
use crate::parser::token::is_token_char; // Import token parser

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

// Validation helper functions - using the existing parsers instead of custom implementations
/// Helper function to validate IPv4 addresses using the parser module
fn is_valid_ipv4(addr: &str) -> bool {
    // Basic format check: must have 4 parts separated by dots
    let parts: Vec<&str> = addr.split('.').collect();
    if parts.len() != 4 {
        return false;
    }

    // Each part must be a valid octet (0-255)
    for part in parts {
        match part.parse::<u8>() {
            Ok(_) => {}, // Valid octet (0-255)
            Err(_) => return false, // Outside 0-255 range or not a number
        }
    }
    
    // If we reach here, all octets are valid
    true
}

/// Helper function to validate IPv6 addresses using the parser module
fn is_valid_ipv6(addr: &str) -> bool {
    // If the address doesn't have brackets, add them for the parser
    let input = if addr.starts_with('[') {
        addr.as_bytes().to_vec()
    } else {
        let mut with_brackets = Vec::with_capacity(addr.len() + 2);
        with_brackets.push(b'[');
        with_brackets.extend_from_slice(addr.as_bytes());
        with_brackets.push(b']');
        with_brackets
    };
    
    match ipv6::ipv6_reference(&input) {
        Ok((remaining, _)) => remaining.is_empty(), // Must consume all input
        Err(_) => false,
    }
}

/// Helper function to validate hostnames using the parser module
fn is_valid_hostname(hostname_str: &str) -> bool {
    // Use the hostname parser from hostname.rs
    let input = hostname_str.as_bytes();
    match hostname::hostname(input) {
        Ok((remaining, _)) => remaining.is_empty() || remaining == b".", // Must consume all input (allow trailing dot)
        Err(_) => false,
    }
}

/// Helper function to validate token format using the parser module
pub fn is_valid_token(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| {
        let byte = c as u8;
        is_token_char(byte)
    })
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
    
    // First check if there's a space in the value at all
    if !value.contains(' ') {
        return Err(Error::SdpParsingError(format!("Invalid fmtp format (missing space): {}", value)));
    }
    
    let parts: Vec<&str> = value.splitn(2, ' ').collect();
    if parts.len() != 2 {
        return Err(Error::SdpParsingError(format!("Invalid fmtp format: {}", value)));
    }

    // Format identifier can be numeric or a token (like "red")
    let format = parts[0].to_string();
    if format.is_empty() {
        return Err(Error::SdpParsingError("Empty format in fmtp".to_string()));
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

/// Parses maxptime attribute: a=maxptime:<maximum packet time>
pub fn parse_maxptime(value: &str) -> Result<u32> {
    let maxptime = value.trim().parse::<u32>()
        .map_err(|_| Error::SdpParsingError(format!("Invalid maxptime value: {}", value)))?;
    
    // Typically maxptime should be reasonable (not too small, not too large)
    if maxptime < 10 || maxptime > 5000 {
        return Err(Error::SdpParsingError(format!("Unreasonable maxptime value: {}", maxptime)));
    }
    
    Ok(maxptime)
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
    // First check if it looks like an IPv4 address (4 parts separated by dots)
    if connection_address.split('.').count() == 4 {
        // Validate each octet (must be 0-255)
        let octets: Vec<&str> = connection_address.split('.').collect();
        for octet in octets {
            if let Err(_) = octet.parse::<u8>() {
                return Err(Error::SdpParsingError(
                    format!("Invalid IPv4 address in candidate: {}", connection_address)
                ));
            }
        }
    } else if connection_address.contains(':') {
        // It's an IPv6 address - use the helper function
        if !is_valid_ipv6(&connection_address) {
            return Err(Error::SdpParsingError(
                format!("Invalid IPv6 address in candidate: {}", connection_address)
            ));
        }
    } else {
        // It's a hostname - do additional validation
        
        // First check for invalid hostname characters that aren't caught by is_valid_hostname
        if connection_address.contains('@') || 
           connection_address.contains('_') || 
           connection_address.contains(' ') ||
           connection_address.contains(':') {
            return Err(Error::SdpParsingError(
                format!("Invalid hostname in candidate (contains invalid characters): {}", connection_address)
            ));
        }
        
        // Then use the helper function
        if !is_valid_hostname(&connection_address) {
            return Err(Error::SdpParsingError(
                format!("Invalid hostname in candidate: {}", connection_address)
            ));
        }
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
                    
                    // Validate raddr - could be IP or hostname
                    if raddr.split('.').count() == 4 {
                        // Looks like IPv4 - validate octets
                        let octets: Vec<&str> = raddr.split('.').collect();
                        for octet in octets {
                            if let Err(_) = octet.parse::<u8>() {
                                return Err(Error::SdpParsingError(
                                    format!("Invalid IPv4 address in raddr: {}", raddr)
                                ));
                            }
                        }
                    } else if raddr.contains(':') {
                        // Looks like IPv6
                        if !is_valid_ipv6(&raddr) {
                            return Err(Error::SdpParsingError(
                                format!("Invalid IPv6 address in raddr: {}", raddr)
                            ));
                        }
                    } else {
                        // Must be a hostname - check for invalid characters
                        if raddr.contains('@') || 
                           raddr.contains('_') || 
                           raddr.contains(' ') ||
                           raddr.contains(':') {
                            return Err(Error::SdpParsingError(
                                format!("Invalid hostname in raddr (contains invalid characters): {}", raddr)
                            ));
                        }
                        
                        // Then use the helper function
                        if !is_valid_hostname(&raddr) {
                            return Err(Error::SdpParsingError(
                                format!("Invalid hostname in raddr: {}", raddr)
                            ));
                        }
                    }
                    
                    related_address = Some(raddr);
                    current_index += 1;
                    
                    // Check if we have rport following this - it's required when raddr is present
                    // We need at least 2 more parts: "rport" and its value
                    if current_index + 1 >= parts.len() || parts[current_index] != "rport" {
                        return Err(Error::SdpParsingError("When raddr is present, rport is required".to_string()));
                    }
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

/// Parses ice-ufrag attribute: a=ice-ufrag:<ufrag>
pub fn parse_ice_ufrag(value: &str) -> Result<String> {
    let ufrag = value.trim();
    
    // Validate the ICE username fragment (ufrag)
    // Per RFC 8839, ufrag must be at least 4 characters and at most 256
    if ufrag.len() < 4 || ufrag.len() > 256 {
        return Err(Error::SdpParsingError(format!("Invalid ice-ufrag length (must be 4-256 chars): {}", ufrag)));
    }
    
    // ICE ufrag should only contain printable ASCII characters
    if !ufrag.chars().all(|c| c.is_ascii() && !c.is_ascii_control()) {
        return Err(Error::SdpParsingError(format!("Invalid ice-ufrag (contains non-printable chars): {}", ufrag)));
    }
    
    Ok(ufrag.to_string())
}

/// Parses ice-pwd attribute: a=ice-pwd:<pwd>
pub fn parse_ice_pwd(value: &str) -> Result<String> {
    let pwd = value.trim();
    
    // Validate the ICE password
    // Per RFC 8839, pwd must be at least 22 characters and at most 256
    if pwd.len() < 22 || pwd.len() > 256 {
        return Err(Error::SdpParsingError(format!("Invalid ice-pwd length (must be 22-256 chars): {}", pwd)));
    }
    
    // ICE pwd should only contain printable ASCII characters
    if !pwd.chars().all(|c| c.is_ascii() && !c.is_ascii_control()) {
        return Err(Error::SdpParsingError(format!("Invalid ice-pwd (contains non-printable chars): {}", pwd)));
    }
    
    Ok(pwd.to_string())
}

/// Parses fingerprint attribute: a=fingerprint:<hash-function> <fingerprint>
pub fn parse_fingerprint(value: &str) -> Result<(String, String)> {
    // Example: sha-256 D1:2C:74:A7:E3:B5:11:04:87:0D:D7:3F:B8:BF:79:7D:CF:76:B3:97:B6:5F:A5:3D:EC:D8:79:49:5C:92:26:E9
    let parts: Vec<&str> = value.splitn(2, ' ').collect();
    if parts.len() != 2 {
        return Err(Error::SdpParsingError(format!("Invalid fingerprint format: {}", value)));
    }
    
    let hash_function = parts[0].trim().to_lowercase();
    let fingerprint = parts[1].trim();
    
    // Validate the hash function
    if !["sha-1", "sha-256", "sha-384", "sha-512", "md5"].contains(&hash_function.as_str()) {
        return Err(Error::SdpParsingError(format!("Unsupported hash function: {}", hash_function)));
    }
    
    // Validate the fingerprint format (colon-separated hex values)
    let fingerprint_parts: Vec<&str> = fingerprint.split(':').collect();
    if fingerprint_parts.is_empty() {
        return Err(Error::SdpParsingError("Empty fingerprint value".to_string()));
    }
    
    // Each segment should be a valid hex value
    for part in &fingerprint_parts {
        if part.is_empty() || part.len() > 2 || !part.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(Error::SdpParsingError(format!("Invalid fingerprint hex value: {}", part)));
        }
    }
    
    Ok((hash_function, fingerprint.to_string()))
}

/// Parses setup attribute: a=setup:<role>
pub fn parse_setup(value: &str) -> Result<String> {
    // Values as per RFC 4145, used for DTLS (RFC 5763)
    match value.trim() {
        "active" | "passive" | "actpass" | "holdconn" => Ok(value.trim().to_string()),
        _ => Err(Error::SdpParsingError(format!("Invalid setup value: {}", value)))
    }
}

/// Parses mid attribute: a=mid:<identification-tag>
pub fn parse_mid(value: &str) -> Result<String> {
    let mid = value.trim();
    
    // Basic validation: mid should not be empty
    if mid.is_empty() {
        return Err(Error::SdpParsingError("Empty mid value".to_string()));
    }
    
    // Per RFC 5888, the identification-tag is a token which means
    // it should consist of ASCII alphanumeric, '-', '.', '!', '%', '*', '_', '+', '`', '\'', '~'
    if !is_valid_token(mid) {
        return Err(Error::SdpParsingError(format!("Invalid mid value (not a valid token): {}", mid)));
    }
    
    Ok(mid.to_string())
}

/// Parses group attribute: a=group:<semantics> <identification-tag> ...
pub fn parse_group(value: &str) -> Result<(String, Vec<String>)> {
    // Example: BUNDLE audio video
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.is_empty() {
        return Err(Error::SdpParsingError("Empty group attribute".to_string()));
    }
    
    let semantics = parts[0].to_string();
    let mut mids = Vec::new();
    
    // Collect all identification tags (mids)
    for part in parts.iter().skip(1) {
        let mid = part.trim();
        if !mid.is_empty() {
            // Each mid should be a valid token
            if !is_valid_token(mid) {
                return Err(Error::SdpParsingError(format!("Invalid mid in group: {}", mid)));
            }
            mids.push(mid.to_string());
        }
    }
    
    // Validate semantics (common values as per RFC 5888, 7104, etc.)
    match semantics.as_str() {
        "BUNDLE" | "LS" | "FID" | "SRF" | "ANAT" => {},
        _ => {
            // Unknown semantics - we'll accept it but log a warning
            // This is not an error as new semantics might be defined in the future
            // println!("Warning: Unknown group semantics: {}", semantics);
        }
    }
    
    Ok((semantics, mids))
}

/// Parses rtcp-mux attribute: a=rtcp-mux
/// This is a flag attribute with no value
pub fn parse_rtcp_mux(_value: &str) -> Result<bool> {
    // rtcp-mux is a flag attribute with no value
    // We could validate that the value is empty, but some implementations
    // might include extra data, so we'll be lenient here
    Ok(true)
}

/// Parses rtcp-fb attribute: a=rtcp-fb:<payload type> <feedback type> [<additional feedback parameters>]
pub fn parse_rtcp_fb(value: &str) -> Result<(String, String, Option<String>)> {
    // Example: 96 nack
    // Example: 96 nack pli
    // Example: * ccm fir
    let parts: Vec<&str> = value.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err(Error::SdpParsingError(format!("Invalid rtcp-fb format: {}", value)));
    }
    
    let payload_type = parts[0].trim();
    let feedback_type = parts[1].trim();
    
    // Payload type should be a number or "*" (meaning all payload types)
    if payload_type != "*" && !payload_type.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::SdpParsingError(format!("Invalid payload type in rtcp-fb: {}", payload_type)));
    }
    
    // Validate feedback type
    if !["nack", "ack", "ccm", "trr-int", "app"].contains(&feedback_type) {
        // Unknown feedback type - some implementations may use custom types, so just warn
        // println!("Warning: Unknown RTCP feedback type: {}", feedback_type);
    }
    
    // Additional parameters are just passed through
    let additional_params = if parts.len() > 2 && !parts[2].trim().is_empty() {
        Some(parts[2].trim().to_string())
    } else {
        None
    };
    
    Ok((payload_type.to_string(), feedback_type.to_string(), additional_params))
}

/// Parses extmap attribute: a=extmap:<id>[/<direction>] <uri> [<extension parameters>]
pub fn parse_extmap(value: &str) -> Result<(u16, Option<String>, String, Option<String>)> {
    // Example: 1 urn:ietf:params:rtp-hdrext:ssrc-audio-level
    // Example: 2/sendrecv urn:ietf:params:rtp-hdrext:toffset
    
    // Split on first space to separate id/direction from URI and parameters
    let parts: Vec<&str> = value.splitn(2, ' ').collect();
    if parts.len() != 2 {
        return Err(Error::SdpParsingError(format!("Invalid extmap format: {}", value)));
    }
    
    // Parse id and optional direction
    let id_part = parts[0].trim();
    let (id_str, direction) = match id_part.split_once('/') {
        Some((id, dir)) => (id, Some(dir.to_string())),
        None => (id_part, None)
    };
    
    // Validate id (1-14 for one-byte header, 15-255 for two-byte header)
    let id = id_str.parse::<u16>()
        .map_err(|_| Error::SdpParsingError(format!("Invalid extmap id: {}", id_str)))?;
    if id < 1 || id > 255 {
        return Err(Error::SdpParsingError(format!("Extmap id out of range (1-255): {}", id)));
    }
    
    // Validate direction if present
    if let Some(dir) = &direction {
        if !["sendonly", "recvonly", "sendrecv", "inactive"].contains(&dir.as_str()) {
            return Err(Error::SdpParsingError(format!("Invalid extmap direction: {}", dir)));
        }
    }
    
    // Parse URI and optional parameters
    let uri_params_part = parts[1].trim();
    let (uri, parameters) = match uri_params_part.split_once(' ') {
        Some((uri, params)) => (uri.to_string(), Some(params.trim().to_string())),
        None => (uri_params_part.to_string(), None)
    };
    
    // Basic URI validation - should start with urn: or http:
    if !uri.starts_with("urn:") && !uri.starts_with("http:") {
        return Err(Error::SdpParsingError(format!("Invalid extmap URI: {}", uri)));
    }
    
    Ok((id, direction, uri, parameters))
}

/// Parses msid attribute: a=msid:<stream identifier> [<track identifier>]
pub fn parse_msid(value: &str) -> Result<(String, Option<String>)> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.is_empty() {
        return Err(Error::SdpParsingError("Empty msid attribute".to_string()));
    }
    
    let stream_id = parts[0].to_string();
    let track_id = if parts.len() > 1 { Some(parts[1].to_string()) } else { None };
    
    // Basic validation - identifiers should not be empty
    if stream_id.is_empty() {
        return Err(Error::SdpParsingError("Empty stream identifier in msid".to_string()));
    }
    
    if let Some(track) = &track_id {
        if track.is_empty() {
            return Err(Error::SdpParsingError("Empty track identifier in msid".to_string()));
        }
    }
    
    Ok((stream_id, track_id))
}

/// Parses bandwidth attribute: b=<bwtype>:<bandwidth>
pub fn parse_bandwidth(value: &str) -> Result<(String, u32)> {
    // Example: b=AS:128
    // Example: b=TIAS:64000
    
    let parts: Vec<&str> = value.split(':').collect();
    if parts.len() != 2 {
        return Err(Error::SdpParsingError(format!("Invalid bandwidth format: {}", value)));
    }
    
    let bwtype = parts[0].trim();
    // Check that bwtype is not empty
    if bwtype.is_empty() {
        return Err(Error::SdpParsingError("Empty bandwidth type".to_string()));
    }
    
    let bandwidth = parts[1].trim().parse::<u32>()
        .map_err(|_| Error::SdpParsingError(format!("Invalid bandwidth value: {}", parts[1])))?;
    
    // Validate bwtype
    match bwtype {
        "CT" | "AS" | "TIAS" | "RS" | "RR" => {}, // Known bandwidth types per various RFCs
        _ => {
            // Unknown bwtype - some implementations may use custom types, so just warn
            // println!("Warning: Unknown bandwidth type: {}", bwtype);
        }
    }
    
    Ok((bwtype.to_string(), bandwidth))
}

/// Parses rid attribute: a=rid:<id> <direction> [pt=<fmt-list>] [<restriction-name>=<restriction-value>]...
/// RFC 8851 defines the Restriction Identifier (RID) attribute
pub fn parse_rid(value: &str) -> Result<(String, String, Vec<String>)> {
    println!("Parsing RID value: '{}'", value);
    
    // Define a nom parser for the RID attribute according to RFC 8851
    fn rid_parser(input: &str) -> IResult<&str, (String, String, Vec<String>)> {
        // Parse the ID (alphanumeric with - and _)
        let (input, id) = take_while1(|c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_')(input)?;
        println!("  ID: '{}', remaining: '{}'", id, input);
        
        // Parse the space and direction
        let (input, _) = space1(input)?;
        let (input, direction) = take_while1(|c: char| c.is_ascii_alphabetic())(input)?;
        println!("  Direction: '{}', remaining: '{}'", direction, input);
        
        // Validate direction
        if direction != "send" && direction != "recv" {
            println!("  Invalid direction: '{}'", direction);
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Tag
            )));
        }
        
        // Parse the remaining restrictions
        let mut remaining = input;
        let mut restrictions = Vec::new();
        
        // Each restriction is whitespace-separated
        while let Ok((rest, _)) = space1::<_, nom::error::Error<_>>(remaining) {
            // We have more content to parse
            remaining = rest;
            println!("  Parsing restriction from: '{}'", remaining);
            
            // Handle special case for pt= format lists
            if remaining.starts_with("pt=") {
                // Parse until next whitespace for payload type
                let (rest, pt_restriction) = take_till1(|c: char| c.is_ascii_whitespace())(remaining)?;
                println!("  Found pt restriction: '{}'", pt_restriction);
                restrictions.push(pt_restriction.to_string());
                remaining = rest;
                continue;
            }
            
            // Parse a single restriction group (until next whitespace)
            let (rest, restriction_group) = take_till1(|c: char| c.is_ascii_whitespace())(remaining)?;
            println!("  Found restriction group: '{}'", restriction_group);
            
            // If semicolons are present, split by semicolons
            if restriction_group.contains(';') {
                println!("  Splitting by semicolons: '{}'", restriction_group);
                for restriction in restriction_group.split(';') {
                    println!("    Split restriction: '{}'", restriction);
                    restrictions.push(restriction.to_string());
                }
            } else {
                // No semicolons, treat as a single restriction
                println!("  No semicolons, using as is: '{}'", restriction_group);
                restrictions.push(restriction_group.to_string());
            }
            
            // Continue from the rest
            remaining = rest;
        }
        
        println!("  Parsed {} restrictions: {:?}", restrictions.len(), restrictions);
        
        Ok((
            remaining,
            (id.to_string(), direction.to_string(), restrictions)
        ))
    }
    
    // Apply the parser to the input
    match rid_parser(value) {
        Ok((_, result)) => {
            println!("Successfully parsed RID with {} restrictions", result.2.len());
            Ok(result)
        },
        Err(e) => {
            println!("Failed to parse RID: {:?}", e);
            Err(Error::SdpParsingError(format!("Invalid RID format: {}", value)))
        }
    }
}

/// Parses simulcast attribute: a=simulcast:<send_streams> <recv_streams>
pub fn parse_simulcast(value: &str) -> Result<(Vec<String>, Vec<String>)> {
    // Example: a=simulcast:send 1,2,3 recv 4,5,6
    // Example: a=simulcast:send 1;2;3 recv 4;5;6  (alternative format with semicolons)
    // Example: a=simulcast:send 1,~2,3;4,~5 recv 6;~7,8;~9
    
    // Define nom parsers for stream identifiers and groups
    fn stream_id(input: &str) -> IResult<&str, &str> {
        take_while1(|c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '~')(input)
    }
    
    // Parse comma-separated stream IDs
    fn stream_list(input: &str) -> IResult<&str, Vec<&str>> {
        separated_list1(
            char(','),
            stream_id
        )(input)
    }
    
    // Parse a single alternative (either a stream list or a single stream ID)
    fn stream_alternative(input: &str) -> IResult<&str, String> {
        let (input, content) = take_till1(|c: char| c == ';' || c.is_ascii_whitespace())(input)?;
        Ok((input, content.to_string()))
    }
    
    // Parse a list of alternatives (separated by semicolons)
    fn stream_alternatives(input: &str) -> IResult<&str, Vec<String>> {
        separated_list1(
            char(';'),
            map(stream_alternative, |s| s)
        )(input)
    }
    
    // Parse the send section
    fn send_section(input: &str) -> IResult<&str, Vec<String>> {
        let (input, _) = tag("send")(input)?;
        let (input, _) = space1(input)?;
        let (input, alternatives) = stream_alternatives(input)?;
        Ok((input, alternatives))
    }
    
    // Parse the recv section
    fn recv_section(input: &str) -> IResult<&str, Vec<String>> {
        let (input, _) = tag("recv")(input)?;
        let (input, _) = space1(input)?;
        let (input, alternatives) = stream_alternatives(input)?;
        Ok((input, alternatives))
    }
    
    // Main parser for the entire simulcast attribute
    fn simulcast_parser(input: &str) -> IResult<&str, (Vec<String>, Vec<String>)> {
        let mut send_streams: Vec<String> = Vec::new();
        let mut recv_streams: Vec<String> = Vec::new();
        let mut remaining = input;
        
        // Try to parse send section
        if let Ok((rest, streams)) = send_section(remaining) {
            send_streams = streams;
            remaining = rest;
            
            // Skip whitespace
            if let Ok((rest, _)) = space0::<_, nom::error::Error<_>>(remaining) {
                remaining = rest;
            }
        }
        
        // Try to parse recv section
        if let Ok((rest, streams)) = recv_section(remaining) {
            recv_streams = streams;
            remaining = rest;
        }
        
        // Ensure we parsed at least something
        if send_streams.is_empty() && recv_streams.is_empty() {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Tag
            )));
        }
        
        Ok((remaining, (send_streams, recv_streams)))
    }
    
    // Apply the parser
    match simulcast_parser(value) {
        Ok((_, result)) => Ok(result),
        Err(e) => {
            println!("Simulcast parsing error: {:?}", e);
            Err(Error::SdpParsingError(format!("Invalid simulcast format: {}", value)))
        }
    }
}

/// Parses scalability mode for AV1, H.264, and VP9: a=fmtp:<payload> scalability-mode=<mode>
/// This is for Scalable Video Coding (SVC) scenarios, often used with simulcast
pub fn parse_scalability_mode(mode: &str) -> Result<(String, Option<u32>, Option<u32>, Option<String>)> {
    // Extracts SVC parameters from mode string like "L2T3" or "S2T3"
    // Returns (pattern, spatial_layers, temporal_layers, extra)
    
    if mode.is_empty() {
        return Err(Error::SdpParsingError("Empty scalability mode".to_string()));
    }
    
    // Basic pattern is a letter followed by optional numbers and more patterns
    let pattern_char = mode.chars().next().unwrap().to_ascii_uppercase();
    
    // Validate pattern character
    if !['L', 'S', 'K'].contains(&pattern_char) {
        return Err(Error::SdpParsingError(format!("Invalid scalability mode pattern: {}", pattern_char)));
    }
    
    let pattern = pattern_char.to_string();
    
    // Parse spatial and temporal layers
    let mut spatial_layers: Option<u32> = None;
    let mut temporal_layers: Option<u32> = None;
    let mut extra: Option<String> = None;
    
    // Simple parsing - in practice would use regex
    if mode.len() > 1 {
        let rest = &mode[1..];
        if rest.contains('T') {
            let parts: Vec<&str> = rest.split('T').collect();
            if parts.len() >= 2 {
                // Try to parse spatial layers (before 'T')
                if !parts[0].is_empty() {
                    if let Ok(num) = parts[0].parse::<u32>() {
                        spatial_layers = Some(num);
                    } else {
                        extra = Some(rest.to_string());
                    }
                }
                
                // Parse temporal layers (after 'T')
                let temporal_part = parts[1];
                if !temporal_part.is_empty() {
                    if let Ok(num) = temporal_part.chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect::<String>()
                        .parse::<u32>() {
                        temporal_layers = Some(num);
                    }
                    
                    // Check for extra info
                    let extra_part = temporal_part.chars()
                        .skip_while(|c| c.is_ascii_digit())
                        .collect::<String>();
                    if !extra_part.is_empty() {
                        extra = Some(extra_part);
                    }
                }
            } else {
                extra = Some(rest.to_string());
            }
        } else if rest.chars().all(|c| c.is_ascii_digit()) {
            // Just a number, likely spatial layers
            if let Ok(num) = rest.parse::<u32>() {
                spatial_layers = Some(num);
            }
        } else {
            // Something else, store as extra
            extra = Some(rest.to_string());
        }
    }
    
    Ok((pattern, spatial_layers, temporal_layers, extra))
}

/// Parses ice-options attribute: a=ice-options:<option-tag> ...
/// Used to indicate ICE extensions, like trickle, according to RFC 8840
pub fn parse_ice_options(value: &str) -> Result<Vec<String>> {
    // Example: a=ice-options:trickle
    // Example: a=ice-options:trickle ice2
    
    let options: Vec<String> = value.split_whitespace()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    
    if options.is_empty() {
        return Err(Error::SdpParsingError("No ice-options specified".to_string()));
    }
    
    // Validate option tags (must be valid tokens)
    for option in &options {
        if !is_valid_token(option) {
            return Err(Error::SdpParsingError(format!("Invalid ice-option token: {}", option)));
        }
    }
    
    Ok(options)
}

/// Parses end-of-candidates attribute: a=end-of-candidates
/// Used in Trickle ICE to indicate the end of candidate trickling
pub fn parse_end_of_candidates(_value: &str) -> Result<bool> {
    // This is a flag attribute with no value
    Ok(true)
}

/// Parses sctp-port attribute: a=sctp-port:<port>
/// Used in WebRTC data channels (RFC 8841)
pub fn parse_sctp_port(value: &str) -> Result<u16> {
    // Example: a=sctp-port:5000
    match value.trim().parse::<u16>() {
        Ok(port) => Ok(port),
        Err(_) => Err(Error::SdpParsingError(format!("Invalid sctp-port value: {}", value)))
    }
}

/// Parses max-message-size attribute: a=max-message-size:<size>
/// Used in WebRTC data channels (RFC 8841)
pub fn parse_max_message_size(value: &str) -> Result<u64> {
    // Example: a=max-message-size:262144
    match value.trim().parse::<u64>() {
        Ok(size) => {
            // Validate reasonable size values
            if size == 0 {
                return Err(Error::SdpParsingError("max-message-size cannot be 0".to_string()));
            }
            
            // RFC 8841 suggests 262144 (2^18) as default max size
            // and mentions an upper limit of 2^53-1
            // We'll be more lenient and just ensure it's positive
            Ok(size)
        },
        Err(_) => Err(Error::SdpParsingError(format!("Invalid max-message-size value: {}", value)))
    }
}

/// Parses sctpmap attribute: a=sctpmap:<port> <app> <streams>
/// Legacy attribute for SCTP in WebRTC data channels (obsolete by RFC 8841)
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

/// Performs cross-attribute validation for a set of SDP attributes.
/// Validates that attributes reference valid values from other attributes.
pub fn validate_attributes(attributes: &[ParsedAttribute]) -> Result<()> {
    // Collect all mid values in the attributes
    let mut mids: Vec<String> = Vec::new();
    let mut has_bundle = false;
    let mut bundle_mids: Vec<String> = Vec::new();
    let mut rids: Vec<String> = Vec::new();
    let mut simulcast_rids: Vec<String> = Vec::new();
    
    // First pass - collect values
    for attr in attributes {
        match attr {
            ParsedAttribute::Mid(mid) => {
                mids.push(mid.clone());
            },
            ParsedAttribute::Group(semantics, group_mids) => {
                if semantics.to_uppercase() == "BUNDLE" {
                    has_bundle = true;
                    bundle_mids = group_mids.clone();
                }
            },
            ParsedAttribute::Rid(rid, _, _) => {
                rids.push(rid.clone());
            },
            ParsedAttribute::Simulcast(send_list, recv_list) => {
                // Extract RIDs from simulcast lists (removing any paused indicators)
                for list in [send_list, recv_list] {
                    for stream_ids in list {
                        for stream_id in stream_ids.split(',') {
                            // Remove pause indicator if present
                            let clean_id = stream_id.trim_start_matches('~').to_string();
                            simulcast_rids.push(clean_id);
                        }
                    }
                }
            },
            _ => {}
        }
    }
    
    // Second pass - validate references
    for attr in attributes {
        match attr {
            ParsedAttribute::Group(semantics, group_mids) => {
                if semantics.to_uppercase() == "BUNDLE" {
                    // Verify all mids in BUNDLE exist
                    for mid in group_mids {
                        if !mids.contains(mid) {
                            return Err(Error::SdpParsingError(
                                format!("BUNDLE references non-existent mid: {}", mid)
                            ));
                        }
                    }
                }
            },
            ParsedAttribute::Simulcast(_, _) => {
                // Verify all RIDs referenced in simulcast exist
                for rid in &simulcast_rids {
                    // The rid could have alternative formats in simulcast syntax
                    let clean_rid = rid.trim_start_matches('~');
                    if !clean_rid.is_empty() && !rids.contains(&clean_rid.to_string()) {
                        return Err(Error::SdpParsingError(
                            format!("Simulcast references non-existent rid: {}", clean_rid)
                        ));
                    }
                }
            },
            _ => {}
        }
    }
    
    Ok(())
}

// Add more attribute parsers as needed (e.g., candidate, ssrc, etc.) 
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
use crate::types::sdp::{RtpMapAttribute, FmtpAttribute, ParsedAttribute, CandidateAttribute, MediaDirection, SsrcAttribute};

// --- Placeholder Attribute Structs (Consider moving to types/sdp_attributes.rs later) ---

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

/// Represents SDP media directionality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaDirection {
    SendRecv,
    SendOnly,
    RecvOnly,
    Inactive,
}

// Add MediaDirection to ParsedAttribute enum
// #[derive(Debug, Clone, PartialEq)]
// pub enum ParsedAttribute {
//    ... 
//    Direction(MediaDirection),
//    Ptime(u32),
//    ... 
//}

// --- Parsing Functions --- 

/// Parses rtpmap attribute: a=rtpmap:<payload type> <encoding name>/<clock rate>[/<encoding parameters>]
pub fn parse_rtpmap(value: &str) -> Result<RtpMapAttribute> {
    // Example: 96 H264/90000
    // Example: 0 PCMU/8000
    // Example: 8 PCMA/8000/1
    let parts: Vec<&str> = value.splitn(2, ' ').collect();
    if parts.len() != 2 {
        return Err(Error::SdpParsingError(format!("Invalid rtpmap format: {}", value)));
    }
    
    let payload_type = parts[0].parse::<u8>()
        .map_err(|_| Error::SdpParsingError(format!("Invalid payload type in rtpmap: {}", parts[0])))?;
    
    let encoding_parts: Vec<&str> = parts[1].splitn(3, '/').collect();
    if encoding_parts.len() < 2 {
         return Err(Error::SdpParsingError(format!("Invalid encoding format in rtpmap: {}", parts[1])));
    }

    let encoding_name = encoding_parts[0].to_string();
    let clock_rate = encoding_parts[1].parse::<u32>()
         .map_err(|_| Error::SdpParsingError(format!("Invalid clock rate in rtpmap: {}", encoding_parts[1])))?;
    let encoding_params = encoding_parts.get(2).map(|s| s.to_string());

    Ok(RtpMapAttribute {
        payload_type,
        encoding_name,
        clock_rate,
        encoding_params,
    })
}

/// Parses fmtp attribute: a=fmtp:<format> <format specific parameters>
pub fn parse_fmtp(value: &str) -> Result<FmtpAttribute> {
    // Example: 97 profile-level-id=42e01f;level-asymmetry-allowed=1;packetization-mode=1
     let parts: Vec<&str> = value.splitn(2, ' ').collect();
    if parts.len() != 2 {
        return Err(Error::SdpParsingError(format!("Invalid fmtp format: {}", value)));
    }

    Ok(FmtpAttribute {
        format: parts[0].to_string(),
        parameters: parts[1].to_string(),
    })
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
    let port = parts[5].parse::<u16>()
        .map_err(|_| Error::SdpParsingError(format!("Invalid port in candidate: {}", parts[5])))?;
    // parts[6] is "typ"
    let candidate_type = parts[7].to_string();
    
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
                    related_address = Some(parts[current_index].to_string());
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
                if current_index < parts.len() && !["raddr", "rport", "typ" /* add more known ext keys */].contains(&parts[current_index]) {
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
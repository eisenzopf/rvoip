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
    let value_to_parse = if value.starts_with("m=") {
        &value[2..]
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
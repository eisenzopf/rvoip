use crate::error::{Error, Result};
use crate::types::sdp::{SdpSession, MediaDescription, Origin, ConnectionData, TimeDescription, ParsedAttribute, RtpMapAttribute, FmtpAttribute, CandidateAttribute, SsrcAttribute};
use bytes::Bytes;
use nom::{
    bytes::complete::{tag, take_till1, take_until},
    character::complete::{char, line_ending, not_line_ending, space0, space1},
    combinator::{map, map_res, opt, recognize},
    multi::{many0, many1},
    sequence::{pair, preceded, terminated, tuple},
    IResult,
};
use std::collections::HashMap;
use std::str::{self, FromStr};
use crate::sdp::attributes; // Import the attributes module itself
use crate::sdp::attributes::MediaDirection; // Import MediaDirection specifically

/// Parses a single SDP line into a key-value pair.
/// Example: "v=0" -> Ok(("", ('v', "0")))
fn parse_sdp_line(input: &str) -> IResult<&str, (char, &str)> {
    // SDP lines are key=value
    // key is a single character
    // value is the rest of the line until CRLF
    let (input, key) = nom::character::complete::anychar(input)?;
    let (input, _) = terminated(char('='), space0)(input)?;
    let (input, value) = not_line_ending(input)?;
    let (input, _) = line_ending(input)?; // Consume CRLF or LF

    // Basic validation: key should be a single char, value shouldn't be empty typically
    // More specific validation happens when building the SdpSession
    if value.is_empty() {
         // Allow empty values for some attributes?
         // For now, let it pass, validate later.
    }

    Ok((input, (key, value.trim())))
}

/// Parses an o= line
fn parse_origin_line(value: &str) -> Result<Origin> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() < 6 {
        return Err(Error::SdpParsingError(format!("Invalid o= line format: {}", value)));
    }
    Ok(Origin {
        username: parts[0].to_string(),
        sess_id: parts[1].to_string(),
        sess_version: parts[2].to_string(),
        net_type: parts[3].to_string(),
        addr_type: parts[4].to_string(),
        unicast_address: parts[5].to_string(),
    })
}

/// Parses a c= line
fn parse_connection_line(value: &str) -> Result<ConnectionData> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() < 3 {
        return Err(Error::SdpParsingError(format!("Invalid c= line format: {}", value)));
    }
    Ok(ConnectionData {
        net_type: parts[0].to_string(),
        addr_type: parts[1].to_string(),
        connection_address: parts[2].to_string(),
    })
}

/// Parses a t= line
fn parse_time_description_line(value: &str) -> Result<TimeDescription> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(Error::SdpParsingError(format!("Invalid t= line format: {}", value)));
    }
    // TODO: Validate parts[0] and parts[1] are valid NTP timestamps (u64)
    Ok(TimeDescription {
        start_time: parts[0].to_string(),
        stop_time: parts[1].to_string(),
    })
}

/// Parses the entire SDP content from bytes into an SdpSession struct.
pub fn parse_sdp(content: &Bytes) -> Result<SdpSession> {
    // Convert bytes to str - SDP is text based
    let sdp_str = str::from_utf8(content)
        .map_err(|e| Error::SdpParsingError(format!("Invalid UTF-8 in SDP: {}", e)))?;

    // Parse all lines into key-value pairs
    let parse_result = many1(parse_sdp_line)(sdp_str);

    match parse_result {
        Ok((remaining_input, lines)) => {
            if !remaining_input.trim().is_empty() {
                // Should ideally parse everything
                println!("SDP Parser Warning: Trailing data after parsing lines: {:?}", remaining_input);
            }

            // Need temporary Option fields for mandatory o, s, t during build
            let mut temp_origin: Option<Origin> = None;
            let mut temp_s_line: Option<String> = None;
            let mut temp_t_lines: Vec<TimeDescription> = Vec::new();
            
            let mut session = SdpSession {
                version: "".to_string(),
                origin: Origin { username: "-".into(), sess_id: "0".into(), sess_version: "0".into(), net_type: "IN".into(), addr_type: "IP4".into(), unicast_address: "0.0.0.0".into() }, // Temp default
                session_name: "".to_string(), // Temp default
                connection_info: None, 
                time_descriptions: Vec::new(), // Temp default
                media_descriptions: Vec::new(),
                direction: None,
                generic_attributes: Vec::new(),
            };

            let mut current_media: Option<MediaDescription> = None;

            for (key, value) in lines {
                match key {
                    'v' => {
                        if value != "0" { return Err(Error::SdpParsingError("Unsupported SDP version".to_string())); }
                        session.version = value.to_string();
                    }
                    'o' => {
                        if temp_origin.is_some() { return Err(Error::SdpParsingError("Duplicate o= line".to_string())); }
                        temp_origin = Some(parse_origin_line(value)?);
                    }
                    's' => {
                         if temp_s_line.is_some() { return Err(Error::SdpParsingError("Duplicate s= line".to_string())); }
                         if value.is_empty() { return Err(Error::SdpParsingError("Empty s= line".to_string())); } 
                         temp_s_line = Some(value.to_string());
                    }
                    'c' => { 
                        let conn_data = parse_connection_line(value)?;
                        if let Some(media) = current_media.as_mut() {
                           if media.connection_info.is_some() { return Err(Error::SdpParsingError("Duplicate c= line for media".to_string())); }
                           media.connection_info = Some(conn_data);
                        } else {
                            if session.connection_info.is_some() { return Err(Error::SdpParsingError("Duplicate session-level c= line".to_string())); }
                            session.connection_info = Some(conn_data);
                        }
                    }
                    't' => { 
                        temp_t_lines.push(parse_time_description_line(value)?);
                    }
                    'a' => { // Attribute
                         let parsed_attr = parse_attribute(value);
                         if let Some(media) = current_media.as_mut() {
                             // Store in media description
                             match parsed_attr {
                                 ParsedAttribute::Ptime(v) => {
                                     if media.ptime.is_some() { println!("SDP Warning: Duplicate ptime attribute for media {}", media.media); }
                                     media.ptime = Some(v);
                                 }
                                 ParsedAttribute::Direction(d) => {
                                      if media.direction.is_some() { println!("SDP Warning: Duplicate direction attribute for media {}", media.media); }
                                     media.direction = Some(d);
                                 }
                                 // Other attribute types go into the generic vec
                                 _ => media.generic_attributes.push(parsed_attr),
                             }
                         } else {
                            // Store in session description
                             match parsed_attr {
                                 ParsedAttribute::Direction(d) => {
                                     if session.direction.is_some() { println!("SDP Warning: Duplicate session-level direction attribute"); }
                                     session.direction = Some(d);
                                 }
                                  // Ptime is typically media-level, but handle if found at session level
                                 ParsedAttribute::Ptime(v) => {
                                     println!("SDP Warning: ptime attribute found at session level (usually media level)");
                                     // Decide whether to store it anyway or just put in generic
                                     session.generic_attributes.push(ParsedAttribute::Ptime(v)); 
                                 }
                                 // Other attribute types go into the generic vec
                                 _ => session.generic_attributes.push(parsed_attr),
                             }
                         }
                    }
                    'm' => { // Media Description
                        if let Some(mut media) = current_media.take() {
                            session.media_descriptions.push(media);
                        }
                        current_media = Some(parse_media_description_line(value)?);
                    }
                    _ => {} // Ignore other lines
                }
            }

            // Add the last media description if it exists
            if let Some(media) = current_media.take() {
                session.media_descriptions.push(media);
            }

            // Assign mandatory fields from temps
            session.origin = temp_origin.ok_or_else(|| Error::SdpParsingError("Missing mandatory o= field".to_string()))?;
            session.session_name = temp_s_line.ok_or_else(|| Error::SdpParsingError("Missing mandatory s= field".to_string()))?;
            if temp_t_lines.is_empty() {
                 return Err(Error::SdpParsingError("Missing mandatory t= field".to_string()));
            }
            session.time_descriptions = temp_t_lines;
            
            // Final validation (connection info)
            if session.connection_info.is_none() && !session.media_descriptions.is_empty() && session.media_descriptions.iter().any(|m| m.connection_info.is_none()) {
                 return Err(Error::SdpParsingError("Missing mandatory c= field (session or all media)".to_string()));
            }

            Ok(session)
        }
        Err(e) => Err(Error::SdpParsingError(format!("Failed parsing SDP lines: {:?}", e))),
    }
}

/// Parses an attribute line (a=key:value or a=key) into a ParsedAttribute enum variant.
fn parse_attribute(value: &str) -> ParsedAttribute {
    if let Some((key, val_part)) = value.split_once(':') {
        let key_trimmed = key.trim();
        let val_trimmed = val_part.trim();
        match key_trimmed {
            "rtpmap" => {
                attributes::parse_rtpmap(val_trimmed)
                    .map(ParsedAttribute::RtpMap)
                    .unwrap_or_else(|e| {
                         println!("SDP Attribute Warning: Failed to parse rtpmap '{}': {}", value, e);
                         ParsedAttribute::Value(key_trimmed.to_string(), val_trimmed.to_string())
                    })
            }
            "fmtp" => {
                 attributes::parse_fmtp(val_trimmed)
                    .map(ParsedAttribute::Fmtp)
                    .unwrap_or_else(|e| {
                        println!("SDP Attribute Warning: Failed to parse fmtp '{}': {}", value, e);
                        ParsedAttribute::Value(key_trimmed.to_string(), val_trimmed.to_string())
                    })
            }
             "ptime" => {
                 attributes::parse_ptime(val_trimmed)
                    .map(ParsedAttribute::Ptime)
                    .unwrap_or_else(|e| {
                        println!("SDP Attribute Warning: Failed to parse ptime '{}': {}", value, e);
                        ParsedAttribute::Value(key_trimmed.to_string(), val_trimmed.to_string())
                    })
            }
            "candidate" => {
                attributes::parse_candidate(val_trimmed)
                    .unwrap_or_else(|e| {
                        println!("SDP Attribute Warning: Failed to parse candidate '{}': {}", value, e);
                        ParsedAttribute::Value(key_trimmed.to_string(), val_trimmed.to_string())
                    })
            }
            "ssrc" => {
                 attributes::parse_ssrc(val_trimmed)
                    .unwrap_or_else(|e| {
                        println!("SDP Attribute Warning: Failed to parse ssrc '{}': {}", value, e);
                        ParsedAttribute::Value(key_trimmed.to_string(), val_trimmed.to_string())
                    })
            }
            // TODO: Add cases for other known attributes (mid, rtcp, etc.)
            _ => ParsedAttribute::Value(key_trimmed.to_string(), val_trimmed.to_string()), // Known key:value format, unknown key
        }
    } else {
        // Handle flag attributes
        let flag_key = value.trim();
        match flag_key {
             "sendrecv" | "sendonly" | "recvonly" | "inactive" => {
                 attributes::parse_direction(flag_key)
                    .map(ParsedAttribute::Direction)
                     .unwrap_or_else(|e| {
                        // This path should ideally not be reached if parse_direction handles the same keys
                        println!("SDP Attribute Warning: Failed to parse direction '{}': {}", value, e);
                        ParsedAttribute::Flag(flag_key.to_string())
                    })
             }
             // Add other known flag attributes here
             _ => ParsedAttribute::Flag(flag_key.to_string()), // Unknown flag
        }
    }
}

/// Parses the media description line (m=...)
fn parse_media_description_line(value: &str) -> Result<MediaDescription> {
     // Format: m=<media> <port> <proto> <fmt> ...
     let parts: Vec<&str> = value.splitn(4, ' ').collect();
     if parts.len() < 4 {
         return Err(Error::SdpParsingError(format!("Invalid m= line format: {}", value)));
     }

     let media = parts[0].to_string();
     let port = parts[1].parse::<u16>()
         .map_err(|_| Error::SdpParsingError(format!("Invalid port in m= line: {}", parts[1])))?;
     let protocol = parts[2].to_string();
     let formats = parts[3].split(' ').map(|s| s.to_string()).collect();

     Ok(MediaDescription {
         media,
         port,
         protocol,
         formats,
         connection_info: None, // Will be filled if c= line appears after m=
         ptime: None, // Initialize new field
         direction: None, // Initialize new field
         generic_attributes: Vec::new(), // Initialize new Vec
     })
} 
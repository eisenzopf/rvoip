//! Main SDP parser implementation
//!
//! This module contains the top-level SDP parsing function that processes
//! complete SDP messages and coordinates the various specialized parsers.

use crate::error::{Error, Result};
use crate::types::sdp::{SdpSession, ParsedAttribute, MediaDescription};
use bytes::Bytes;
use std::str;

use super::session_parser;
use super::line_parser::parse_sdp_line;
use super::attribute_parser::parse_attribute;
use super::media_parser::parse_media_description_line;
use super::time_parser::{parse_time_description_line, parse_repeat_time_line};
use super::validation;

/// Parses the entire SDP content from bytes into an SdpSession struct.
///
/// This is the main entry point for parsing SDP content. It handles the complete
/// parsing process according to RFC 8866, including:
/// - Parsing all SDP lines (v=, o=, s=, etc.)
/// - Validating line order and mandatory fields
/// - Processing both session-level and media-level attributes
///
/// # Parameters
///
/// - `content`: The SDP content as bytes
///
/// # Returns
///
/// - `Ok(SdpSession)` if parsing succeeds
/// - `Err(Error)` with a descriptive error message if parsing fails
pub fn parse_sdp(content: &Bytes) -> Result<SdpSession> {
    // Convert bytes to string first
    let sdp_str = match str::from_utf8(content) {
        Ok(s) => s,
        Err(_) => return Err(Error::SdpParsingError("SDP content is not valid UTF-8".to_string())),
    };
    
    // Split the content into lines
    let lines: Vec<&str> = sdp_str.lines().collect();
    
    // Define the state for tracking the current parsing section
    #[derive(PartialEq)]
    enum SdpParseSection {
        SessionHeader,
        MediaDescription,
    }
    
    // Define the state for tracking field order according to RFC 8866
    #[derive(PartialEq, PartialOrd)]
    enum FieldOrder {
        Version,     // v= (must be first)
        Origin,      // o= (must be second)
        SessionName, // s= (must be third)
        SessionLevel, // All other session-level fields (more lenient ordering)
        Media,       // m= (starts media section, must be after session fields)
    }
    
    let mut parse_section = SdpParseSection::SessionHeader;
    let mut field_position = FieldOrder::Version; // Must start with version
    
    // Initialize a session with default values
    let mut session = session_parser::init_session_description();
    let mut found_session_name = false;
    let mut found_origin = false;
    let mut found_version = false;
    let mut current_media_description: Option<MediaDescription> = None;
    
    // Process each line of the SDP content
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        i += 1; // Move to the next line
        
        // Skip empty lines
        if line.is_empty() {
            continue;
        }
        
        // Parse the line into key and value
        let (key, value) = match parse_sdp_line(line) {
            Ok((_, result)) => result,
            Err(_) => return Err(Error::SdpParsingError(format!("Failed to parse SDP line: {}", line))),
        };
        
        // Check field order according to RFC 8866, but be lenient where possible
        match key {
            'v' => {
                // Version must be the first field
                if field_position != FieldOrder::Version {
                    return Err(Error::SdpParsingError("v= must be the first line in SDP".to_string()));
                }
                field_position = FieldOrder::Origin;
            },
            'o' => {
                // Origin must come after version but before any m= line
                if field_position < FieldOrder::Origin {
                    return Err(Error::SdpParsingError("o= must come after v=".to_string()));
                }
                if field_position > FieldOrder::SessionLevel {
                    return Err(Error::SdpParsingError("o= must come before m=".to_string()));
                }
                field_position = FieldOrder::SessionName;
            },
            's' => {
                // Session name must come after origin but before any m= line
                if field_position < FieldOrder::SessionName {
                    return Err(Error::SdpParsingError("s= must come after o=".to_string()));
                }
                if field_position > FieldOrder::SessionLevel {
                    return Err(Error::SdpParsingError("s= must come before m=".to_string()));
                }
                field_position = FieldOrder::SessionLevel;
            },
            'm' => {
                if field_position < FieldOrder::SessionLevel {
                    return Err(Error::SdpParsingError("m= must come after v=, o=, and s=".to_string()));
                }
                field_position = FieldOrder::Media;
            },
            _ => {
                // For all other fields, just ensure they come after v=, o=, s= and in the right section
                if field_position < FieldOrder::SessionName {
                    return Err(Error::SdpParsingError(format!("{}= must come after v=, o=, and s=", key)));
                }
                
                // Once we're in the session level fields or media section, be lenient with ordering
                if parse_section == SdpParseSection::SessionHeader {
                    field_position = FieldOrder::SessionLevel;
                }
            }
        }
        
        // Process the line based on its type
        match key {
            // v= (Protocol Version)
            'v' => {
                if found_version {
                    return Err(Error::SdpParsingError("Multiple v= lines found".to_string()));
                }
                
                if value != "0" {
                    return Err(Error::SdpParsingError(format!("Unsupported SDP version: {}", value)));
                }
                
                session.version = value.to_string();
                found_version = true;
            },
            
            // o= (Origin)
            'o' => {
                if found_origin {
                    return Err(Error::SdpParsingError("Multiple o= lines found".to_string()));
                }
                
                let origin = session_parser::parse_origin_line(value)?;
                session.origin = origin;
                found_origin = true;
            },
            
            // s= (Session Name)
            's' => {
                if found_session_name {
                    return Err(Error::SdpParsingError("Multiple s= lines found".to_string()));
                }
                
                session.session_name = value.to_string();
                found_session_name = true;
            },
            
            // i= (Session Information)
            'i' => {
                match parse_section {
                    SdpParseSection::SessionHeader => {
                        if session.session_info.is_some() {
                            return Err(Error::SdpParsingError("Multiple session-level i= lines found".to_string()));
                        }
                        session.session_info = Some(value.to_string());
                    },
                    SdpParseSection::MediaDescription => {
                        if let Some(ref mut md) = current_media_description {
                            // Media description doesn't have an information field in this codebase
                            // Just ignore it or add it as an attribute
                        } else {
                            return Err(Error::SdpParsingError("i= line found outside of media section".to_string()));
                        }
                    }
                }
            },
            
            // u= (URI)
            'u' => {
                if session.uri.is_some() {
                    return Err(Error::SdpParsingError("Multiple u= lines found".to_string()));
                }
                
                session.uri = Some(value.to_string());
            },
            
            // e= (Email Address)
            'e' => {
                if session.email.is_some() {
                    return Err(Error::SdpParsingError("Multiple e= lines found".to_string()));
                }
                session.email = Some(value.to_string());
            },
            
            // p= (Phone Number)
            'p' => {
                if session.phone.is_some() {
                    return Err(Error::SdpParsingError("Multiple p= lines found".to_string()));
                }
                session.phone = Some(value.to_string());
            },
            
            // c= (Connection Data)
            'c' => {
                match parse_section {
                    SdpParseSection::SessionHeader => {
                        if session.connection_info.is_some() {
                            return Err(Error::SdpParsingError("Multiple session-level c= lines found".to_string()));
                        }
                        session.connection_info = Some(session_parser::parse_connection_line(value)?);
                    },
                    SdpParseSection::MediaDescription => {
                        if let Some(md) = &mut current_media_description {
                            if md.connection_info.is_some() {
                                return Err(Error::SdpParsingError("Multiple media-level c= lines found".to_string()));
                            }
                            md.connection_info = Some(session_parser::parse_connection_line(value)?);
                        } else {
                            return Err(Error::SdpParsingError("c= line found outside of media section".to_string()));
                        }
                    }
                }
            },
            
            // t= (Timing)
            't' => {
                let time_desc = parse_time_description_line(value)?;
                session.time_descriptions.push(time_desc);
            },
            
            // r= (Repeat Times)
            'r' => {
                if session.time_descriptions.is_empty() {
                    return Err(Error::SdpParsingError("r= line found before any t= line".to_string()));
                }
                
                let last_timing = session.time_descriptions.last_mut().unwrap();
                let repeat_time = parse_repeat_time_line(value)?;
                last_timing.repeat_times.push(repeat_time);
            },
            
            // z= (Time Zones)
            'z' => {
                // Time zones not directly supported in the type, add as an attribute
                session = session.with_attribute(ParsedAttribute::Value("time-zones".to_string(), value.to_string()));
            },
            
            // k= (Encryption Key)
            'k' => {
                // Encryption key not directly supported in the type, add as an attribute
                let attr = ParsedAttribute::Value("encryption-key".to_string(), value.to_string());
                match parse_section {
                    SdpParseSection::SessionHeader => {
                        session = session.with_attribute(attr);
                    },
                    SdpParseSection::MediaDescription => {
                        if let Some(md) = &mut current_media_description {
                            *md = md.clone().with_attribute(attr);
                        } else {
                            return Err(Error::SdpParsingError("k= line found outside of media section".to_string()));
                        }
                    }
                }
            },
            
            // a= (Attribute)
            'a' => {
                let attribute = parse_attribute(value)?;
                
                match parse_section {
                    SdpParseSection::SessionHeader => {
                        session = session.with_attribute(attribute);
                    },
                    SdpParseSection::MediaDescription => {
                        if let Some(md) = &mut current_media_description {
                            *md = md.clone().with_attribute(attribute);
                        } else {
                            return Err(Error::SdpParsingError("a= line found outside of media section".to_string()));
                        }
                    }
                }
            },
            
            // m= (Media Description)
            'm' => {
                // If we were already parsing a media section, add it to the session
                if let Some(md) = current_media_description.take() {
                    session.media_descriptions.push(md);
                }
                
                // Start a new media section
                current_media_description = Some(parse_media_description_line(value)?);
                parse_section = SdpParseSection::MediaDescription;
            },
            
            // Unknown line type
            _ => {
                return Err(Error::SdpParsingError(format!("Unknown SDP line type: {}", key)));
            }
        }
    }
    
    // Add the final media description if there is one
    if let Some(md) = current_media_description {
        session.media_descriptions.push(md);
    }
    
    // Validate that required fields were found
    if !found_version {
        return Err(Error::SdpParsingError("Missing v= line".to_string()));
    }
    
    if !found_origin {
        return Err(Error::SdpParsingError("Missing o= line".to_string()));
    }
    
    if !found_session_name {
        return Err(Error::SdpParsingError("Missing s= line".to_string()));
    }
    
    if session.time_descriptions.is_empty() {
        return Err(Error::SdpParsingError("Missing t= line".to_string()));
    }
    
    // Validate the overall SDP structure
    validation::validate_sdp(&session)?;
    
    Ok(session)
} 
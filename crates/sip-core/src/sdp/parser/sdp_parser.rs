//! # SDP Parser Implementation
//!
//! This module provides the core implementation for parsing Session Description Protocol
//! (SDP) messages according to [RFC 8866](https://tools.ietf.org/html/rfc8866) (which obsoletes RFC 4566).
//!
//! SDP is a format for describing multimedia communication sessions for the purposes of
//! session announcement, session invitation, and parameter negotiation. It is widely used 
//! in VoIP applications, WebRTC, and other real-time multimedia applications.
//!
//! ## Parsing Process
//!
//! The SDP parsing process involves several steps:
//!
//! 1. Validating that the content is valid UTF-8 text
//! 2. Splitting the content into lines
//! 3. Parsing each line according to its type (v=, o=, s=, etc.)
//! 4. Enforcing the required field order according to the RFC
//! 5. Building a structured representation of the SDP session
//! 6. Validating the completeness and correctness of the data
//!
//! ## SDP Structure
//!
//! An SDP message consists of a session-level section followed by zero or more media-level
//! sections. The session-level section starts with a v= line and continues until the first
//! m= line. Each media-level section starts with an m= line and continues until the next
//! m= line or the end of the message.
//!
//! ### Required Fields
//!
//! According to RFC 8866, the following fields are mandatory in the session-level section:
//!
//! - `v=` - Protocol version (must be "0")
//! - `o=` - Origin (specifies the originator of the session)
//! - `s=` - Session name
//! - `t=` - Timing (start and stop times)
//!
//! ### Connection Information
//!
//! A session must include connection information in at least one of:
//! - The session-level section (c= line)
//! - Each media-level section that uses a non-connection-oriented transport
//!
//! ## Field Order
//!
//! The SDP specification mandates a specific order for fields:
//!
//! 1. Session-level fields must appear in a specific order:
//!    - v= (version) must be first
//!    - o= (origin) must be second
//!    - s= (session name) must be third
//!    - Other session-level fields follow in a more relaxed order
//!
//! 2. Media-level sections must follow all session-level fields,
//!    and each media-level section begins with an m= line.

use crate::error::{Error, Result};
use crate::types::sdp::{ConnectionData, MediaDescription, Origin, ParsedAttribute, RepeatTime, SdpSession, TimeDescription};
use crate::sdp::parser::attribute_parser;
use crate::sdp::parser::validation;
use crate::sdp::attributes::MediaDirection;
use bytes::Bytes;
use std::str::{self, FromStr};

use super::session_parser;
use super::line_parser::parse_sdp_line;
use super::media_parser::parse_media_description_line;
use super::time_parser::{parse_time_description_line, parse_repeat_time_line};

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
///
/// # Examples
///
/// ```
/// use bytes::Bytes;
/// use rvoip_sip_core::sdp::parser::parse_sdp;
/// use rvoip_sip_core::sdp::attributes::MediaDirection;
///
/// // Example SDP message from RFC 8866
/// let sdp_str = "\
/// v=0
/// o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5
/// s=SDP Seminar
/// c=IN IP4 224.2.17.12/127
/// t=2873397496 2873404696
/// a=recvonly
/// m=audio 49170 RTP/AVP 0
/// ";
///
/// // Parse the SDP message
/// let session = parse_sdp(&Bytes::from(sdp_str)).unwrap();
///
/// // Access session information
/// assert_eq!(session.origin.username, "jdoe");
/// assert_eq!(session.session_name, "SDP Seminar");
/// assert!(matches!(session.direction, Some(MediaDirection::RecvOnly)));
///
/// // Access media descriptions
/// assert_eq!(session.media_descriptions.len(), 1);
/// let media = &session.media_descriptions[0];
/// assert_eq!(media.media, "audio");
/// assert_eq!(media.port, 49170);
/// ```
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
                // Media must come after session-level fields
                if field_position < FieldOrder::SessionLevel {
                    return Err(Error::SdpParsingError("m= must come after v=, o=, and s=".to_string()));
                }
                
                // Check if at least one timing description is present before media section
                if session.time_descriptions.is_empty() {
                    return Err(Error::SdpParsingError("m= must come after at least one t= line".to_string()));
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
                // Parse attribute line into key and value
                let mut parts = value.splitn(2, ':');
                let key = parts.next().unwrap_or("").trim();
                let val = parts.next().unwrap_or("").trim();
                
                handle_attribute(&mut session, current_media_description.as_mut(), key, val)?;
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
    
    // Validate the resulting SDP session
    validation::validate_sdp(&session)?;
    
    Ok(session)
}

/// Handles parsing and processing of an SDP attribute line.
///
/// This function parses attribute lines (a=) and adds them to either the session or
/// the current media description, handling special attributes like media direction.
///
/// # Parameters
///
/// - `session`: The session to which the attribute may be added
/// - `current_media`: The current media description being parsed, if any
/// - `key`: The attribute name
/// - `value`: The attribute value, if any
///
/// # Returns
///
/// - `Ok(())` if the attribute was successfully processed
/// - `Err(Error)` if there was an error parsing the attribute
///
/// # Examples
///
/// This function is called internally by parse_sdp to process attribute lines:
/// ```rust,no_run
/// # use rvoip_sip_core::types::sdp::{SdpSession, Origin, MediaDescription};
/// # use rvoip_sip_core::error::{Error, Result};
/// # 
/// # // This is a simplified version of the real handle_attribute function for example purposes
/// # fn handle_attribute(session: &mut SdpSession, current_media: Option<&mut MediaDescription>, 
/// #                  key: &str, value: &str) -> Result<()> {
/// #    // Simplified implementation for doctest
/// #    Ok(())
/// # }
/// #
/// // Create a test session
/// let origin = Origin {
///     username: "test".to_string(),
///     sess_id: "123".to_string(),
///     sess_version: "1".to_string(),
///     net_type: "IN".to_string(),
///     addr_type: "IP4".to_string(),
///     unicast_address: "127.0.0.1".to_string(),
/// };
///
/// let mut session = SdpSession::new(origin, "Test Session");
/// let mut media = MediaDescription::new("audio", 49170, "RTP/AVP", vec!["0".to_string()]);
///
/// // Process various attribute types
/// let _ = handle_attribute(&mut session, None, "sendrecv", "");       // Direction at session level
/// let _ = handle_attribute(&mut session, Some(&mut media), "ptime", "20"); // ptime at media level
/// ```
fn handle_attribute(session: &mut SdpSession, current_media: Option<&mut MediaDescription>, key: &str, value: &str) -> Result<()> {
    // Attributes that require values but are missing them should be rejected
    match key {
        "rtpmap" | "fmtp" | "candidate" | "ssrc" | "mid" | "msid" | "ice-ufrag" | "ice-pwd" | "fingerprint" | "setup" | "rid" | "extmap" => {
            if value.is_empty() {
                return Err(Error::SdpParsingError(format!("Attribute '{}' requires a value but none was provided", key)));
            }
        },
        _ => {},
    }
    
    // Create a formatted attribute line for the parser
    let attr_line = if value.is_empty() {
        key.to_string()
    } else {
        format!("{}:{}", key, value)
    };
    
    let parsed_attr = attribute_parser::parse_attribute(&attr_line)?;
    
    if let Some(media) = current_media {
        // Media-level attributes
        if let ParsedAttribute::Direction(direction) = parsed_attr {
            media.direction = Some(direction);
        } else if let ParsedAttribute::Ptime(ptime) = parsed_attr {
            media.ptime = Some(ptime as u32);
        } else {
            media.generic_attributes.push(parsed_attr);
        }
    } else {
        // Session-level attributes
        if let ParsedAttribute::Direction(direction) = parsed_attr {
            session.direction = Some(direction);
        } else {
            session.generic_attributes.push(parsed_attr);
        }
    }
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn test_parse_minimal_valid_sdp() {
        // Test a minimal valid SDP with only required fields
        let sdp_str = "\
v=0
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5
s=SDP Seminar
c=IN IP4 224.2.17.12
t=0 0
";
        let result = parse_sdp(&Bytes::from(sdp_str));
        assert!(result.is_ok());
        
        let session = result.unwrap();
        assert_eq!(session.version, "0");
        assert_eq!(session.origin.username, "jdoe");
        assert_eq!(session.origin.sess_id, "2890844526");
        assert_eq!(session.origin.sess_version, "2890842807");
        assert_eq!(session.origin.net_type, "IN");
        assert_eq!(session.origin.addr_type, "IP4");
        assert_eq!(session.origin.unicast_address, "10.47.16.5");
        assert_eq!(session.session_name, "SDP Seminar");
        assert_eq!(session.time_descriptions.len(), 1);
        assert_eq!(session.time_descriptions[0].start_time, "0");
        assert_eq!(session.time_descriptions[0].stop_time, "0");
    }

    #[test]
    fn test_parse_rfc8866_example() {
        // Test the example from RFC 8866 section 5
        let sdp_str = "\
v=0
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5
s=SDP Seminar
i=A Seminar on the session description protocol
u=http://www.example.com/seminars/sdp.pdf
e=j.doe@example.com (Jane Doe)
p=+1 617 555-6011
c=IN IP4 224.2.17.12/127
t=2873397496 2873404696
r=7d 1h 0 25h
m=audio 49170 RTP/AVP 0
m=video 51372 RTP/AVP 99
a=rtpmap:99 h263-1998/90000
";
        let result = parse_sdp(&Bytes::from(sdp_str));
        assert!(result.is_ok());
        
        let session = result.unwrap();
        assert_eq!(session.session_info.unwrap(), "A Seminar on the session description protocol");
        assert_eq!(session.uri.unwrap(), "http://www.example.com/seminars/sdp.pdf");
        assert_eq!(session.email.unwrap(), "j.doe@example.com (Jane Doe)");
        assert_eq!(session.phone.unwrap(), "+1 617 555-6011");
        
        let conn = session.connection_info.unwrap();
        assert_eq!(conn.net_type, "IN");
        assert_eq!(conn.addr_type, "IP4");
        assert_eq!(conn.connection_address, "224.2.17.12");
        assert_eq!(conn.ttl, Some(127));
        
        assert_eq!(session.time_descriptions.len(), 1);
        let timing = &session.time_descriptions[0];
        assert_eq!(timing.start_time, "2873397496");
        assert_eq!(timing.stop_time, "2873404696");
        assert_eq!(timing.repeat_times.len(), 1);
        
        assert_eq!(session.media_descriptions.len(), 2);
        let audio = &session.media_descriptions[0];
        assert_eq!(audio.media, "audio");
        assert_eq!(audio.port, 49170);
        assert_eq!(audio.protocol, "RTP/AVP");
        assert_eq!(audio.formats, vec!["0"]);
        
        let video = &session.media_descriptions[1];
        assert_eq!(video.media, "video");
        assert_eq!(video.port, 51372);
        assert_eq!(video.protocol, "RTP/AVP");
        assert_eq!(video.formats, vec!["99"]);
    }

    #[test]
    fn test_parse_webrtc_sdp() {
        // Test a WebRTC style SDP with ICE and DTLS
        let sdp_str = "\
v=0
o=- 20518 0 IN IP4 0.0.0.0
s=-
c=IN IP4 192.168.1.100
t=0 0
a=group:BUNDLE audio video
a=ice-ufrag:F7gI
a=ice-pwd:x9cml/YzichV2+XlhiMu8g
a=fingerprint:sha-256 D1:2C:74:A7:E3:B5:8D:B1:69:6C:72:E9:6F:7F:79:5B
a=setup:actpass
m=audio 49170 UDP/TLS/RTP/SAVPF 111
a=mid:audio
a=sendrecv
a=rtpmap:111 opus/48000/2
a=candidate:0 1 UDP 2122194687 192.168.1.100 49170 typ host
m=video 56789 UDP/TLS/RTP/SAVPF 96 97
a=mid:video
a=sendrecv
a=rtpmap:96 VP8/90000
a=rtpmap:97 rtx/90000
a=fmtp:97 apt=96
";
        let result = parse_sdp(&Bytes::from(sdp_str));
        assert!(result.is_ok());
        
        let session = result.unwrap();
        
        // Check ICE attributes
        let mut found_ice_ufrag = false;
        let mut found_ice_pwd = false;
        let mut found_fingerprint = false;
        
        for attr in &session.generic_attributes {
            match attr {
                ParsedAttribute::IceUfrag(ufrag) => {
                    assert_eq!(ufrag, "F7gI");
                    found_ice_ufrag = true;
                },
                ParsedAttribute::IcePwd(pwd) => {
                    assert_eq!(pwd, "x9cml/YzichV2+XlhiMu8g");
                    found_ice_pwd = true;
                },
                ParsedAttribute::Fingerprint(algo, _) => {
                    assert_eq!(algo, "sha-256");
                    found_fingerprint = true;
                },
                _ => {}
            }
        }
        
        assert!(found_ice_ufrag, "Missing ice-ufrag attribute");
        assert!(found_ice_pwd, "Missing ice-pwd attribute");
        assert!(found_fingerprint, "Missing fingerprint attribute");
        
        // Check media sections
        assert_eq!(session.media_descriptions.len(), 2);
        
        // Check direction
        let audio = &session.media_descriptions[0];
        assert!(matches!(audio.direction, Some(MediaDirection::SendRecv)));
        
        // Check rtpmap
        let mut found_opus = false;
        for attr in &audio.generic_attributes {
            if let ParsedAttribute::RtpMap(rtpmap) = attr {
                if rtpmap.payload_type == 111 {
                    assert_eq!(rtpmap.encoding_name, "opus");
                    assert_eq!(rtpmap.clock_rate, 48000);
                    assert_eq!(rtpmap.encoding_params, Some("2".to_string()));
                    found_opus = true;
                }
            }
        }
        assert!(found_opus, "Missing opus rtpmap attribute");
    }

    #[test]
    fn test_parse_invalid_field_order() {
        // Test s= before o=
        let sdp_str = "\
v=0
s=SDP Test
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5
c=IN IP4 224.2.17.12
t=0 0
";
        let result = parse_sdp(&Bytes::from(sdp_str));
        assert!(result.is_err(), "s= before o= should be rejected");

        // Test o= before v=
        let sdp_str = "\
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5
v=0
s=SDP Test
c=IN IP4 224.2.17.12
t=0 0
";
        let result = parse_sdp(&Bytes::from(sdp_str));
        assert!(result.is_err(), "o= before v= should be rejected");
        
        // Test m= before t=
        let sdp_str = "\
v=0
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5
s=SDP Test
c=IN IP4 224.2.17.12
m=audio 49170 RTP/AVP 0
t=0 0
";
        let result = parse_sdp(&Bytes::from(sdp_str));
        assert!(result.is_err(), "m= before t= should be rejected");
    }

    #[test]
    fn test_parse_missing_mandatory_fields() {
        // Test missing v=
        let sdp_str = "\
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5
s=SDP Test
c=IN IP4 224.2.17.12
t=0 0
";
        assert!(parse_sdp(&Bytes::from(sdp_str)).is_err());

        // Test missing o=
        let sdp_str = "\
v=0
s=SDP Test
c=IN IP4 224.2.17.12
t=0 0
";
        assert!(parse_sdp(&Bytes::from(sdp_str)).is_err());

        // Test missing s=
        let sdp_str = "\
v=0
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5
c=IN IP4 224.2.17.12
t=0 0
";
        assert!(parse_sdp(&Bytes::from(sdp_str)).is_err());

        // Test missing t=
        let sdp_str = "\
v=0
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5
s=SDP Test
c=IN IP4 224.2.17.12
";
        assert!(parse_sdp(&Bytes::from(sdp_str)).is_err());
    }

    #[test]
    fn test_parse_media_attributes() {
        // Test media-level attributes
        let sdp_str = "\
v=0
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5
s=SDP Test
c=IN IP4 224.2.17.12
t=0 0
m=audio 49170 RTP/AVP 0
a=ptime:20
a=maxptime:40
a=recvonly
";
        let result = parse_sdp(&Bytes::from(sdp_str)).unwrap();
        let media = &result.media_descriptions[0];
        
        // Check ptime attribute
        assert_eq!(media.ptime, Some(20));
        
        // Check direction attribute
        assert!(matches!(media.direction, Some(MediaDirection::RecvOnly)));
        
        // Check that maxptime is in generic attributes
        let mut found_maxptime = false;
        for attr in &media.generic_attributes {
            if let ParsedAttribute::MaxPtime(val) = attr {
                assert_eq!(*val, 40);
                found_maxptime = true;
                break;
            } else if let ParsedAttribute::Value(key, val) = attr {
                if key == "maxptime" && val == "40" {
                    found_maxptime = true;
                    break;
                }
            }
        }
        assert!(found_maxptime, "maxptime attribute not found");
    }

    #[test]
    fn test_parse_repeat_time() {
        // Test repeat time parsing
        let sdp_str = "\
v=0
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5
s=SDP Test
c=IN IP4 224.2.17.12
t=2873397496 2873404696
r=604800 3600 0 90000
r=604800 3600 90000 180000
";
        let result = parse_sdp(&Bytes::from(sdp_str)).unwrap();
        
        // Check repeat times
        assert_eq!(result.time_descriptions.len(), 1);
        let time_desc = &result.time_descriptions[0];
        assert_eq!(time_desc.repeat_times.len(), 2);
        
        let r1 = &time_desc.repeat_times[0];
        assert_eq!(r1.repeat_interval, 604800);
        assert_eq!(r1.active_duration, 3600);
        assert_eq!(r1.offsets.len(), 2);
        assert_eq!(r1.offsets[0], 0);
        assert_eq!(r1.offsets[1], 90000);
        
        let r2 = &time_desc.repeat_times[1];
        assert_eq!(r2.offsets[0], 90000);
        assert_eq!(r2.offsets[1], 180000);
    }

    #[test]
    fn test_parse_connection_info() {
        // Test connection info at session level
        let sdp_str = "\
v=0
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5
s=SDP Test
c=IN IP4 224.2.17.12/127
t=0 0
m=audio 49170 RTP/AVP 0
";
        let result = parse_sdp(&Bytes::from(sdp_str)).unwrap();
        let conn = result.connection_info.unwrap();
        assert_eq!(conn.connection_address, "224.2.17.12");
        assert_eq!(conn.ttl, Some(127));
        
        // Test connection info at media level only
        let sdp_str = "\
v=0
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5
s=SDP Test
t=0 0
m=audio 49170 RTP/AVP 0
c=IN IP4 224.2.17.12/127
";
        let result = parse_sdp(&Bytes::from(sdp_str)).unwrap();
        assert!(result.connection_info.is_none());
        let media = &result.media_descriptions[0];
        let conn = media.connection_info.as_ref().unwrap();
        assert_eq!(conn.connection_address, "224.2.17.12");
        assert_eq!(conn.ttl, Some(127));
    }

    #[test]
    fn test_parse_ipv6_connection() {
        // Test IPv6 connection data
        let sdp_str = "\
v=0
o=jdoe 2890844526 2890842807 IN IP6 2001:db8::1
s=SDP Test
c=IN IP6 FF15::101/3
t=0 0
m=audio 49170 RTP/AVP 0
";
        let result = parse_sdp(&Bytes::from(sdp_str)).unwrap();
        let conn = result.connection_info.unwrap();
        assert_eq!(conn.net_type, "IN");
        assert_eq!(conn.addr_type, "IP6");
        assert_eq!(conn.connection_address, "FF15::101");
        assert_eq!(conn.ttl, Some(3));
    }

    #[test]
    fn test_parse_direction_attributes() {
        // Test session-level direction attribute
        let sdp_str = "\
v=0
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5
s=SDP Test
c=IN IP4 224.2.17.12
t=0 0
a=sendrecv
m=audio 49170 RTP/AVP 0
";
        let result = parse_sdp(&Bytes::from(sdp_str)).unwrap();
        assert!(matches!(result.direction, Some(MediaDirection::SendRecv)));
        
        // Test media-level direction attribute overriding session-level
        let sdp_str = "\
v=0
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5
s=SDP Test
c=IN IP4 224.2.17.12
t=0 0
a=sendrecv
m=audio 49170 RTP/AVP 0
a=sendonly
";
        let result = parse_sdp(&Bytes::from(sdp_str)).unwrap();
        assert!(matches!(result.direction, Some(MediaDirection::SendRecv)));
        let media = &result.media_descriptions[0];
        assert!(matches!(media.direction, Some(MediaDirection::SendOnly)));
    }

    #[test]
    fn test_parse_multiple_media_sections() {
        // Test multiple media sections
        let sdp_str = "\
v=0
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5
s=SDP Test
c=IN IP4 224.2.17.12
t=0 0
m=audio 49170 RTP/AVP 0
a=rtpmap:0 PCMU/8000
m=video 51372 RTP/AVP 31
a=rtpmap:31 H261/90000
m=application 32416 udp wb
a=orient:portrait
";
        let result = parse_sdp(&Bytes::from(sdp_str)).unwrap();
        
        // Check media counts
        assert_eq!(result.media_descriptions.len(), 3);
        
        // Check audio media
        let audio = &result.media_descriptions[0];
        assert_eq!(audio.media, "audio");
        assert_eq!(audio.port, 49170);
        
        // Check video media
        let video = &result.media_descriptions[1];
        assert_eq!(video.media, "video");
        assert_eq!(video.port, 51372);
        
        // Check application media
        let app = &result.media_descriptions[2];
        assert_eq!(app.media, "application");
        assert_eq!(app.port, 32416);
        assert_eq!(app.protocol, "udp");
        assert_eq!(app.formats, vec!["wb"]);
    }

    #[test]
    fn test_duplicate_fields() {
        // Test duplicate session-level fields
        let sdp_str = "\
v=0
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5
s=SDP Test
s=Duplicate Session Name
c=IN IP4 224.2.17.12
t=0 0
";
        let result = parse_sdp(&Bytes::from(sdp_str));
        assert!(result.is_err());
        
        // Test duplicate media-level fields
        let sdp_str = "\
v=0
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5
s=SDP Test
c=IN IP4 224.2.17.12
t=0 0
m=audio 49170 RTP/AVP 0
c=IN IP4 224.2.17.12
c=IN IP4 224.2.17.13
";
        let result = parse_sdp(&Bytes::from(sdp_str));
        assert!(result.is_err());
    }

    #[test]
    fn test_handle_attribute() {
        // Create a basic origin for the session
        let origin = Origin {
            username: "test".to_string(),
            sess_id: "123".to_string(),
            sess_version: "1".to_string(),
            net_type: "IN".to_string(),
            addr_type: "IP4".to_string(),
            unicast_address: "127.0.0.1".to_string(),
        };
        
        // Test session attribute handling
        let mut session = SdpSession::new(origin, "Test Session");
        assert!(handle_attribute(&mut session, None, "sendrecv", "").is_ok());
        assert!(matches!(session.direction, Some(MediaDirection::SendRecv)));
        
        // Test media attribute handling
        let mut md = MediaDescription::new("audio", 49170, "RTP/AVP", vec!["0".to_string()]);
        assert!(handle_attribute(&mut session, Some(&mut md), "ptime", "20").is_ok());
        assert_eq!(md.ptime, Some(20));
        
        // Test attribute requiring value
        let result = handle_attribute(&mut session, None, "rtpmap", "");
        assert!(result.is_err());
    }
} 
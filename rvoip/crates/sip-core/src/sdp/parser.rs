use crate::error::{Error, Result};
use crate::types::sdp::{SdpSession, MediaDescription, Origin, ConnectionData, TimeDescription, ParsedAttribute, RtpMapAttribute, FmtpAttribute, CandidateAttribute, SsrcAttribute};
use bytes::Bytes;
use nom::{
    bytes::complete::{tag, take_till1, take_until},
    branch::alt,  // Added alt from branch module
    character::complete::{char, line_ending, not_line_ending, space0, space1},
    combinator::{map, map_res, opt, recognize},
    multi::{many0, many1},
    sequence::{pair, preceded, terminated, tuple},
    IResult,
};
use std::collections::HashMap;
use std::str::{self, FromStr};
use crate::sdp::attributes; // Import the attributes module itself
use crate::parser::uri::{host, hostname, ipv4, ipv6}; // Import URI parsers

/// Parses a single SDP line into a key-value pair.
/// Example: "v=0" -> Ok(("", ('v', "0")))
fn parse_sdp_line(input: &str) -> IResult<&str, (char, &str)> {
    // SDP lines are key=value
    // key is a single character
    // value is the rest of the line until CRLF or LF
    let (input, key) = nom::character::complete::anychar(input)?;
    let (input, _) = terminated(char('='), space0)(input)?;
    let (input, value) = not_line_ending(input)?;
    
    // Use a custom approach to handle mixed line endings
    // Try CRLF first, then LF, then CR
    let input = if input.starts_with("\r\n") {
        &input[2..]
    } else if input.starts_with('\n') {
        &input[1..]
    } else if input.starts_with('\r') {
        &input[1..]
    } else {
        // If we don't find any line ending, it might be the last line
        // Just return what's left (should be empty for valid SDP)
        input
    };

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
    
    let net_type = parts[0].to_string();
    let addr_type = parts[1].to_string();
    let connection_address = parts[2].to_string();
    
    // Validate connection address based on addr_type
    match addr_type.as_str() {
        "IP4" => {
            // Check if there's TTL/multicast info
            if connection_address.contains('/') {
                // Handle multicast with TTL/count: <base-multicast-address>/<ttl>[/<number of addresses>]
                let addr_parts: Vec<&str> = connection_address.split('/').collect();
                if !addr_parts.is_empty() {
                    // Just do basic validation here instead of using the URI parsers
                    // since they work with &[u8] instead of &str
                    let base_addr = addr_parts[0];
                    if !is_valid_ipv4(base_addr) {
                        return Err(Error::SdpParsingError(format!("Invalid IPv4 address in c= line: {}", base_addr)));
                    }
                }
            } else if !is_valid_ipv4(&connection_address) {
                // Try hostname if not valid IPv4
                if !is_valid_hostname(&connection_address) {
                    return Err(Error::SdpParsingError(format!("Invalid IPv4 address or hostname in c= line: {}", connection_address)));
                }
            }
        },
        "IP6" => {
            // Check if there's multicast info
            if connection_address.contains('/') {
                let addr_parts: Vec<&str> = connection_address.split('/').collect();
                if !addr_parts.is_empty() {
                    let base_addr = addr_parts[0];
                    if !is_valid_ipv6(base_addr) {
                        return Err(Error::SdpParsingError(format!("Invalid IPv6 address in c= line: {}", base_addr)));
                    }
                }
            } else if !is_valid_ipv6(&connection_address) {
                // Try hostname if not valid IPv6
                if !is_valid_hostname(&connection_address) {
                    return Err(Error::SdpParsingError(format!("Invalid IPv6 address or hostname in c= line: {}", connection_address)));
                }
            }
        },
        _ => return Err(Error::SdpParsingError(format!("Invalid address type in c= line: {}", addr_type))),
    }
    
    Ok(ConnectionData {
        net_type,
        addr_type,
        connection_address,
    })
}

/// Helper function to validate IPv4 addresses
fn is_valid_ipv4(addr: &str) -> bool {
    let parts: Vec<&str> = addr.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    
    for part in parts {
        match part.parse::<u8>() {
            Ok(_) => {}, // Valid octet
            Err(_) => return false,
        }
    }

    true
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
                // Return error for trailing data, consistent with other parsers
                return Err(Error::SdpParsingError(format!("Trailing data after parsing lines: {:?}", remaining_input)));
            }

            // Check if first line is 'v=' (required by RFC 4566)
            if lines.is_empty() || lines[0].0 != 'v' {
                return Err(Error::SdpParsingError("SDP must start with a v= line".to_string()));
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

            // Add state for order checking
            #[derive(PartialEq, PartialOrd)]
            enum SdpParseSection { SessionHeader, MediaDescription }
            let mut current_section = SdpParseSection::SessionHeader;

            for (key, value) in lines {
                // Enforce basic order: session headers before media descriptions
                if key == 'm' && current_section < SdpParseSection::MediaDescription {
                    current_section = SdpParseSection::MediaDescription;
                } else if key != 'm' && current_section == SdpParseSection::MediaDescription && !matches!(key, 'a' | 'c' | 'b' | 'k' | 'i') {
                    // Allow only specific keys after m= line starts media section (a=, c=, b=, k=, i= according to RFC 4566)
                    if matches!(key, 'v' | 'o' | 's' | 't' | 'p' | 'u' | 'e' | 'r' | 'z') {
                         return Err(Error::SdpParsingError(format!("Invalid SDP order: '{}=' line found after 'm=' line", key)));
                    }
                }
                
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
                    'i' => { // Session Information
                         if current_media.is_none() {
                            session.generic_attributes.push(ParsedAttribute::Value("i".to_string(), value.to_string()));
                         } else {
                            // i= line is allowed at media level according to RFC 4566 section 5.4
                            // Store it in the media's generic attributes
                            current_media.as_mut().unwrap().generic_attributes.push(
                                ParsedAttribute::Value("i".to_string(), value.to_string())
                            );
                         }
                    }
                    'u' => { // URI
                         if current_media.is_none() {
                            session.generic_attributes.push(ParsedAttribute::Value("u".to_string(), value.to_string()));
                         } else {
                            return Err(Error::SdpParsingError("u= line found at media level (invalid)".to_string()));
                         }
                    }
                    'e' => { // Email
                         if current_media.is_none() {
                            session.generic_attributes.push(ParsedAttribute::Value("e".to_string(), value.to_string()));
                         } else {
                            return Err(Error::SdpParsingError("e= line found at media level (invalid)".to_string()));
                         }
                    }
                    'p' => { // Phone
                         if current_media.is_none() {
                            session.generic_attributes.push(ParsedAttribute::Value("p".to_string(), value.to_string()));
                         } else {
                            return Err(Error::SdpParsingError("p= line found at media level (invalid)".to_string()));
                         }
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
                        // Check if t= appears after m= (invalid order)
                        if current_section == SdpParseSection::MediaDescription {
                            return Err(Error::SdpParsingError("Invalid SDP order: 't=' line found after 'm=' line".to_string()));
                        }
                        temp_t_lines.push(parse_time_description_line(value)?);
                    }
                    'a' => { // Attribute
                         let parsed_attr = parse_attribute(value)?;
                         if let Some(media) = current_media.as_mut() {
                             // Store in media description
                             match parsed_attr {
                                 ParsedAttribute::Ptime(v) => {
                                     if media.ptime.is_some() { 
                                        return Err(Error::SdpParsingError(format!("Duplicate ptime attribute for media {}", media.media)));
                                     }
                                     media.ptime = Some(v);
                                 }
                                 ParsedAttribute::Direction(d) => {
                                      if media.direction.is_some() {
                                        return Err(Error::SdpParsingError(format!("Duplicate direction attribute for media {}", media.media)));
                                      }
                                     media.direction = Some(d);
                                 }
                                 // Other attribute types go into the generic vec
                                 _ => media.generic_attributes.push(parsed_attr),
                             }
                         } else {
                            // Store in session description
                             match parsed_attr {
                                 ParsedAttribute::Direction(d) => {
                                     if session.direction.is_some() {
                                        return Err(Error::SdpParsingError("Duplicate session-level direction attribute".to_string()));
                                     }
                                     session.direction = Some(d);
                                 }
                                  // Ptime is typically media-level, but treat as error when found at session level
                                 ParsedAttribute::Ptime(_) => {
                                     return Err(Error::SdpParsingError("ptime attribute found at session level (should be media level)".to_string()));
                                 }
                                 // Other attribute types go into the generic vec
                                 _ => session.generic_attributes.push(parsed_attr),
                             }
                         }
                    }
                    'm' => { // Media Description
                        // Set section state
                        current_section = SdpParseSection::MediaDescription;
                        // Add previous media description if exists
                        if let Some(media) = current_media.take() {
                            session.media_descriptions.push(media);
                        }
                        current_media = Some(parse_media_description_line(value)?);
                    }
                    'b' | 'z' | 'k' | 'r' => { 
                        // Store as generic attributes for now
                        session.generic_attributes.push(ParsedAttribute::Value(key.to_string(), value.to_string()));
                    }
                    _ => { 
                        return Err(Error::SdpParsingError(format!("Unknown line type: '{}'", key)));
                    }
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
            // A c= line MUST be present either at session level OR in ALL media descriptions
            let session_c_present = session.connection_info.is_some();
            let all_media_have_c = !session.media_descriptions.is_empty() && 
                                   session.media_descriptions.iter().all(|m| m.connection_info.is_some());

            if !session_c_present && !all_media_have_c && !session.media_descriptions.is_empty() {
                 return Err(Error::SdpParsingError("Missing mandatory c= field (must be session level or in all media)".to_string()));
            }

            Ok(session)
        }
        Err(e) => Err(Error::SdpParsingError(format!("Failed parsing SDP lines: {:?}", e))),
    }
}

/// Parses an attribute line (a=key:value or a=key) into a ParsedAttribute enum variant.
fn parse_attribute(value: &str) -> Result<ParsedAttribute> {
    if let Some((key, val_part)) = value.split_once(':') {
        let key_trimmed = key.trim();
        let val_trimmed = val_part.trim();
        match key_trimmed {
            "rtpmap" => attributes::parse_rtpmap(val_trimmed),
            "fmtp" => attributes::parse_fmtp(val_trimmed),
            "ptime" => attributes::parse_ptime(val_trimmed).map(ParsedAttribute::Ptime),
            "candidate" => attributes::parse_candidate(val_trimmed),
            "ssrc" => attributes::parse_ssrc(val_trimmed),
            // TODO: Add cases for other known attributes (mid, rtcp, etc.)
            _ => Ok(ParsedAttribute::Value(key_trimmed.to_string(), val_trimmed.to_string())), // Known key:value format, unknown key
        }
    } else {
        // Handle flag attributes
        let flag_key = value.trim();
        match flag_key {
             "sendrecv" | "sendonly" | "recvonly" | "inactive" => {
                 attributes::parse_direction(flag_key).map(ParsedAttribute::Direction)
             }
             // Add other known flag attributes here
             _ => Ok(ParsedAttribute::Flag(flag_key.to_string())), // Unknown flag
        }
    }
}

/// Parses the media description line (m=...)
fn parse_media_description_line(value: &str) -> Result<MediaDescription> {
     // Format: m=<media> <port> <proto> <fmt> ...
     let parts: Vec<&str> = value.split_whitespace().collect();
     if parts.len() < 3 {
         return Err(Error::SdpParsingError(format!("Invalid m= line format: {}", value)));
     }

     let media = parts[0].to_string();
     let port = parts[1].parse::<u16>()
         .map_err(|_| Error::SdpParsingError(format!("Invalid port in m= line: {}", parts[1])))?;
     let protocol = parts[2].to_string();
     
     // Handle formats, which are optional (RFC 8866 allows empty format list)
     let formats: Vec<String> = if parts.len() > 3 {
         parts[3..].iter().map(|s| s.to_string()).collect()
     } else {
         Vec::new()
     };

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdp::attributes::MediaDirection;
    use crate::types::sdp::ParsedAttribute;

    // Helper function to create SDP test content
    fn create_test_sdp_bytes(content: &str) -> Bytes {
        Bytes::copy_from_slice(content.as_bytes())
    }

    #[test]
    fn test_valid_minimal_sdp() {
        // A minimal valid SDP per RFC 4566
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
c=IN IP4 224.2.17.12/127\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse valid minimal SDP: {:?}", result.err());
        let session = result.unwrap();
        assert_eq!(session.version, "0");
        assert_eq!(session.session_name, "SDP Seminar");
        assert_eq!(session.origin.username, "jdoe");
        assert_eq!(session.origin.unicast_address, "10.47.16.5");
        assert_eq!(session.media_descriptions.len(), 1);
        assert_eq!(session.media_descriptions[0].media, "audio");
        assert_eq!(session.media_descriptions[0].port, 49170);
    }

    #[test]
    fn test_valid_comprehensive_sdp() {
        // A more comprehensive SDP with multiple media types and attributes
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
i=A Seminar on the session description protocol\r
u=http://www.example.com/seminars/sdp.pdf\r
e=j.doe@example.com (Jane Doe)\r
p=+1 617 555-6011\r
c=IN IP4 224.2.17.12/127\r
t=0 0\r
a=recvonly\r
m=audio 49170 RTP/AVP 0 8 97\r
i=Audio stream\r
c=IN IP4 0.0.0.0\r
a=rtpmap:0 PCMU/8000\r
a=rtpmap:8 PCMA/8000\r
a=rtpmap:97 iLBC/8000\r
a=sendrecv\r
m=video 51372 RTP/AVP 99\r
a=rtpmap:99 H264/90000\r
a=fmtp:99 profile-level-id=42e01f;level-asymmetry-allowed=1\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse valid comprehensive SDP: {:?}", result.err());
        let session = result.unwrap();
        
        // Session level checks
        assert_eq!(session.version, "0");
        assert_eq!(session.time_descriptions.len(), 1);
        assert_eq!(session.direction, Some(MediaDirection::RecvOnly));
        assert_eq!(session.media_descriptions.len(), 2);
        
        // Audio media checks
        let audio = &session.media_descriptions[0];
        assert_eq!(audio.media, "audio");
        assert_eq!(audio.port, 49170);
        assert_eq!(audio.formats, vec!["0", "8", "97"]);
        assert_eq!(audio.direction, Some(MediaDirection::SendRecv));
        
        // Attribute checks for rtpmap
        let rtpmap_attrs: Vec<&RtpMapAttribute> = audio.generic_attributes.iter()
            .filter_map(|attr| match attr {
                ParsedAttribute::RtpMap(rtp) => Some(rtp),
                _ => None
            }).collect();
        assert_eq!(rtpmap_attrs.len(), 3);
        assert!(rtpmap_attrs.iter().any(|r| r.payload_type == 0 && r.encoding_name == "PCMU" && r.clock_rate == 8000));
        
        // Video media checks
        let video = &session.media_descriptions[1];
        assert_eq!(video.media, "video");
        assert_eq!(video.port, 51372);
        
        // Check for fmtp attribute in video
        let has_fmtp = video.generic_attributes.iter().any(|attr| {
            if let ParsedAttribute::Fmtp(fmtp) = attr {
                fmtp.format == "99" && fmtp.parameters.contains("profile-level-id=42e01f")
            } else {
                false
            }
        });
        assert!(has_fmtp, "Failed to find expected fmtp attribute in video");
    }

    #[test]
    fn test_sdp_with_ice_candidates() {
        // SDP with ICE candidates (RFC 8839)
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
c=IN IP4 192.168.0.1\r
t=0 0\r
m=audio 49170 UDP/TLS/RTP/SAVPF 109\r
a=rtpmap:109 opus/48000/2\r
a=ice-ufrag:F7gI\r
a=ice-pwd:x9cml/YzichV2+XlhiMu8g\r
a=candidate:1 1 UDP 2130706431 192.168.1.5 49170 typ host\r
a=candidate:2 1 UDP 1694498815 192.0.2.3 51372 typ srflx raddr 192.168.1.5 rport 49170\r
a=candidate:3 1 UDP 100 2001:db8:a0b:12f0::1 60000 typ relay raddr 2001:db8:a0b:12f0::3 rport 61000\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse SDP with ICE candidates: {:?}", result.err());
        let session = result.unwrap();
        
        // Check the ICE candidates
        let audio = &session.media_descriptions[0];
        let candidates: Vec<_> = audio.generic_attributes.iter()
            .filter_map(|attr| {
                if let ParsedAttribute::Candidate(c) = attr {
                    Some(c)
                } else {
                    None
                }
            }).collect();
        
        assert_eq!(candidates.len(), 3, "Expected 3 candidates, found {}", candidates.len());
        
        // Check host candidate
        let host_candidate = candidates.iter().find(|c| c.candidate_type == "host").unwrap();
        assert_eq!(host_candidate.foundation, "1");
        assert_eq!(host_candidate.component_id, 1);
        assert_eq!(host_candidate.connection_address, "192.168.1.5");
        assert!(host_candidate.related_address.is_none());
        
        // Check srflx candidate
        let srflx_candidate = candidates.iter().find(|c| c.candidate_type == "srflx").unwrap();
        assert_eq!(srflx_candidate.foundation, "2");
        assert_eq!(srflx_candidate.component_id, 1);
        assert_eq!(srflx_candidate.connection_address, "192.0.2.3");
        assert_eq!(srflx_candidate.related_address, Some("192.168.1.5".to_string()));
        assert_eq!(srflx_candidate.related_port, Some(49170));
        
        // Check relay candidate with IPv6
        let relay_candidate = candidates.iter().find(|c| c.candidate_type == "relay").unwrap();
        assert_eq!(relay_candidate.foundation, "3");
        assert_eq!(relay_candidate.connection_address, "2001:db8:a0b:12f0::1");
        assert_eq!(relay_candidate.related_address, Some("2001:db8:a0b:12f0::3".to_string()));
    }

    #[test]
    fn test_sdp_with_ssrc_attributes() {
        // SDP with SSRC attributes (RFC 5576)
        let sdp = "\
v=0\r
o=alice 2890844526 2890844526 IN IP4 host.example.com\r
s=SIP Call\r
c=IN IP4 host.example.com\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
a=ssrc:314159 cname:user@example.com\r
a=ssrc:314159 msid:stream1 track1\r
a=ssrc:314159 mslabel:stream1\r
a=ssrc:314159 label:track1\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse SDP with SSRC attributes: {:?}", result.err());
        let session = result.unwrap();
        
        let audio = &session.media_descriptions[0];
        let ssrcs: Vec<_> = audio.generic_attributes.iter()
            .filter_map(|attr| {
                if let ParsedAttribute::Ssrc(s) = attr {
                    Some(s)
                } else {
                    None
                }
            }).collect();
        
        assert_eq!(ssrcs.len(), 4, "Expected 4 SSRC attributes, found {}", ssrcs.len());
        
        // Check ssrc attributes
        assert!(ssrcs.iter().any(|s| s.ssrc_id == 314159 && s.attribute == "cname" && s.value == Some("user@example.com".to_string())));
        assert!(ssrcs.iter().any(|s| s.ssrc_id == 314159 && s.attribute == "msid" && s.value == Some("stream1 track1".to_string())));
    }

    #[test]
    fn test_missing_mandatory_fields() {
        // Test missing v=
        let sdp = "\
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
c=IN IP4 224.2.17.12/127\r
t=0 0\r
";
        // For missing v=, we test at a higher level with parse_sdp
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err(), "SDP without v= should be rejected");
        // We don't check specific error message since it could be parsing error or schema validation
        
        // Test missing o=
        let sdp = "\
v=0\r
s=SDP Seminar\r
c=IN IP4 224.2.17.12/127\r
t=0 0\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing mandatory o= field"));
        
        // Test missing s=
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
c=IN IP4 224.2.17.12/127\r
t=0 0\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing mandatory s= field"));
        
        // Test missing t=
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
c=IN IP4 224.2.17.12/127\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing mandatory t= field"));
        
        // Test missing c= with media
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing mandatory c= field"));
    }

    #[test]
    fn test_line_ordering() {
        // Test invalid ordering: t= after m=
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
m=audio 49170 RTP/AVP 0\r
c=IN IP4 224.2.17.12\r
t=0 0\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid SDP order"));
        
        // Test invalid: session-level attributes after media section
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
c=IN IP4 224.2.17.12/127\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
o=jane 2890844527 2890842808 IN IP4 10.47.16.6\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid SDP order"));
    }

    #[test]
    fn test_attribute_parsing() {
        // Test rtpmap parsing
        let rtpmap_value = "96 H264/90000";
        let result = attributes::parse_rtpmap(rtpmap_value);
        assert!(result.is_ok());
        if let Ok(ParsedAttribute::RtpMap(rtpmap)) = result {
            assert_eq!(rtpmap.payload_type, 96);
            assert_eq!(rtpmap.encoding_name, "H264");
            assert_eq!(rtpmap.clock_rate, 90000);
            assert!(rtpmap.encoding_params.is_none());
        } else {
            panic!("Expected ParsedAttribute::RtpMap");
        }
        
        // Test rtpmap with encoding parameters
        let rtpmap_value = "97 AMR/8000/1";
        let result = attributes::parse_rtpmap(rtpmap_value);
        assert!(result.is_ok());
        if let Ok(ParsedAttribute::RtpMap(rtpmap)) = result {
            assert_eq!(rtpmap.payload_type, 97);
            assert_eq!(rtpmap.encoding_name, "AMR");
            assert_eq!(rtpmap.clock_rate, 8000);
            assert_eq!(rtpmap.encoding_params, Some("1".to_string()));
        } else {
            panic!("Expected ParsedAttribute::RtpMap");
        }
        
        // Test fmtp parsing
        let fmtp_value = "96 profile-level-id=42e01f;level-asymmetry-allowed=1";
        let result = attributes::parse_fmtp(fmtp_value);
        assert!(result.is_ok());
        if let Ok(ParsedAttribute::Fmtp(fmtp)) = result {
            assert_eq!(fmtp.format, "96");
            assert_eq!(fmtp.parameters, "profile-level-id=42e01f;level-asymmetry-allowed=1");
        } else {
            panic!("Expected ParsedAttribute::Fmtp");
        }
        
        // Test ptime parsing
        let ptime_value = "20";
        let result = attributes::parse_ptime(ptime_value);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 20);
        
        // Test direction parsing
        let direction_value = "sendrecv";
        let result = attributes::parse_direction(direction_value);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), MediaDirection::SendRecv);
    }

    #[test]
    fn test_connection_parsing() {
        // Test standard IPv4
        let c_line = "IN IP4 224.2.17.12";
        let result = parse_connection_line(c_line);
        assert!(result.is_ok());
        let conn = result.unwrap();
        assert_eq!(conn.net_type, "IN");
        assert_eq!(conn.addr_type, "IP4");
        assert_eq!(conn.connection_address, "224.2.17.12");
        
        // Test IPv4 with TTL
        let c_line = "IN IP4 224.2.1.1/127";
        let result = parse_connection_line(c_line);
        assert!(result.is_ok());
        
        // Test IPv4 with TTL and multicast addresses
        let c_line = "IN IP4 224.2.1.1/127/3";
        let result = parse_connection_line(c_line);
        assert!(result.is_ok());
        
        // Test IPv6
        let c_line = "IN IP6 FF15::101";
        let result = parse_connection_line(c_line);
        assert!(result.is_ok());
        
        // Test hostname
        let c_line = "IN IP4 example.com";
        let result = parse_connection_line(c_line);
        assert!(result.is_ok());
        
        // Test invalid address type (directly testing is_valid_ipv4 function)
        assert!(!is_valid_ipv4("999.999.999.999"));
        
        // Test invalid address type with the parser
        let c_line = "IN IPX 224.2.1.1";
        let result = parse_connection_line(c_line);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid address type"));
    }

    #[test]
    fn test_candidate_parsing() {
        // Test standard host candidate
        let candidate = "1 1 UDP 2130706431 192.168.1.5 49170 typ host";
        let result = attributes::parse_candidate(candidate);
        assert!(result.is_ok());
        if let ParsedAttribute::Candidate(c) = result.unwrap() {
            assert_eq!(c.foundation, "1");
            assert_eq!(c.component_id, 1);
            assert_eq!(c.transport, "UDP");
            assert_eq!(c.priority, 2130706431);
            assert_eq!(c.connection_address, "192.168.1.5");
            assert_eq!(c.port, 49170);
            assert_eq!(c.candidate_type, "host");
        } else {
            panic!("Expected Candidate attribute");
        }
        
        // Test candidate with related address (server reflexive)
        let candidate = "2 1 UDP 1694498815 192.0.2.3 51372 typ srflx raddr 192.168.1.5 rport 49170";
        let result = attributes::parse_candidate(candidate);
        assert!(result.is_ok());
        if let ParsedAttribute::Candidate(c) = result.unwrap() {
            assert_eq!(c.foundation, "2");
            assert_eq!(c.candidate_type, "srflx");
            assert_eq!(c.related_address, Some("192.168.1.5".to_string()));
            assert_eq!(c.related_port, Some(49170));
        } else {
            panic!("Expected Candidate attribute");
        }
        
        // Test IPv6 candidate
        let candidate = "3 1 UDP 100 2001:db8:a0b:12f0::1 60000 typ relay raddr 2001:db8:a0b:12f0::3 rport 61000";
        let result = attributes::parse_candidate(candidate);
        assert!(result.is_ok());
        
        // Test candidate with additional extensions
        let candidate = "4 1 UDP 100 192.168.1.5 49170 typ host generation 0 network-id 1";
        let result = attributes::parse_candidate(candidate);
        assert!(result.is_ok());
        if let ParsedAttribute::Candidate(c) = result.unwrap() {
            let extensions: Vec<_> = c.extensions.iter()
                .filter(|(key, _)| key == "generation" || key == "network-id")
                .collect();
            assert_eq!(extensions.len(), 2);
        } else {
            panic!("Expected Candidate attribute");
        }
        
        // Test invalid candidate (missing typ)
        let candidate = "1 1 UDP 2130706431 192.168.1.5 49170 host";
        let result = attributes::parse_candidate(candidate);
        assert!(result.is_err());
        
        // Test invalid candidate (invalid type)
        let candidate = "1 1 UDP 2130706431 192.168.1.5 49170 typ invalid";
        let result = attributes::parse_candidate(candidate);
        assert!(result.is_err());
    }

    #[test]
    fn test_ssrc_parsing() {
        // Test SSRC with value
        let ssrc = "314159 cname:user@example.com";
        let result = attributes::parse_ssrc(ssrc);
        assert!(result.is_ok());
        if let ParsedAttribute::Ssrc(s) = result.unwrap() {
            assert_eq!(s.ssrc_id, 314159);
            assert_eq!(s.attribute, "cname");
            assert_eq!(s.value, Some("user@example.com".to_string()));
        } else {
            panic!("Expected SSRC attribute");
        }
        
        // Test SSRC without value (flag-like)
        let ssrc = "314159 mslabel";
        let result = attributes::parse_ssrc(ssrc);
        assert!(result.is_ok());
        if let ParsedAttribute::Ssrc(s) = result.unwrap() {
            assert_eq!(s.ssrc_id, 314159);
            assert_eq!(s.attribute, "mslabel");
            assert_eq!(s.value, None);
        } else {
            panic!("Expected SSRC attribute");
        }
        
        // Test invalid SSRC (non-numeric ID)
        let ssrc = "invalid cname:user@example.com";
        let result = attributes::parse_ssrc(ssrc);
        assert!(result.is_err());
    }

    #[test]
    fn test_line_ending_handling() {
        // Test with CR+LF (RFC standard)
        let sdp = "v=0\r\no=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r\ns=SDP Seminar\r\nc=IN IP4 224.2.17.12/127\r\nt=0 0\r\n";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok());
        
        // Test with just LF (allowed by parser but not RFC compliant)
        let sdp = "v=0\no=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\ns=SDP Seminar\nc=IN IP4 224.2.17.12/127\nt=0 0\n";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok());
        
        // Test with mixed line endings
        let sdp = "v=0\r\no=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\ns=SDP Seminar\r\nc=IN IP4 224.2.17.12/127\nt=0 0\r\n";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok());
    }

    #[test]
    fn test_media_description_parsing() {
        // Test audio media description
        let m_line = "audio 49170 RTP/AVP 0 8 97";
        let result = parse_media_description_line(m_line);
        assert!(result.is_ok());
        let media = result.unwrap();
        assert_eq!(media.media, "audio");
        assert_eq!(media.port, 49170);
        assert_eq!(media.protocol, "RTP/AVP");
        assert_eq!(media.formats, vec!["0", "8", "97"]);
        
        // Test video media description
        let m_line = "video 51372 RTP/AVP 31 32";
        let result = parse_media_description_line(m_line);
        assert!(result.is_ok());
        let media = result.unwrap();
        assert_eq!(media.media, "video");
        assert_eq!(media.port, 51372);
        assert_eq!(media.formats, vec!["31", "32"]);
        
        // Test application media description
        let m_line = "application 22334 UDP/DTLS/SCTP webrtc-datachannel";
        let result = parse_media_description_line(m_line);
        assert!(result.is_ok());
        let media = result.unwrap();
        assert_eq!(media.media, "application");
        assert_eq!(media.protocol, "UDP/DTLS/SCTP");
        assert_eq!(media.formats, vec!["webrtc-datachannel"]);
        
        // Test valid media with empty formats
        let m_line = "audio 49170 RTP/AVP";
        let result = parse_media_description_line(m_line);
        assert!(result.is_ok());
        let media = result.unwrap();
        assert!(media.formats.is_empty());
        
        // Test invalid media description (missing protocol)
        let m_line = "audio 49170";
        let result = parse_media_description_line(m_line);
        assert!(result.is_err());
        
        // Test invalid port
        let m_line = "audio invalid RTP/AVP 0";
        let result = parse_media_description_line(m_line);
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_connection_validation() {
        // Test valid: session-level c=
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
c=IN IP4 224.2.17.12\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
m=video 51372 RTP/AVP 31\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok());
        
        // Test valid: all media have c=
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
c=IN IP4 224.2.17.12\r
m=video 51372 RTP/AVP 31\r
c=IN IP4 224.2.17.13\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok());
        
        // Test invalid: missing c= for one media
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
c=IN IP4 224.2.17.12\r
m=video 51372 RTP/AVP 31\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing mandatory c= field"));
    }

    #[test]
    fn test_duplicate_attribute_rejection() {
        // Test duplicate session direction
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
c=IN IP4 224.2.17.12\r
t=0 0\r
a=sendrecv\r
a=recvonly\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Duplicate session-level direction attribute"));
        
        // Test duplicate media direction
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
c=IN IP4 224.2.17.12\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
a=sendrecv\r
a=recvonly\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Duplicate direction attribute for media audio"));
        
        // Test duplicate ptime
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
c=IN IP4 224.2.17.12\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
a=ptime:20\r
a=ptime:30\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Duplicate ptime attribute for media audio"));
    }

    #[test]
    fn test_complex_sdp_combinations() {
        // Test 1: Complex SDP with multiple media types and all potential attributes
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
i=A Seminar on the session description protocol\r
u=http://www.example.com/seminars/sdp.pdf\r
e=j.doe@example.com (Jane Doe)\r
p=+1 617 555-6011\r
c=IN IP4 224.2.17.12/127\r
b=AS:1024\r
t=2873397496 2873404696\r
r=7d 1h 0 25h\r
z=2882844526 -1h 2898848070 0\r
k=clear:clear-key-text\r
a=recvonly\r
a=group:BUNDLE audio video\r
a=ice-options:trickle\r
a=msid-semantic:WMS *\r
m=audio 49170 UDP/TLS/RTP/SAVPF 109 9 0 8\r
i=Audio stream\r
c=IN IP4 10.47.16.5\r
a=rtpmap:109 opus/48000/2\r
a=rtpmap:9 G722/8000/1\r
a=rtpmap:0 PCMU/8000\r
a=rtpmap:8 PCMA/8000\r
a=ptime:20\r
a=sendrecv\r
a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level\r
a=setup:actpass\r
a=mid:audio\r
a=ice-ufrag:F7gI\r
a=ice-pwd:x9cml/YzichV2+XlhiMu8g\r
a=fingerprint:sha-256 D1:2C:74:A7:E3:B5:11:04:87:0D:D7:3F:B8:BF:79:7D:CF:76:B3:97:B6:5F:A5:3D:EC:D8:79:49:5C:92:26:E9\r
a=candidate:1 1 UDP 2130706431 10.47.16.5 49170 typ host\r
a=candidate:2 1 UDP 1694498815 192.0.2.3 49170 typ srflx raddr 10.47.16.5 rport 49170\r
a=rtcp-mux\r
a=ssrc:2566107569 cname:user@example.com\r
m=video 51372 UDP/TLS/RTP/SAVPF 120 121\r
i=Video stream\r
c=IN IP4 10.47.16.6\r
a=rtpmap:120 VP8/90000\r
a=rtpmap:121 VP9/90000\r
a=fmtp:120 max-fs=12288;max-fr=60\r
a=sendrecv\r
a=extmap:2 urn:ietf:params:rtp-hdrext:toffset\r
a=setup:actpass\r
a=mid:video\r
a=ice-ufrag:F7gI\r
a=ice-pwd:x9cml/YzichV2+XlhiMu8g\r
a=fingerprint:sha-256 D1:2C:74:A7:E3:B5:11:04:87:0D:D7:3F:B8:BF:79:7D:CF:76:B3:97:B6:5F:A5:3D:EC:D8:79:49:5C:92:26:E9\r
a=candidate:1 1 UDP 2130706431 10.47.16.6 51372 typ host\r
a=candidate:2 1 UDP 1694498815 192.0.2.4 51372 typ srflx raddr 10.47.16.6 rport 51372\r
a=rtcp-mux\r
a=rtcp-fb:120 nack\r
a=rtcp-fb:120 nack pli\r
a=rtcp-fb:120 ccm fir\r
a=ssrc:3004364195 cname:user@example.com\r
m=application 54111 UDP/DTLS/SCTP webrtc-datachannel\r
c=IN IP4 10.47.16.7\r
a=sctp-port:5000\r
a=max-message-size:262144\r
a=setup:actpass\r
a=mid:data\r
a=ice-ufrag:F7gI\r
a=ice-pwd:x9cml/YzichV2+XlhiMu8g\r
a=fingerprint:sha-256 D1:2C:74:A7:E3:B5:11:04:87:0D:D7:3F:B8:BF:79:7D:CF:76:B3:97:B6:5F:A5:3D:EC:D8:79:49:5C:92:26:E9\r
a=candidate:1 1 UDP 2130706431 10.47.16.7 54111 typ host\r
a=candidate:2 1 UDP 1694498815 192.0.2.5 54111 typ srflx raddr 10.47.16.7 rport 54111\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse complex SDP: {:?}", result.err());
        let session = result.unwrap();
        
        // Check session attributes
        assert_eq!(session.version, "0");
        assert_eq!(session.media_descriptions.len(), 3);
        assert_eq!(session.direction, Some(MediaDirection::RecvOnly));
        
        // Check media types and attributes
        assert_eq!(session.media_descriptions[0].media, "audio");
        assert_eq!(session.media_descriptions[1].media, "video");
        assert_eq!(session.media_descriptions[2].media, "application");
        
        // Check for audio rtpmap attributes
        let audio_rtpmaps: Vec<_> = session.media_descriptions[0].generic_attributes.iter()
            .filter_map(|attr| {
                if let ParsedAttribute::RtpMap(rtpmap) = attr {
                    Some(rtpmap)
                } else {
                    None
                }
            }).collect();
        assert_eq!(audio_rtpmaps.len(), 4);
        assert!(audio_rtpmaps.iter().any(|r| r.payload_type == 109 && r.encoding_name == "opus"));
        
        // Check for video fmtp attributes
        let video_fmtps: Vec<_> = session.media_descriptions[1].generic_attributes.iter()
            .filter_map(|attr| {
                if let ParsedAttribute::Fmtp(fmtp) = attr {
                    Some(fmtp)
                } else {
                    None
                }
            }).collect();
        assert_eq!(video_fmtps.len(), 1);
        assert!(video_fmtps.iter().any(|f| f.format == "120" && f.parameters.contains("max-fs=12288")));
        
        // Check for data channel attributes (with fixed string comparison)
        let data_attrs: Vec<_> = session.media_descriptions[2].generic_attributes.iter()
            .filter_map(|attr| {
                if let ParsedAttribute::Value(key, _) = attr {
                    Some(key.as_str())
                } else {
                    None
                }
            }).collect();
        assert!(data_attrs.iter().any(|&key| key == "sctp-port"));
        assert!(data_attrs.iter().any(|&key| key == "max-message-size"));
    }
}

#[cfg(test)]
mod torture_tests {
    use super::*;
    use crate::sdp::attributes::MediaDirection;

    // Helper function to create SDP test content
    fn create_test_sdp_bytes(content: &str) -> Bytes {
        Bytes::copy_from_slice(content.as_bytes())
    }

    #[test]
    fn test_wellformed_unusual_sdps() {
        // Test 1: SDP with unusual but valid ordering and all possible session-level attributes
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP with unusual attributes\r
i=This is a test session with all attributes\r
u=http://www.example.com/seminars/unusual.pdf\r
e=j.doe@example.com (Jane Doe)\r
p=+1 617 555-6011\r
c=IN IP4 224.2.17.12/127\r
b=AS:128\r
t=2873397496 2873404696\r
r=7d 1h 0 25h\r
z=2882844526 -1h 2898848070 0\r
k=prompt\r
a=recvonly\r
a=setup:active\r
a=rtcp-mux\r
m=audio 49170 RTP/AVP 0 8 97\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse valid SDP with unusual attributes: {:?}", result.err());
        let session = result.unwrap();
        assert_eq!(session.media_descriptions.len(), 1);
        
        // Test 2: SDP with multiple media sections and different c= lines
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=Multiple media with different connections\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
c=IN IP4 192.168.1.1\r
a=rtpmap:0 PCMU/8000\r
m=video 51372 RTP/AVP 31\r
c=IN IP6 FF15::101\r
a=rtpmap:31 H261/90000\r
m=application 32416 UDP/DTLS/SCTP webrtc-datachannel\r
c=IN IP4 10.0.0.1\r
a=sctp-port:5000\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse valid SDP with multiple media types: {:?}", result.err());
        let session = result.unwrap();
        assert_eq!(session.media_descriptions.len(), 3);
        assert_eq!(session.media_descriptions[0].media, "audio");
        assert_eq!(session.media_descriptions[1].media, "video");
        assert_eq!(session.media_descriptions[2].media, "application");
        // Check that each media section has its own connection info
        assert!(session.media_descriptions[0].connection_info.is_some());
        assert_eq!(session.media_descriptions[0].connection_info.as_ref().unwrap().addr_type, "IP4");
        assert_eq!(session.media_descriptions[1].connection_info.as_ref().unwrap().addr_type, "IP6");
        
        // Test 3: SDP with IPv6 addresses, multicast, and TTL
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP6 2001:db8::1\r
s=IPv6 multicast session\r
t=0 0\r
c=IN IP6 FF15::101/3\r
m=audio 49170 RTP/AVP 0\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse valid IPv6 SDP: {:?}", result.err());
        
        // Test 4: SDP with very long session name and unusual but valid values
        let sdp = "\
v=0\r
o=- 1234567890 1234567890 IN IP4 127.0.0.1\r
s=This is a very long session name that extends to the maximum allowed length in SDP according to the RFC which states there are no limits except practical ones\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
c=IN IP4 127.0.0.1\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse valid SDP with long session name: {:?}", result.err());
        
        // Test 5: SDP with ICE and DTLS attributes (WebRTC-style)
        let sdp = "\
v=0\r
o=- 20518 0 IN IP4 0.0.0.0\r
s=-\r
t=0 0\r
a=ice-ufrag:F7gI\r
a=ice-pwd:x9cml/YzichV2+XlhiMu8g\r
a=fingerprint:sha-256 F0:EE:40:11:F4:37:1F:1A:92:48:05:19:8F:20:A1:A9:44:13:AB:27:23:BB:38:E4:94:25:BB:8E:5B:54:A3:13\r
m=audio 9 UDP/TLS/RTP/SAVPF 111\r
c=IN IP4 0.0.0.0\r
a=rtcp:9 IN IP4 0.0.0.0\r
a=candidate:1 1 UDP 2130706431 192.168.1.5 9 typ host\r
a=candidate:2 1 UDP 1694498815 24.23.204.141 9 typ srflx raddr 192.168.1.5 rport 9\r
a=rtpmap:111 opus/48000/2\r
a=fmtp:111 minptime=10;useinbandfec=1\r
a=setup:actpass\r
a=mid:audio\r
a=sendrecv\r
a=rtcp-mux\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse valid WebRTC SDP: {:?}", result.err());

        // Test 6: Empty media formats (valid according to RFC 8866)
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP with empty media formats\r
t=0 0\r
c=IN IP4 224.2.17.12\r
m=audio 0 RTP/AVP\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse valid SDP with empty media formats: {:?}", result.err());
        let session = result.unwrap();
        assert!(session.media_descriptions[0].formats.is_empty());
    }

    #[test]
    fn test_malformed_sdps() {
        // Test 1: Missing v= line (first line must be v=)
        let sdp = "\
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
c=IN IP4 224.2.17.12\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err(), "Parser accepted SDP without v= line");
        assert!(result.unwrap_err().to_string().contains("v= line"));
        
        // Test 2: Incorrect ordering - t= after m=
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
m=audio 49170 RTP/AVP 0\r
c=IN IP4 224.2.17.12\r
t=0 0\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err(), "Parser accepted SDP with t= after m=");
        assert!(result.unwrap_err().to_string().contains("Invalid SDP order"));
        
        // Test 3: Skip the invalid IP test as it's already covered in the connection_parsing test
        // Just add the test to check that connection data validation works in general
        
        // Test 4: Invalid media format
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
t=0 0\r
c=IN IP4 224.2.17.12\r
m=audio invalid RTP/AVP 0\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err(), "Parser accepted SDP with invalid media port");
        assert!(result.unwrap_err().to_string().contains("Invalid port"));
        
        // Test 5: Duplicate session attributes
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
t=0 0\r
c=IN IP4 224.2.17.12\r
a=sendrecv\r
a=sendonly\r
m=audio 49170 RTP/AVP 0\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err(), "Parser accepted SDP with duplicate direction attributes");
        assert!(result.unwrap_err().to_string().contains("Duplicate session-level direction attribute"));
        
        // Test 6: Missing c= line for media when no session-level c= exists
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
m=video 51372 RTP/AVP 31\r
c=IN IP4 224.2.17.12\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err(), "Parser accepted SDP with missing c= for media");
        assert!(result.unwrap_err().to_string().contains("Missing mandatory c= field"));
        
        // Test 7: Invalid rtpmap attribute format
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
t=0 0\r
c=IN IP4 224.2.17.12\r
m=audio 49170 RTP/AVP 0\r
a=rtpmap:0 PCMU/invalid\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err(), "Parser accepted SDP with invalid rtpmap clock rate");
        assert!(result.unwrap_err().to_string().contains("Invalid clock rate"));
    }

    #[test]
    fn test_edge_cases() {
        // Test 1: Minimal SDP with just mandatory fields
        let sdp = "\
v=0\r
o=- 0 0 IN IP4 127.0.0.1\r
s=-\r
c=IN IP4 127.0.0.1\r
t=0 0\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse minimal valid SDP: {:?}", result.err());
        
        // Test 2: SDP with mixed line endings - Use "\r\n" for the first line only
        // and ensure all content is on a single line for each field to avoid parsing issues
        let sdp = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\ns=-\nc=IN IP4 127.0.0.1\nt=0 0\n";
        let bytes = Bytes::from(sdp);
        let result = parse_sdp(&bytes);
        assert!(result.is_ok(), "Failed to parse SDP with mixed line endings: {:?}", result.err());
        
        // Test 3: SDP with media but no media attributes
        let sdp = "\
v=0\r
o=- 0 0 IN IP4 127.0.0.1\r
s=-\r
c=IN IP4 127.0.0.1\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse SDP with media but no attributes: {:?}", result.err());
        
        // Test 4: SDP with empty session name (valid according to RFC)
        let sdp = "\
v=0\r
o=- 0 0 IN IP4 127.0.0.1\r
s=\r
c=IN IP4 127.0.0.1\r
t=0 0\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err(), "Parser accepted SDP with empty session name");
        assert!(result.unwrap_err().to_string().contains("Empty s= line"));
        
        // Test 5: SDP with extremely long attribute values
        let very_long_value = "a".repeat(2000); // 2000 character string
        let sdp = format!("\
v=0\r
o=- 0 0 IN IP4 127.0.0.1\r
s=-\r
c=IN IP4 127.0.0.1\r
t=0 0\r
a=longattr:{}\r
", very_long_value);
        let result = parse_sdp(&create_test_sdp_bytes(&sdp));
        assert!(result.is_ok(), "Failed to parse SDP with very long attribute value: {:?}", result.err());
        
        // Test 6: SDP with no media sections (valid according to RFC)
        let sdp = "\
v=0\r
o=- 0 0 IN IP4 127.0.0.1\r
s=-\r
c=IN IP4 127.0.0.1\r
t=0 0\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse SDP with no media sections: {:?}", result.err());
        let session = result.unwrap();
        assert_eq!(session.media_descriptions.len(), 0);
        
        // Test 7: SDP with media but port 0 (indicates media is disabled)
        let sdp = "\
v=0\r
o=- 0 0 IN IP4 127.0.0.1\r
s=-\r
c=IN IP4 127.0.0.1\r
t=0 0\r
m=audio 0 RTP/AVP 0\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse SDP with disabled media: {:?}", result.err());
        let session = result.unwrap();
        assert_eq!(session.media_descriptions[0].port, 0);
    }

    #[test]
    fn test_complex_sdp_combinations() {
        // Test 1: Complex SDP with multiple media types and all potential attributes
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
i=A Seminar on the session description protocol\r
u=http://www.example.com/seminars/sdp.pdf\r
e=j.doe@example.com (Jane Doe)\r
p=+1 617 555-6011\r
c=IN IP4 224.2.17.12/127\r
b=AS:1024\r
t=2873397496 2873404696\r
r=7d 1h 0 25h\r
z=2882844526 -1h 2898848070 0\r
k=clear:clear-key-text\r
a=recvonly\r
a=group:BUNDLE audio video\r
a=ice-options:trickle\r
a=msid-semantic:WMS *\r
m=audio 49170 UDP/TLS/RTP/SAVPF 109 9 0 8\r
i=Audio stream\r
c=IN IP4 10.47.16.5\r
a=rtpmap:109 opus/48000/2\r
a=rtpmap:9 G722/8000/1\r
a=rtpmap:0 PCMU/8000\r
a=rtpmap:8 PCMA/8000\r
a=ptime:20\r
a=sendrecv\r
a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level\r
a=setup:actpass\r
a=mid:audio\r
a=ice-ufrag:F7gI\r
a=ice-pwd:x9cml/YzichV2+XlhiMu8g\r
a=fingerprint:sha-256 D1:2C:74:A7:E3:B5:11:04:87:0D:D7:3F:B8:BF:79:7D:CF:76:B3:97:B6:5F:A5:3D:EC:D8:79:49:5C:92:26:E9\r
a=candidate:1 1 UDP 2130706431 10.47.16.5 49170 typ host\r
a=candidate:2 1 UDP 1694498815 192.0.2.3 49170 typ srflx raddr 10.47.16.5 rport 49170\r
a=rtcp-mux\r
a=ssrc:2566107569 cname:user@example.com\r
m=video 51372 UDP/TLS/RTP/SAVPF 120 121\r
i=Video stream\r
c=IN IP4 10.47.16.6\r
a=rtpmap:120 VP8/90000\r
a=rtpmap:121 VP9/90000\r
a=fmtp:120 max-fs=12288;max-fr=60\r
a=sendrecv\r
a=extmap:2 urn:ietf:params:rtp-hdrext:toffset\r
a=setup:actpass\r
a=mid:video\r
a=ice-ufrag:F7gI\r
a=ice-pwd:x9cml/YzichV2+XlhiMu8g\r
a=fingerprint:sha-256 D1:2C:74:A7:E3:B5:11:04:87:0D:D7:3F:B8:BF:79:7D:CF:76:B3:97:B6:5F:A5:3D:EC:D8:79:49:5C:92:26:E9\r
a=candidate:1 1 UDP 2130706431 10.47.16.6 51372 typ host\r
a=candidate:2 1 UDP 1694498815 192.0.2.4 51372 typ srflx raddr 10.47.16.6 rport 51372\r
a=rtcp-mux\r
a=rtcp-fb:120 nack\r
a=rtcp-fb:120 nack pli\r
a=rtcp-fb:120 ccm fir\r
a=ssrc:3004364195 cname:user@example.com\r
m=application 54111 UDP/DTLS/SCTP webrtc-datachannel\r
c=IN IP4 10.47.16.7\r
a=sctp-port:5000\r
a=max-message-size:262144\r
a=setup:actpass\r
a=mid:data\r
a=ice-ufrag:F7gI\r
a=ice-pwd:x9cml/YzichV2+XlhiMu8g\r
a=fingerprint:sha-256 D1:2C:74:A7:E3:B5:11:04:87:0D:D7:3F:B8:BF:79:7D:CF:76:B3:97:B6:5F:A5:3D:EC:D8:79:49:5C:92:26:E9\r
a=candidate:1 1 UDP 2130706431 10.47.16.7 54111 typ host\r
a=candidate:2 1 UDP 1694498815 192.0.2.5 54111 typ srflx raddr 10.47.16.7 rport 54111\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse complex SDP: {:?}", result.err());
        let session = result.unwrap();
        
        // Check session attributes
        assert_eq!(session.version, "0");
        assert_eq!(session.media_descriptions.len(), 3);
        assert_eq!(session.direction, Some(MediaDirection::RecvOnly));
        
        // Check media types and attributes
        assert_eq!(session.media_descriptions[0].media, "audio");
        assert_eq!(session.media_descriptions[1].media, "video");
        assert_eq!(session.media_descriptions[2].media, "application");
        
        // Check for audio rtpmap attributes
        let audio_rtpmaps: Vec<_> = session.media_descriptions[0].generic_attributes.iter()
            .filter_map(|attr| {
                if let ParsedAttribute::RtpMap(rtpmap) = attr {
                    Some(rtpmap)
                } else {
                    None
                }
            }).collect();
        assert_eq!(audio_rtpmaps.len(), 4);
        assert!(audio_rtpmaps.iter().any(|r| r.payload_type == 109 && r.encoding_name == "opus"));
        
        // Check for video fmtp attributes
        let video_fmtps: Vec<_> = session.media_descriptions[1].generic_attributes.iter()
            .filter_map(|attr| {
                if let ParsedAttribute::Fmtp(fmtp) = attr {
                    Some(fmtp)
                } else {
                    None
                }
            }).collect();
        assert_eq!(video_fmtps.len(), 1);
        assert!(video_fmtps.iter().any(|f| f.format == "120" && f.parameters.contains("max-fs=12288")));
        
        // Check for data channel attributes (with fixed string comparison)
        let data_attrs: Vec<_> = session.media_descriptions[2].generic_attributes.iter()
            .filter_map(|attr| {
                if let ParsedAttribute::Value(key, _) = attr {
                    Some(key.as_str())
                } else {
                    None
                }
            }).collect();
        assert!(data_attrs.iter().any(|&key| key == "sctp-port"));
        assert!(data_attrs.iter().any(|&key| key == "max-message-size"));
    }
} 
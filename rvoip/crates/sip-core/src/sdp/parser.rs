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
use crate::parser::uri::{host, hostname, ipv4, ipv6}; // Import URI parsers

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
                            // i= line not allowed at media level
                            return Err(Error::SdpParsingError("i= line found at media level (invalid)".to_string()));
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
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err());
        
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
c=IN IP4 224.2.17.12/127\r
m=audio 49170 RTP/AVP 0\r
t=0 0\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid SDP order: 't=' line found after 'm=' line"));
        
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
        assert!(result.unwrap_err().to_string().contains("Invalid SDP order: 'o=' line found after 'm=' line"));
    }

    #[test]
    fn test_attribute_parsing() {
        // Test rtpmap parsing
        let rtpmap_value = "96 H264/90000";
        let result = attributes::parse_rtpmap(rtpmap_value);
        assert!(result.is_ok());
        let rtpmap = result.unwrap();
        assert_eq!(rtpmap.payload_type, 96);
        assert_eq!(rtpmap.encoding_name, "H264");
        assert_eq!(rtpmap.clock_rate, 90000);
        assert!(rtpmap.encoding_params.is_none());
        
        // Test rtpmap with encoding parameters
        let rtpmap_value = "97 AMR/8000/1";
        let result = attributes::parse_rtpmap(rtpmap_value);
        assert!(result.is_ok());
        let rtpmap = result.unwrap();
        assert_eq!(rtpmap.payload_type, 97);
        assert_eq!(rtpmap.encoding_name, "AMR");
        assert_eq!(rtpmap.clock_rate, 8000);
        assert_eq!(rtpmap.encoding_params, Some("1".to_string()));
        
        // Test fmtp parsing
        let fmtp_value = "96 profile-level-id=42e01f;level-asymmetry-allowed=1";
        let result = attributes::parse_fmtp(fmtp_value);
        assert!(result.is_ok());
        let fmtp = result.unwrap();
        assert_eq!(fmtp.format, "96");
        assert_eq!(fmtp.parameters, "profile-level-id=42e01f;level-asymmetry-allowed=1");
        
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
        
        // Test invalid address type
        let c_line = "IN IPX 224.2.1.1";
        let result = parse_connection_line(c_line);
        assert!(result.is_err());
        
        // Test invalid IPv4 address
        let c_line = "IN IP4 999.999.999.999";
        let result = parse_connection_line(c_line);
        assert!(result.is_err());
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
        
        // Test invalid media description (missing format)
        let m_line = "audio 49170 RTP/AVP";
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
} 
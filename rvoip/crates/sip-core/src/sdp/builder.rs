/// Builder API for creating and modifying SDP sessions
///
/// This module provides a fluent builder interface for constructing SDP sessions,
/// making it easy to create complex SDP messages for SIP and WebRTC applications.
use crate::types::sdp::{
    SdpSession, Origin, ConnectionData, TimeDescription, MediaDescription,
    ParsedAttribute, RtpMapAttribute, FmtpAttribute, CandidateAttribute,
    SsrcAttribute, RepeatTime,
};
use crate::sdp::attributes::{MediaDirection, rid::{RidAttribute, RidDirection}};
use crate::error::{Error, Result};
use std::collections::HashMap;
use std::str::FromStr;

/// Builder for SDP sessions with a fluent interface
///
/// # Examples
///
/// ```
/// use rvoip_sip_core::sdp::SdpBuilder;
/// use rvoip_sip_core::sdp::attributes::MediaDirection;
///
/// let sdp = SdpBuilder::new("My Session")
///     .origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100")
///     .connection("IN", "IP4", "192.168.1.100")
///     .time("0", "0")
///     .media_audio(49170, "RTP/AVP")
///         .formats(&["0", "8"])
///         .rtpmap("0", "PCMU/8000")
///         .rtpmap("8", "PCMA/8000")
///         .direction(MediaDirection::SendRecv)
///         .done()
///     .build();
/// ```
pub struct SdpBuilder {
    session: SdpSession,
}

impl SdpBuilder {
    /// Create a new SDP builder with the specified session name
    pub fn new(session_name: impl Into<String>) -> Self {
        // Create default origin
        let origin = Origin {
            username: "-".to_string(),
            sess_id: "0".to_string(),
            sess_version: "0".to_string(),
            net_type: "IN".to_string(),
            addr_type: "IP4".to_string(),
            unicast_address: "0.0.0.0".to_string(),
        };
        
        let mut session = SdpSession::new(origin, session_name);
        
        // Clear default time description - we'll add our own later
        session.time_descriptions.clear();
        
        Self { session }
    }

    /// Set the SDP origin (o=) field
    pub fn origin(mut self, username: impl Into<String>, sess_id: impl Into<String>, 
                 sess_version: impl Into<String>, net_type: impl Into<String>, 
                 addr_type: impl Into<String>, unicast_address: impl Into<String>) -> Self {
        self.session.origin = Origin {
            username: username.into(),
            sess_id: sess_id.into(),
            sess_version: sess_version.into(),
            net_type: net_type.into(),
            addr_type: addr_type.into(),
            unicast_address: unicast_address.into(),
        };
        self
    }

    /// Set the session information (i=)
    pub fn info(mut self, info: impl Into<String>) -> Self {
        self.session.session_info = Some(info.into());
        self
    }

    /// Set the URI (u=)
    pub fn uri(mut self, uri: impl Into<String>) -> Self {
        self.session.uri = Some(uri.into());
        self
    }

    /// Set the email address (e=)
    pub fn email(mut self, email: impl Into<String>) -> Self {
        self.session.email = Some(email.into());
        self
    }

    /// Set the phone number (p=)
    pub fn phone(mut self, phone: impl Into<String>) -> Self {
        self.session.phone = Some(phone.into());
        self
    }

    /// Set the connection data (c=)
    pub fn connection(mut self, net_type: impl Into<String>, addr_type: impl Into<String>, 
                     connection_address: impl Into<String>) -> Self {
        self.session.connection_info = Some(ConnectionData {
            net_type: net_type.into(),
            addr_type: addr_type.into(),
            connection_address: connection_address.into(),
            ttl: None,
            multicast_count: None,
        });
        self
    }

    /// Set the connection data with multicast parameters
    pub fn connection_multicast(mut self, net_type: impl Into<String>, addr_type: impl Into<String>, 
                               connection_address: impl Into<String>, ttl: u8, multicast_count: Option<u32>) -> Self {
        self.session.connection_info = Some(ConnectionData {
            net_type: net_type.into(),
            addr_type: addr_type.into(),
            connection_address: connection_address.into(),
            ttl: Some(ttl),
            multicast_count,
        });
        self
    }

    /// Add a time description (t=)
    pub fn time(mut self, start_time: impl Into<String>, stop_time: impl Into<String>) -> Self {
        self.session.time_descriptions.push(TimeDescription {
            start_time: start_time.into(),
            stop_time: stop_time.into(),
            repeat_times: Vec::new(),
        });
        self
    }

    /// Add a time description with repeat times
    pub fn time_with_repeats(mut self, start_time: impl Into<String>, stop_time: impl Into<String>, 
                            repeat_times: Vec<RepeatTime>) -> Self {
        self.session.time_descriptions.push(TimeDescription {
            start_time: start_time.into(),
            stop_time: stop_time.into(),
            repeat_times,
        });
        self
    }

    /// Set bandwidth information (b=)
    pub fn bandwidth(mut self, bwtype: impl Into<String>, bandwidth: u64) -> Self {
        self.session.generic_attributes.push(
            ParsedAttribute::Bandwidth(bwtype.into(), bandwidth)
        );
        self
    }

    /// Set session-level media direction
    pub fn direction(mut self, direction: MediaDirection) -> Self {
        self.session.direction = Some(direction);
        self.session.generic_attributes.push(ParsedAttribute::Direction(direction));
        self
    }

    /// Set ICE ufrag (a=ice-ufrag)
    pub fn ice_ufrag(mut self, ufrag: impl Into<String>) -> Self {
        self.session.generic_attributes.push(
            ParsedAttribute::IceUfrag(ufrag.into())
        );
        self
    }

    /// Set ICE password (a=ice-pwd)
    pub fn ice_pwd(mut self, pwd: impl Into<String>) -> Self {
        self.session.generic_attributes.push(
            ParsedAttribute::IcePwd(pwd.into())
        );
        self
    }

    /// Set ICE options (a=ice-options)
    pub fn ice_options(mut self, options: Vec<impl Into<String>>) -> Self {
        let options = options.into_iter().map(|opt| opt.into()).collect();
        self.session.generic_attributes.push(
            ParsedAttribute::IceOptions(options)
        );
        self
    }

    /// Add ICE lite indicator (a=ice-lite)
    pub fn ice_lite(mut self) -> Self {
        self.session.generic_attributes.push(
            ParsedAttribute::Flag("ice-lite".to_string())
        );
        self
    }

    /// Set DTLS fingerprint (a=fingerprint)
    pub fn fingerprint(mut self, hash_function: impl Into<String>, fingerprint: impl Into<String>) -> Self {
        self.session.generic_attributes.push(
            ParsedAttribute::Fingerprint(hash_function.into(), fingerprint.into())
        );
        self
    }

    /// Set group attribute (a=group)
    pub fn group(mut self, semantics: impl Into<String>, ids: &[impl AsRef<str>]) -> Self {
        let ids = ids.iter().map(|id| id.as_ref().to_string()).collect();
        self.session.generic_attributes.push(
            ParsedAttribute::Group(semantics.into(), ids)
        );
        self
    }

    /// Add a custom attribute (a=)
    pub fn attribute(mut self, name: impl Into<String>, value: Option<impl Into<String>>) -> Self {
        match value {
            Some(val) => self.session.generic_attributes.push(
                ParsedAttribute::Value(name.into(), val.into())
            ),
            None => self.session.generic_attributes.push(
                ParsedAttribute::Flag(name.into())
            ),
        }
        self
    }

    /// Start building an audio media section
    pub fn media_audio(self, port: u16, protocol: impl Into<String>) -> MediaBuilder<Self> {
        MediaBuilder::new(self, "audio", port, protocol)
    }

    /// Start building a video media section
    pub fn media_video(self, port: u16, protocol: impl Into<String>) -> MediaBuilder<Self> {
        MediaBuilder::new(self, "video", port, protocol)
    }

    /// Start building an application media section
    pub fn media_application(self, port: u16, protocol: impl Into<String>) -> MediaBuilder<Self> {
        MediaBuilder::new(self, "application", port, protocol)
    }

    /// Start building a custom media section
    pub fn media(self, media_type: impl Into<String>, port: u16, protocol: impl Into<String>) -> MediaBuilder<Self> {
        MediaBuilder::new(self, media_type, port, protocol)
    }

    /// Build the final SDP session
    ///
    /// This method validates the SDP session before returning it to ensure
    /// it's well-formed according to RFC 8866. If validation fails, an error
    /// is returned with a description of the issue.
    ///
    /// # Returns
    /// A Result containing either the valid SdpSession or an Error
    pub fn build(self) -> Result<SdpSession> {
        // Validate the SDP before returning
        crate::sdp::parser::validate_sdp(&self.session)?;
        
        // Return the validated session
        Ok(self.session)
    }
}

/// Builder for SDP media sections with a fluent interface
pub struct MediaBuilder<P> {
    parent: P,
    media: MediaDescription,
}

impl<P> MediaBuilder<P> {
    /// Create a new media builder
    fn new(parent: P, media_type: impl Into<String>, port: u16, protocol: impl Into<String>) -> Self {
        let media = MediaDescription::new(
            media_type.into(),
            port,
            protocol.into(),
            Vec::new(), // formats will be added later
        );
        
        Self { parent, media }
    }

    /// Set the media formats (payload types)
    pub fn formats(mut self, formats: &[impl AsRef<str>]) -> Self {
        self.media.formats = formats.iter()
            .map(|f| f.as_ref().to_string())
            .collect();
        self
    }

    /// Set the media-level connection information (c=)
    pub fn connection(mut self, net_type: impl Into<String>, addr_type: impl Into<String>, 
                     connection_address: impl Into<String>) -> Self {
        self.media.connection_info = Some(ConnectionData {
            net_type: net_type.into(),
            addr_type: addr_type.into(),
            connection_address: connection_address.into(),
            ttl: None,
            multicast_count: None,
        });
        self
    }

    /// Set the media-level connection information with multicast parameters
    pub fn connection_multicast(mut self, net_type: impl Into<String>, addr_type: impl Into<String>, 
                               connection_address: impl Into<String>, ttl: u8, multicast_count: Option<u32>) -> Self {
        self.media.connection_info = Some(ConnectionData {
            net_type: net_type.into(),
            addr_type: addr_type.into(),
            connection_address: connection_address.into(),
            ttl: Some(ttl),
            multicast_count,
        });
        self
    }

    /// Set the direction of the media (sendrecv, sendonly, recvonly, inactive)
    pub fn direction(mut self, direction: MediaDirection) -> Self {
        self.media.direction = Some(direction);
        self.media.generic_attributes.push(ParsedAttribute::Direction(direction));
        self
    }

    /// Set the ptime attribute
    pub fn ptime(mut self, ptime: u64) -> Self {
        self.media.ptime = Some(ptime as u32);
        self.media.generic_attributes.push(ParsedAttribute::Ptime(ptime));
        self
    }

    /// Set the maxptime attribute
    pub fn maxptime(mut self, maxptime: u64) -> Self {
        self.media.generic_attributes.push(ParsedAttribute::MaxPtime(maxptime));
        self
    }

    /// Add an rtpmap attribute
    pub fn rtpmap(mut self, payload_type: impl AsRef<str>, encoding_str: impl Into<String>) -> Self {
        let encoding_str = encoding_str.into();
        let encoding_parts: Vec<&str> = encoding_str.split('/').collect();
        if encoding_parts.len() < 2 {
            // Skip invalid rtpmap
            return self;
        }

        let encoding_name = encoding_parts[0].to_string();
        let clock_rate = encoding_parts[1].parse::<u32>().unwrap_or(8000);
        let encoding_params = if encoding_parts.len() > 2 {
            Some(encoding_parts[2].to_string())
        } else {
            None
        };
        
        let payload_type = payload_type.as_ref().parse::<u8>().unwrap_or(0);
        self.media.generic_attributes.push(ParsedAttribute::RtpMap(RtpMapAttribute {
            payload_type,
            encoding_name,
            clock_rate,
            encoding_params,
        }));
        self
    }

    /// Add an fmtp attribute
    pub fn fmtp(mut self, format: impl AsRef<str>, parameters: impl Into<String>) -> Self {
        self.media.generic_attributes.push(ParsedAttribute::Fmtp(FmtpAttribute {
            format: format.as_ref().to_string(),
            parameters: parameters.into(),
        }));
        self
    }

    /// Add a mid attribute
    pub fn mid(mut self, mid: impl Into<String>) -> Self {
        self.media.generic_attributes.push(ParsedAttribute::Mid(mid.into()));
        self
    }

    /// Add an rtcp-mux attribute
    pub fn rtcp_mux(mut self) -> Self {
        self.media.generic_attributes.push(ParsedAttribute::RtcpMux);
        self
    }

    /// Add an rtcp-fb attribute
    pub fn rtcp_fb(mut self, payload_type: impl Into<String>, feedback_type: impl Into<String>, 
                   param: Option<impl Into<String>>) -> Self {
        let param = param.map(|p| p.into());
        self.media.generic_attributes.push(ParsedAttribute::RtcpFb(
            payload_type.into(),
            feedback_type.into(),
            param
        ));
        self
    }

    /// Add a setup attribute
    pub fn setup(mut self, role: impl Into<String>) -> Self {
        self.media.generic_attributes.push(ParsedAttribute::Setup(role.into()));
        self
    }

    /// Add an extmap attribute
    pub fn extmap(mut self, id: u8, direction: Option<impl Into<String>>, 
                 uri: impl Into<String>, params: Option<impl Into<String>>) -> Self {
        let direction = direction.map(|d| d.into());
        let params = params.map(|p| p.into());
        self.media.generic_attributes.push(ParsedAttribute::ExtMap(
            id, direction, uri.into(), params
        ));
        self
    }

    /// Add an ICE candidate
    pub fn ice_candidate(mut self, candidate_str: impl AsRef<str>) -> Self {
        // Parse candidate string - this is simplified, production code would need more validation
        let parts: Vec<&str> = candidate_str.as_ref().split_whitespace().collect();
        if parts.len() < 8 {
            // Skip invalid candidate
            return self;
        }

        let mut candidate = CandidateAttribute {
            foundation: parts[0].to_string(),
            component_id: parts[1].parse().unwrap_or(1),
            transport: parts[2].to_string(),
            priority: parts[3].parse().unwrap_or(0),
            connection_address: parts[4].to_string(),
            port: parts[5].parse().unwrap_or(0),
            candidate_type: parts[7].to_string(),
            related_address: None,
            related_port: None,
            extensions: Vec::new(),
        };
        
        // Process optional parameters
        let mut i = 8;
        while i + 1 < parts.len() {
            match parts[i] {
                "raddr" => {
                    candidate.related_address = Some(parts[i+1].to_string());
                    i += 2;
                },
                "rport" => {
                    candidate.related_port = parts[i+1].parse().ok();
                    i += 2;
                },
                _ => {
                    if i + 1 < parts.len() {
                        candidate.extensions.push((parts[i].to_string(), Some(parts[i+1].to_string())));
                        i += 2;
                    } else {
                        candidate.extensions.push((parts[i].to_string(), None));
                        i += 1;
                    }
                }
            }
        }
        
        self.media.generic_attributes.push(ParsedAttribute::Candidate(candidate));
        self
    }

    /// Add an ICE ufrag attribute
    pub fn ice_ufrag(mut self, ufrag: impl Into<String>) -> Self {
        self.media.generic_attributes.push(ParsedAttribute::IceUfrag(ufrag.into()));
        self
    }

    /// Add an ICE pwd attribute
    pub fn ice_pwd(mut self, pwd: impl Into<String>) -> Self {
        self.media.generic_attributes.push(ParsedAttribute::IcePwd(pwd.into()));
        self
    }

    /// Add an end-of-candidates attribute
    pub fn end_of_candidates(mut self) -> Self {
        self.media.generic_attributes.push(ParsedAttribute::EndOfCandidates);
        self
    }

    /// Add a SSRC attribute
    pub fn ssrc(mut self, ssrc_id: u32, attribute: impl Into<String>, value: Option<impl Into<String>>) -> Self {
        let value = value.map(|v| v.into());
        self.media.generic_attributes.push(ParsedAttribute::Ssrc(SsrcAttribute {
            ssrc_id,
            attribute: attribute.into(),
            value,
        }));
        self
    }

    /// Add a RID attribute
    pub fn rid(mut self, id: impl Into<String>, direction: RidDirection, 
              formats: &[impl AsRef<str>], 
              restrictions: &[(impl AsRef<str>, impl AsRef<str>)]) -> Self {
        let formats = formats.iter().map(|f| f.as_ref().to_string()).collect();
        let restrictions = restrictions.iter()
            .map(|(k, v)| (k.as_ref().to_string(), v.as_ref().to_string()))
            .collect();
            
        self.media.generic_attributes.push(ParsedAttribute::Rid(RidAttribute {
            id: id.into(),
            direction,
            formats,
            restrictions,
        }));
        self
    }

    /// Add a simulcast attribute
    pub fn simulcast(mut self, send_streams: Vec<impl Into<String>>, recv_streams: Vec<impl Into<String>>) -> Self {
        let send = send_streams.into_iter().map(|s| s.into()).collect();
        let recv = recv_streams.into_iter().map(|s| s.into()).collect();
        self.media.generic_attributes.push(ParsedAttribute::Simulcast(send, recv));
        self
    }

    /// Add bandwidth information
    pub fn bandwidth(mut self, bwtype: impl Into<String>, bandwidth: u64) -> Self {
        self.media.generic_attributes.push(ParsedAttribute::Bandwidth(bwtype.into(), bandwidth));
        self
    }

    /// Add a custom attribute
    pub fn attribute(mut self, name: impl Into<String>, value: Option<impl Into<String>>) -> Self {
        match value {
            Some(val) => self.media.generic_attributes.push(
                ParsedAttribute::Value(name.into(), val.into())
            ),
            None => self.media.generic_attributes.push(
                ParsedAttribute::Flag(name.into())
            ),
        }
        self
    }
}

// Implementation for returning to SdpBuilder
impl MediaBuilder<SdpBuilder> {
    /// Complete the media section and return to the SdpBuilder
    pub fn done(self) -> SdpBuilder {
        let mut parent = self.parent;
        parent.session.add_media(self.media);
        parent
    }
}

// Allow modifying existing SDP sessions
impl SdpSession {
    /// Create a builder from an existing SDP session
    ///
    /// # Returns
    /// A new SdpBuilder initialized with this session's data
    pub fn into_builder(self) -> SdpBuilder {
        SdpBuilder { session: self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic_sdp_builder() {
        let sdp = SdpBuilder::new("Test Session")
            .origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100")
            .connection("IN", "IP4", "192.168.1.100")
            .time("0", "0")
            .media_audio(49170, "RTP/AVP")
                .formats(&["0", "8"])
                .rtpmap("0", "PCMU/8000")
                .rtpmap("8", "PCMA/8000")
                .direction(MediaDirection::SendRecv)
                .done()
            .build()
            .expect("Valid SDP should build without errors");
            
        assert_eq!(sdp.session_name, "Test Session");
        assert_eq!(sdp.origin.sess_id, "1234567890");
        assert_eq!(sdp.time_descriptions.len(), 1);
        assert_eq!(sdp.time_descriptions[0].start_time, "0");
        assert_eq!(sdp.time_descriptions[0].stop_time, "0");
        assert_eq!(sdp.media_descriptions.len(), 1);
        
        let media = &sdp.media_descriptions[0];
        assert_eq!(media.media, "audio");
        assert_eq!(media.port, 49170);
        assert_eq!(media.formats, vec!["0", "8"]);
        assert_eq!(media.direction, Some(MediaDirection::SendRecv));
    }
    
    #[test]
    fn test_webrtc_sdp_builder() {
        let sdp = SdpBuilder::new("WebRTC Session")
            .origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100")
            .ice_ufrag("F7gI")
            .ice_pwd("x9cml/YzichV2+XlhiMu8g")
            .fingerprint("sha-256", "D2:FA:0E:C3:22:59:5E:14:95:69:92:3D:13:B4:84:24:2C:C2:A2:C0:3E:FD:34:8E:5E:EA:6F:AF:52:CE:E6:0F")
            .group("BUNDLE", &["audio", "video"])
            .time("0", "0")
            .connection("IN", "IP4", "192.168.1.100")  // Add connection for validation
            .media_audio(9, "UDP/TLS/RTP/SAVPF")
                .formats(&["111"])
                .rtpmap("111", "opus/48000/2")
                .fmtp("111", "minptime=10;useinbandfec=1")
                .mid("audio")
                .rtcp_mux()
                .ice_candidate("1 1 UDP 2130706431 192.168.1.100 9 typ host")
                .direction(MediaDirection::SendRecv)
                .setup("actpass")
                .done()
            .build()
            .expect("Valid WebRTC SDP should build without errors");
            
        assert_eq!(sdp.session_name, "WebRTC Session");
        
        // Check session-level attributes
        let ice_ufrag = sdp.generic_attributes.iter().find_map(|attr| {
            if let ParsedAttribute::IceUfrag(ufrag) = attr {
                Some(ufrag)
            } else {
                None
            }
        });
        assert!(ice_ufrag.is_some());
        assert_eq!(ice_ufrag.unwrap(), "F7gI");
        
        // Check media-level attributes
        let media = &sdp.media_descriptions[0];
        assert_eq!(media.media, "audio");
        assert_eq!(media.port, 9);
        assert_eq!(media.protocol, "UDP/TLS/RTP/SAVPF");
        assert_eq!(media.formats, vec!["111"]);
        
        // Check for rtcp-mux attribute in media section
        let has_rtcp_mux = media.generic_attributes.iter().any(|attr| {
            matches!(attr, ParsedAttribute::RtcpMux)
        });
        assert!(has_rtcp_mux);
    }
    
    #[test]
    fn test_converting_existing_session() {
        // Create a basic session first
        let session = SdpBuilder::new("Original Session")
            .origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100")
            .connection("IN", "IP4", "192.168.1.100")  // Add connection for validation
            .time("0", "0")
            .build()
            .expect("Valid SDP should build without errors");
            
        // Now convert it to a builder and add more
        let modified_session = session.into_builder()
            .connection("IN", "IP4", "192.168.1.200")  // Change IP
            .media_audio(49170, "RTP/AVP")
                .formats(&["0"])
                .done()
            .build()
            .expect("Valid modified SDP should build without errors");
            
        assert_eq!(modified_session.session_name, "Original Session");
        assert_eq!(modified_session.media_descriptions.len(), 1);
        
        if let Some(conn) = &modified_session.connection_info {
            assert_eq!(conn.connection_address, "192.168.1.200");
        } else {
            panic!("Connection info should be present");
        }
    }
    
    #[test]
    fn test_validation_failures() {
        // Test missing time description
        let result = SdpBuilder::new("Invalid Session")
            .origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100")
            .connection("IN", "IP4", "192.168.1.100")
            // No time description added
            .build();
        
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("time description"));
        }
        
        // Test missing connection data
        let result = SdpBuilder::new("Invalid Session")
            .origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100")
            .time("0", "0")
            // No connection data at session level
            .media_audio(49170, "RTP/AVP")
                .formats(&["0"])
                // No connection data at media level
                .done()
            .build();
        
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("Connection information"));
        }
        
        // Test missing formats in media
        let result = SdpBuilder::new("Invalid Session")
            .origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100")
            .connection("IN", "IP4", "192.168.1.100")
            .time("0", "0")
            .media_audio(49170, "RTP/AVP")
                // No formats added
                .done()
            .build();
        
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("must have at least one format"));
        }
    }
} 
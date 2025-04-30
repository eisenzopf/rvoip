/// Utility functions for creating SDP objects for testing and examples
use crate::types::sdp::{
    SdpSession, ConnectionData, MediaDescription, TimeDescription,
    Origin, ParsedAttribute, FmtpAttribute,
    RtpMapAttribute,
};
use crate::sdp::attributes::MediaDirection;

/// Create an Origin struct for an SDP session
pub fn create_origin(username: &str, sess_id: &str, sess_version: &str, 
                  net_type: &str, addr_type: &str, unicast_address: &str) -> Origin {
    Origin {
        username: username.to_string(),
        sess_id: sess_id.to_string(),
        sess_version: sess_version.to_string(),
        net_type: net_type.to_string(),
        addr_type: addr_type.to_string(),
        unicast_address: unicast_address.to_string(),
    }
}

/// Create a SendRecv media direction attribute
pub fn sendrecv_attr() -> ParsedAttribute {
    ParsedAttribute::Direction(MediaDirection::SendRecv)
}

/// Create a RecvOnly media direction attribute
pub fn recvonly_attr() -> ParsedAttribute {
    ParsedAttribute::Direction(MediaDirection::RecvOnly)
}

/// Create a SendOnly media direction attribute
pub fn sendonly_attr() -> ParsedAttribute {
    ParsedAttribute::Direction(MediaDirection::SendOnly)
}

/// Create an Inactive media direction attribute
pub fn inactive_attr() -> ParsedAttribute {
    ParsedAttribute::Direction(MediaDirection::Inactive)
}

/// Create an RTP map attribute
pub fn rtpmap_attr(payload_type: &str, encoding_str: &str) -> ParsedAttribute {
    let encoding_parts: Vec<&str> = encoding_str.split('/').collect();
    let encoding_name = encoding_parts[0].to_string();
    let clock_rate = encoding_parts[1].parse::<u32>().unwrap_or(8000);
    let encoding_params = if encoding_parts.len() > 2 {
        Some(encoding_parts[2].to_string())
    } else {
        None
    };
    
    ParsedAttribute::RtpMap(RtpMapAttribute {
        payload_type: payload_type.parse::<u8>().unwrap_or(0),
        encoding_name,
        clock_rate,
        encoding_params,
    })
}

/// Create an FMTP attribute
pub fn fmtp_attr(payload_type: &str, params: &str) -> ParsedAttribute {
    ParsedAttribute::Fmtp(FmtpAttribute {
        format: payload_type.to_string(),
        parameters: params.to_string(),
    })
}

/// Create a ptime attribute
pub fn ptime_attr(ptime: u64) -> ParsedAttribute {
    ParsedAttribute::Ptime(ptime)
}

/// Create a maxptime attribute
pub fn maxptime_attr(maxptime: u64) -> ParsedAttribute {
    ParsedAttribute::MaxPtime(maxptime)
}

/// Create a mid attribute
pub fn mid_attr(mid: &str) -> ParsedAttribute {
    ParsedAttribute::Mid(mid.to_string())
}

/// Create a group attribute
pub fn group_attr(semantics: &str, ids: &[&str]) -> ParsedAttribute {
    ParsedAttribute::Group(
        semantics.to_string(),
        ids.iter().map(|id| id.to_string()).collect()
    )
}

/// Create an ICE ufrag attribute
pub fn ice_ufrag_attr(ufrag: &str) -> ParsedAttribute {
    ParsedAttribute::IceUfrag(ufrag.to_string())
}

/// Create an ICE pwd attribute
pub fn ice_pwd_attr(pwd: &str) -> ParsedAttribute {
    ParsedAttribute::IcePwd(pwd.to_string())
}

/// Create a fingerprint attribute
pub fn fingerprint_attr(hash_function: &str, fingerprint: &str) -> ParsedAttribute {
    ParsedAttribute::Fingerprint(hash_function.to_string(), fingerprint.to_string())
}

/// Create a setup attribute
pub fn setup_attr(setup: &str) -> ParsedAttribute {
    ParsedAttribute::Setup(setup.to_string())
}

/// Create an rtcp-mux attribute
pub fn rtcp_mux_attr() -> ParsedAttribute {
    ParsedAttribute::RtcpMux
}

/// Create a ConnectionData struct
pub fn create_connection(net_type: &str, addr_type: &str, connection_address: &str) -> ConnectionData {
    ConnectionData {
        net_type: net_type.to_string(),
        addr_type: addr_type.to_string(),
        connection_address: connection_address.to_string(),
        ttl: None,
        multicast_count: None,
    }
}

/// Create a TimeDescription struct
pub fn create_time(start_time: &str, stop_time: &str) -> TimeDescription {
    TimeDescription {
        start_time: start_time.to_string(),
        stop_time: stop_time.to_string(),
        repeat_times: vec![],
    }
}

/// Create a MediaDescription struct
pub fn create_media(media_type: &str, port: u16, protocol: &str, formats: &[&str]) -> MediaDescription {
    MediaDescription::new(
        media_type.to_string(),
        port,
        protocol.to_string(),
        formats.iter().map(|f| f.to_string()).collect()
    )
}

/// Add bandwidth information to an SDP session
pub fn add_session_bandwidth(session: &mut SdpSession, bwtype: &str, bandwidth: &str) {
    let bw_value = bandwidth.parse::<u64>().unwrap_or(0);
    session.generic_attributes.push(ParsedAttribute::Bandwidth(bwtype.to_string(), bw_value));
}

/// Add bandwidth information to a media description
pub fn add_media_bandwidth(media: &mut MediaDescription, bwtype: &str, bandwidth: &str) {
    let bw_value = bandwidth.parse::<u64>().unwrap_or(0);
    media.generic_attributes.push(ParsedAttribute::Bandwidth(bwtype.to_string(), bw_value));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::sdp::{SdpSession, MediaDescription, ParsedAttribute, FmtpAttribute};
    use crate::sdp::attributes::MediaDirection;

    #[test]
    fn test_basic_sdp_session() {
        let origin = create_origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100");
        let connection = create_connection("IN", "IP4", "192.168.1.100");
        let time = create_time("0", "0");
        
        let mut session = SdpSession::new(origin, "VoIP Call".to_string());
        session = session.with_connection_data(connection);
        // Don't add a time description since it's already initialized with one

        assert_eq!(session.version, "0");
        assert_eq!(session.origin.username, "-");
        assert_eq!(session.origin.sess_id, "1234567890");
        assert_eq!(session.origin.sess_version, "2");
        assert_eq!(session.origin.unicast_address, "192.168.1.100");
        assert_eq!(session.session_name, "VoIP Call");
        assert!(session.connection_info.is_some());
        if let Some(conn) = &session.connection_info {
            assert_eq!(conn.connection_address, "192.168.1.100");
        }
        assert_eq!(session.time_descriptions.len(), 1);
        assert_eq!(session.time_descriptions[0].start_time, "0");
        assert_eq!(session.time_descriptions[0].stop_time, "0");
    }

    #[test]
    fn test_sdp_with_media() {
        let origin = create_origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100");
        let time = create_time("0", "0");
        
        let mut session = SdpSession::new(origin, "VoIP Call".to_string());
        session.time_descriptions.push(time);
        
        let mut media = create_media("audio", 49170, "RTP/AVP", &["0", "8"]);
        media.generic_attributes.push(rtpmap_attr("0", "PCMU/8000"));
        media.generic_attributes.push(rtpmap_attr("8", "PCMA/8000"));
        media.generic_attributes.push(sendrecv_attr());
        
        // Explicitly set media direction
        media.direction = Some(MediaDirection::SendRecv);
        
        session.add_media(media);

        assert_eq!(session.media_descriptions.len(), 1);
        let media = &session.media_descriptions[0];
        assert_eq!(media.media, "audio");
        assert_eq!(media.port, 49170);
        assert_eq!(media.protocol, "RTP/AVP");
        assert_eq!(media.formats, vec!["0", "8"]);
        
        // Verify rtpmap attributes
        let rtpmaps: Vec<_> = media.generic_attributes.iter()
            .filter_map(|attr| {
                if let ParsedAttribute::RtpMap(rtpmap) = attr {
                    Some(rtpmap)
                } else {
                    None
                }
            })
            .collect();
        
        assert_eq!(rtpmaps.len(), 2);
        assert_eq!(rtpmaps[0].payload_type, 0);
        assert_eq!(rtpmaps[0].encoding_name, "PCMU");
        assert_eq!(rtpmaps[0].clock_rate, 8000);
        
        // Verify direction
        assert_eq!(media.direction, Some(MediaDirection::SendRecv));
    }

    #[test]
    fn test_multiple_media_sections() {
        let origin = create_origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100");
        let time = create_time("0", "0");
        
        let mut session = SdpSession::new(origin, "VoIP Call".to_string());
        session.time_descriptions.push(time);
        
        // Add audio media
        let mut audio_media = create_media("audio", 49170, "RTP/AVP", &["0", "8"]);
        audio_media.generic_attributes.push(rtpmap_attr("0", "PCMU/8000"));
        audio_media.generic_attributes.push(sendrecv_attr());
        // Explicitly set media direction
        audio_media.direction = Some(MediaDirection::SendRecv);
        session.add_media(audio_media);
        
        // Add video media
        let mut video_media = create_media("video", 51372, "RTP/AVP", &["96"]);
        video_media.generic_attributes.push(rtpmap_attr("96", "H264/90000"));
        video_media.generic_attributes.push(sendrecv_attr());
        // Explicitly set media direction
        video_media.direction = Some(MediaDirection::SendRecv);
        session.add_media(video_media);

        assert_eq!(session.media_descriptions.len(), 2);
        
        // Verify audio media
        let audio = &session.media_descriptions[0];
        assert_eq!(audio.media, "audio");
        assert_eq!(audio.port, 49170);
        assert_eq!(audio.formats, vec!["0", "8"]);
        assert_eq!(audio.direction, Some(MediaDirection::SendRecv));
        
        // Verify video media
        let video = &session.media_descriptions[1];
        assert_eq!(video.media, "video");
        assert_eq!(video.port, 51372);
        assert_eq!(video.formats, vec!["96"]);
        assert_eq!(video.direction, Some(MediaDirection::SendRecv));
    }

    #[test]
    fn test_webrtc_attributes() {
        let origin = create_origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100");
        let time = create_time("0", "0");
        
        let mut session = SdpSession::new(origin, "WebRTC Call".to_string());
        session.time_descriptions.push(time);
        
        // Add session attributes
        session.generic_attributes.push(ice_ufrag_attr("F7gI"));
        session.generic_attributes.push(ice_pwd_attr("x9cml/YzichV2+XlhiMu8g"));
        session.generic_attributes.push(group_attr("BUNDLE", &["audio", "video"]));
        
        // Add media section
        let mut audio_media = create_media("audio", 9, "UDP/TLS/RTP/SAVPF", &["111"]);
        audio_media.generic_attributes.push(mid_attr("audio"));
        audio_media.generic_attributes.push(rtpmap_attr("111", "opus/48000/2"));
        audio_media.generic_attributes.push(fmtp_attr("111", "minptime=10;useinbandfec=1"));
        audio_media.generic_attributes.push(rtcp_mux_attr());
        audio_media.generic_attributes.push(setup_attr("actpass"));
        audio_media.generic_attributes.push(fingerprint_attr("sha-256", "D2:FA:0E:C3:22:59:5E:14:95:69:92:3D:13:B4:84:24:2C:C2:A2:C0:3E:FD:34:8E:5E:EA:6F:AF:52:CE:E6:0F"));
        
        session.add_media(audio_media);

        // Check session-level WebRTC attributes
        let ice_ufrag = session.generic_attributes.iter().find_map(|attr| {
            if let ParsedAttribute::IceUfrag(ufrag) = attr {
                Some(ufrag)
            } else {
                None
            }
        });
        
        assert!(ice_ufrag.is_some());
        assert_eq!(ice_ufrag.unwrap(), "F7gI");
        
        // Check BUNDLE group
        let bundle = session.generic_attributes.iter().find_map(|attr| {
            if let ParsedAttribute::Group(semantic, ids) = attr {
                if semantic == "BUNDLE" {
                    Some(ids)
                } else {
                    None
                }
            } else {
                None
            }
        });
        
        assert!(bundle.is_some());
        let ids = bundle.unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], "audio");
        assert_eq!(ids[1], "video");
        
        // Check media-level attributes
        let audio = &session.media_descriptions[0];
        
        // Check mid
        let mid = audio.generic_attributes.iter().find_map(|attr| {
            if let ParsedAttribute::Mid(mid) = attr {
                Some(mid)
            } else {
                None
            }
        });
        
        assert!(mid.is_some());
        assert_eq!(mid.unwrap(), "audio");
        
        // Check rtcp-mux
        let has_rtcp_mux = audio.generic_attributes.iter().any(|attr| {
            matches!(attr, ParsedAttribute::RtcpMux)
        });
        
        assert!(has_rtcp_mux);
    }
    
    #[test]
    fn test_complete_sdp_with_all_fields() {
        // Create origin for the SDP
        let origin = create_origin("-", "1234567890", "2", "IN", "IP4", "192.168.1.100");
        
        // Create base session
        let mut session = SdpSession::new(origin, "A Complete SDP Example".to_string());
        
        // Add session-level information
        session.session_info = Some("Test session".to_string());
        session.uri = Some("https://example.com/session".to_string());
        session.email = Some("user@example.com".to_string());
        session.phone = Some("+1 617 555 6011".to_string());
        
        // Add bandwidth information
        add_session_bandwidth(&mut session, "AS", "128");
        
        // Replace the default time description with our custom one
        session.time_descriptions.clear();
        session.time_descriptions.push(create_time("2873397496", "2873404696"));
        
        // Verify time settings are correct
        assert_eq!(session.time_descriptions[0].start_time, "2873397496");
        assert_eq!(session.time_descriptions[0].stop_time, "2873404696");
        
        // ... rest of test ...
    }
} 
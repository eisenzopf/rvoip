//! SIP/SDP Integration
//!
//! This module provides utilities for integrating SIP messages with SDP session descriptions.
//! It includes helper functions to create SDP from SIP message information and vice versa,
//! as well as common media profile helpers for typical use cases.

use std::time::{SystemTime, UNIX_EPOCH};
use crate::types::{
    uri::Uri,
    from::From,
    to::To,
    contact::Contact,
    via::Via,
    address::Address,
    sip_message::Message,
    sip_request::Request,
    sip_response::Response,
};
use crate::RequestBuilder;
use crate::ResponseBuilder;
use crate::sdp::{
    SdpBuilder,
    attributes::MediaDirection,
};
use crate::types::sdp::SdpSession;

/// Helper for extracting host and IP information from SIP URIs and headers
fn extract_ip_from_uri(uri: &Uri) -> Option<String> {
    // Get the host part of the URI - proper implementation
    Some(uri.host.to_string())
}

/// Generate a default session ID for SDP
///
/// This uses the current Unix timestamp as a default session ID, which is a common
/// practice in SDP implementations.
pub fn default_session_id() -> String {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs().to_string(),
        Err(_) => "0".to_string(),
    }
}

/// Create an SDP session from a SIP request
///
/// This function extracts information from SIP headers to create a basic SDP session.
/// It's useful for generating the initial SDP offer from an INVITE request.
///
/// # Parameters
/// - `request`: The SIP request to extract information from
/// - `session_name`: Optional name for the SDP session (defaults to "Session")
///
/// # Returns
/// An SdpBuilder with basic information populated from the SIP request
pub fn sdp_from_request(request: &Request, session_name: Option<&str>) -> SdpBuilder {
    let session_name = session_name.unwrap_or("Session");
    let mut builder = SdpBuilder::new(session_name);

    // Extract From header for o= line
    if let Some(from) = request.typed_header::<From>() {
        let username = from.display_name.clone().unwrap_or_else(|| "-".to_string());
        
        // From address() returns &Address not Option<&Address>
        let address = from.address();
        let sess_id = default_session_id();
        
        // Try to extract IP from either Contact or From header
        let ip = if let Some(contact) = request.typed_header::<Contact>() {
            // Contact address() returns Option<&Address> not &Address
            if let Some(contact_addr) = contact.address() {
                extract_ip_from_uri(&contact_addr.uri)
                    .or_else(|| extract_ip_from_uri(&address.uri))
                    .unwrap_or_else(|| "0.0.0.0".to_string())
            } else {
                extract_ip_from_uri(&address.uri).unwrap_or_else(|| "0.0.0.0".to_string())
            }
        } else {
            extract_ip_from_uri(&address.uri).unwrap_or_else(|| "0.0.0.0".to_string())
        };
        
        builder = builder.origin(&username, &sess_id, &sess_id, "IN", "IP4", &ip);
        
        // Set connection data
        builder = builder.connection("IN", "IP4", &ip);
    }
    
    // Always add a time field (required by SDP spec)
    builder = builder.time("0", "0");
    
    builder
}

/// Create an SDP session from a SIP response
///
/// This function extracts information from SIP headers to create a basic SDP session.
/// It's useful for generating the SDP answer from a 200 OK response to an INVITE.
///
/// # Parameters
/// - `response`: The SIP response to extract information from
/// - `session_name`: Optional name for the SDP session (defaults to "Session")
///
/// # Returns
/// An SdpBuilder with basic information populated from the SIP response
pub fn sdp_from_response(response: &Response, session_name: Option<&str>) -> SdpBuilder {
    let session_name = session_name.unwrap_or("Session");
    let mut builder = SdpBuilder::new(session_name);

    // Extract To header for o= line
    if let Some(to) = response.typed_header::<To>() {
        let username = to.display_name.clone().unwrap_or_else(|| "-".to_string());
        
        // To address() returns &Address not Option<&Address>
        let address = to.address();
        let sess_id = default_session_id();
        
        // Try to extract IP from either Contact or To header
        let ip = if let Some(contact) = response.typed_header::<Contact>() {
            // Contact address() returns Option<&Address> not &Address
            if let Some(contact_addr) = contact.address() {
                extract_ip_from_uri(&contact_addr.uri)
                    .or_else(|| extract_ip_from_uri(&address.uri))
                    .unwrap_or_else(|| "0.0.0.0".to_string())
            } else {
                extract_ip_from_uri(&address.uri).unwrap_or_else(|| "0.0.0.0".to_string())
            }
        } else {
            extract_ip_from_uri(&address.uri).unwrap_or_else(|| "0.0.0.0".to_string())
        };
        
        builder = builder.origin(&username, &sess_id, &sess_id, "IN", "IP4", &ip);
        
        // Set connection data
        builder = builder.connection("IN", "IP4", &ip);
    }
    
    // Always add a time field (required by SDP spec)
    builder = builder.time("0", "0");
    
    builder
}

/// Create a basic audio-only SDP profile
///
/// Creates an SDP session with a single audio media line using common codecs.
///
/// # Parameters
/// - `builder`: The SdpBuilder to add the audio profile to
/// - `port`: The port for RTP audio
/// - `codecs`: Optional slice of codec payload types (defaults to PCMU/8000 and PCMA/8000)
///
/// # Returns
/// The SdpBuilder with audio media added
pub fn add_audio_profile(
    builder: SdpBuilder, 
    port: u16, 
    codecs: Option<&[&str]>
) -> SdpBuilder {
    let codecs = codecs.unwrap_or(&["0", "8"]);
    
    let mut media_builder = builder.media_audio(port, "RTP/AVP")
        .formats(codecs)
        .direction(MediaDirection::SendRecv);
    
    // Add rtpmap entries for common codecs
    for &codec in codecs {
        match codec {
            "0" => { media_builder = media_builder.rtpmap("0", "PCMU/8000"); },
            "8" => { media_builder = media_builder.rtpmap("8", "PCMA/8000"); },
            "9" => { media_builder = media_builder.rtpmap("9", "G722/8000"); },
            "101" => { 
                media_builder = media_builder
                    .rtpmap("101", "telephone-event/8000")
                    .fmtp("101", "0-16"); 
            },
            _ => {} // Skip other codecs
        }
    }
    
    media_builder.done()
}

/// Create a basic video-only SDP profile
///
/// Creates an SDP session with a single video media line using common codecs.
///
/// # Parameters
/// - `builder`: The SdpBuilder to add the video profile to
/// - `port`: The port for RTP video
/// - `codecs`: Optional slice of codec payload types (defaults to H264 and VP8)
///
/// # Returns
/// The SdpBuilder with video media added
pub fn add_video_profile(
    builder: SdpBuilder, 
    port: u16, 
    codecs: Option<&[&str]>
) -> SdpBuilder {
    let codecs = codecs.unwrap_or(&["96", "97"]);
    
    let mut media_builder = builder.media_video(port, "RTP/AVP")
        .formats(codecs)
        .direction(MediaDirection::SendRecv);
    
    // Add rtpmap entries for common codecs
    for &codec in codecs {
        match codec {
            "96" => { 
                media_builder = media_builder
                    .rtpmap("96", "VP8/90000")
                    .rtcp_fb("96", "nack", Some("pli"))
                    .rtcp_fb("96", "ccm", Some("fir")); 
            },
            "97" => { 
                media_builder = media_builder
                    .rtpmap("97", "H264/90000")
                    .fmtp("97", "profile-level-id=42e01f")
                    .rtcp_fb("97", "nack", Some("pli"))
                    .rtcp_fb("97", "ccm", Some("fir")); 
            },
            _ => {} // Skip other codecs
        }
    }
    
    media_builder.done()
}

/// Create a complete audio+video SDP profile
///
/// Creates an SDP session with both audio and video media lines using common codecs.
///
/// # Parameters
/// - `builder`: The SdpBuilder to add the audio/video profile to
/// - `audio_port`: The port for RTP audio
/// - `video_port`: The port for RTP video
/// - `audio_codecs`: Optional slice of audio codec payload types
/// - `video_codecs`: Optional slice of video codec payload types
///
/// # Returns
/// The SdpBuilder with audio and video media added
pub fn add_av_profile(
    builder: SdpBuilder,
    audio_port: u16,
    video_port: u16,
    audio_codecs: Option<&[&str]>,
    video_codecs: Option<&[&str]>
) -> SdpBuilder {
    let builder = add_audio_profile(builder, audio_port, audio_codecs);
    add_video_profile(builder, video_port, video_codecs)
}

/// Create a WebRTC-compatible SDP profile
///
/// Creates an SDP session with WebRTC-specific attributes and media configuration.
///
/// # Parameters
/// - `builder`: The SdpBuilder to add the WebRTC profile to
/// - `audio_port`: The port for RTP audio (typically 9 for WebRTC)
/// - `video_port`: The port for RTP video (typically 9 for WebRTC)
/// - `ice_ufrag`: ICE username fragment
/// - `ice_pwd`: ICE password
/// - `fingerprint`: DTLS fingerprint string
/// - `include_video`: Whether to include video (defaults to true)
///
/// # Returns
/// The SdpBuilder with WebRTC media configuration
pub fn add_webrtc_profile(
    builder: SdpBuilder,
    audio_port: u16,
    video_port: u16,
    ice_ufrag: &str,
    ice_pwd: &str,
    fingerprint: &str,
    include_video: bool
) -> SdpBuilder {
    let builder = builder
        .ice_ufrag(ice_ufrag)
        .ice_pwd(ice_pwd)
        .fingerprint("sha-256", fingerprint);
    
    let rtpmap_audio = &["111", "103"];
    let rtpmap_video = &["96", "97"];
    
    // Add audio
    let mut media_builder = builder.media_audio(audio_port, "UDP/TLS/RTP/SAVPF")
        .formats(rtpmap_audio)
        .rtpmap("111", "opus/48000/2")
        .rtpmap("103", "ISAC/16000")
        .fmtp("111", "minptime=10;useinbandfec=1")
        .direction(MediaDirection::SendRecv)
        .rtcp_mux()
        .setup("actpass")
        .ice_ufrag(ice_ufrag)
        .ice_pwd(ice_pwd)
        .mid("audio");
    
    let builder = media_builder.done();
    
    // Add video if requested
    if !include_video {
        return builder;
    }
    
    let mut media_builder = builder.media_video(video_port, "UDP/TLS/RTP/SAVPF")
        .formats(rtpmap_video)
        .rtpmap("96", "VP8/90000")
        .rtpmap("97", "H264/90000")
        .fmtp("97", "profile-level-id=42e01f")
        .rtcp_fb("96", "nack", Some("pli"))
        .rtcp_fb("96", "ccm", Some("fir"))
        .rtcp_fb("97", "nack", Some("pli"))
        .rtcp_fb("97", "ccm", Some("fir"))
        .direction(MediaDirection::SendRecv)
        .rtcp_mux()
        .setup("actpass")
        .ice_ufrag(ice_ufrag)
        .ice_pwd(ice_pwd)
        .mid("video");
    
    let builder = media_builder.done();
    
    // Add bundle for WebRTC
    builder.group("BUNDLE", &["audio", "video"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        Method, StatusCode, Uri,
        call_id::CallId, 
        cseq::CSeq, 
        content_type::ContentType, 
        content_length::ContentLength, 
        max_forwards::MaxForwards,
        headers::TypedHeader,
        param::Param,
        contact::ContactParamInfo,
    };
    use std::str::FromStr;
    
    #[test]
    fn test_sdp_from_request() {
        // Create a SIP request
        let invite = RequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
            .header(TypedHeader::From(From::new(
                Address {
                    display_name: Some("Alice".to_string()),
                    uri: Uri::from_str("sip:alice@atlanta.com").unwrap(),
                    params: vec![],
                }
            )))
            .header(TypedHeader::To(To::new(
                Address {
                    display_name: Some("Bob".to_string()),
                    uri: Uri::from_str("sip:bob@example.com").unwrap(),
                    params: vec![],
                }
            )))
            .header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.com")))
            .header(TypedHeader::CSeq(CSeq::new(314159, Method::Invite)))
            .header(TypedHeader::Via(
                Via::new_simple("SIP", "2.0", "UDP", "pc33.atlanta.com", None, vec![]).unwrap()
            ))
            .header(TypedHeader::Contact(Contact::new_params(vec![
                ContactParamInfo {
                    address: Address {
                        display_name: None,
                        uri: Uri::from_str("sip:alice@pc33.atlanta.com").unwrap(),
                        params: vec![],
                    }
                }
            ])))
            .build();
        
        // Create SDP from the request
        let sdp_builder = sdp_from_request(&invite, Some("Audio Call"));
        
        // Add media and build the session
        let sdp = add_audio_profile(sdp_builder, 49170, None)
            .build()
            .expect("Valid SDP");
        
        // Check that SDP contains expected values
        assert_eq!(sdp.session_name, "Audio Call");
        
        let origin = &sdp.origin;
        assert_eq!(origin.username, "Alice");
        
        let media = &sdp.media_descriptions[0];
        assert_eq!(media.media, "audio");
        assert_eq!(media.port, 49170);
        assert!(media.formats.contains(&"0".to_string()));
        assert!(media.formats.contains(&"8".to_string()));
    }
    
    #[test]
    fn test_sdp_from_response() {
        // Create a SIP response - SimpleResponseBuilder::new returns Self not Result<Self, Error>
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .header(TypedHeader::From(From::new(
                Address {
                    display_name: Some("Alice".to_string()),
                    uri: Uri::from_str("sip:alice@atlanta.com").unwrap(),
                    params: vec![],
                }
            )))
            .header(TypedHeader::To(To::new(
                Address {
                    display_name: Some("Bob".to_string()),
                    uri: Uri::from_str("sip:bob@example.com").unwrap(),
                    params: vec![],
                }
            )))
            .header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.com")))
            .header(TypedHeader::CSeq(CSeq::new(314159, Method::Invite)))
            .header(TypedHeader::Contact(Contact::new_params(vec![
                ContactParamInfo {
                    address: Address {
                        display_name: None,
                        uri: Uri::from_str("sip:bob@example.com").unwrap(),
                        params: vec![],
                    }
                }
            ])))
            .build();
        
        // Create SDP from the response
        let sdp_builder = sdp_from_response(&response, Some("Answer Session"));
        
        // Add media and build the session
        let sdp = add_av_profile(sdp_builder, 49170, 49180, None, None)
            .build()
            .expect("Valid SDP");
        
        // Check that SDP contains expected values
        assert_eq!(sdp.session_name, "Answer Session");
        
        let origin = &sdp.origin;
        assert_eq!(origin.username, "Bob");
        
        assert_eq!(sdp.media_descriptions.len(), 2);
        assert_eq!(sdp.media_descriptions[0].media, "audio");
        assert_eq!(sdp.media_descriptions[1].media, "video");
    }
} 
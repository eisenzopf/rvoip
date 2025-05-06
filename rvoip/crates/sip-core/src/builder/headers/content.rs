//! Content header builders
//!
//! This module provides builder methods for content-related headers.

use crate::error::{Error, Result};
use crate::types::{
    header::Header,
    headers::{HeaderName, typed_header::TypedHeaderTrait},
    TypedHeader,
    content_type::ContentType,
    content_length::ContentLength,
    sdp::SdpSession,
};
use crate::parser::headers::content_type::ContentTypeValue;
use crate::builder::headers::HeaderSetter;
use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
#[cfg(feature = "sdp")]
use crate::sdp::{SdpBuilder, integration};
use bytes::Bytes;
use crate::builder::headers::content_type::ContentTypeBuilderExt;

/// Extension trait that adds content-related building capabilities to request and response builders
pub trait ContentBuilderExt: ContentTypeBuilderExt {
    /// Add an SDP session as the message body
    ///
    /// This method sets the Content-Type to application/sdp and adds the SDP session
    /// as the message body.
    ///
    /// # Parameters
    ///
    /// - `sdp`: The SDP session to add as the body
    fn sdp_body(self, sdp: &SdpSession) -> Self;
    
    // Note: The following methods are commented out because they require complex rebuilding
    // of the request/response which is not compatible with the current builder design.
    // These methods can be reimplemented in the future with a different approach.
    
    /*
    /// Generate an SDP session from SIP information and add it as the body
    ///
    /// This method extracts information from the SIP message to create an SDP session
    /// with a basic audio profile, and sets it as the message body.
    ///
    /// # Parameters
    ///
    /// - `session_name`: Optional name for the SDP session
    /// - `port`: The port for the audio media
    /// - `codecs`: Optional list of codec payload types
    fn auto_sdp_audio_body(self, session_name: Option<&str>, port: u16, codecs: Option<&[&str]>) -> Self;
    
    /// Generate an SDP session with audio and video from SIP information
    ///
    /// This method extracts information from the SIP message to create an SDP session
    /// with both audio and video profiles, and sets it as the message body.
    ///
    /// # Parameters
    ///
    /// - `session_name`: Optional name for the SDP session
    /// - `audio_port`: The port for the audio media
    /// - `video_port`: The port for the video media
    /// - `audio_codecs`: Optional list of audio codec payload types
    /// - `video_codecs`: Optional list of video codec payload types
    fn auto_sdp_av_body(
        self, 
        session_name: Option<&str>, 
        audio_port: u16, 
        video_port: u16,
        audio_codecs: Option<&[&str]>,
        video_codecs: Option<&[&str]>
    ) -> Self;
    
    /// Generate a WebRTC-compatible SDP session from SIP information
    ///
    /// This method creates a WebRTC-ready SDP session with ICE and DTLS
    /// parameters, and sets it as the message body.
    ///
    /// # Parameters
    ///
    /// - `session_name`: Optional name for the SDP session
    /// - `ice_ufrag`: ICE username fragment (random string)
    /// - `ice_pwd`: ICE password (random string)
    /// - `fingerprint`: DTLS fingerprint
    /// - `include_video`: Whether to include video in the SDP
    fn auto_sdp_webrtc_body(
        self,
        session_name: Option<&str>,
        ice_ufrag: &str,
        ice_pwd: &str,
        fingerprint: &str,
        include_video: bool
    ) -> Self;
    */
}

/// Implementation for new SimpleRequestBuilder
impl ContentBuilderExt for SimpleRequestBuilder {
    fn sdp_body(self, sdp: &SdpSession) -> Self {
        let sdp_string = sdp.to_string();
        self.content_type_sdp()
            .body(Bytes::from(sdp_string))
    }
}

/// Implementation for new SimpleResponseBuilder
impl ContentBuilderExt for SimpleResponseBuilder {
    fn sdp_body(self, sdp: &SdpSession) -> Self {
        let sdp_string = sdp.to_string();
        self.content_type_sdp()
            .body(Bytes::from(sdp_string))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        method::Method, StatusCode,
        from::From, to::To, contact::Contact,
        via::Via,
        uri::Uri, address::Address, call_id::CallId, cseq::CSeq,
        contact::ContactParamInfo,
    };
    use std::str::FromStr;
    use crate::types::headers::HeaderAccess;
    use crate::builder::request::SimpleRequestBuilder;
    use crate::builder::response::SimpleResponseBuilder;

    #[test]
    fn test_request_content_type_shortcuts() {
        // Test for SimpleRequestBuilder
        let request = SimpleRequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .content_type_sdp()
            .build();
            
        let content_type_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "application/sdp");
    }
    
    #[cfg(feature = "sdp")]
    #[test]
    fn test_simple_request_sdp_body() {
        // Create SDP body
        let sdp = SdpSession::from_str("\
v=0\r\n\
o=alice 2890844526 2890844526 IN IP4 alice.example.org\r\n\
s=SIP Call\r\n\
c=IN IP4 alice.example.org\r\n\
t=0 0\r\n\
m=audio 49170 RTP/AVP 0\r\n\
a=rtpmap:0 PCMU/8000\r\n").unwrap();
        
        // Test with SimpleRequestBuilder
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .sdp_body(&sdp)
            .build();
            
        // Verify Content-Type header
        let content_type_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "application/sdp");
        
        // Verify body content
        assert!(!request.body().is_empty());
        let body_str = std::str::from_utf8(request.body()).unwrap();
        assert!(body_str.contains("v=0"));
        assert!(body_str.contains("m=audio 49170 RTP/AVP 0"));
    }
} 
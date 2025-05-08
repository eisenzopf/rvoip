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

/// Content Body Builder for SIP Messages
///
/// This module provides builder methods for setting message body content in SIP messages,
/// working in conjunction with Content-Type, Content-Length, and other content-related headers.
///
/// ## SIP Message Content Overview
///
/// SIP messages can contain various types of content in their bodies, which is described by
/// the Content-Type header as defined in [RFC 3261 Section 20.15](https://datatracker.ietf.org/doc/html/rfc3261#section-20.15).
/// The Content-Length header [RFC 3261 Section 20.14](https://datatracker.ietf.org/doc/html/rfc3261#section-20.14)
/// specifies the size of the body.
///
/// ## Purpose of Message Content
///
/// Message bodies in SIP serve several critical functions:
///
/// 1. **Media negotiation**: SDP content in INVITE requests describes media capabilities
/// 2. **Instant messaging**: Text or MIME content in MESSAGE requests for IM applications
/// 3. **Event notification**: XML or JSON content in NOTIFY requests for events and presence
/// 4. **File transfer**: Binary content for delivering files or images
/// 5. **Application data**: Custom application data exchange between SIP endpoints
///
/// ## Common Content Types in SIP
///
/// - **application/sdp**: Session Description Protocol for media negotiation
/// - **text/plain**: Simple text messages
/// - **application/xml**: XML-based content (PIDF, XCAP, etc.)
/// - **application/json**: JSON-formatted data
/// - **message/sipfrag**: SIP message fragments (used in NOTIFY for REFER status)
/// - **multipart/mixed**: Multiple content parts with different types
/// - **image/jpeg**, **image/png**: Image content for avatar updates or visual voicemail
///
/// ## Relationship with other headers
///
/// - **Content-Type**: Specifies the MIME type of the message body
/// - **Content-Length**: Indicates the size of the body in octets
/// - **Content-Encoding**: Specifies any compression or encoding applied to the body
/// - **Content-Language**: Indicates the natural language of the body content
/// - **Content-Disposition**: Indicates how the body should be interpreted
/// - **MIME-Version**: Required when using multipart content types
///
/// # Examples
///
/// ## INVITE with SDP for Audio Call
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ContentBuilderExt;
/// use rvoip_sip_core::types::Method;
/// use rvoip_sip_core::sdp::SdpBuilder;
///
/// // Create an SDP session for an audio call
/// let sdp = SdpBuilder::new("Audio Call")
///     .origin("alice", "2890844526", "2890844526", "IN", "IP4", "alice.example.org")
///     .connection("IN", "IP4", "alice.example.org")
///     .time("0", "0")
///     .media("audio", 49170, "RTP/AVP")
///     .formats(&["0", "8", "96"])
///     .attribute("rtpmap", Some("0 PCMU/8000"))
///     .attribute("rtpmap", Some("8 PCMA/8000"))
///     .attribute("rtpmap", Some("96 telephone-event/8000"))
///     .attribute("ptime", Some("20"))
///     .done()
///     .build()
///     .unwrap();
///
/// // Create an INVITE request for an audio call
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Bob", "sip:bob@example.com", None)
///     .contact("<sip:alice@192.0.2.1:5060>", None)
///     .sdp_body(&sdp)  // Sets Content-Type to application/sdp automatically
///     .build();
/// ```
///
/// ## Audio and Video Call with SRTP
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ContentBuilderExt;
/// use rvoip_sip_core::types::Method;
/// use rvoip_sip_core::sdp::SdpBuilder;
///
/// // Create an SDP session for a secure audio/video call
/// let sdp = SdpBuilder::new("Secure Audio/Video Call")
///     .origin("alice", "2890844526", "2890844526", "IN", "IP4", "alice.example.org")
///     .connection("IN", "IP4", "alice.example.org")
///     .time("0", "0")
///     // Add audio media with SRTP
///     .media("audio", 49170, "RTP/SAVP")
///     .formats(&["0", "8", "96"])
///     .attribute("rtpmap", Some("0 PCMU/8000"))
///     .attribute("rtpmap", Some("8 PCMA/8000"))
///     .attribute("rtpmap", Some("96 telephone-event/8000"))
///     .attribute("crypto", Some("1 AES_CM_128_HMAC_SHA1_80 inline:PS1uQCVeeCFCanVmcjkpPywjNWhcYD0mXXtxaVBR|2^20|1:32"))
///     .done()
///     // Add video media with SRTP
///     .media("video", 51372, "RTP/SAVP")
///     .formats(&["97", "98"])
///     .attribute("rtpmap", Some("97 H264/90000"))
///     .attribute("rtpmap", Some("98 VP8/90000"))
///     .attribute("crypto", Some("1 AES_CM_128_HMAC_SHA1_80 inline:d0RmdmcmVCspeEc3QGZiNWpVLFJhQX1cfHAwJSoj|2^20|1:32"))
///     .done()
///     .build()
///     .unwrap();
///
/// // Create an INVITE request for a secure audio/video call
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("secure-call"))
///     .to("Bob", "sip:bob@example.com", None)
///     .contact("<sip:alice@203.0.113.5:5061;transport=tls>", None)
///     .sdp_body(&sdp)  // Sets Content-Type to application/sdp automatically
///     .build();
/// ```
///
/// ## WebRTC Media Negotiation
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ContentBuilderExt;
/// use rvoip_sip_core::types::Method;
/// use rvoip_sip_core::sdp::SdpBuilder;
///
/// // Create an SDP session for WebRTC (with ICE and DTLS)
/// let sdp = SdpBuilder::new("WebRTC Call")
///     .origin("-", "1545997027", "1", "IN", "IP4", "198.51.100.1")
///     .time("0", "0")
///     .attribute("group", Some("BUNDLE 0 1"))
///     .attribute("ice-options", Some("trickle"))
///     .attribute("msid-semantic", Some("WMS myWebRTCStream"))
///     // Add audio media
///     .media("audio", 49203, "UDP/TLS/RTP/SAVPF")
///     .formats(&["111", "103", "104", "9", "0", "8"])
///     .connection("IN", "IP4", "198.51.100.1")
///     .attribute("rtcp", Some("60065 IN IP4 198.51.100.1"))
///     .attribute("candidate", Some("1 1 UDP 2113937151 192.168.1.4 49203 typ host"))
///     .attribute("candidate", Some("2 1 UDP 1845501695 198.51.100.1 49203 typ srflx raddr 192.168.1.4 rport 49203"))
///     .attribute("ice-ufrag", Some("F7gI"))
///     .attribute("ice-pwd", Some("x9cml/YzichV2+XlhiMu8rbz"))
///     .attribute("fingerprint", Some("sha-256 D2:B9:31:8F:DF:24:D8:0E:ED:D2:EF:25:9E:AF:6F:B8:05:A3:75:A1:A4:1C:6B:1E:55:02:A4:F9:6B:CA:F7:E6"))
///     .attribute("setup", Some("actpass"))
///     .attribute("mid", Some("0"))
///     .attribute("rtpmap", Some("111 opus/48000/2"))
///     .attribute("rtcp-fb", Some("111 transport-cc"))
///     .attribute("fmtp", Some("111 minptime=10;useinbandfec=1"))
///     .attribute("rtpmap", Some("103 ISAC/16000"))
///     .attribute("rtpmap", Some("104 ISAC/32000"))
///     .attribute("rtpmap", Some("9 G722/8000"))
///     .attribute("rtpmap", Some("0 PCMU/8000"))
///     .attribute("rtpmap", Some("8 PCMA/8000"))
///     .attribute("extmap", Some("1 urn:ietf:params:rtp-hdrext:ssrc-audio-level"))
///     .attribute("ssrc", Some("2655508255 cname:ConferenceTool"))
///     .attribute("ssrc", Some("2655508255 msid:myWebRTCStream audioTrack"))
///     .attribute("ssrc", Some("2655508255 mslabel:myWebRTCStream"))
///     .attribute("ssrc", Some("2655508255 label:audioTrack"))
///     .done()
///     // Add video media
///     .media("video", 49203, "UDP/TLS/RTP/SAVPF")
///     .formats(&["96", "97", "98", "99", "100", "101", "102"])
///     .connection("IN", "IP4", "198.51.100.1")
///     .attribute("rtcp", Some("60065 IN IP4 198.51.100.1"))
///     .attribute("candidate", Some("1 1 UDP 2113937151 192.168.1.4 49203 typ host"))
///     .attribute("candidate", Some("2 1 UDP 1845501695 198.51.100.1 49203 typ srflx raddr 192.168.1.4 rport 49203"))
///     .attribute("ice-ufrag", Some("F7gI"))
///     .attribute("ice-pwd", Some("x9cml/YzichV2+XlhiMu8rbz"))
///     .attribute("fingerprint", Some("sha-256 D2:B9:31:8F:DF:24:D8:0E:ED:D2:EF:25:9E:AF:6F:B8:05:A3:75:A1:A4:1C:6B:1E:55:02:A4:F9:6B:CA:F7:E6"))
///     .attribute("setup", Some("actpass"))
///     .attribute("mid", Some("1"))
///     .attribute("rtpmap", Some("96 VP8/90000"))
///     .attribute("rtcp-fb", Some("96 ccm fir"))
///     .attribute("rtcp-fb", Some("96 nack"))
///     .attribute("rtcp-fb", Some("96 nack pli"))
///     .attribute("rtcp-fb", Some("96 goog-remb"))
///     .attribute("rtpmap", Some("97 rtx/90000"))
///     .attribute("fmtp", Some("97 apt=96"))
///     .attribute("rtpmap", Some("98 H264/90000"))
///     .attribute("rtcp-fb", Some("98 ccm fir"))
///     .attribute("rtcp-fb", Some("98 nack"))
///     .attribute("rtcp-fb", Some("98 nack pli"))
///     .attribute("fmtp", Some("98 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"))
///     .attribute("ssrc-group", Some("FID 1629250457 3327211740"))
///     .attribute("ssrc", Some("1629250457 cname:ConferenceTool"))
///     .attribute("ssrc", Some("1629250457 msid:myWebRTCStream videoTrack"))
///     .attribute("ssrc", Some("1629250457 mslabel:myWebRTCStream"))
///     .attribute("ssrc", Some("1629250457 label:videoTrack"))
///     .done()
///     .build()
///     .unwrap();
///
/// // Create a WebRTC SIP INVITE
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:webrtc-gateway.example.com").unwrap()
///     .from("WebRTC User", "sip:webuser@example.com", Some("webrtc1"))
///     .to("SIP Peer", "sip:sipuser@example.com", None)
///     .contact("<sip:webuser@198.51.100.1:5060;transport=ws>", None)
///     .sdp_body(&sdp)  // Sets Content-Type to application/sdp automatically
///     .build();
/// ```
pub trait ContentBuilderExt: ContentTypeBuilderExt {
    /// Add an SDP session as the message body
    ///
    /// This method sets the Content-Type to application/sdp and adds the SDP session
    /// as the message body. It automatically handles setting both the header and the body.
    /// 
    /// SDP (Session Description Protocol) is defined in [RFC 4566](https://datatracker.ietf.org/doc/html/rfc4566)
    /// and is commonly used in SIP for media negotiation, particularly in INVITE requests
    /// and their responses.
    ///
    /// # Parameters
    ///
    /// - `sdp`: The SDP session to add as the body
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ## Basic Audio Call Setup
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ContentBuilderExt;
    /// use rvoip_sip_core::types::Method;
    /// use rvoip_sip_core::sdp::SdpBuilder;
    ///
    /// // Create an SDP builder for a basic audio call
    /// let sdp = SdpBuilder::new("Phone Call")
    ///     .origin("alice", "123", "456", "IN", "IP4", "192.0.2.1")
    ///     .connection("IN", "IP4", "192.0.2.1")
    ///     .time("0", "0")
    ///     .media("audio", 49170, "RTP/AVP")
    ///     .formats(&["0", "8"])  // PCMU and PCMA
    ///     .attribute("rtpmap", Some("0 PCMU/8000"))
    ///     .attribute("rtpmap", Some("8 PCMA/8000"))
    ///     .attribute("ptime", Some("20"))
    ///     .done()
    ///     .build()
    ///     .unwrap();
    ///
    /// // Create an INVITE request with the SDP body
    /// let invite = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("tag123"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .contact("<sip:alice@192.0.2.1:5060>", None)
    ///     .sdp_body(&sdp)
    ///     .build();
    /// ```
    ///
    /// ## SIP Response with SDP Answer
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::builder::headers::ContentBuilderExt;
    /// use rvoip_sip_core::types::StatusCode;
    /// use rvoip_sip_core::sdp::SdpBuilder;
    ///
    /// // Create an SDP answer selecting specific codecs
    /// let sdp = SdpBuilder::new("Answer")
    ///     .origin("bob", "789", "012", "IN", "IP4", "192.0.2.2")
    ///     .connection("IN", "IP4", "192.0.2.2")
    ///     .time("0", "0")
    ///     .media("audio", 45678, "RTP/AVP")
    ///     .formats(&["0"])  // Only accepting PCMU
    ///     .attribute("rtpmap", Some("0 PCMU/8000"))
    ///     .attribute("ptime", Some("20"))
    ///     .done()
    ///     .build()
    ///     .unwrap();
    ///
    /// // Create a 200 OK response with the SDP answer
    /// let response = SimpleResponseBuilder::new(StatusCode::Ok, Some("OK"))
    ///     .from("Bob", "sip:bob@example.com", Some("tag456"))
    ///     .to("Alice", "sip:alice@example.com", Some("tag123"))  // Preserve tag from request
    ///     .contact("<sip:bob@192.0.2.2:5060>", None)
    ///     .sdp_body(&sdp)
    ///     .build();
    /// ```
    ///
    /// ## Conference Call Setup
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ContentBuilderExt;
    /// use rvoip_sip_core::types::Method;
    /// use rvoip_sip_core::sdp::SdpBuilder;
    ///
    /// // Create SDP for a conference call with multiple audio formats
    /// let sdp = SdpBuilder::new("Conference Call")
    ///     .origin("confserver", "2890844527", "2890844527", "IN", "IP4", "conference.example.com")
    ///     .info("Conference call with audio mixing")
    ///     .connection("IN", "IP4", "conference.example.com")
    ///     .time("0", "0")
    ///     .media("audio", 49170, "RTP/AVP")
    ///     .formats(&["0", "8", "96", "97"])
    ///     .attribute("rtpmap", Some("0 PCMU/8000"))
    ///     .attribute("rtpmap", Some("8 PCMA/8000"))
    ///     .attribute("rtpmap", Some("96 opus/48000/2"))
    ///     .attribute("rtpmap", Some("97 telephone-event/8000"))
    ///     .attribute("fmtp", Some("97 0-16"))
    ///     .attribute("ssrc", Some("1234567890 cname:conf@example.com"))
    ///     .attribute("ssrc", Some("1234567890 label:conference-mixer"))
    ///     .attribute("maxptime", Some("40"))
    ///     .attribute("recvonly", None::<&str>)
    ///     .done()
    ///     .build()
    ///     .unwrap();
    ///
    /// // Create an INVITE to join a conference
    /// let invite = SimpleRequestBuilder::invite("sip:conference@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("confcall"))
    ///     .to("Conference", "sip:conference@example.com", None)
    ///     .contact("<sip:alice@192.0.2.1:5060>", None)
    ///     .sdp_body(&sdp)
    ///     .build();
    /// ```
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
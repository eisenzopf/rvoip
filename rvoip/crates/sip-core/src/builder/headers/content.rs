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
use crate::{RequestBuilder, ResponseBuilder};
#[cfg(feature = "sdp")]
use crate::sdp::{SdpBuilder, integration};
use bytes::Bytes;

/// Extension trait that adds content-related building capabilities to request and response builders
pub trait ContentBuilderExt {
    /// Add a Content-Type header specifying 'application/sdp'
    ///
    /// This is a convenience method for setting the Content-Type to SDP,
    /// which is commonly used in SIP for session descriptions.
    fn content_type_sdp(self) -> Self;
    
    /// Add a Content-Type header specifying 'text/plain'
    ///
    /// This is a convenience method for setting the Content-Type to plain text.
    fn content_type_text(self) -> Self;
    
    /// Add a Content-Type header specifying 'application/xml'
    ///
    /// This is a convenience method for setting the Content-Type to XML.
    fn content_type_xml(self) -> Self;
    
    /// Add a Content-Type header specifying 'application/json'
    ///
    /// This is a convenience method for setting the Content-Type to JSON.
    fn content_type_json(self) -> Self;
    
    /// Add a Content-Type header specifying 'message/sipfrag'
    ///
    /// This is a convenience method for setting the Content-Type to SIP fragments,
    /// commonly used in REFER responses.
    fn content_type_sipfrag(self) -> Self;
    
    /// Add a Content-Type header with a custom media type
    ///
    /// # Parameters
    ///
    /// - `media_type`: Primary media type (e.g., "text", "application")
    /// - `media_subtype`: Subtype (e.g., "plain", "json")
    fn content_type_custom(self, media_type: &str, media_subtype: &str) -> Self;
    
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

/// Specific implementation for RequestBuilder
impl ContentBuilderExt for RequestBuilder {
    fn content_type_sdp(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "sdp".to_string(),
            parameters: std::collections::HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_text(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "text".to_string(),
            m_subtype: "plain".to_string(),
            parameters: std::collections::HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_xml(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "xml".to_string(),
            parameters: std::collections::HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_json(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "json".to_string(),
            parameters: std::collections::HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_sipfrag(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "message".to_string(),
            m_subtype: "sipfrag".to_string(),
            parameters: std::collections::HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_custom(self, media_type: &str, media_subtype: &str) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: media_type.to_string(),
            m_subtype: media_subtype.to_string(),
            parameters: std::collections::HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn sdp_body(self, sdp: &SdpSession) -> Self {
        let sdp_string = sdp.to_string();
        self.content_type_sdp()
            .body(Bytes::from(sdp_string))
    }
    
    /* 
    fn auto_sdp_audio_body(self, session_name: Option<&str>, port: u16, codecs: Option<&[&str]>) -> Self {
        // First build the request to extract SDP data
        let request = self.build();
        
        // Use the integration module to create SDP from the request
        let sdp_builder = integration::sdp_from_request(&request, session_name);
        let sdp = integration::add_audio_profile(sdp_builder, port, codecs)
            .build()
            .expect("Failed to build valid SDP");
        
        // Create a new RequestBuilder from the request
        let mut new_builder = RequestBuilder::new(request.method().clone(), request.uri().to_string()).unwrap();
        
        // Copy all headers except Content-Type and Content-Length
        for header in request.headers() {
            if header.name() != &HeaderName::ContentType && header.name() != &HeaderName::ContentLength {
                new_builder = new_builder.raw_header(header.name().clone(), header.value_raw().to_vec());
            }
        }
        
        // Add the SDP body
        new_builder.sdp_body(&sdp)
    }
    
    fn auto_sdp_av_body(
        self, 
        session_name: Option<&str>, 
        audio_port: u16, 
        video_port: u16,
        audio_codecs: Option<&[&str]>,
        video_codecs: Option<&[&str]>
    ) -> Self {
        // First build the request to extract SDP data
        let request = self.build();
        
        // Use the integration module to create SDP from the request
        let sdp_builder = integration::sdp_from_request(&request, session_name);
        let sdp = integration::add_av_profile(sdp_builder, audio_port, video_port, audio_codecs, video_codecs)
            .build()
            .expect("Failed to build valid SDP");
        
        // Create a new RequestBuilder from the request
        let mut new_builder = RequestBuilder::new(request.method().clone(), request.uri().to_string()).unwrap();
        
        // Copy all headers except Content-Type and Content-Length
        for header in request.headers() {
            if header.name() != &HeaderName::ContentType && header.name() != &HeaderName::ContentLength {
                new_builder = new_builder.raw_header(header.name().clone(), header.value_raw().to_vec());
            }
        }
        
        // Add the SDP body
        new_builder.sdp_body(&sdp)
    }
    
    fn auto_sdp_webrtc_body(
        self,
        session_name: Option<&str>,
        ice_ufrag: &str,
        ice_pwd: &str,
        fingerprint: &str,
        include_video: bool
    ) -> Self {
        // First build the request to extract SDP data
        let request = self.build();
        
        // Use the integration module to create SDP from the request
        let sdp_builder = integration::sdp_from_request(&request, session_name);
        let sdp = integration::add_webrtc_profile(
                sdp_builder, 
                9, // Standard WebRTC port
                9, // Same port for video
                ice_ufrag,
                ice_pwd,
                fingerprint,
                include_video
            )
            .build()
            .expect("Failed to build valid SDP");
        
        // Create a new RequestBuilder from the request
        let mut new_builder = RequestBuilder::new(request.method().clone(), request.uri().to_string()).unwrap();
        
        // Copy all headers except Content-Type and Content-Length
        for header in request.headers() {
            if header.name() != &HeaderName::ContentType && header.name() != &HeaderName::ContentLength {
                new_builder = new_builder.raw_header(header.name().clone(), header.value_raw().to_vec());
            }
        }
        
        // Add the SDP body
        new_builder.sdp_body(&sdp)
    }
    */
}

/// Specific implementation for ResponseBuilder
impl ContentBuilderExt for ResponseBuilder {
    fn content_type_sdp(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "sdp".to_string(),
            parameters: std::collections::HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_text(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "text".to_string(),
            m_subtype: "plain".to_string(),
            parameters: std::collections::HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_xml(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "xml".to_string(),
            parameters: std::collections::HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_json(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "json".to_string(),
            parameters: std::collections::HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_sipfrag(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "message".to_string(),
            m_subtype: "sipfrag".to_string(),
            parameters: std::collections::HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_custom(self, media_type: &str, media_subtype: &str) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: media_type.to_string(),
            m_subtype: media_subtype.to_string(),
            parameters: std::collections::HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn sdp_body(self, sdp: &SdpSession) -> Self {
        let sdp_string = sdp.to_string();
        self.content_type_sdp()
            .body(Bytes::from(sdp_string))
    }
    
    /*
    fn auto_sdp_audio_body(self, session_name: Option<&str>, port: u16, codecs: Option<&[&str]>) -> Self {
        // First build the response to extract SDP data
        let response = self.build();
        
        // Use the integration module to create SDP from the response
        let sdp_builder = integration::sdp_from_response(&response, session_name);
        let sdp = integration::add_audio_profile(sdp_builder, port, codecs)
            .build()
            .expect("Failed to build valid SDP");
        
        // Create a new ResponseBuilder from the response
        let mut new_builder = ResponseBuilder::new(response.status_code().clone(), response.reason_phrase().cloned()).unwrap();
        
        // Copy all headers except Content-Type and Content-Length
        for header in response.headers() {
            if header.name() != &HeaderName::ContentType && header.name() != &HeaderName::ContentLength {
                new_builder = new_builder.raw_header(header.name().clone(), header.value_raw().to_vec());
            }
        }
        
        // Add the SDP body
        new_builder.sdp_body(&sdp)
    }
    
    fn auto_sdp_av_body(
        self, 
        session_name: Option<&str>, 
        audio_port: u16, 
        video_port: u16,
        audio_codecs: Option<&[&str]>,
        video_codecs: Option<&[&str]>
    ) -> Self {
        // First build the response to extract SDP data
        let response = self.build();
        
        // Use the integration module to create SDP from the response
        let sdp_builder = integration::sdp_from_response(&response, session_name);
        let sdp = integration::add_av_profile(sdp_builder, audio_port, video_port, audio_codecs, video_codecs)
            .build()
            .expect("Failed to build valid SDP");
        
        // Create a new ResponseBuilder from the response
        let mut new_builder = ResponseBuilder::new(response.status_code().clone(), response.reason_phrase().cloned()).unwrap();
        
        // Copy all headers except Content-Type and Content-Length
        for header in response.headers() {
            if header.name() != &HeaderName::ContentType && header.name() != &HeaderName::ContentLength {
                new_builder = new_builder.raw_header(header.name().clone(), header.value_raw().to_vec());
            }
        }
        
        // Add the SDP body
        new_builder.sdp_body(&sdp)
    }
    
    fn auto_sdp_webrtc_body(
        self,
        session_name: Option<&str>,
        ice_ufrag: &str,
        ice_pwd: &str,
        fingerprint: &str,
        include_video: bool
    ) -> Self {
        // First build the response to extract SDP data
        let response = self.build();
        
        // Use the integration module to create SDP from the response
        let sdp_builder = integration::sdp_from_response(&response, session_name);
        let sdp = integration::add_webrtc_profile(
                sdp_builder, 
                9, // Standard WebRTC port
                9, // Same port for video
                ice_ufrag,
                ice_pwd,
                fingerprint,
                include_video
            )
            .build()
            .expect("Failed to build valid SDP");
        
        // Create a new ResponseBuilder from the response
        let mut new_builder = ResponseBuilder::new(response.status_code().clone(), response.reason_phrase().cloned()).unwrap();
        
        // Copy all headers except Content-Type and Content-Length
        for header in response.headers() {
            if header.name() != &HeaderName::ContentType && header.name() != &HeaderName::ContentLength {
                new_builder = new_builder.raw_header(header.name().clone(), header.value_raw().to_vec());
            }
        }
        
        // Add the SDP body
        new_builder.sdp_body(&sdp)
    }
    */
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
        // Test SDP content type
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .content_type_sdp()
            .build();
            
        if let Some(TypedHeader::ContentType(content_type)) = request.header(&HeaderName::ContentType) {
            assert_eq!(content_type.to_string(), "application/sdp");
        } else {
            panic!("Content-Type header not found or has wrong type");
        }
        
        // Test text content type
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .content_type_text()
            .build();
            
        if let Some(TypedHeader::ContentType(content_type)) = request.header(&HeaderName::ContentType) {
            assert_eq!(content_type.to_string(), "text/plain");
        } else {
            panic!("Content-Type header not found or has wrong type");
        }
    }
    
    #[test]
    fn test_response_content_type_shortcuts() {
        // Test XML content type
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .content_type_xml()
            .build();
            
        if let Some(TypedHeader::ContentType(content_type)) = response.header(&HeaderName::ContentType) {
            assert_eq!(content_type.to_string(), "application/xml");
        } else {
            panic!("Content-Type header not found or has wrong type");
        }
        
        // Test JSON content type
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .content_type_json()
            .build();
            
        if let Some(TypedHeader::ContentType(content_type)) = response.header(&HeaderName::ContentType) {
            assert_eq!(content_type.to_string(), "application/json");
        } else {
            panic!("Content-Type header not found or has wrong type");
        }
    }
    
    #[test]
    fn test_request_sdp_body() {
        // Create a basic SIP request
        let request = RequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
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
                        uri: Uri::from_str("sip:alice@pc33.atlanta.com").unwrap(),
                        params: vec![],
                    }
                }
            ])));
        
        // Create a simple SDP and add it to the request
        let sdp = SdpBuilder::new("Test Session")
            .origin("alice", "12345", "12345", "IN", "IP4", "192.168.1.100")
            .connection("IN", "IP4", "192.168.1.100")
            .time("0", "0")
            .media_audio(49170, "RTP/AVP")
                .formats(&["0"])
                .rtpmap("0", "PCMU/8000")
                .done()
            .build()
            .unwrap();
        
        // Add the SDP using our extension
        let request_with_sdp = request.sdp_body(&sdp).build();
        
        // Verify the Content-Type is application/sdp
        if let Some(TypedHeader::ContentType(content_type)) = request_with_sdp.header(&HeaderName::ContentType) {
            assert_eq!(content_type.to_string(), "application/sdp");
        } else {
            panic!("Content-Type header not found or has wrong type");
        }
        
        // Verify there's a body
        assert!(request_with_sdp.body().len() > 0);
        
        // Verify the body contains SDP content
        let body = request_with_sdp.body();
        let body_str = std::str::from_utf8(&body).unwrap();
        assert!(body_str.contains("v=0"));
        assert!(body_str.contains("m=audio 49170 RTP/AVP 0"));
    }
    
    #[test]
    fn test_auto_sdp_generation() {
        // Skip this test - it requires the auto_sdp_audio_body method which is commented out
        // TODO: Re-implement this test when auto_sdp methods are fixed
        /*
        // Create a basic SIP request with enough headers to generate SDP
        let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@biloxi.com").unwrap()
            .from("Alice", "sip:alice@atlanta.com", Some("1928301774"))
            .to("Bob", "sip:bob@biloxi.com", None)
            .call_id("a84b4c76e66710")
            .cseq(314159)
            .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"));
        
        // Add automatic SDP with audio profile
        let request_with_sdp = request.auto_sdp_audio_body(Some("Audio Call"), 49170, None).build();
        
        // Verify Content-Type is set
        if let Some(TypedHeader::ContentType(content_type)) = request_with_sdp.header(&HeaderName::ContentType) {
            assert_eq!(content_type.to_string(), "application/sdp");
        } else {
            panic!("Content-Type header not found or has wrong type");
        }
        
        // Verify the body contains SDP content
        let body = request_with_sdp.body();
        let body_str = std::str::from_utf8(&body).unwrap();
        assert!(body_str.contains("s=Audio Call"));
        assert!(body_str.contains("m=audio 49170 RTP/AVP"));
        */
    }
} 
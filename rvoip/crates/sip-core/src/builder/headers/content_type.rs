use crate::types::{
    content_type::ContentType,
    TypedHeader,
};
use crate::parser::headers::content_type::ContentTypeValue;
use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::builder::headers::HeaderSetter;
use std::collections::HashMap;
use std::str::FromStr;

/// Content-Type Header Builder for SIP Messages
///
/// This module provides builder methods for the Content-Type header in SIP messages,
/// which specifies the media type of the message body.
///
/// ## SIP Content-Type Header Overview
///
/// The Content-Type header is defined in [RFC 3261 Section 20.15](https://datatracker.ietf.org/doc/html/rfc3261#section-20.15)
/// as part of the core SIP protocol. It follows the syntax and semantics defined in 
/// [RFC 2045](https://datatracker.ietf.org/doc/html/rfc2045) for MIME media types.
///
/// ## Purpose of Content-Type Header
///
/// The Content-Type header serves several critical purposes in SIP:
///
/// 1. It identifies the format and encoding of the message body
/// 2. It enables proper parsing and interpretation of the message payload
/// 3. It allows for multiple content types via multipart MIME messages
/// 4. It facilitates negotiation of acceptable content types between endpoints
///
/// ## Common MIME Types in SIP
///
/// - **application/sdp**: Session Description Protocol for media negotiation (most common in INVITE)
/// - **text/plain**: Simple text messages (used in MESSAGE requests)
/// - **application/xml**: XML-based content (for XCAP, presence, etc.)
/// - **application/json**: JSON-formatted data (for various application protocols)
/// - **message/sipfrag**: SIP message fragments (used in NOTIFY for REFER status)
/// - **multipart/mixed**: Mixed content types bundled together
/// - **multipart/alternative**: Alternative representations of the same content
/// - **multipart/related**: Related content with inline references between parts
///
/// ## Relationship with other headers
///
/// - **Content-Type** + **Content-Length**: Together specify what and how much content is present
/// - **Content-Type** + **MIME-Version**: MIME-Version (typically "1.0") should be included when using multipart content
/// - **Content-Type** + **Accept**: Content-Type specifies what is sent, Accept indicates what can be received
/// - **Content-Type** + **Content-Disposition**: Content-Disposition indicates how the content should be handled
/// - **Content-Type** + **Content-ID**: Content-ID provides identifiers for parts in multipart contents
///
/// ## Special Considerations
///
/// 1. **Parameters**: Content-Type may include parameters (e.g., `charset=UTF-8`, `boundary=xyz`)
/// 2. **Multipart Content**: When sending multiple body parts, use multipart/* types with a boundary parameter
/// 3. **MIME-Version**: Include a MIME-Version header when using multipart content
/// 4. **Compact Form**: Content-Type has the compact form 'c', though this is less commonly used
///
/// # Examples
///
/// ## Basic Content Types
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
/// use rvoip_sip_core::types::Method;
/// use rvoip_sip_core::sdp::SdpBuilder;
///
/// // Create a request with Content-Type set to application/sdp using SdpBuilder
/// let sdp = SdpBuilder::new("Call with Alice")
///     .origin("alice", "2890844526", "2890844526", "IN", "IP4", "192.0.2.1")
///     .connection("IN", "IP4", "192.0.2.1")
///     .time("0", "0")
///     .media("audio", 49170, "RTP/AVP")
///     .formats(&["0", "8"])
///     .attribute("rtpmap", Some("0 PCMU/8000"))
///     .attribute("rtpmap", Some("8 PCMA/8000"))
///     .done()
///     .build()
///     .unwrap();
///
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
///     .content_type_sdp()
///     .body(sdp.to_string())
///     .build();
///     
/// // Create a request with Content-Type set to text/plain
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:example.com").unwrap()
///     .content_type_text()
///     .body("Hello, this is a text message sent via SIP")
///     .build();
/// ```
///
/// ## Advanced Content Types with Parameters
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a request with Content-Type including charset parameter
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:example.com").unwrap()
///     .content_type("application/xml; charset=UTF-8")
///     .body(concat!(
///         "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
///         "<message>\r\n",
///         "  <to>Bob</to>\r\n",
///         "  <from>Alice</from>\r\n",
///         "  <content>Hello with non-ASCII characters: áéíóú</content>\r\n",
///         "</message>\r\n"
///     ))
///     .build();
///
/// // Create a request with custom application type
/// let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .content_type_custom("application", "xml+soap")
///     .body(concat!(
///         "<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\">\r\n",
///         "  <soap:Body>\r\n",
///         "    <registerRequest xmlns=\"http://example.org/register\">\r\n",
///         "      <username>alice</username>\r\n",
///         "      <password>secret</password>\r\n", 
///         "    </registerRequest>\r\n",
///         "  </soap:Body>\r\n",
///         "</soap:Envelope>\r\n"
///     ))
///     .build();
/// ```
///
/// ## Multipart MIME Content
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentTypeBuilderExt, MimeVersionBuilderExt};
/// use rvoip_sip_core::builder::multipart::{MultipartBuilder, MultipartPartBuilder};
/// use rvoip_sip_core::types::Method;
/// use rvoip_sip_core::sdp::SdpBuilder;
///
/// // Build a multipart/mixed message with text and application parts
/// let multipart = MultipartBuilder::mixed()
///     // Add a text part
///     .add_part(
///         MultipartPartBuilder::new()
///             .content_type("text/plain")
///             .body("This is the first part of the multipart message.")
///             .build()
///     )
///     // Add an application part
///     .add_part(
///         MultipartPartBuilder::new()
///             .content_type("application/json")
///             .body(r#"{"status":"success","code":200,"message":"Operation completed"}"#)
///             .build()
///     )
///     .build();
///
/// // Create the message with the multipart content
/// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
///     .mime_version(1, 0)  // Required for multipart MIME content
///     .content_type(&multipart.content_type())
///     .body(multipart.body())
///     .build();
/// ```
///
/// ## SIP MESSAGE for Instant Messaging
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Plain text message (most basic)
/// let text_message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
///     .content_type_text()
///     .body("Hey Bob, are you available for a call?")
///     .build();
///
/// // HTML-formatted message (richer formatting)
/// let html_message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
///     .content_type("text/html")
///     .body(concat!(
///         "<html><body>",
///         "<h1>Meeting Reminder</h1>",
///         "<p>Don't forget our <b>team meeting</b> at 3pm today!</p>",
///         "<p>Location: <a href=\"https://example.com/room\">Conference Room A</a></p>",
///         "</body></html>"
///     ))
///     .build();
///
/// // CPIM-wrapped message (for federation between different IM systems)
/// let cpim_message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
///     .content_type("message/cpim")
///     .body(concat!(
///         "From: <sip:alice@example.com>\r\n",
///         "To: <sip:bob@example.net>\r\n",
///         "DateTime: 2023-05-15T14:33:22Z\r\n",
///         "Content-Type: text/plain; charset=utf-8\r\n",
///         "\r\n",
///         "Hello Bob, this is a CPIM wrapped message!"
///     ))
///     .build();
/// ```
///
/// ## Basic Content-Type Headers
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
/// use rvoip_sip_core::types::Method;
/// use rvoip_sip_core::sdp::SdpBuilder;
///
/// // Create a request with Content-Type set to application/sdp
/// let sdp = SdpBuilder::new("Call with Alice")
///     .origin("alice", "2890844526", "2890844526", "IN", "IP4", "192.0.2.1")
///     .connection("IN", "IP4", "192.0.2.1")
///     .time("0", "0")
///     .media("audio", 49170, "RTP/AVP")
///     .formats(&["0", "8"])
///     .attribute("rtpmap", Some("0 PCMU/8000"))
///     .attribute("rtpmap", Some("8 PCMA/8000"))
///     .done()
///     .build()
///     .unwrap();
///     
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
///     .content_type_sdp()
///     .body(sdp.to_string())
///     .build();
///     
/// // Create a request with Content-Type set to application/json
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:example.com").unwrap()
///     .content_type_json()
///     .body(r#"{"message": "Hello world", "from": "Alice", "timestamp": 1620000000}"#)
///     .build();
/// ```
///
/// ## Content-Type with Parameters
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a request with Content-Type including character set parameter
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:example.com").unwrap()
///     .content_type("application/xml; charset=UTF-8")
///     .body(concat!(
///         "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
///         "<message>\r\n",
///         "  <to>Bob</to>\r\n",
///         "  <from>Alice</from>\r\n",
///         "  <content>Hello with non-ASCII characters: áéíóú</content>\r\n",
///         "</message>\r\n"
///     ))
///     .build();
/// ```
///
/// ## Special Content Types for SIP Extensions
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a NOTIFY request with SIP fragment body (used in REFER/NOTIFY scenarios)
/// let request = SimpleRequestBuilder::new(Method::Notify, "sip:user@example.com").unwrap()
///     .content_type_sipfrag()  // Sets Content-Type: message/sipfrag
///     .body("SIP/2.0 200 OK\r\nCSeq: 1 INVITE\r\nContact: <sip:bob@192.0.2.4>\r\n")
///     .build();
///     
/// // Create a MESSAGE with multimedia messaging content
/// let message = SimpleRequestBuilder::new(Method::Message, "sip:user@example.com").unwrap()
///     .content_type_custom("application", "vnd.3gpp.sms")  // 3GPP SMS format
///     .body("01000B915121551532F40000A723719C0E4ACF41F4329E0E")  // Binary SMS content (hex encoded)
///     .build();
///     
/// // Create a MESSAGE with CPIM content for instant messaging federation
/// let message = SimpleRequestBuilder::new(Method::Message, "sip:user@example.com").unwrap()
///     .content_type_custom("message", "cpim")  // Common Presence and Instant Messaging
///     .body(concat!(
///         "From: <sip:alice@example.com>\r\n",
///         "To: <sip:bob@example.net>\r\n",
///         "DateTime: 2023-05-15T14:33:22Z\r\n",
///         "Content-Type: text/plain; charset=utf-8\r\n",
///         "\r\n",
///         "Hello Bob, this is a CPIM wrapped message"
///     ))
///     .build();
/// ```
///
/// ## Multipart MIME Content with MIME-Version
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentTypeBuilderExt, MimeVersionBuilderExt};
/// use rvoip_sip_core::builder::multipart::{MultipartBuilder, MultipartPartBuilder};
/// use rvoip_sip_core::types::Method;
/// use rvoip_sip_core::sdp::SdpBuilder;
///
/// // Create an INVITE with both SDP and XML metadata using multipart/mixed
/// // First, create the SDP using SdpBuilder
/// let sdp = SdpBuilder::new("WebRTC Call")
///     .origin("alice", "1234567890", "1", "IN", "IP4", "192.0.2.1")
///     .time("0", "0")
///     .connection("IN", "IP4", "192.0.2.1")
///     .media("audio", 49170, "RTP/AVP")
///     .formats(&["0", "8"])
///     .attribute("rtpmap", Some("0 PCMU/8000"))
///     .attribute("rtpmap", Some("8 PCMA/8000"))
///     .done()
///     .build()
///     .unwrap();
///
/// // Then create the XML metadata about the call
/// let metadata_xml = concat!(
///     "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
///     "<session-metadata xmlns=\"http://example.org/schemas/call-metadata\">\r\n",
///     "  <session-type>customer-support</session-type>\r\n",
///     "  <priority>high</priority>\r\n",
///     "  <case-id>CS-12345</case-id>\r\n",
///     "  <agent-info>\r\n",
///     "    <name>Alice Smith</name>\r\n",
///     "    <department>Technical Support</department>\r\n",
///     "    <expertise>Networking</expertise>\r\n",
///     "  </agent-info>\r\n",
///     "</session-metadata>\r\n"
/// );
///
/// // Build the multipart body with both SDP and XML parts
/// let multipart = MultipartBuilder::mixed()
///     // Add SDP part with content-disposition
///     .add_part(
///         MultipartPartBuilder::new()
///             .content_type("application/sdp")
///             .content_disposition("session")
///             .body(sdp.to_string())
///             .build()
///     )
///     // Add XML metadata part with content-id
///     .add_part(
///         MultipartPartBuilder::new()
///             .content_type("application/xml")
///             .content_id("<metadata@call.example.com>")
///             .body(metadata_xml)
///             .build()
///     )
///     .build();
///
/// // Create the INVITE request with multipart content and MIME-Version header
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:support@example.com").unwrap()
///     .mime_version(1, 0)  // Required when using multipart MIME content
///     .content_type(&multipart.content_type())
///     .body(multipart.body())
///     .build();
///
/// // The resulting message will have:
/// // - MIME-Version: 1.0 header
/// // - Content-Type: multipart/mixed;boundary="..." header
/// // - A multipart body with both SDP and XML parts, each with their own headers
/// ```
///
/// ## Content Negotiation with Accept Header
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
/// use rvoip_sip_core::types::{Method, StatusCode, TypedHeader};
/// use std::collections::HashMap;
///
/// // Client performs content negotiation by indicating preferred content types
/// let options_request = SimpleRequestBuilder::new(Method::Options, "sip:media-server.example.com").unwrap()
///     // Client would add Accept headers here to indicate supported content types
///     .build();
///
/// // Server responds with supported content types
/// let options_response = SimpleResponseBuilder::new(StatusCode::Ok, None)
///     .content_type_json()
///     .body(r#"{
///         "supported_content_types": [
///             "application/sdp",
///             "application/json",
///             "multipart/mixed",
///             "text/plain"
///         ],
///         "supported_codecs": [
///             {"name": "PCMU", "rate": 8000, "channels": 1},
///             {"name": "PCMA", "rate": 8000, "channels": 1},
///             {"name": "opus", "rate": 48000, "channels": 2}
///         ]
///     }"#)
///     .build();
/// ```
///
/// ## WebRTC Integration with SDP
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentTypeBuilderExt, FromBuilderExt, ToBuilderExt, MimeVersionBuilderExt};
/// use rvoip_sip_core::types::Method;
/// use rvoip_sip_core::sdp::{SdpBuilder, attributes::MediaDirection};
///
/// // Create an INVITE with WebRTC SDP offer including ICE candidates, DTLS fingerprints, etc.
/// let webrtc_sdp = SdpBuilder::new("WebRTC Session")
///     .origin("-", "1620046190", "1", "IN", "IP4", "0.0.0.0")
///     .time("0", "0")
///     .connection("IN", "IP4", "0.0.0.0")
///     .attribute("group", Some("BUNDLE audio video"))
///     .attribute("ice-options", Some("trickle"))
///     .attribute("msid-semantic", Some("WMS myStreamId"))
///     .attribute("fingerprint", Some("sha-256 11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00"))
///     .attribute("setup", Some("actpass"))
///     // Audio m-line
///     .media("audio", 9, "UDP/TLS/RTP/SAVPF")
///     .formats(&["111", "103", "104", "9", "0", "8", "106", "105", "13", "110", "112", "113", "126"])
///     .attribute("mid", Some("audio"))
///     .attribute("rtcp-mux", None::<String>)
///     .attribute("rtpmap", Some("111 opus/48000/2"))
///     .attribute("rtpmap", Some("103 ISAC/16000"))
///     .attribute("rtpmap", Some("104 ISAC/32000"))
///     .attribute("rtpmap", Some("9 G722/8000"))
///     .attribute("rtpmap", Some("0 PCMU/8000"))
///     .attribute("rtpmap", Some("8 PCMA/8000"))
///     .attribute("ice-ufrag", Some("f9VNxLFnYLSIFxwy"))
///     .attribute("ice-pwd", Some("e2L+D3XLNoQubRpHLxHQGjOJ"))
///     // Include one ICE candidate as example
///     .attribute("candidate", Some("1 1 UDP 2122252543 192.168.1.100 49827 typ host"))
///     .attribute("end-of-candidates", None::<String>)
///     // Direction and other media-specific attributes
///     .attribute("sendrecv", None::<String>)
///     .attribute("rtcp-fb", Some("111 transport-cc"))
///     .attribute("fmtp", Some("111 minptime=10;useinbandfec=1"))
///     .done()
///     .build()
///     .unwrap();
///
/// // Create the INVITE request with the WebRTC SDP
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Bob", "sip:bob@example.com", None)
///     .content_type_sdp()
///     .body(webrtc_sdp.to_string())
///     .build();
/// ```
pub trait ContentTypeBuilderExt {
    /// Add a Content-Type header specifying 'application/sdp'
    ///
    /// This is a convenience method for setting the Content-Type to SDP,
    /// which is commonly used in SIP for session descriptions.
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    /// use rvoip_sip_core::sdp::SdpBuilder;
    ///
    /// // Create an INVITE with SDP body for call setup
    /// let sdp = SdpBuilder::new("Call with Alice")
    ///     .origin("alice", "2890844526", "2890844526", "IN", "IP4", "192.0.2.1")
    ///     .connection("IN", "IP4", "192.0.2.1")
    ///     .time("0", "0")
    ///     .media("audio", 49170, "RTP/AVP")
    ///     .formats(&["0", "8"])
    ///     .attribute("rtpmap", Some("0 PCMU/8000"))
    ///     .attribute("rtpmap", Some("8 PCMA/8000"))
    ///     .done()
    ///     .build()
    ///     .unwrap();
    ///
    /// let invite = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    ///     .content_type_sdp()
    ///     .body(sdp.to_string())
    ///     .build();
    /// ```
    fn content_type_sdp(self) -> Self;
    
    /// Add a Content-Type header specifying 'text/plain'
    ///
    /// This is a convenience method for setting the Content-Type to plain text.
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a MESSAGE request with plain text body
    /// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
    ///     .content_type_text()
    ///     .body("Hello, this is a text message")
    ///     .build();
    /// ```
    fn content_type_text(self) -> Self;
    
    /// Add a Content-Type header specifying 'application/xml'
    ///
    /// This is a convenience method for setting the Content-Type to XML.
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a MESSAGE request with XML body
    /// let xml_content = concat!(
    ///     "<?xml version=\"1.0\"?>\r\n",
    ///     "<message>\r\n",
    ///     "  <text>Hello world</text>\r\n",
    ///     "  <importance>high</importance>\r\n",
    ///     "</message>\r\n"
    /// );
    /// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
    ///     .content_type_xml()
    ///     .body(xml_content)
    ///     .build();
    /// ```
    fn content_type_xml(self) -> Self;
    
    /// Add a Content-Type header specifying 'application/json'
    ///
    /// This is a convenience method for setting the Content-Type to JSON.
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a MESSAGE request with JSON body
    /// let json_content = r#"{
    ///   "message": "Hello world",
    ///   "priority": "normal",
    ///   "sender": {
    ///     "name": "Alice",
    ///     "uri": "sip:alice@example.com"
    ///   },
    ///   "timestamp": 1620000000
    /// }"#;
    /// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
    ///     .content_type_json()
    ///     .body(json_content)
    ///     .build();
    /// ```
    fn content_type_json(self) -> Self;
    
    /// Add a Content-Type header specifying 'message/sipfrag'
    ///
    /// This is a convenience method for setting the Content-Type to SIP fragments,
    /// commonly used in REFER responses.
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a NOTIFY request with SIP fragment body
    /// // This is typically used when reporting status of a REFER request
    /// let notify = SimpleRequestBuilder::new(Method::Notify, "sip:bob@example.com").unwrap()
    ///     .content_type_sipfrag()
    ///     .body(concat!(
    ///         "SIP/2.0 200 OK\r\n",
    ///         "CSeq: 1 INVITE\r\n",
    ///         "Contact: <sip:alice@192.0.2.3>\r\n",
    ///         "Content-Type: application/sdp\r\n",
    ///         "Content-Length: 0\r\n"
    ///     ))
    ///     .build();
    /// ```
    fn content_type_sipfrag(self) -> Self;
    
    /// Add a Content-Type header with a custom media type
    ///
    /// # Parameters
    ///
    /// - `media_type`: Primary media type (e.g., "text", "application")
    /// - `media_subtype`: Subtype (e.g., "plain", "json")
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Typical custom Content-Types in SIP applications:
    ///
    /// // SMS over SIP (3GPP specification)
    /// let sms_message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
    ///     .content_type_custom("application", "vnd.3gpp.sms")
    ///     .body("01000B915121551532F40000A723719C0E4ACF41F4329E0E")  // Binary SMS content (hex encoded)
    ///     .build();
    ///     
    /// // MSRP relay setup (Message Session Relay Protocol)
    /// let msrp_message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
    ///     .content_type_custom("message", "msrp-setup")
    ///     .body(concat!(
    ///         "m=message 7654 TCP/MSRP *\r\n",
    ///         "a=accept-types:text/plain text/html message/cpim\r\n",
    ///         "a=path:msrp://atlanta.example.com:7654/jshA7weztas;tcp\r\n"
    ///     ))
    ///     .build();
    ///     
    /// // PIDF presence information
    /// let publish = SimpleRequestBuilder::new(Method::Publish, "sip:bob@example.com").unwrap()
    ///     .content_type_custom("application", "pidf+xml")
    ///     .body(concat!(
    ///         "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
    ///         "<presence xmlns=\"urn:ietf:params:xml:ns:pidf\" entity=\"sip:alice@example.com\">\r\n",
    ///         "  <tuple id=\"a123\">\r\n",
    ///         "    <status><basic>open</basic></status>\r\n",
    ///         "    <contact>sip:alice@192.0.2.1</contact>\r\n",
    ///         "  </tuple>\r\n",
    ///         "</presence>\r\n"
    ///     ))
    ///     .build();
    /// ```
    fn content_type_custom(self, media_type: &str, media_subtype: &str) -> Self;
    
    /// Add a Content-Type header
    ///
    /// Creates and adds a Content-Type header as specified in [RFC 3261 Section 20.15](https://datatracker.ietf.org/doc/html/rfc3261#section-20.15).
    /// This method allows setting a Content-Type directly from a string.
    ///
    /// # Parameters
    /// - `content_type`: The content type string (e.g., "application/sdp")
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a message with Content-Type including parameters
    /// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
    ///     .content_type("application/xml; charset=UTF-8")
    ///     .body(concat!(
    ///         "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
    ///         "<message>\r\n",
    ///         "  <to>Bob</to>\r\n",
    ///         "  <from>Alice</from>\r\n",
    ///         "  <content>Message with UTF-8 characters: áéíóú</content>\r\n",
    ///         "</message>\r\n"
    ///     ))
    ///     .build();
    ///     
    /// // Create a message with multipart Content-Type using the MultipartBuilder
    /// use rvoip_sip_core::builder::headers::MimeVersionBuilderExt;
    /// use rvoip_sip_core::builder::multipart::{MultipartBuilder, MultipartPartBuilder};
    ///
    /// // Build a multipart/mixed message with text and application parts
    /// let multipart = MultipartBuilder::mixed()
    ///     // Add a text part
    ///     .add_part(
    ///         MultipartPartBuilder::new()
    ///             .content_type("text/plain")
    ///             .body("This is the first part of the multipart message.")
    ///             .build()
    ///     )
    ///     // Add an application part
    ///     .add_part(
    ///         MultipartPartBuilder::new()
    ///             .content_type("application/json")
    ///             .body(r#"{"status":"success","code":200,"message":"Operation completed"}"#)
    ///             .build()
    ///     )
    ///     .build();
    ///
    /// // Create the message with the multipart content
    /// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
    ///     .mime_version(1, 0)  // Required for multipart MIME content
    ///     .content_type(&multipart.content_type())
    ///     .body(multipart.body())
    ///     .build();
    /// ```
    ///
    /// # Note
    ///
    /// If the content type cannot be parsed, this method will silently fail.
    fn content_type(self, content_type: &str) -> Self;
}

impl ContentTypeBuilderExt for SimpleRequestBuilder {
    fn content_type_sdp(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "sdp".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_text(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "text".to_string(),
            m_subtype: "plain".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_xml(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "xml".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_json(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "json".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_sipfrag(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "message".to_string(),
            m_subtype: "sipfrag".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_custom(self, media_type: &str, media_subtype: &str) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: media_type.to_string(),
            m_subtype: media_subtype.to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type(self, content_type: &str) -> Self {
        match ContentType::from_str(content_type) {
            Ok(ct) => {
                self.header(TypedHeader::ContentType(ct))
            },
            Err(_) => self // Silently fail if content type is invalid
        }
    }
}

impl ContentTypeBuilderExt for SimpleResponseBuilder {
    fn content_type_sdp(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "sdp".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_text(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "text".to_string(),
            m_subtype: "plain".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_xml(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "xml".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_json(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "json".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_sipfrag(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "message".to_string(),
            m_subtype: "sipfrag".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_custom(self, media_type: &str, media_subtype: &str) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: media_type.to_string(),
            m_subtype: media_subtype.to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type(self, content_type: &str) -> Self {
        match ContentType::from_str(content_type) {
            Ok(ct) => {
                self.header(TypedHeader::ContentType(ct))
            },
            Err(_) => self // Silently fail if content type is invalid
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Method, StatusCode};
    use crate::types::headers::HeaderAccess;
    use crate::builder::headers::cseq::CSeqBuilderExt;
    use crate::builder::headers::from::FromBuilderExt;
    use crate::builder::headers::to::ToBuilderExt;
    use std::str::FromStr;

    #[test]
    fn test_request_content_type_shortcuts() {
        // Test SDP content type
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .content_type_sdp()
            .body(concat!(
                "v=0\r\n",
                "o=alice 2890844526 2890844526 IN IP4 192.0.2.1\r\n",
                "s=Call with Alice\r\n",
                "c=IN IP4 192.0.2.1\r\n",  // Connection info required by SDP
                "t=0 0\r\n",
                "m=audio 49170 RTP/AVP 0 8\r\n",
                "a=rtpmap:0 PCMU/8000\r\n",
                "a=rtpmap:8 PCMA/8000\r\n"
            ))
            .build();
            
        let content_type_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "application/sdp");
        
        // Test text content type
        let request = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
            .content_type_text()
            .body("Hello, this is a text message sent via SIP")
            .build();
            
        let content_type_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "text/plain");
    }
    
    #[test]
    fn test_response_content_type_shortcuts() {
        // Test XML content type
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .cseq_with_method(101, Method::Invite)
            .content_type_xml()
            .build();
            
        let content_type_headers = response.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "application/xml");
        
        // Test JSON content type
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .cseq_with_method(101, Method::Invite)
            .content_type_json()
            .build();
            
        let content_type_headers = response.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "application/json");
    }
    
    #[test]
    fn test_content_type_string() {
        // Test content_type method with valid content type
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .content_type("application/sdp")
            .body(concat!(
                "v=0\r\n",
                "o=alice 2890844526 2890844526 IN IP4 192.0.2.1\r\n",
                "s=Call with Alice\r\n",
                "c=IN IP4 192.0.2.1\r\n",
                "t=0 0\r\n",
                "m=audio 49170 RTP/AVP 0 8\r\n",
                "a=rtpmap:0 PCMU/8000\r\n",
                "a=rtpmap:8 PCMA/8000\r\n"
            ))
            .build();
            
        let content_type_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "application/sdp");
        
        // Test content_type method with parameters
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .content_type("application/sdp; charset=UTF-8")
            .body(concat!(
                "v=0\r\n",
                "o=alice 2890844526 2890844526 IN IP4 192.0.2.1\r\n",
                "s=Call with Alice\r\n",
                "c=IN IP4 192.0.2.1\r\n",
                "t=0 0\r\n",
                "m=audio 49170 RTP/AVP 0 8\r\n",
                "a=rtpmap:0 PCMU/8000\r\n",
                "a=rtpmap:8 PCMA/8000\r\n"
            ))
            .build();
            
        let content_type_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "application/sdp;charset=\"UTF-8\"");
    }
    
    #[test]
    fn test_custom_content_type() {
        // Test custom content type
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .content_type_custom("application", "vnd.3gpp.sms")
            .body(concat!(
                "<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\">\r\n",
                "  <soap:Body>\r\n",
                "    <registerRequest xmlns=\"http://example.org/register\">\r\n",
                "      <username>alice</username>\r\n",
                "      <password>secret</password>\r\n", 
                "    </registerRequest>\r\n",
                "  </soap:Body>\r\n",
                "</soap:Envelope>\r\n"
            ))
            .build();
            
        let content_type_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "application/vnd.3gpp.sms");
    }
    
    #[test]
    fn test_content_type_sipfrag() {
        // Test sipfrag content type for request
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .content_type_sipfrag()
            .body(concat!(
                "SIP/2.0 200 OK\r\n",
                "CSeq: 1 INVITE\r\n",
                "Contact: <sip:alice@192.0.2.3>\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: 0\r\n"
            ))
            .build();
            
        let content_type_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "message/sipfrag");
        
        // Test sipfrag content type for response
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .cseq_with_method(101, Method::Invite)
            .content_type_sipfrag()
            .build();
            
        let content_type_headers = response.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "message/sipfrag");
    }
} 
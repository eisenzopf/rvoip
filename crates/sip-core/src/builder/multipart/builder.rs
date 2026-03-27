//! MultipartBuilder and MultipartBuilt - higher-level multipart builder

use crate::types::multipart::{MultipartBody, MimePart, ParsedBody};
use crate::types::header::{Header, HeaderName};
use crate::types::TypedHeader;
use bytes::Bytes;
use rand::{thread_rng, Rng};
use rand::distributions::Alphanumeric;
use std::iter;
use super::part_builder::MultipartPartBuilder;
use super::MultipartBodyBuilder;

/// Builder for creating multipart MIME bodies with multiple parts.
///
/// This builder provides a higher-level interface than MultipartBodyBuilder,
/// with convenient factory methods for common multipart types and better
/// integration with MultipartPartBuilder. It's designed to make creating SIP
/// messages with complex multipart content easier and more intuitive.
///
/// ## Key Features:
///
/// - Specialized constructors for common multipart types: `mixed()`, `alternative()`, `related()`
/// - Simple API for adding MIME parts using `MultipartPartBuilder`
/// - Easy integration with SIP message builders via `content_type()` and `body()` methods
/// - Support for preamble, epilogue, and custom boundaries
/// - Type parameters for multipart/related content
///
/// ## Common Multipart Types in SIP
///
/// - **multipart/mixed**: For mixed content with no special relationship (most common)
/// - **multipart/alternative**: For alternative representations of the same content
/// - **multipart/related**: For related content where parts reference each other
///
/// ## Real-world SIP Multipart Scenarios
///
/// - SIP INVITE with SDP and call metadata (multipart/mixed)
/// - SIP MESSAGE with alternative text and HTML content (multipart/alternative)
/// - SIP PUBLISH with PIDF presence document and referenced avatar image (multipart/related)
/// - SIP INFO with multiple media control commands
/// - SIP NOTIFY with document updates containing inline resources
///
/// # Examples
///
/// ## Basic multipart/mixed with text and image
///
/// ```rust
/// use rvoip_sip_core::builder::multipart::{MultipartBuilder, MultipartPartBuilder};
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentTypeBuilderExt, MimeVersionBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create a multipart/mixed body with text and image (base64 encoded)
/// let multipart = MultipartBuilder::mixed()
///     .add_part(
///         MultipartPartBuilder::new()
///             .content_type("text/plain")
///             .body("Check out this image I'm sending you!")
///             .build()
///     )
///     .add_part(
///         MultipartPartBuilder::new()
///             .content_type("image/png")
///             .content_transfer_encoding("base64")
///             .content_id("<image1@example.com>")
///             .content_disposition("attachment; filename=logo.png")
///             .body("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
///             .build()
///     )
///     .build();
///
/// // Create a SIP MESSAGE with the multipart body
/// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
///     .mime_version(1, 0)  // Required for multipart content
///     .content_type(&multipart.content_type())
///     .body(multipart.body())
///     .build();
/// ```
///
/// ## INVITE with SDP and call metadata (real-world example)
///
/// ```rust
/// use rvoip_sip_core::builder::multipart::{MultipartBuilder, MultipartPartBuilder};
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentTypeBuilderExt, MimeVersionBuilderExt};
/// use rvoip_sip_core::builder::headers::{FromBuilderExt, ToBuilderExt, ContactBuilderExt};
/// use rvoip_sip_core::types::Method;
/// use rvoip_sip_core::sdp::SdpBuilder;
///
/// // Create an SDP offer
/// let sdp = SdpBuilder::new("SIP Call with Metadata")
///     .origin("alice", "2890844526", "2890844526", "IN", "IP4", "198.51.100.33")
///     .connection("IN", "IP4", "198.51.100.33")
///     .time("0", "0")
///     .media_audio(49170, "RTP/AVP")
///         .formats(&["0", "8", "96"])
///         .rtpmap("0", "PCMU/8000")
///         .rtpmap("8", "PCMA/8000")
///         .rtpmap("96", "opus/48000/2")
///         .ptime(20)
///         .done()
///     .build()
///     .unwrap();
///
/// // Create XML call metadata
/// let call_metadata = r#"<?xml version="1.0" encoding="UTF-8"?>
/// <call-metadata xmlns="urn:example:callmeta">
///   <call-type>support</call-type>
///   <priority>high</priority>
///   <reference>CASE-12345</reference>
///   <customer>
///     <account-id>ACC987654</account-id>
///     <membership-level>premium</membership-level>
///   </customer>
/// </call-metadata>"#;
///
/// // Create a multipart/mixed body with SDP and metadata
/// let multipart = MultipartBuilder::mixed()
///     .add_part(
///         MultipartPartBuilder::new()
///             .content_type("application/sdp")
///             .content_disposition("session")
///             .body(sdp.to_string())
///             .build()
///     )
///     .add_part(
///         MultipartPartBuilder::new()
///             .content_type("application/call-metadata+xml")
///             .content_disposition("handling=optional")
///             .body(call_metadata)
///             .build()
///     )
///     .build();
///
/// // Create a SIP INVITE with the multipart body
/// let invite = SimpleRequestBuilder::invite("sip:support@example.com").unwrap()
///     .from("Alice Smith", "sip:alice@example.com", Some("a73kssle"))
///     .to("Support", "sip:support@example.com", None)
///     .contact("sip:alice@198.51.100.33:5060", None)
///     .mime_version(1, 0)  // Required for multipart content
///     .content_type(&multipart.content_type())
///     .body(multipart.body())
///     .build();
/// ```
///
/// ## SIP PUBLISH with Presence and Referenced Avatar (multipart/related)
///
/// ```rust
/// use rvoip_sip_core::builder::multipart::{MultipartBuilder, MultipartPartBuilder};
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentTypeBuilderExt, MimeVersionBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create a multipart/related body with PIDF document that references an image
/// let multipart = MultipartBuilder::related()
///     .type_parameter("application/pidf+xml")  // The root document type
///     .add_part(
///         MultipartPartBuilder::new()
///             .content_type("application/pidf+xml")
///             .content_id("<presence123@example.com>")
///             .body(r#"<?xml version="1.0" encoding="UTF-8"?>
/// <presence xmlns="urn:ietf:params:xml:ns:pidf" entity="sip:alice@example.com">
///   <tuple id="a1">
///     <status><basic>open</basic></status>
///     <note>Available</note>
///     <note>My avatar: <img src="cid:avatar123@example.com"/></note>
///   </tuple>
/// </presence>"#)
///             .build()
///     )
///     .add_part(
///         MultipartPartBuilder::new()
///             .content_type("image/jpeg")
///             .content_id("<avatar123@example.com>")
///             .content_transfer_encoding("base64")
///             .content_disposition("inline")
///             .body("/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAoHBwgHBgoICAgLCgoLDh...")
///             .build()
///     )
///     .build();
///
/// // Create a SIP PUBLISH with the multipart body
/// let publish = SimpleRequestBuilder::new(Method::Publish, "sip:alice@example.com;method=PUBLISH").unwrap()
///     .mime_version(1, 0)
///     .content_type(&multipart.content_type())
///     .body(multipart.body())
///     .build();
/// ```
#[derive(Debug, Clone, Default)]
pub struct MultipartBuilder {
    boundary: Option<String>,
    subtype: String,
    type_param: Option<String>,
    parts: Vec<MimePart>,
    preamble: Option<String>,
    epilogue: Option<String>,
}

impl MultipartBuilder {
    /// Creates a multipart/mixed builder.
    ///
    /// Multipart/mixed is used for content with different types that don't
    /// have a specific relationship to each other.
    ///
    /// # Returns
    ///
    /// A new MultipartBuilder configured for multipart/mixed
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBuilder;
    ///
    /// let builder = MultipartBuilder::mixed();
    /// ```
    pub fn mixed() -> Self {
        Self {
            subtype: "mixed".to_string(),
            ..Default::default()
        }
    }

    /// Creates a multipart/alternative builder.
    ///
    /// Multipart/alternative is used when the same content is provided in
    /// different formats, with the last part being the preferred format.
    ///
    /// # Returns
    ///
    /// A new MultipartBuilder configured for multipart/alternative
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBuilder;
    ///
    /// let builder = MultipartBuilder::alternative();
    /// ```
    pub fn alternative() -> Self {
        Self {
            subtype: "alternative".to_string(),
            ..Default::default()
        }
    }

    /// Creates a multipart/related builder.
    ///
    /// Multipart/related is used when parts reference each other, such as an
    /// HTML document with embedded images.
    ///
    /// # Returns
    ///
    /// A new MultipartBuilder configured for multipart/related
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBuilder;
    ///
    /// let builder = MultipartBuilder::related();
    /// ```
    pub fn related() -> Self {
        Self {
            subtype: "related".to_string(),
            ..Default::default()
        }
    }

    /// Sets a custom boundary string for the multipart body.
    ///
    /// By default, a random boundary is generated when building.
    ///
    /// # Parameters
    ///
    /// - `boundary`: The boundary string to use
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBuilder;
    ///
    /// let builder = MultipartBuilder::mixed()
    ///     .boundary("custom-boundary-123");
    /// ```
    pub fn boundary(mut self, boundary: impl Into<String>) -> Self {
        self.boundary = Some(boundary.into());
        self
    }

    /// Sets the type parameter for multipart/related content.
    ///
    /// The type parameter indicates the MIME type of the "root" part
    /// in a multipart/related body.
    ///
    /// # Parameters
    ///
    /// - `type_param`: The type parameter value
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBuilder;
    ///
    /// let builder = MultipartBuilder::related()
    ///     .type_parameter("text/html");
    /// ```
    pub fn type_parameter(mut self, type_param: impl Into<String>) -> Self {
        self.type_param = Some(type_param.into());
        self
    }

    /// Sets the preamble text that appears before the first boundary.
    ///
    /// # Parameters
    ///
    /// - `preamble`: The preamble text
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBuilder;
    ///
    /// let builder = MultipartBuilder::mixed()
    ///     .preamble("This is a multipart message in MIME format.");
    /// ```
    pub fn preamble(mut self, preamble: impl Into<String>) -> Self {
        self.preamble = Some(preamble.into());
        self
    }

    /// Sets the epilogue text that appears after the final boundary.
    ///
    /// # Parameters
    ///
    /// - `epilogue`: The epilogue text
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBuilder;
    ///
    /// let builder = MultipartBuilder::mixed()
    ///     .epilogue("End of multipart message.");
    /// ```
    pub fn epilogue(mut self, epilogue: impl Into<String>) -> Self {
        self.epilogue = Some(epilogue.into());
        self
    }

    /// Adds a MIME part to the multipart body.
    ///
    /// # Parameters
    ///
    /// - `part`: The MimePart to add
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::{MultipartBuilder, MultipartPartBuilder};
    ///
    /// let part = MultipartPartBuilder::new()
    ///     .content_type("text/plain")
    ///     .body("This is text content")
    ///     .build();
    ///
    /// let builder = MultipartBuilder::mixed()
    ///     .add_part(part);
    /// ```
    pub fn add_part(mut self, part: MimePart) -> Self {
        self.parts.push(part);
        self
    }

    /// Returns the Content-Type header value for this multipart body.
    ///
    /// This includes the multipart type, boundary parameter, and any other
    /// parameters like the type parameter for multipart/related.
    ///
    /// # Returns
    ///
    /// A string containing the Content-Type value
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBuilder;
    ///
    /// let multipart = MultipartBuilder::mixed()
    ///     .boundary("custom-boundary")
    ///     .build();
    ///
    /// let content_type = multipart.content_type();
    /// assert_eq!(content_type, "multipart/mixed; boundary=\"custom-boundary\"");
    /// ```
    pub fn content_type(&self) -> String {
        let mut content_type = format!("multipart/{}", self.subtype);
        
        // Add boundary parameter
        if let Some(boundary) = &self.boundary {
            content_type.push_str(&format!("; boundary=\"{}\"", boundary));
        }
        
        // Add type parameter for multipart/related
        if let Some(type_param) = &self.type_param {
            content_type.push_str(&format!("; type=\"{}\"", type_param));
        }
        
        content_type
    }

    /// Returns the body content for this multipart message.
    ///
    /// # Returns
    ///
    /// A string containing the serialized multipart body
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::{MultipartBuilder, MultipartPartBuilder};
    ///
    /// let multipart = MultipartBuilder::mixed()
    ///     .add_part(
    ///         MultipartPartBuilder::new()
    ///             .content_type("text/plain")
    ///             .body("Text content")
    ///             .build()
    ///     )
    ///     .build();
    ///
    /// let body = multipart.body();
    /// assert!(body.contains("Content-Type: text/plain"));
    /// assert!(body.contains("Text content"));
    /// ```
    pub fn body(&self) -> String {
        let boundary = self.boundary.clone().unwrap_or_else(|| {
            let random_suffix: String = iter::repeat(())
                .map(|()| thread_rng().sample(Alphanumeric))
                .map(char::from)
                .take(16)
                .collect();
                
            format!("boundary-{}", random_suffix)
        });

        let mut body_builder = MultipartBodyBuilder::new()
            .boundary(boundary);
            
        // Add all parts
        let mut builder = body_builder;
        for part in &self.parts {
            builder = builder.add_mime_part(part.clone());
        }
        
        // Add preamble and epilogue if present
        if let Some(preamble) = &self.preamble {
            builder = builder.preamble(preamble);
        }
        
        if let Some(epilogue) = &self.epilogue {
            builder = builder.epilogue(epilogue);
        }
        
        builder.build().to_string()
    }

    /// Builds a MultipartBody from this builder.
    ///
    /// # Returns
    ///
    /// A MultipartBody instance
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::{MultipartBuilder, MultipartPartBuilder};
    ///
    /// let multipart = MultipartBuilder::mixed()
    ///     .add_part(
    ///         MultipartPartBuilder::new()
    ///             .content_type("text/plain")
    ///             .body("Text content")
    ///             .build()
    ///     )
    ///     .build();
    /// ```
    pub fn build(self) -> MultipartBuilt {
        let boundary = self.boundary.unwrap_or_else(|| {
            let random_suffix: String = iter::repeat(())
                .map(|()| thread_rng().sample(Alphanumeric))
                .map(char::from)
                .take(16)
                .collect();
                
            format!("boundary-{}", random_suffix)
        });

        let inner_multipart = MultipartBody {
            boundary: boundary.clone(),
            parts: self.parts,
            preamble: self.preamble.map(Bytes::from),
            epilogue: self.epilogue.map(Bytes::from),
        };

        MultipartBuilt {
            boundary,
            subtype: self.subtype,
            type_param: self.type_param,
            inner_multipart,
        }
    }
}

/// Represents a built multipart MIME body with convenient methods for integration with SIP messages.
#[derive(Debug, Clone)]
pub struct MultipartBuilt {
    boundary: String,
    subtype: String,
    type_param: Option<String>,
    inner_multipart: MultipartBody,
}

impl MultipartBuilt {
    /// Returns the Content-Type header value for this multipart body.
    ///
    /// This includes the multipart type, boundary parameter, and any other
    /// parameters like the type parameter for multipart/related.
    ///
    /// # Returns
    ///
    /// A string containing the Content-Type value
    pub fn content_type(&self) -> String {
        let mut content_type = format!("multipart/{}", self.subtype);
        
        // Add boundary parameter
        content_type.push_str(&format!("; boundary=\"{}\"", self.boundary));
        
        // Add type parameter for multipart/related
        if let Some(type_param) = &self.type_param {
            content_type.push_str(&format!("; type=\"{}\"", type_param));
        }
        
        content_type
    }

    /// Returns the body content for this multipart message.
    ///
    /// # Returns
    ///
    /// A string containing the serialized multipart body
    pub fn body(&self) -> String {
        self.inner_multipart.to_string()
    }
}

use crate::error::{Error, Result};
use std::collections::HashMap;
use std::str::FromStr;
use crate::types::{
    TypedHeader,
    header::TypedHeaderTrait,
    headers::header_access::HeaderAccess,
};
use crate::types::content_disposition::{ContentDisposition, DispositionType, Handling};
use super::HeaderSetter;

/// Content-Disposition Header Builder for SIP Messages
///
/// This module provides builder methods for the Content-Disposition header in SIP messages,
/// which defines how the recipient should process the message body.
///
/// ## SIP Content-Disposition Header Overview
///
/// The Content-Disposition header is defined in [RFC 3261 Section 20.11](https://datatracker.ietf.org/doc/html/rfc3261#section-20.11)
/// and extended in [RFC 5621](https://datatracker.ietf.org/doc/html/rfc5621) for handling disposition types.
/// It specifies how the message body should be interpreted by the User Agent and may include
/// parameters that further describe the disposition behavior.
///
/// ## Purpose of Content-Disposition Header
///
/// The Content-Disposition header serves several important purposes in SIP:
///
/// 1. It indicates whether the content is related to the current SIP session or call
/// 2. It specifies if the body should be rendered to the user or processed automatically
/// 3. It defines handling requirements (optional vs. required processing)
/// 4. It allows for specialized content types like icons, alerts, or early media
///
/// ## Common Disposition Types in SIP
///
/// - **session**: The body is related to the SIP session (default for SDP)
/// - **render**: The body should be rendered to the user
/// - **icon**: The body contains an icon related to the session
/// - **alert**: The body contains alerting information (like a custom ringtone)
/// - **attachment**: The body should be treated as an attachment (similar to email)
///
/// ## Common Parameters
///
/// - **handling**: Can be "optional" or "required", indicating how the UA should handle unknown disposition types
/// - **size**: For icon disposition, indicates the icon size
/// - **creation-date**, **modification-date**, **filename**: For attachment-type content
///
/// ## Special Considerations
///
/// 1. **Handling Parameter**: When set to "required", the UA must understand the disposition type or reject the message
/// 2. **Multiple Bodies**: In multipart MIME content, each part may have its own Content-Disposition
/// 3. **Defaults**: For SDP content, "session" is assumed if no Content-Disposition is provided
/// 4. **Security**: The Content-Disposition provides security by preventing automatic rendering of potentially harmful content
///
/// ## Relationship with other headers
///
/// - **Content-Disposition** + **Content-Type**: Together define how content should be processed
/// - **Content-Disposition** + **MIME-Version**: Required for more complex MIME handling
/// - **Content-Disposition** + **Call-Info**: May both reference external resources like icons
///
/// # Examples
///
/// ## SDP with Session Disposition
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentDispositionExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create an INVITE with SDP that has session disposition
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:recipient@example.com").unwrap()
///     .content_type_sdp()
///     .content_disposition_session("required")  // Session-related content, must be understood
///     .body("v=0\r\no=user 123 456 IN IP4 192.0.2.1\r\ns=Example\r\nc=IN IP4 192.0.2.1\r\nt=0 0\r\nm=audio 49172 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n")
///     .build();
/// ```
///
/// ## Message with Image Attachment
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentDispositionExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::types::Method;
/// use std::collections::HashMap;
///
/// // Create parameters for attachment
/// let mut params = HashMap::new();
/// params.insert("handling".to_string(), "optional".to_string());
/// params.insert("filename".to_string(), "logo.jpg".to_string());
///
/// // Create a MESSAGE with an image attachment
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:user@example.com").unwrap()
///     .content_type("image/jpeg")
///     .content_disposition("attachment", params)  // Body is an attachment
///     .body("Simulated image content")
///     .build();
/// ```
///
/// ## Ringtone Alert
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentDispositionExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create an INVITE with a custom ringtone
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
///     .content_type("audio/wav")
///     .content_disposition_alert(Some("optional"))  // Alert content (ringtone)
///     .body("Simulated ringtone audio data")
///     .build();
/// ```

/// Extension trait for adding Content-Disposition header building capabilities
///
/// This trait provides methods to add Content-Disposition headers to SIP messages, indicating
/// how the message body should be processed by the recipient.
///
/// ## When to use Content-Disposition
///
/// Content-Disposition is particularly useful in the following scenarios:
///
/// 1. **Call establishment**: To identify SDP as session-related content
/// 2. **Rich messaging**: When sending content that should be displayed to the user
/// 3. **Custom alerts**: For providing custom ringtones or visual alerts
/// 4. **File transfers**: When sending attachments through SIP MESSAGE
/// 5. **Caller ID enhancement**: When providing caller icons or images
///
/// ## Best Practices
///
/// - Use "required" handling only when absolutely necessary
/// - Include appropriate parameters for each disposition type
/// - For critical session information, always use session disposition
/// - When sending attachments, include relevant metadata like filename
/// - For icons and alerts, ensure the recipient is likely to support the format
///
/// # Examples
///
/// ## Call Setup with Early Media
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentDispositionExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create an INVITE with SDP marked as session content
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:contact-center@example.com").unwrap()
///     .from("Customer", "sip:customer@example.com", Some("tag1234"))
///     .to("Contact Center", "sip:contact-center@example.com", None)
///     .content_type_sdp()
///     .content_disposition_session("required")  // Session-related content that must be understood
///     .body("v=0\r\no=customer 123 456 IN IP4 192.0.2.1\r\ns=Support Call\r\nc=IN IP4 192.0.2.1\r\nt=0 0\r\nm=audio 49172 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n")
///     .build();
/// ```
///
/// ## Business Card Exchange
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentDispositionExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::types::Method;
/// use std::collections::HashMap;
///
/// // Parameters for the vCard attachment
/// let mut params = HashMap::new();
/// params.insert("handling".to_string(), "optional".to_string());
/// params.insert("filename".to_string(), "business-card.vcf".to_string());
///
/// // Create a MESSAGE with a vCard attachment
/// let vcard = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Alice Smith\r\nTEL:+1-555-123-4567\r\nEMAIL:alice@example.com\r\nEND:VCARD\r\n";
///
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("msg1"))
///     .to("Bob", "sip:bob@example.com", None)
///     .content_type("text/vcard")
///     .content_disposition("attachment", params)
///     .body(vcard)
///     .build();
/// ```
pub trait ContentDispositionExt {
    /// Add a Content-Disposition header with session disposition type
    ///
    /// The "session" disposition type indicates that the body is related to the SIP session
    /// or dialog. This is the default disposition for SDP content and typically used for
    /// session establishment or modification.
    ///
    /// # Arguments
    ///
    /// * `handling` - The handling parameter (optional or required)
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ContentDispositionExt};
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    /// 
    /// // Create an INVITE with SDP content
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("invite123"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .content_type_sdp()
    ///     .content_disposition_session("required")  // Session content that must be processed
    ///     .body("v=0\r\no=alice 123 456 IN IP4 192.0.2.1\r\ns=Call\r\nt=0 0\r\nm=audio 49172 RTP/AVP 0\r\n")
    ///     .build();
    /// ```
    ///
    /// # RFC Reference
    /// 
    /// As per [RFC 5621 Section 3](https://datatracker.ietf.org/doc/html/rfc5621#section-3),
    /// the "session" disposition type indicates that the body is associated with the session
    /// and is the default disposition type for SDP content.
    fn content_disposition_session(self, handling: &str) -> Self;

    /// Add a Content-Disposition header with render disposition type
    ///
    /// The "render" disposition type indicates that the body should be displayed or rendered
    /// to the user. This is appropriate for content that should be presented directly to
    /// the user, such as text messages, images, or other media.
    ///
    /// # Arguments
    ///
    /// * `handling` - The handling parameter (optional or required)
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ContentDispositionExt};
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    /// 
    /// // Create a MESSAGE with text content to be rendered
    /// let request = SimpleRequestBuilder::new(Method::Message, "sip:user@example.com").unwrap()
    ///     .from("System", "sip:system@example.com", Some("msg456"))
    ///     .to("User", "sip:user@example.com", None)
    ///     .content_type_text()
    ///     .content_disposition_render("optional")  // Content should be rendered to the user
    ///     .body("Your subscription will expire in 3 days. Please renew.")
    ///     .build();
    /// ```
    ///
    /// # RFC Reference
    /// 
    /// As per [RFC 5621 Section 3](https://datatracker.ietf.org/doc/html/rfc5621#section-3),
    /// the "render" disposition type indicates that the body should be displayed or rendered
    /// to the user in the normal way.
    fn content_disposition_render(self, handling: &str) -> Self;

    /// Add a Content-Disposition header with icon disposition type
    ///
    /// The "icon" disposition type indicates that the body contains an icon
    /// associated with the session or caller. This can be used to provide
    /// a visual identifier for a call or caller.
    ///
    /// # Arguments
    ///
    /// * `size` - The size parameter for the icon (e.g., "32" for 32x32 pixels)
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ContentDispositionExt};
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    /// 
    /// // Create an INVITE with caller icon
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .from("Company", "sip:company@example.com", Some("inv789"))
    ///     .to("Customer", "sip:bob@example.com", None)
    ///     .content_type("image/png")
    ///     .content_disposition_icon("64")  // 64x64 pixel icon
    ///     .body("Simulated image data for an icon")
    ///     .build();
    /// ```
    ///
    /// # RFC Reference
    /// 
    /// As per [RFC 5621 Section 3](https://datatracker.ietf.org/doc/html/rfc5621#section-3),
    /// the "icon" disposition type indicates that the body contains an icon image
    /// associated with the session.
    fn content_disposition_icon(self, size: &str) -> Self;

    /// Add a Content-Disposition header with alert disposition type
    ///
    /// The "alert" disposition type indicates that the body contains alerting
    /// information, such as a custom ringtone or visual alert for incoming calls.
    ///
    /// # Arguments
    ///
    /// * `handling` - Optional handling parameter (optional or required)
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ContentDispositionExt};
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    /// 
    /// // Create an INVITE with custom ringtone
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:user@example.com").unwrap()
    ///     .from("Priority", "sip:priority@example.com", Some("urgent"))
    ///     .to("User", "sip:user@example.com", None)
    ///     .content_type("audio/wav")
    ///     .content_disposition_alert(Some("optional"))  // Custom ringtone
    ///     .body("Simulated audio data for a custom ringtone")
    ///     .build();
    /// ```
    ///
    /// # RFC Reference
    /// 
    /// As per [RFC 5621 Section 3](https://datatracker.ietf.org/doc/html/rfc5621#section-3),
    /// the "alert" disposition type indicates that the body contains information that 
    /// should be rendered by the UA when the session is being alerted.
    fn content_disposition_alert(self, handling: Option<&str>) -> Self;

    /// Add a Content-Disposition header with a custom disposition type
    ///
    /// This method allows you to specify any disposition type, including
    /// standard types not covered by convenience methods or custom types
    /// specific to your application.
    ///
    /// # Arguments
    ///
    /// * `disposition_type` - The disposition type string
    /// * `params` - Map of parameter names to values
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ContentDispositionExt};
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    /// use std::collections::HashMap;
    /// 
    /// // Create parameters for a file attachment
    /// let mut params = HashMap::new();
    /// params.insert("handling".to_string(), "optional".to_string());
    /// params.insert("filename".to_string(), "document.pdf".to_string());
    /// params.insert("creation-date".to_string(), "2023-06-15T14:30:00Z".to_string());
    /// 
    /// // Create a MESSAGE with a document attachment
    /// let request = SimpleRequestBuilder::new(Method::Message, "sip:colleague@example.com").unwrap()
    ///     .from("Sender", "sip:sender@example.com", Some("doc123"))
    ///     .to("Colleague", "sip:colleague@example.com", None)
    ///     .content_type("application/pdf")
    ///     .content_disposition("attachment", params)  // File attachment with metadata
    ///     .body("Simulated PDF document data")
    ///     .build();
    /// ```
    ///
    /// # Standard Disposition Types
    ///
    /// Beyond the common types that have dedicated methods, other standard types include:
    ///
    /// - **attachment**: Content should be saved and processed separately (like email attachments)
    /// - **inline**: Content should be displayed as part of the message
    /// - **preview**: Content is a preview of other content
    ///
    /// # Custom Parameters
    ///
    /// Common parameters include:
    ///
    /// - **filename**: Name for saving the content
    /// - **creation-date**, **modification-date**: Content timestamps
    /// - **size**: Size information (required for icon disposition)
    /// - **handling**: How UAs should handle unknown disposition types
    fn content_disposition(self, disposition_type: &str, params: HashMap<String, String>) -> Self;
}

impl<T> ContentDispositionExt for T 
where 
    T: HeaderSetter,
{
    fn content_disposition_session(self, handling: &str) -> Self {
        let mut params = HashMap::new();
        params.insert("handling".to_string(), handling.to_string());

        // Create Content-Disposition with session type
        let header_value = ContentDisposition {
            disposition_type: DispositionType::Session,
            params,
        };
        
        // Debug print
        tracing::error!("Setting ContentDisposition header: {:?}", header_value);
        
        // Try to convert it to a header and back to see if conversion is working
        let header = header_value.to_header();
        tracing::error!("Created header: {:?}", header);
        
        match ContentDisposition::from_header(&header) {
            Ok(cd) => tracing::error!("Converted back to ContentDisposition: {:?}", cd),
            Err(e) => tracing::error!("Failed to convert back: {:?}", e),
        }
        
        self.set_header(header_value)
    }

    fn content_disposition_render(self, handling: &str) -> Self {
        let mut params = HashMap::new();
        params.insert("handling".to_string(), handling.to_string());

        // Create Content-Disposition with render type
        let header_value = ContentDisposition {
            disposition_type: DispositionType::Render,
            params,
        };
        self.set_header(header_value)
    }

    fn content_disposition_icon(self, size: &str) -> Self {
        let mut params = HashMap::new();
        params.insert("size".to_string(), size.to_string());

        // Create Content-Disposition with icon type
        let header_value = ContentDisposition {
            disposition_type: DispositionType::Icon,
            params,
        };
        self.set_header(header_value)
    }

    fn content_disposition_alert(self, handling: Option<&str>) -> Self {
        let mut params = HashMap::new();
        if let Some(h) = handling {
            params.insert("handling".to_string(), h.to_string());
        }

        // Create Content-Disposition with alert type
        let header_value = ContentDisposition {
            disposition_type: DispositionType::Alert,
            params,
        };
        self.set_header(header_value)
    }

    fn content_disposition(self, disposition_type: &str, params: HashMap<String, String>) -> Self {
        // Parse the disposition type
        let disp_type = match DispositionType::from_str(disposition_type) {
            Ok(dt) => dt,
            Err(_) => return self, // Return self unchanged if parsing fails
        };
        
        // Create Content-Disposition
        let header_value = ContentDisposition {
            disposition_type: disp_type,
            params,
        };
        
        self.set_header(header_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::SimpleRequestBuilder;
    use crate::types::header::HeaderName;
    
    #[test]
    fn test_content_disposition_session() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .content_disposition_session("optional")
            .build();
            
        // Check if Content-Disposition header exists with the correct value
        let header = request.header(&HeaderName::ContentDisposition);
        tracing::error!("DEBUG: Header type: {:?}", header.map(|h| h.name()));
        assert!(header.is_some(), "Content-Disposition header not found");
        
        // Try with typed_header instead
        let typed_header = request.typed_header::<ContentDisposition>();
        if let Some(content_disp) = typed_header {
            // Check for correct disposition type
            assert_eq!(content_disp.disposition_type, DispositionType::Session, 
                      "Expected disposition type 'session', got '{:?}'", content_disp.disposition_type);
            
            // Check for the handling parameter
            let handling = content_disp.params.get("handling");
            assert_eq!(handling, Some(&"optional".to_string()), 
                      "Expected handling parameter 'optional', got '{:?}'", handling);
        } else {
            panic!("Expected Content-Disposition header via typed_header but got None");
        }
    }
    
    #[test]
    fn test_content_disposition_render() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .content_disposition_render("required")
            .build();
            
        // Check if Content-Disposition header exists with the correct value
        let header = request.header(&HeaderName::ContentDisposition);
        assert!(header.is_some(), "Content-Disposition header not found");
        
        // Try with typed_header instead
        let typed_header = request.typed_header::<ContentDisposition>();
        if let Some(content_disp) = typed_header {
            // Check for correct disposition type
            assert_eq!(content_disp.disposition_type, DispositionType::Render, 
                      "Expected disposition type 'render', got '{:?}'", content_disp.disposition_type);
            
            // Check for the handling parameter
            let handling = content_disp.params.get("handling");
            assert_eq!(handling, Some(&"required".to_string()), 
                      "Expected handling parameter 'required', got '{:?}'", handling);
        } else {
            panic!("Expected Content-Disposition header via typed_header but got None");
        }
    }
    
    #[test]
    fn test_content_disposition_icon() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .content_disposition_icon("32")
            .build();
            
        // Check if Content-Disposition header exists with the correct value
        let header = request.header(&HeaderName::ContentDisposition);
        assert!(header.is_some(), "Content-Disposition header not found");
        
        // Try with typed_header instead
        let typed_header = request.typed_header::<ContentDisposition>();
        if let Some(content_disp) = typed_header {
            // Check for correct disposition type
            assert_eq!(content_disp.disposition_type, DispositionType::Icon, 
                      "Expected disposition type 'icon', got '{:?}'", content_disp.disposition_type);
            
            // Check for the size parameter
            let size = content_disp.params.get("size");
            assert_eq!(size, Some(&"32".to_string()), 
                      "Expected size parameter '32', got '{:?}'", size);
        } else {
            panic!("Expected Content-Disposition header via typed_header but got None");
        }
    }
    
    #[test]
    fn test_content_disposition_custom() {
        let mut params = HashMap::new();
        params.insert("handling".to_string(), "optional".to_string());
        params.insert("custom".to_string(), "value".to_string());
        
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .content_disposition("custom-disp", params)
            .build();
            
        // Check if Content-Disposition header exists with the correct value
        let header = request.header(&HeaderName::ContentDisposition);
        assert!(header.is_some(), "Content-Disposition header not found");
        
        // Try with typed_header instead
        let typed_header = request.typed_header::<ContentDisposition>();
        if let Some(content_disp) = typed_header {
            // Check for correct disposition type
            assert_eq!(content_disp.disposition_type, DispositionType::Other("custom-disp".to_string()), 
                      "Expected disposition type 'custom-disp', got '{:?}'", content_disp.disposition_type);
            
            // Check for the handling parameter
            let handling = content_disp.params.get("handling");
            assert_eq!(handling, Some(&"optional".to_string()), 
                      "Expected handling parameter 'optional', got '{:?}'", handling);
            
            // Check for the custom parameter
            let custom = content_disp.params.get("custom");
            assert_eq!(custom, Some(&"value".to_string()), 
                      "Expected custom parameter 'value', got '{:?}'", custom);
        } else {
            panic!("Expected Content-Disposition header via typed_header but got None");
        }
    }
} 
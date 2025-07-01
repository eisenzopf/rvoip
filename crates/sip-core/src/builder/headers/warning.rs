use crate::error::{Error, Result};
use crate::types::{
    warning::{Warning, WarningHeader},
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
    uri::Uri,
};
use super::HeaderSetter;

/// Warning header builder
///
/// This module provides builder methods for the Warning header in SIP messages.
///
/// ## SIP Warning Header Overview
///
/// The Warning header is defined in [RFC 3261 Section 20.43](https://datatracker.ietf.org/doc/html/rfc3261#section-20.43)
/// as part of the core SIP protocol. It is used to carry additional information about the status of a response.
/// Warning headers are sent with responses and contain a three-digit warning code, host, and warning text.
///
/// ## Purpose of Warning Header
///
/// The Warning header serves several important purposes in SIP:
///
/// 1. It provides additional information about why a particular request was not fulfilled
/// 2. It helps with debugging issues in SIP transactions
/// 3. It indicates specific compatibility or resource problems
/// 4. It can suggest alternative actions or provide more context about errors
///
/// ## Standard Warning Codes
///
/// RFC 3261 defines several standard warning codes:
///
/// - **300**: Incompatible network protocol
/// - **301**: Incompatible network address formats
/// - **302**: Incompatible transport protocol
/// - **303**: Incompatible bandwidth units
/// - **305**: Incompatible media format
/// - **306**: Attribute not understood
/// - **307**: Session description parameter not understood
/// - **330**: Multicast not available
/// - **331**: Unicast not available
/// - **370**: Insufficient bandwidth
/// - **399**: Miscellaneous warning
///
/// ## Format
///
/// ```text
/// Warning: 307 example.com "Session parameter 'foo' not understood"
/// ```
///
/// ## Relationship with other headers
///
/// - **Warning** vs **Reason**: Warning provides information about the response itself,
///   while Reason (RFC 3326) explains why a request was rejected
/// - **Warning** vs **Error-Info**: Warning is included in the response itself,
///   while Error-Info points to additional information about an error
///
/// # Examples
///
/// ## Media Format Incompatibility
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::WarningBuilderExt};
///
/// // Scenario: Media server cannot handle the requested codec
///
/// // Create a 488 Not Acceptable Here response with a warning
/// let response = SimpleResponseBuilder::new(StatusCode::NotAcceptableHere, None)
///     .from("Media Server", "sip:media@example.com", Some("ms1"))
///     .to("Caller", "sip:caller@example.com", Some("c123"))
///     // Add a warning about incompatible media format
///     .warning_incompatible_media_format("H.265 codec not supported")
///     .build();
///
/// // The response now includes Warning: 305 media.example.com "H.265 codec not supported"
/// ```
///
/// ## Multiple Warnings
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::WarningBuilderExt};
///
/// // Create a 606 Not Acceptable response with multiple warnings
/// let response = SimpleResponseBuilder::new(StatusCode::NotAcceptable, None)
///     .from("SIP Proxy", "sip:proxy@example.com", Some("px1"))
///     .to("Caller", "sip:caller@example.com", Some("c456"))
///     // Add warnings about bandwidth and media format issues
///     .warning_insufficient_bandwidth("Video requires at least 1Mbps")
///     .warning_incompatible_media_format("Audio codec OPUS not supported")
///     .build();
///
/// // The response includes multiple Warning headers
/// ```
///
/// ## Custom Warning Code
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::WarningBuilderExt};
///
/// // Create a 503 Service Unavailable response with a custom warning
/// let response = SimpleResponseBuilder::new(StatusCode::ServiceUnavailable, None)
///     .from("SIP Server", "sip:server@example.com", Some("sv1"))
///     .to("Caller", "sip:caller@example.com", Some("c789"))
///     // Add a miscellaneous warning with custom text
///     .warning(399, Uri::sip("server.example.com"), "Server overloaded, try again in 30 seconds")
///     .build();
///
/// // The response includes Warning: 399 server.example.com "Server overloaded, try again in 30 seconds"
/// ```
pub trait WarningBuilderExt {
    /// Add a Warning header with the specified warning code, agent, and text
    ///
    /// This method adds a Warning header with the specified warning code, agent URI,
    /// and warning text. It's the most general method that allows setting any warning.
    ///
    /// # Parameters
    ///
    /// * `code` - The warning code (300-399)
    /// * `agent` - The URI of the warning agent
    /// * `text` - The warning text
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Warning header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::WarningBuilderExt};
    ///
    /// // Create a 400 Bad Request response with a custom warning
    /// let response = SimpleResponseBuilder::new(StatusCode::BadRequest, None)
    ///     .from("SIP Server", "sip:server@example.com", Some("sv1"))
    ///     .to("Caller", "sip:caller@example.com", Some("c123"))
    ///     .warning(399, Uri::sip("server.example.com"), "Malformed SDP in request")
    ///     .build();
    ///
    /// // The response includes Warning: 399 server.example.com "Malformed SDP in request"
    /// ```
    fn warning(self, code: u16, agent: Uri, text: impl Into<String>) -> Self;
    
    /// Add multiple warnings to a message
    ///
    /// This method adds multiple Warning headers to a message at once.
    ///
    /// # Parameters
    ///
    /// * `warnings` - A vector of Warning objects
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Warning headers added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::WarningBuilderExt};
    ///
    /// // Create some warnings
    /// let warning1 = Warning::new(370, Uri::sip("server1.example.com"), "Insufficient bandwidth");
    /// let warning2 = Warning::new(305, Uri::sip("server2.example.com"), "Incompatible codec");
    ///
    /// // Create a response with multiple warnings
    /// let response = SimpleResponseBuilder::new(StatusCode::NotAcceptable, None)
    ///     .from("SIP Server", "sip:server@example.com", Some("sv1"))
    ///     .to("Caller", "sip:caller@example.com", Some("c123"))
    ///     .warnings(vec![warning1, warning2])
    ///     .build();
    ///
    /// // The response includes both warnings
    /// ```
    fn warnings(self, warnings: Vec<Warning>) -> Self;
    
    /// Add a Warning header for incompatible network protocol (300)
    ///
    /// This convenience method adds a Warning header with code 300,
    /// indicating that the server has received a protocol which is not
    /// understood or is incompatible with the protocol version supported.
    ///
    /// # Parameters
    ///
    /// * `text` - Additional warning text
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Warning header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::WarningBuilderExt};
    ///
    /// // Create a 400 Bad Request response with protocol incompatibility warning
    /// let response = SimpleResponseBuilder::new(StatusCode::BadRequest, None)
    ///     .from("SIP Server", "sip:server@example.com", Some("sv1"))
    ///     .to("Caller", "sip:caller@example.com", Some("c123"))
    ///     .warning_incompatible_protocol("Only SIP/2.0 is supported")
    ///     .build();
    ///
    /// // The response includes Warning: 300 server.example.com "Only SIP/2.0 is supported"
    /// ```
    fn warning_incompatible_protocol(self, text: impl Into<String>) -> Self;
    
    /// Add a Warning header for incompatible network address formats (301)
    ///
    /// This convenience method adds a Warning header with code 301,
    /// indicating that the server has received a request that contains
    /// network address formats that are not understood or are incompatible.
    ///
    /// # Parameters
    ///
    /// * `text` - Additional warning text
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Warning header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::WarningBuilderExt};
    ///
    /// // Create a 400 Bad Request response with address format warning
    /// let response = SimpleResponseBuilder::new(StatusCode::BadRequest, None)
    ///     .from("SIP Server", "sip:server@example.com", Some("sv1"))
    ///     .to("Caller", "sip:caller@example.com", Some("c123"))
    ///     .warning_incompatible_address_format("IPv6 addresses not supported")
    ///     .build();
    ///
    /// // The response includes Warning: 301 server.example.com "IPv6 addresses not supported"
    /// ```
    fn warning_incompatible_address_format(self, text: impl Into<String>) -> Self;
    
    /// Add a Warning header for incompatible transport protocol (302)
    ///
    /// This convenience method adds a Warning header with code 302,
    /// indicating that the server has received a request that requires
    /// a transport protocol not supported.
    ///
    /// # Parameters
    ///
    /// * `text` - Additional warning text
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Warning header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::WarningBuilderExt};
    ///
    /// // Create a 400 Bad Request response with transport protocol warning
    /// let response = SimpleResponseBuilder::new(StatusCode::BadRequest, None)
    ///     .from("SIP Server", "sip:server@example.com", Some("sv1"))
    ///     .to("Caller", "sip:caller@example.com", Some("c123"))
    ///     .warning_incompatible_transport("SCTP transport not supported")
    ///     .build();
    ///
    /// // The response includes Warning: 302 server.example.com "SCTP transport not supported"
    /// ```
    fn warning_incompatible_transport(self, text: impl Into<String>) -> Self;
    
    /// Add a Warning header for incompatible media format (305)
    ///
    /// This convenience method adds a Warning header with code 305,
    /// indicating that the server has received a request for a media format
    /// it cannot process or support.
    ///
    /// # Parameters
    ///
    /// * `text` - Additional warning text
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Warning header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::WarningBuilderExt};
    ///
    /// // Create a 488 Not Acceptable Here response with media format warning
    /// let response = SimpleResponseBuilder::new(StatusCode::NotAcceptableHere, None)
    ///     .from("Media Server", "sip:media@example.com", Some("ms1"))
    ///     .to("Caller", "sip:caller@example.com", Some("c123"))
    ///     .warning_incompatible_media_format("VP9 video codec not supported")
    ///     .build();
    ///
    /// // The response includes Warning: 305 media.example.com "VP9 video codec not supported"
    /// ```
    fn warning_incompatible_media_format(self, text: impl Into<String>) -> Self;
    
    /// Add a Warning header for insufficient bandwidth (370)
    ///
    /// This convenience method adds a Warning header with code 370,
    /// indicating that the server has insufficient bandwidth to carry
    /// the requested media session.
    ///
    /// # Parameters
    ///
    /// * `text` - Additional warning text
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Warning header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::WarningBuilderExt};
    ///
    /// // Create a 480 Temporarily Unavailable response with bandwidth warning
    /// let response = SimpleResponseBuilder::new(StatusCode::TemporarilyUnavailable, None)
    ///     .from("Media Server", "sip:media@example.com", Some("ms1"))
    ///     .to("Caller", "sip:caller@example.com", Some("c123"))
    ///     .warning_insufficient_bandwidth("HD video requires at least 2 Mbps")
    ///     .build();
    ///
    /// // The response includes Warning: 370 media.example.com "HD video requires at least 2 Mbps"
    /// ```
    fn warning_insufficient_bandwidth(self, text: impl Into<String>) -> Self;
    
    /// Add a Warning header for miscellaneous warning (399)
    ///
    /// This convenience method adds a Warning header with code 399,
    /// which is used for miscellaneous warnings that do not fall under
    /// any of the other categories.
    ///
    /// # Parameters
    ///
    /// * `text` - Additional warning text
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Warning header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::WarningBuilderExt};
    ///
    /// // Create a 503 Service Unavailable response with miscellaneous warning
    /// let response = SimpleResponseBuilder::new(StatusCode::ServiceUnavailable, None)
    ///     .from("SIP Server", "sip:server@example.com", Some("sv1"))
    ///     .to("Caller", "sip:caller@example.com", Some("c123"))
    ///     .warning_miscellaneous("Server maintenance in progress, try again in 5 minutes")
    ///     .build();
    ///
    /// // The response includes Warning: 399 server.example.com "Server maintenance in progress, try again in 5 minutes"
    /// ```
    fn warning_miscellaneous(self, text: impl Into<String>) -> Self;
}

impl<T> WarningBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn warning(self, code: u16, agent: Uri, text: impl Into<String>) -> Self {
        // Create a single Warning
        let warning = Warning::new(code, agent, text);
        
        // Create a WarningHeader with the single warning
        let warning_header = WarningHeader::new(vec![warning]);
        
        // Use the HeaderSetter trait to set the header
        self.set_header(warning_header)
    }
    
    fn warnings(self, warnings: Vec<Warning>) -> Self {
        // Create a WarningHeader with the warnings
        let warning_header = WarningHeader::new(warnings);
        
        // Use the HeaderSetter trait to set the header
        self.set_header(warning_header)
    }
    
    fn warning_incompatible_protocol(self, text: impl Into<String>) -> Self {
        let agent = Uri::sip("sip-server");
        self.warning(300, agent, text)
    }
    
    fn warning_incompatible_address_format(self, text: impl Into<String>) -> Self {
        let agent = Uri::sip("sip-server");
        self.warning(301, agent, text)
    }
    
    fn warning_incompatible_transport(self, text: impl Into<String>) -> Self {
        let agent = Uri::sip("sip-server");
        self.warning(302, agent, text)
    }
    
    fn warning_incompatible_media_format(self, text: impl Into<String>) -> Self {
        let agent = Uri::sip("sip-server");
        self.warning(305, agent, text)
    }
    
    fn warning_insufficient_bandwidth(self, text: impl Into<String>) -> Self {
        let agent = Uri::sip("sip-server");
        self.warning(370, agent, text)
    }
    
    fn warning_miscellaneous(self, text: impl Into<String>) -> Self {
        let agent = Uri::sip("sip-server");
        self.warning(399, agent, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_warning_custom() {
        let agent = Uri::sip("test-server.example.com");
        let response = ResponseBuilder::new(StatusCode::NotAcceptable, None)
            .warning(305, agent.clone(), "Incompatible media format")
            .build();
            
        if let Some(TypedHeader::Warning(warnings)) = response.header(&HeaderName::Warning) {
            assert_eq!(warnings.len(), 1);
            let warning = &warnings[0];
            assert_eq!(warning.code, 305);
            assert_eq!(warning.agent.host.to_string(), "test-server.example.com");
            assert_eq!(warning.text, "Incompatible media format");
        } else {
            panic!("Warning header not found or has wrong type");
        }
    }

    #[test]
    fn test_warning_convenience_methods() {
        // Test incompatible media format warning
        let response = ResponseBuilder::new(StatusCode::NotAcceptableHere, None)
            .warning_incompatible_media_format("H.265 codec not supported")
            .build();
            
        // First check if we have the header
        let all_headers = response.all_headers();
        let warning_headers: Vec<_> = all_headers.iter()
            .filter(|h| h.name() == HeaderName::Warning)
            .collect();
        
        assert!(!warning_headers.is_empty(), "Warning header not found");
        
        if let Some(TypedHeader::Warning(warnings)) = response.header(&HeaderName::Warning) {
            assert_eq!(warnings.len(), 1);
            let warning = &warnings[0];
            assert_eq!(warning.code, 305);
            assert_eq!(warning.text, "H.265 codec not supported");
        } else {
            panic!("Warning header not found or has wrong type");
        }
        
        // Test insufficient bandwidth warning
        let response = ResponseBuilder::new(StatusCode::TemporarilyUnavailable, None)
            .warning_insufficient_bandwidth("Not enough bandwidth for video")
            .build();
            
        // First check if we have the header
        let all_headers = response.all_headers();
        let warning_headers: Vec<_> = all_headers.iter()
            .filter(|h| h.name() == HeaderName::Warning)
            .collect();
        
        assert!(!warning_headers.is_empty(), "Warning header not found");
        
        if let Some(TypedHeader::Warning(warnings)) = response.header(&HeaderName::Warning) {
            assert_eq!(warnings.len(), 1);
            let warning = &warnings[0];
            assert_eq!(warning.code, 370);
            assert_eq!(warning.text, "Not enough bandwidth for video");
        } else {
            panic!("Warning header not found or has wrong type");
        }
    }

    #[test]
    fn test_multiple_warnings() {
        // Test adding multiple warnings
        let response = ResponseBuilder::new(StatusCode::NotAcceptable, None)
            .warning_incompatible_protocol("SIP/3.0 not supported")
            .warning_incompatible_media_format("H.265 codec not supported")
            .build();
            
        // Get all headers
        let headers = response.all_headers();
        
        // Count Warning headers
        let warning_headers = headers.iter()
            .filter_map(|h| if let TypedHeader::Warning(_) = h { Some(h) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(warning_headers.len(), 2);
    }
    
    #[test]
    fn test_warnings_method() {
        // Test adding multiple warnings at once
        let warning1 = Warning::new(370, Uri::sip("server1.example.com"), "Insufficient bandwidth");
        let warning2 = Warning::new(305, Uri::sip("server2.example.com"), "Incompatible codec");
        
        let response = ResponseBuilder::new(StatusCode::NotAcceptable, None)
            .warnings(vec![warning1, warning2])
            .build();
            
        if let Some(TypedHeader::Warning(warnings)) = response.header(&HeaderName::Warning) {
            assert_eq!(warnings.len(), 2);
            assert_eq!(warnings[0].code, 370);
            assert_eq!(warnings[1].code, 305);
        } else {
            panic!("Warning header not found or has wrong type");
        }
    }

    #[test]
    fn test_warning_codes() {
        // Test all warning code convenience methods
        let response = ResponseBuilder::new(StatusCode::BadRequest, None)
            .warning_incompatible_protocol("Protocol error")
            .build();
            
        // First check if we have the header
        let all_headers = response.all_headers();
        let warning_headers: Vec<_> = all_headers.iter()
            .filter(|h| h.name() == HeaderName::Warning)
            .collect();
        
        assert!(!warning_headers.is_empty(), "Warning header not found");
        
        if let Some(TypedHeader::Warning(warnings)) = response.header(&HeaderName::Warning) {
            assert_eq!(warnings.len(), 1);
            let warning = &warnings[0];
            assert_eq!(warning.code, 300);
        } else {
            panic!("Warning header not found or has wrong type");
        }
        
        let response = ResponseBuilder::new(StatusCode::BadRequest, None)
            .warning_incompatible_address_format("Address format error")
            .build();
            
        // First check if we have the header
        let all_headers = response.all_headers();
        let warning_headers: Vec<_> = all_headers.iter()
            .filter(|h| h.name() == HeaderName::Warning)
            .collect();
        
        assert!(!warning_headers.is_empty(), "Warning header not found");
        
        if let Some(TypedHeader::Warning(warnings)) = response.header(&HeaderName::Warning) {
            assert_eq!(warnings.len(), 1);
            let warning = &warnings[0];
            assert_eq!(warning.code, 301);
        } else {
            panic!("Warning header not found or has wrong type");
        }
        
        let response = ResponseBuilder::new(StatusCode::BadRequest, None)
            .warning_incompatible_transport("Transport error")
            .build();
            
        // First check if we have the header
        let all_headers = response.all_headers();
        let warning_headers: Vec<_> = all_headers.iter()
            .filter(|h| h.name() == HeaderName::Warning)
            .collect();
        
        assert!(!warning_headers.is_empty(), "Warning header not found");
        
        if let Some(TypedHeader::Warning(warnings)) = response.header(&HeaderName::Warning) {
            assert_eq!(warnings.len(), 1);
            let warning = &warnings[0];
            assert_eq!(warning.code, 302);
        } else {
            panic!("Warning header not found or has wrong type");
        }
        
        let response = ResponseBuilder::new(StatusCode::ServiceUnavailable, None)
            .warning_miscellaneous("Miscellaneous warning")
            .build();
            
        // First check if we have the header
        let all_headers = response.all_headers();
        let warning_headers: Vec<_> = all_headers.iter()
            .filter(|h| h.name() == HeaderName::Warning)
            .collect();
        
        assert!(!warning_headers.is_empty(), "Warning header not found");
        
        if let Some(TypedHeader::Warning(warnings)) = response.header(&HeaderName::Warning) {
            assert_eq!(warnings.len(), 1);
            let warning = &warnings[0];
            assert_eq!(warning.code, 399);
        } else {
            panic!("Warning header not found or has wrong type");
        }
    }
} 
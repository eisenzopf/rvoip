use crate::error::{Error, Result};
use crate::types::{
    error_info::{ErrorInfo, ErrorInfoHeader, ErrorInfoList},
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;

/// Error-Info header builder
///
/// This module provides builder methods for the Error-Info header in SIP messages.
///
/// ## SIP Error-Info Header Overview
///
/// The Error-Info header is defined in [RFC 3261 Section 20.18](https://datatracker.ietf.org/doc/html/rfc3261#section-20.18)
/// as part of the core SIP protocol. It provides a pointer to additional information about an error
/// returned in a response. This header is most commonly used with 3xx, 4xx, 5xx, and 6xx responses,
/// but can be included in any response.
///
/// ## Format
///
/// ```text
/// Error-Info: <sip:busy@example.com>;reason=busy
/// Error-Info: <https://example.com/errors/busy.html>
/// ```
///
/// ## Purpose of Error-Info Header
///
/// The Error-Info header serves several specific purposes in SIP:
///
/// 1. Provides a URI pointing to additional information about an error condition
/// 2. Allows servers to direct clients to human-readable error explanations
/// 3. Enables inclusion of error details that would be too verbose for the reason phrase
/// 4. Facilitates internationalization of error information
/// 5. Can point to alternative services that might satisfy the client's request
///
/// ## Common Parameters
///
/// - **reason**: Short explanation of the error condition
/// - **language**: Indicates the language of the referenced error information
/// - **description**: Brief textual description of the error
/// - **retry-after**: Suggested time to retry the request
///
/// ## Examples
///
/// ## 404 Not Found Response with Error-Info URL
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::ErrorInfoBuilderExt};
///
/// // Scenario: A user has been moved or is unavailable
///
/// // Create a 404 Not Found response with error information
/// let response = SimpleResponseBuilder::new(StatusCode::NotFound, Some("Not Found"))
///     .from("Server", "sip:pbx.example.com", Some("xyzzy"))
///     .to("User", "sip:user@example.com", Some("123abc"))
///     // Provide a URI where more information can be found
///     .error_info_uri("https://example.com/errors/user-not-found.html")
///     .build();
///
/// // The client can use this URL to display more information about why
/// // the user was not found and possible next steps
/// ```
///
/// ## 486 Busy Here with Informational URI
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::ErrorInfoBuilderExt};
///
/// // Scenario: Called party is busy on another call
///
/// // Create a 486 Busy Here response with error information
/// let response = SimpleResponseBuilder::new(StatusCode::BusyHere, Some("Busy Here"))
///     .from("Bob", "sip:bob@example.com", Some("tag456"))
///     .to("Alice", "sip:alice@example.com", Some("tag123"))
///     // Add an error info URI with a reason parameter
///     .error_info_uri_with_param("sip:busy@example.com", "reason", "in-call")
///     // Could also add a Retry-After header to suggest when to call back
///     .build();
///
/// // This tells the caller not only that the callee is busy,
/// // but provides additional context about the busy state (in a call)
/// ```
///
/// ## 500 Server Error with Multiple Error Resources
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::ErrorInfoBuilderExt};
///
/// // Scenario: Server encountered an internal error during call processing
///
/// // Create a 500 Server Error response with multiple error resources
/// let response = SimpleResponseBuilder::new(StatusCode::ServerInternalError, Some("Server Error"))
///     .from("SIP Server", "sip:pbx.example.com", Some("pbxtag"))
///     .to("User", "sip:user@example.com", Some("usertag"))
///     // First, add a link to an audio file explaining the error
///     .error_info_uri("http://example.com/sounds/server-error.wav")
///     // Then add a link to an HTML page with more details and a comment
///     .error_info_uri_with_comment("https://example.com/errors/server-error.html", "See details here")
///     .build();
///
/// // The client can access multiple error resources:
/// // - An audio announcement explaining the error
/// // - A web page with more details about the error
/// ```
pub trait ErrorInfoBuilderExt {
    /// Add an Error-Info header with a URI
    ///
    /// This method adds an Error-Info header with a URI pointing to additional
    /// information about an error.
    ///
    /// # Parameters
    ///
    /// * `uri` - The URI pointing to error information
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Error-Info header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::ErrorInfoBuilderExt};
    ///
    /// // Create a 404 response with error information
    /// let response = SimpleResponseBuilder::new(StatusCode::NotFound, Some("Not Found"))
    ///     .from("Server", "sip:server.example.com", None)
    ///     .to("User", "sip:user@example.com", None)
    ///     .error_info_uri("https://example.com/errors/user-not-found.html")
    ///     .build();
    ///
    /// // The response now includes an Error-Info header with the specified URI
    /// ```
    fn error_info_uri(self, uri: &str) -> Self;
    
    /// Add an Error-Info header with a URI and a parameter
    ///
    /// This method adds an Error-Info header with a URI and a single parameter,
    /// such as a reason code or language indicator.
    ///
    /// # Parameters
    ///
    /// * `uri` - The URI pointing to error information
    /// * `param_name` - The parameter name
    /// * `param_value` - The parameter value
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Error-Info header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::ErrorInfoBuilderExt};
    ///
    /// // Create a 486 response with error information and a reason parameter
    /// let response = SimpleResponseBuilder::new(StatusCode::BusyHere, Some("Busy Here"))
    ///     .from("Bob", "sip:bob@example.com", Some("tag456"))
    ///     .to("Alice", "sip:alice@example.com", Some("tag123"))
    ///     .error_info_uri_with_param("sip:busy@example.com", "reason", "in-call")
    ///     .build();
    ///
    /// // The response now includes an Error-Info header with the URI and parameter
    /// ```
    fn error_info_uri_with_param(self, uri: &str, param_name: &str, param_value: &str) -> Self;
    
    /// Add an Error-Info header with a URI and multiple parameters
    ///
    /// This method adds an Error-Info header with a URI and multiple parameters
    /// specified as key-value pairs.
    ///
    /// # Parameters
    ///
    /// * `uri` - The URI pointing to error information
    /// * `params` - A vector of parameter (name, value) tuples
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Error-Info header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::ErrorInfoBuilderExt};
    ///
    /// // Create a 503 response with error information and multiple parameters
    /// let response = SimpleResponseBuilder::new(StatusCode::ServiceUnavailable, Some("Service Unavailable"))
    ///     .from("Server", "sip:server.example.com", Some("xyz"))
    ///     .to("User", "sip:user@example.com", Some("abc"))
    ///     .error_info_uri_with_params(
    ///         "sip:overloaded@example.com", 
    ///         vec![
    ///             ("reason", "capacity-exceeded"),
    ///             ("retry-after", "300")
    ///         ]
    ///     )
    ///     .build();
    ///
    /// // The response now includes an Error-Info header with multiple parameters
    /// ```
    fn error_info_uri_with_params(self, uri: &str, params: Vec<(&str, &str)>) -> Self;
    
    /// Add an Error-Info header with a URI and a comment
    ///
    /// This method adds an Error-Info header with a URI and a human-readable comment
    /// explaining the error information.
    ///
    /// # Parameters
    ///
    /// * `uri` - The URI pointing to error information
    /// * `comment` - A human-readable comment about the error information
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Error-Info header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::ErrorInfoBuilderExt};
    ///
    /// // Create a 500 response with error information and a comment
    /// let response = SimpleResponseBuilder::new(StatusCode::ServerInternalError, Some("Server Error"))
    ///     .from("Server", "sip:server.example.com", Some("srv-tag"))
    ///     .to("User", "sip:user@example.com", Some("usr-tag"))
    ///     .error_info_uri_with_comment(
    ///         "https://example.com/errors/server-error.html",
    ///         "See this page for error details and status"
    ///     )
    ///     .build();
    ///
    /// // The response now includes an Error-Info header with a URI and comment
    /// ```
    fn error_info_uri_with_comment(self, uri: &str, comment: &str) -> Self;
    
    /// Add multiple Error-Info headers to a response
    ///
    /// This method adds multiple Error-Info headers to a response, each with a different
    /// URI pointing to different error information resources.
    ///
    /// # Parameters
    ///
    /// * `uris` - A vector of URIs pointing to error information
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Error-Info headers added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::ErrorInfoBuilderExt};
    ///
    /// // Create a 500 response with multiple error information URIs
    /// let response = SimpleResponseBuilder::new(StatusCode::ServerInternalError, Some("Server Error"))
    ///     .from("Server", "sip:server.example.com", Some("xyz"))
    ///     .to("User", "sip:user@example.com", Some("abc"))
    ///     .error_info_uris(vec![
    ///         "http://example.com/sounds/server-error.wav",
    ///         "https://example.com/errors/server-error.html"
    ///     ])
    ///     .build();
    ///
    /// // The response now includes multiple Error-Info headers
    /// ```
    fn error_info_uris(self, uris: Vec<&str>) -> Self;
}

impl<T> ErrorInfoBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn error_info_uri(self, uri: &str) -> Self {
        let error_info = ErrorInfo::new(uri);
        let mut header = ErrorInfoHeader::new();
        header.error_info_list.add(error_info);
        self.set_header(header)
    }
    
    fn error_info_uri_with_param(self, uri: &str, param_name: &str, param_value: &str) -> Self {
        let error_info = ErrorInfo::new(uri).with_param(param_name, param_value);
        let mut header = ErrorInfoHeader::new();
        header.error_info_list.add(error_info);
        self.set_header(header)
    }
    
    fn error_info_uri_with_params(self, uri: &str, params: Vec<(&str, &str)>) -> Self {
        let mut error_info = ErrorInfo::new(uri);
        for (name, value) in params {
            error_info = error_info.with_param(name, value);
        }
        let mut header = ErrorInfoHeader::new();
        header.error_info_list.add(error_info);
        self.set_header(header)
    }
    
    fn error_info_uri_with_comment(self, uri: &str, comment: &str) -> Self {
        let error_info = ErrorInfo::new(uri).with_comment(comment);
        let mut header = ErrorInfoHeader::new();
        header.error_info_list.add(error_info);
        self.set_header(header)
    }
    
    fn error_info_uris(self, uris: Vec<&str>) -> Self {
        let mut header = ErrorInfoHeader::new();
        for uri in uris {
            header.error_info_list.add(ErrorInfo::new(uri));
        }
        self.set_header(header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;
    use std::convert::TryFrom;
    use crate::types::header::TypedHeaderTrait;
    use crate::types::TypedHeader;

    #[test]
    fn test_error_info_uri() {
        let response = ResponseBuilder::new(StatusCode::NotFound, Some("Not Found"))
            .error_info_uri("https://example.com/errors/user-not-found.html")
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::ErrorInfo(header)) = response.header(&HeaderName::ErrorInfo) {
            assert_eq!(header.error_info_list.len(), 1);
            assert_eq!(header.error_info_list.items[0].uri, "https://example.com/errors/user-not-found.html");
        } else {
            panic!("Error-Info header not found or has wrong type");
        }
    }

    #[test]
    fn test_error_info_uri_with_param() {
        let response = ResponseBuilder::new(StatusCode::BusyHere, Some("Busy Here"))
            .error_info_uri_with_param("sip:busy@example.com", "reason", "in-call")
            .build();
            
        if let Some(TypedHeader::ErrorInfo(header)) = response.header(&HeaderName::ErrorInfo) {
            assert_eq!(header.error_info_list.len(), 1);
            assert_eq!(header.error_info_list.items[0].uri, "sip:busy@example.com");
            assert_eq!(header.error_info_list.items[0].parameters.get("reason").unwrap(), "in-call");
        } else {
            panic!("Error-Info header not found or has wrong type");
        }
    }

    #[test]
    fn test_error_info_uri_with_params() {
        let response = ResponseBuilder::new(StatusCode::ServiceUnavailable, Some("Service Unavailable"))
            .error_info_uri_with_params(
                "sip:overloaded@example.com", 
                vec![
                    ("reason", "capacity-exceeded"),
                    ("retry-after", "300")
                ]
            )
            .build();
            
        if let Some(TypedHeader::ErrorInfo(header)) = response.header(&HeaderName::ErrorInfo) {
            assert_eq!(header.error_info_list.len(), 1);
            assert_eq!(header.error_info_list.items[0].uri, "sip:overloaded@example.com");
            assert_eq!(header.error_info_list.items[0].parameters.get("reason").unwrap(), "capacity-exceeded");
            assert_eq!(header.error_info_list.items[0].parameters.get("retry-after").unwrap(), "300");
        } else {
            panic!("Error-Info header not found or has wrong type");
        }
    }

    #[test]
    fn test_error_info_uri_with_comment() {
        let response = ResponseBuilder::new(StatusCode::ServerInternalError, Some("Server Error"))
            .error_info_uri_with_comment(
                "https://example.com/errors/server-error.html",
                "See this page for error details and status"
            )
            .build();
            
        if let Some(TypedHeader::ErrorInfo(header)) = response.header(&HeaderName::ErrorInfo) {
            assert_eq!(header.error_info_list.len(), 1);
            assert_eq!(header.error_info_list.items[0].uri, "https://example.com/errors/server-error.html");
            assert_eq!(header.error_info_list.items[0].comment.as_ref().unwrap(), "See this page for error details and status");
        } else {
            panic!("Error-Info header not found or has wrong type");
        }
    }

    #[test]
    fn test_error_info_uris() {
        let response = ResponseBuilder::new(StatusCode::ServerInternalError, Some("Server Error"))
            .error_info_uris(vec![
                "http://example.com/sounds/server-error.wav",
                "https://example.com/errors/server-error.html"
            ])
            .build();
            
        if let Some(TypedHeader::ErrorInfo(header)) = response.header(&HeaderName::ErrorInfo) {
            assert_eq!(header.error_info_list.len(), 2);
            assert_eq!(header.error_info_list.items[0].uri, "http://example.com/sounds/server-error.wav");
            assert_eq!(header.error_info_list.items[1].uri, "https://example.com/errors/server-error.html");
        } else {
            panic!("Error-Info header not found or has wrong type");
        }
    }

    #[test]
    fn test_multiple_error_info_methods() {
        let response = ResponseBuilder::new(StatusCode::ServerInternalError, Some("Server Error"))
            .error_info_uri("http://example.com/sounds/server-error.wav")
            .error_info_uri_with_comment(
                "https://example.com/errors/server-error.html",
                "See this page for error details"
            )
            .build();
        
        if let Some(TypedHeader::ErrorInfo(header)) = response.header(&HeaderName::ErrorInfo) {
            assert_eq!(header.error_info_list.len(), 1);
            assert_eq!(header.error_info_list.items[0].uri, "http://example.com/sounds/server-error.wav");
        } else {
            panic!("Error-Info header not found in response");
        }
    }

    #[test]
    fn test_debug_error_info_header() {
        // Create a simple ErrorInfoHeader
        let error_info = ErrorInfo::new("sip:busy@example.com").with_param("reason", "busy");
        let mut header = ErrorInfoHeader::new();
        header.error_info_list.add(error_info);
        
        // Convert to a generic Header through TypedHeaderTrait
        let generic_header = header.to_header();
        println!("Generic header: {:?}", generic_header);
        
        // Try to convert back to TypedHeader
        match TypedHeader::try_from(generic_header) {
            Ok(typed_header) => {
                println!("Converted to TypedHeader: {:?}", typed_header);
                // Check if it's the correct variant
                match typed_header {
                    TypedHeader::ErrorInfo(ei) => {
                        println!("Successfully got ErrorInfoHeader: {:?}", ei);
                    },
                    _ => {
                        println!("Got wrong TypedHeader variant: {:?}", typed_header);
                    }
                }
            },
            Err(e) => {
                println!("Failed to convert to TypedHeader: {:?}", e);
            }
        }
        
        // Now try via the builder
        let response = ResponseBuilder::new(StatusCode::NotFound, Some("Not Found"))
            .error_info_uri("sip:busy@example.com")
            .build();
            
        println!("Response headers: {:?}", response.headers);
        
        // Check if the header is present
        if let Some(h) = response.header(&HeaderName::ErrorInfo) {
            println!("Found ErrorInfo header: {:?}", h);
        } else {
            println!("ErrorInfo header not found in response");
            for (i, h) in response.headers.iter().enumerate() {
                println!("Header {}: {:?}", i, h);
            }
        }
    }
} 
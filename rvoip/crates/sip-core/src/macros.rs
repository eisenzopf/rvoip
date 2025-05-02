//! # SIP Macros
//!
//! This module provides macros for creating SIP messages with a more concise syntax.
//!
//! The macros make it easy to create SIP requests and responses with a declarative syntax,
//! while handling all the underlying complexity of properly formatting headers and parameters.
//!
//! ## Features
//!
//! - **Declarative Syntax**: Create requests and responses with a clear, readable format
//! - **Simple Parameter Handling**: Specify fields like `from_name`, `from_uri`, etc. directly
//! - **Sensible Defaults**: Optional parameters can be omitted
//! - **Robust Error Handling**: Fallbacks for parsing failures
//! - **Full Header Support**: Add both standard and custom headers
//!
//! ## Usage
//!
//! ### Creating a SIP Request
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use rvoip_sip_core::sip_request;
//!
//! let request = sip_request! {
//!     method: Method::Invite,
//!     uri: "sip:bob@example.com",
//!     from_name: "Alice", 
//!     from_uri: "sip:alice@example.com", 
//!     from_tag: "1928301774",
//!     to_name: "Bob", 
//!     to_uri: "sip:bob@example.com",
//!     call_id: "a84b4c76e66710@pc33.atlanta.example.com",
//!     cseq: 1,
//!     via_host: "alice.example.com:5060", 
//!     via_transport: "UDP", 
//!     via_branch: "z9hG4bK776asdhds",
//!     max_forwards: 70,
//!     body: "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n"
//! };
//! ```
//!
//! ### Creating a SIP Response
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use rvoip_sip_core::sip_response;
//!
//! let response = sip_response! {
//!     status: StatusCode::Ok,
//!     reason: "OK",
//!     from_name: "Alice", 
//!     from_uri: "sip:alice@example.com", 
//!     from_tag: "1928301774",
//!     to_name: "Bob", 
//!     to_uri: "sip:bob@example.com", 
//!     to_tag: "a6c85cf",
//!     call_id: "a84b4c76e66710@pc33.atlanta.example.com",
//!     cseq: 314159, 
//!     cseq_method: Method::Invite,
//!     via_host: "pc33.atlanta.com", 
//!     via_transport: "UDP", 
//!     via_branch: "z9hG4bK776asdhds"
//! };
//! ```
//!
//! ### Adding Custom Headers
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use rvoip_sip_core::sip_request;
//!
//! let request = sip_request! {
//!     method: Method::Invite,
//!     uri: "sip:bob@example.com",
//!     from_name: "Alice", 
//!     from_uri: "sip:alice@example.com",
//!     headers: {
//!         UserAgent: "My SIP Client 1.0",
//!         Subject: "Important Call",
//!         Priority: "urgent",
//!         CustomHeader: "Custom Value"
//!     }
//! };
//! ```

use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::types::{Method, StatusCode, TypedHeader, uri::Uri};
use std::str::FromStr;

/// Helper macro to convert optional parameters to Option<T>
#[macro_export]
#[doc(hidden)]
macro_rules! option_expr {
    () => { None::<String> };
    ($expr:expr) => { Some($expr.to_string()) };
}

/// Macro for creating SIP request messages with a concise syntax.
///
/// # Examples
///
/// ```rust
/// # use rvoip_sip_core::sip_request;
/// # use rvoip_sip_core::types::{Method, StatusCode};
/// let request = sip_request! {
///     method: Method::Invite,
///     uri: "sip:bob@example.com",
///     from_name: "Alice", 
///     from_uri: "sip:alice@example.com", 
///     from_tag: "1928301774",
///     to_name: "Bob", 
///     to_uri: "sip:bob@example.com",
///     call_id: "a84b4c76e66710@pc33.atlanta.example.com",
///     cseq: 1,
///     via_host: "alice.example.com:5060", 
///     via_transport: "UDP", 
///     via_branch: "z9hG4bK776asdhds",
///     max_forwards: 70,
///     body: "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n"
/// };
/// ```
#[macro_export]
macro_rules! sip_request {
    (
        method: $method:expr,
        uri: $uri:expr
        $(, from_name: $from_name:expr)?
        $(, from_uri: $from_uri:expr)?
        $(, from_tag: $from_tag:expr)?
        $(, to_name: $to_name:expr)?
        $(, to_uri: $to_uri:expr)?
        $(, to_tag: $to_tag:expr)?
        $(, call_id: $call_id:expr)?
        $(, cseq: $cseq:expr)?
        $(, via_host: $via_host:expr)?
        $(, via_transport: $via_transport:expr)?
        $(, via_branch: $via_branch:expr)?
        $(, max_forwards: $max_forwards:expr)?
        $(, contact_uri: $contact_uri:expr)?
        $(, contact_name: $contact_name:expr)?
        $(, content_type: $content_type:expr)?
        $(, headers: {
            $($header_name:ident : $header_value:expr),* $(,)?
        })?
        $(, body: $body:expr)?
        $(,)?
    ) => {
        {
            use $crate::builder::SimpleRequestBuilder;
            use $crate::types::TypedHeader;
            use std::str::FromStr;

            // Create the builder with method and URI
            let mut builder = SimpleRequestBuilder::new($method, $uri)
                .expect("Failed to create SimpleRequestBuilder with the provided URI");
            
            // Add From header if required parts are provided
            if let (Some(name), Some(uri)) = (
                option_expr!($($from_name)?), 
                option_expr!($($from_uri)?)
            ) {
                let tag = option_expr!($($from_tag)?);
                builder = builder.from(&name, &uri, tag.as_deref());
            }
            
            // Add To header if required parts are provided
            if let (Some(name), Some(uri)) = (
                option_expr!($($to_name)?), 
                option_expr!($($to_uri)?)
            ) {
                let tag = option_expr!($($to_tag)?);
                builder = builder.to(&name, &uri, tag.as_deref());
            }
            
            // Add Call-ID if provided
            if let Some(call_id) = option_expr!($($call_id)?) {
                builder = builder.call_id(&call_id);
            }
            
            // Add CSeq if provided
            if let Some(cseq_str) = option_expr!($($cseq)?) {
                // Convert string to u32 if needed
                let cseq_num = match cseq_str.parse::<u32>() {
                    Ok(num) => num,
                    Err(_) => 0, // Fallback value
                };
                builder = builder.cseq(cseq_num);
            }
            
            // Add Via if required parts are provided
            if let (Some(host), Some(transport)) = (
                option_expr!($($via_host)?), 
                option_expr!($($via_transport)?)
            ) {
                let branch = option_expr!($($via_branch)?);
                builder = builder.via(&host, &transport, branch.as_deref());
            }
            
            // Add Max-Forwards if provided
            if let Some(max_forwards_str) = option_expr!($($max_forwards)?) {
                // Convert string to u32 if needed
                let max_forwards_num = match max_forwards_str.parse::<u32>() {
                    Ok(num) => num,
                    Err(_) => 70, // Default fallback value
                };
                builder = builder.max_forwards(max_forwards_num);
            }
            
            // Add Contact if provided
            if let Some(uri) = option_expr!($($contact_uri)?) {
                let name = option_expr!($($contact_name)?);
                builder = builder.contact(&uri, name.as_deref());
            }
            
            // Add Content-Type if provided
            if let Some(content_type) = option_expr!($($content_type)?) {
                builder = builder.content_type(&content_type);
            }
            
            // Add custom headers if provided
            $(
                $(
                    let header_name = stringify!($header_name);
                    let header_value = $header_value;
                    
                    // Special handling for common headers
                    match header_name {
                        "MaxForwards" => {
                            builder = builder.max_forwards(header_value.parse::<u32>().expect("Invalid Max-Forwards value"));
                        },
                        "UserAgent" => {
                            // Handle User-Agent header with custom logic if needed
                            builder = builder.header(TypedHeader::UserAgent(vec![header_value.to_string()]));
                        },
                        "ContentType" => {
                            // Special handling for ContentType to use the content_type method
                            builder = builder.content_type(&header_value);
                        },
                        _ => {
                            // Generic header handling
                            use $crate::types::header::{HeaderName, HeaderValue};
                            
                            // Handle capitalization for header names
                            let name = if header_name.contains('_') {
                                // Convert snake_case to Header-Case
                                header_name.split('_')
                                    .map(|part| {
                                        let mut chars = part.chars();
                                        match chars.next() {
                                            Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                                            None => String::new()
                                        }
                                    })
                                    .collect::<Vec<_>>()
                                    .join("-")
                            } else {
                                // Just capitalize the first letter
                                let mut chars = header_name.chars();
                                match chars.next() {
                                    Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                                    None => String::new()
                                }
                            };
                            
                            builder = builder.header(TypedHeader::Other(
                                HeaderName::Other(name),
                                HeaderValue::text(header_value)
                            ));
                        }
                    }
                )*
            )?
            
            // Add body if provided
            if let Some(body) = option_expr!($($body)?) {
                builder = builder.body(body);
            }
            
            // Build the final request
            builder.build()
        }
    };
}

/// Macro for creating SIP response messages with a concise syntax.
///
/// # Examples
///
/// ```rust
/// # use rvoip_sip_core::sip_response;
/// # use rvoip_sip_core::types::{StatusCode, Method};
/// let response = sip_response! {
///     status: StatusCode::Ok,
///     reason: "OK",
///     from_name: "Alice", 
///     from_uri: "sip:alice@example.com", 
///     from_tag: "1928301774",
///     to_name: "Bob", 
///     to_uri: "sip:bob@example.com", 
///     to_tag: "a6c85cf",
///     call_id: "a84b4c76e66710",
///     cseq: 314159, 
///     cseq_method: Method::Invite,
///     via_host: "pc33.atlanta.com", 
///     via_transport: "UDP", 
///     via_branch: "z9hG4bK776asdhds",
///     max_forwards: 70,
///     body: "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n"
/// };
/// ```
#[macro_export]
macro_rules! sip_response {
    (
        status: $status:expr
        $(, reason: $reason:expr)?
        $(, from_name: $from_name:expr)?
        $(, from_uri: $from_uri:expr)?
        $(, from_tag: $from_tag:expr)?
        $(, to_name: $to_name:expr)?
        $(, to_uri: $to_uri:expr)?
        $(, to_tag: $to_tag:expr)?
        $(, call_id: $call_id:expr)?
        $(, cseq: $cseq:expr)?
        $(, cseq_method: $cseq_method:expr)?
        $(, via_host: $via_host:expr)?
        $(, via_transport: $via_transport:expr)?
        $(, via_branch: $via_branch:expr)?
        $(, max_forwards: $max_forwards:expr)?
        $(, contact_uri: $contact_uri:expr)?
        $(, contact_name: $contact_name:expr)?
        $(, content_type: $content_type:expr)?
        $(, headers: {
            $($header_name:ident : $header_value:expr),* $(,)?
        })?
        $(, body: $body:expr)?
        $(,)?
    ) => {
        {
            use $crate::builder::SimpleResponseBuilder;
            use $crate::types::TypedHeader;
            use std::str::FromStr;
            
            // Create the builder with status and optional reason
            let reason = option_expr!($($reason)?);
            let mut builder = SimpleResponseBuilder::new($status, reason.as_deref());
            
            // Add From header if required parts are provided
            if let (Some(name), Some(uri)) = (
                option_expr!($($from_name)?), 
                option_expr!($($from_uri)?)
            ) {
                let tag = option_expr!($($from_tag)?);
                builder = builder.from(&name, &uri, tag.as_deref());
            }
            
            // Add To header if required parts are provided
            if let (Some(name), Some(uri)) = (
                option_expr!($($to_name)?), 
                option_expr!($($to_uri)?)
            ) {
                let tag = option_expr!($($to_tag)?);
                builder = builder.to(&name, &uri, tag.as_deref());
            }
            
            // Add Call-ID if provided
            if let Some(call_id) = option_expr!($($call_id)?) {
                builder = builder.call_id(&call_id);
            }
            
            // Add CSeq if all required parts are provided
            if let (Some(seq_str), Some(method_str)) = (
                option_expr!($($cseq)?),
                option_expr!($($cseq_method)?)
            ) {
                // Convert string to u32 if needed
                let cseq_num = match seq_str.parse::<u32>() {
                    Ok(num) => num,
                    Err(_) => 0, // Fallback value
                };
                
                // Parse the method string to a Method enum
                let method_enum = Method::from_str(&method_str)
                    .unwrap_or(Method::Invite); // Default fallback
                
                builder = builder.cseq(cseq_num, method_enum);
            }
            
            // Add Via if required parts are provided
            if let (Some(host), Some(transport)) = (
                option_expr!($($via_host)?), 
                option_expr!($($via_transport)?)
            ) {
                let branch = option_expr!($($via_branch)?);
                builder = builder.via(&host, &transport, branch.as_deref());
            }
            
            // Add Max-Forwards if provided
            if let Some(max_forwards_str) = option_expr!($($max_forwards)?) {
                // Convert string to u8 if needed
                let max_forwards_num = match max_forwards_str.parse::<u8>() {
                    Ok(num) => num,
                    Err(_) => 70, // Default fallback value
                };
                
                // Headers are added with the header() method in response
                use $crate::types::header::{HeaderName, HeaderValue};
                use $crate::types::max_forwards::MaxForwards;
                
                builder = builder.header(TypedHeader::MaxForwards(
                    MaxForwards::new(max_forwards_num)
                ));
            }
            
            // Add Contact if provided
            if let Some(uri) = option_expr!($($contact_uri)?) {
                let name = option_expr!($($contact_name)?);
                builder = builder.contact(&uri, name.as_deref());
            }
            
            // Add Content-Type if provided
            if let Some(content_type) = option_expr!($($content_type)?) {
                builder = builder.content_type(&content_type);
            }
            
            // Add custom headers if provided
            $(
                $(
                    let header_name = stringify!($header_name);
                    let header_value = $header_value;
                    
                    // Special handling for specific headers
                    match header_name {
                        "MaxForwards" => {
                            // Add with the header method
                            use $crate::types::header::{HeaderName, HeaderValue};
                            use $crate::types::max_forwards::MaxForwards;
                            
                            builder = builder.header(TypedHeader::MaxForwards(
                                MaxForwards::new(header_value.parse::<u8>().expect("Invalid Max-Forwards value"))
                            ));
                        },
                        "Server" => {
                            // Handle Server header
                            builder = builder.header(TypedHeader::Server(vec![header_value.to_string()]));
                        },
                        "ContentType" => {
                            // Special handling for ContentType to use the content_type method
                            builder = builder.content_type(&header_value);
                        },
                        _ => {
                            // Generic header handling
                            use $crate::types::header::{HeaderName, HeaderValue};
                            
                            // Handle capitalization for header names
                            let name = if header_name.contains('_') {
                                // Convert snake_case to Header-Case
                                header_name.split('_')
                                    .map(|part| {
                                        let mut chars = part.chars();
                                        match chars.next() {
                                            Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                                            None => String::new()
                                        }
                                    })
                                    .collect::<Vec<_>>()
                                    .join("-")
                            } else {
                                // Just capitalize the first letter
                                let mut chars = header_name.chars();
                                match chars.next() {
                                    Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                                    None => String::new()
                                }
                            };
                            
                            builder = builder.header(TypedHeader::Other(
                                HeaderName::Other(name),
                                HeaderValue::text(header_value)
                            ));
                        }
                    }
                )*
            )?
            
            // Add body if provided
            if let Some(body) = option_expr!($($body)?) {
                builder = builder.body(body);
            }
            
            // Build the final response
            builder.build()
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        Method, StatusCode, uri::Uri, Address, TypedHeader, header::{HeaderName, HeaderValue},
        sip_request::Request, sip_response::Response,
    };

    #[test]
    fn test_sip_request_basic() {
        // Test a basic INVITE request
        let request = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            from_name: "Alice", 
            from_uri: "sip:alice@example.com", 
            from_tag: "1928301774",
            to_name: "Bob", 
            to_uri: "sip:bob@example.com",
            call_id: "a84b4c76e66710@pc33.atlanta.example.com",
            cseq: 1,
            via_host: "alice.example.com:5060", 
            via_transport: "UDP", 
            via_branch: "z9hG4bK776asdhds",
            max_forwards: 70
        };

        // Check method and URI
        assert_eq!(request.method, Method::Invite);
        assert_eq!(request.uri.to_string(), "sip:bob@example.com");
        
        // Check headers
        let from = request.from().unwrap();
        let to = request.to().unwrap();
        let call_id = request.call_id().unwrap();
        let cseq = request.cseq().unwrap();
        let via = request.first_via().unwrap();
        
        // Verify content
        assert_eq!(from.address().display_name(), Some("Alice"));
        assert_eq!(from.address().uri.to_string(), "sip:alice@example.com");
        assert_eq!(from.tag(), Some("1928301774"));
        
        assert_eq!(to.address().display_name(), Some("Bob"));
        assert_eq!(to.address().uri.to_string(), "sip:bob@example.com");
        
        assert_eq!(call_id.value(), "a84b4c76e66710@pc33.atlanta.example.com");
        assert_eq!(cseq.sequence(), 1);
        assert_eq!(*cseq.method(), Method::Invite);
        
        // Via info is stored differently in the Via struct
        assert!(via.branch().is_some());
        assert_eq!(via.branch().unwrap(), "z9hG4bK776asdhds");
    }

    #[test]
    fn test_sip_request_with_body() {
        // Test INVITE with SDP body
        let sdp_body = "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n";
        
        let request = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            headers: {
                From: "Alice <sip:alice@example.com>;tag=1928301774",
                To: "Bob <sip:bob@example.com>",
                CallId: "a84b4c76e66710@pc33.atlanta.example.com",
                Via: "SIP/2.0/UDP alice.example.com:5060;branch=z9hG4bK776asdhds",
                ContentType: "application/sdp",
                MaxForwards: "70"
            },
            body: sdp_body
        };

        // Check body content
        assert_eq!(String::from_utf8_lossy(&request.body), sdp_body);
        
        // Check Content-Type header
        let content_type = request.typed_header::<crate::types::content_type::ContentType>().unwrap();
        assert_eq!(content_type.to_string(), "application/sdp");
    }
    
    #[test]
    fn test_sip_response_basic() {
        // Test a basic 200 OK response
        let response = sip_response! {
            status: StatusCode::Ok,
            reason: "OK",
            from_name: "Alice", 
            from_uri: "sip:alice@example.com", 
            from_tag: "1928301774",
            to_name: "Bob", 
            to_uri: "sip:bob@example.com", 
            to_tag: "as83kd9bs",
            call_id: "a84b4c76e66710@pc33.atlanta.example.com",
            cseq: 1, 
            cseq_method: Method::Invite,
            via_host: "alice.example.com:5060", 
            via_transport: "UDP", 
            via_branch: "z9hG4bK776asdhds",
            max_forwards: 70
        };

        // Check status and reason
        assert_eq!(response.status, StatusCode::Ok);
        assert_eq!(response.reason, Some("OK".to_string()));
        
        // Check From/To tags
        let from = response.from().unwrap();
        let to = response.to().unwrap();
        
        assert_eq!(from.tag(), Some("1928301774"));
        assert_eq!(to.tag(), Some("as83kd9bs"));
        
        // Check other basic headers
        let call_id = response.call_id().unwrap();
        let cseq = response.cseq().unwrap();
        
        assert_eq!(call_id.value(), "a84b4c76e66710@pc33.atlanta.example.com");
        assert_eq!(cseq.sequence(), 1);
        assert_eq!(*cseq.method(), Method::Invite);
    }

    #[test]
    fn test_sip_response_with_body() {
        // Test a 200 OK with SDP body
        let sdp_body = "v=0\r\no=bob 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n";
        
        let response = sip_response! {
            status: StatusCode::Ok,
            reason: "OK",
            headers: {
                From: "Alice <sip:alice@example.com>;tag=1928301774",
                To: "Bob <sip:bob@example.com>;tag=as83kd9bs",
                CallId: "a84b4c76e66710@pc33.atlanta.example.com",
                CSeq: "1 INVITE",
                Via: "SIP/2.0/UDP alice.example.com:5060;branch=z9hG4bK776asdhds",
                ContentType: "application/sdp",
                MaxForwards: "70"
            },
            body: sdp_body
        };

        // Check body
        assert_eq!(String::from_utf8_lossy(&response.body), sdp_body);
        
        // Check Content-Type
        let content_type = response.typed_header::<crate::types::content_type::ContentType>().unwrap();
        assert_eq!(content_type.to_string(), "application/sdp");
    }
} 
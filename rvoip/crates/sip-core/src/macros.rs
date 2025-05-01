/// Macro for creating SIP request messages with a more concise syntax.
///
/// # Examples
///
/// ```
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
            use $crate::types::Method;
            use std::str::FromStr;
            use $crate::builder::RequestBuilder;
            use $crate::types::{TypedHeader, header::{HeaderName, HeaderValue}};
            use $crate::types::uri::{Uri, Host, Scheme};
            use $crate::types::Version;
            
            // Create a RequestBuilder from the method and URI string
            let mut builder = RequestBuilder::new($method, $uri)
                .expect("Failed to create RequestBuilder with the provided URI");
            
            // Add From header if all required parts are provided
            if let (Some(name), Some(uri)) = (
                option_expr!($($from_name)?), 
                option_expr!($($from_uri)?)
            ) {
                // Use the from() method but store the returned AddressBuilder
                let address_builder = builder.from(name, uri);
                
                // Add tag if provided
                let address_builder = if let Some(tag) = option_expr!($($from_tag)?) {
                    address_builder.with_tag(tag)
                } else {
                    address_builder
                };
                
                // Call done() to get back the RequestBuilder
                builder = address_builder.done();
            }
            
            // Add To header if all required parts are provided
            if let (Some(name), Some(uri)) = (
                option_expr!($($to_name)?), 
                option_expr!($($to_uri)?)
            ) {
                // Use the to() method but store the returned AddressBuilder, then call done()
                builder = builder.to(name, uri).done();
            }
            
            // Add Call-ID if provided
            if let Some(call_id) = option_expr!($($call_id)?) {
                builder = builder.call_id(call_id);
            }
            
            // Add CSeq if provided
            if let Some(cseq) = option_expr!($($cseq)?) {
                builder = builder.cseq(cseq);
            }
            
            // Add Via if all required parts are provided
            if let (Some(host), Some(transport)) = (
                option_expr!($($via_host)?), 
                option_expr!($($via_transport)?)
            ) {
                // Use the via() method but store the returned ViaBuilder
                let via_builder = builder.via(host, transport);
                
                // Add branch if provided
                let via_builder = if let Some(branch) = option_expr!($($via_branch)?) {
                    via_builder.with_branch(branch)
                } else {
                    via_builder
                };
                
                // Call done() to get back the RequestBuilder
                builder = via_builder.done();
            }
            
            // Add Max-Forwards if provided
            if let Some(max_forwards) = option_expr!($($max_forwards)?) {
                builder = builder.max_forwards(max_forwards);
            }
            
            // Add Contact if provided
            if let Some(contact_uri) = option_expr!($($contact_uri)?) {
                builder = builder.contact(contact_uri)
                    .expect("Contact URI parse error");
            }
            
            // Add Contact with name if provided
            if let (Some(name), Some(uri)) = (
                option_expr!($($contact_name)?), 
                option_expr!($($contact_uri)?)
            ) {
                builder = builder.contact_with_name(name, uri)
                    .expect("Contact URI parse error");
            }
            
            // Add Content-Type if provided
            if let Some(content_type) = option_expr!($($content_type)?) {
                builder = builder.content_type(content_type)
                    .expect("Content-Type parse error");
            }
            
            // Add custom headers if provided
            $(
                $(
                    builder = match stringify!($header_name) {
                        "UserAgent" => {
                            builder.header(TypedHeader::Other(
                                HeaderName::UserAgent, 
                                HeaderValue::text($header_value)
                            ))
                        },
                        "Server" => {
                            builder.header(TypedHeader::Other(
                                HeaderName::Server, 
                                HeaderValue::text($header_value)
                            ))
                        },
                        "Accept" => {
                            builder.header(TypedHeader::Other(
                                HeaderName::Accept, 
                                HeaderValue::text($header_value)
                            ))
                        },
                        "Warning" => {
                            builder.header(TypedHeader::Other(
                                HeaderName::Warning, 
                                HeaderValue::text($header_value)
                            ))
                        },
                        "MaxForwards" => {
                            builder.max_forwards($header_value.parse::<u32>().expect("Invalid Max-Forwards value"))
                        },
                        header_name => {
                            // Capitalize first letter and handle underscores
                            let mut name = header_name.to_string();
                            if !name.is_empty() {
                                let first_char = name.remove(0).to_uppercase().to_string();
                                name = first_char + &name;
                                
                                // Replace underscores with hyphens and capitalize each word
                                let parts: Vec<&str> = name.split('_').collect();
                                if parts.len() > 1 {
                                    name = parts.iter().map(|part| {
                                        if !part.is_empty() {
                                            let mut p = part.to_string();
                                            let first = p.remove(0).to_uppercase().to_string();
                                            first + &p
                                        } else {
                                            String::new()
                                        }
                                    }).collect::<Vec<_>>().join("-");
                                }
                            }
                            
                            builder.header(TypedHeader::Other(
                                HeaderName::Other(name), 
                                HeaderValue::text($header_value)
                            ))
                        }
                    };
                )*
            )?
            
            // Add body if provided
            if let Some(body) = option_expr!($($body)?) {
                builder = builder.body(body);
            }

            builder.build()
        }
    };
}

// Helper macro to convert optional parameters to Option<T>
#[macro_export]
#[doc(hidden)]
macro_rules! option_expr {
    () => { None };
    ($expr:expr) => { Some($expr) };
}

/// Macro for creating SIP response messages with a more concise syntax.
///
/// # Examples
///
/// ```
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
            use $crate::types::{StatusCode, Method};
            use std::str::FromStr;
            use $crate::builder::ResponseBuilder;
            use $crate::types::{TypedHeader, header::{HeaderName, HeaderValue}};
            use $crate::types::Version;
            
            // Create a ResponseBuilder with the status code
            let mut builder = ResponseBuilder::new($status);
            
            // Add reason phrase if provided
            if let Some(reason) = option_expr!($($reason)?) {
                builder = builder.reason(reason);
            }
            
            // Add From header if all required parts are provided
            if let (Some(name), Some(uri)) = (
                option_expr!($($from_name)?), 
                option_expr!($($from_uri)?)
            ) {
                // Use the from() method but store the returned AddressBuilder
                let address_builder = builder.from(name, uri);
                
                // Add tag if provided
                let address_builder = if let Some(tag) = option_expr!($($from_tag)?) {
                    address_builder.with_tag(tag)
                } else {
                    address_builder
                };
                
                // Call done() to get back the ResponseBuilder
                builder = address_builder.done();
            }
            
            // Add To header if all required parts are provided
            if let (Some(name), Some(uri)) = (
                option_expr!($($to_name)?), 
                option_expr!($($to_uri)?)
            ) {
                // Use the to() method but store the returned AddressBuilder
                let address_builder = builder.to(name, uri);
                
                // Add tag if provided
                let address_builder = if let Some(tag) = option_expr!($($to_tag)?) {
                    address_builder.with_tag(tag)
                } else {
                    address_builder
                };
                
                // Call done() to get back the ResponseBuilder
                builder = address_builder.done();
            }
            
            // Add Call-ID if provided
            if let Some(call_id) = option_expr!($($call_id)?) {
                builder = builder.call_id(call_id);
            }
            
            // Add CSeq if all required parts are provided
            if let (Some(seq), Some(method)) = (
                option_expr!($($cseq)?),
                option_expr!($($cseq_method)?)
            ) {
                builder = builder.cseq(seq, method);
            }
            
            // Add Via if all required parts are provided
            if let (Some(host), Some(transport)) = (
                option_expr!($($via_host)?), 
                option_expr!($($via_transport)?)
            ) {
                // Use the via() method but store the returned ViaBuilder
                let via_builder = builder.via(host, transport);
                
                // Add branch if provided
                let via_builder = if let Some(branch) = option_expr!($($via_branch)?) {
                    via_builder.with_branch(branch)
                } else {
                    via_builder
                };
                
                // Call done() to get back the ResponseBuilder
                builder = via_builder.done();
            }
            
            // Add Contact if provided
            if let Some(contact_uri) = option_expr!($($contact_uri)?) {
                builder = builder.contact(contact_uri)
                    .expect("Contact URI parse error");
            }
            
            // Add Contact with name if provided
            if let (Some(name), Some(uri)) = (
                option_expr!($($contact_name)?), 
                option_expr!($($contact_uri)?)
            ) {
                builder = builder.contact_with_name(name, uri)
                    .expect("Contact URI parse error");
            }
            
            // Add Content-Type if provided
            if let Some(content_type) = option_expr!($($content_type)?) {
                builder = builder.content_type(content_type)
                    .expect("Content-Type parse error");
            }
            
            // Add custom headers if provided
            $(
                $(
                    builder = match stringify!($header_name) {
                        "Server" => {
                            builder.header(TypedHeader::Other(
                                HeaderName::Server, 
                                HeaderValue::text($header_value)
                            ))
                        },
                        "Warning" => {
                            builder.header(TypedHeader::Other(
                                HeaderName::Warning, 
                                HeaderValue::text($header_value)
                            ))
                        },
                        header_name => {
                            // Capitalize first letter and handle underscores
                            let mut name = header_name.to_string();
                            if !name.is_empty() {
                                let first_char = name.remove(0).to_uppercase().to_string();
                                name = first_char + &name;
                                
                                // Replace underscores with hyphens and capitalize each word
                                let parts: Vec<&str> = name.split('_').collect();
                                if parts.len() > 1 {
                                    name = parts.iter().map(|part| {
                                        if !part.is_empty() {
                                            let mut p = part.to_string();
                                            let first = p.remove(0).to_uppercase().to_string();
                                            first + &p
                                        } else {
                                            String::new()
                                        }
                                    }).collect::<Vec<_>>().join("-");
                                }
                            }
                            
                            builder.header(TypedHeader::Other(
                                HeaderName::Other(name), 
                                HeaderValue::text($header_value)
                            ))
                        }
                    };
                )*
            )?
            
            // Add body if provided
            if let Some(body) = option_expr!($($body)?) {
                builder = builder.body(body);
            }

            builder.build()
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::{
        sip_request,
        sip_response,
        types::{
            Method, StatusCode, uri::Uri, uri::Scheme,
            TypedHeader, header::{HeaderName, HeaderValue},
            sip_request::Request, sip_response::Response,
        },
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
        let from = request.headers.iter().find(|h| h.to_string().starts_with("From:"))
            .expect("From header missing");
        let to = request.headers.iter().find(|h| h.to_string().starts_with("To:"))
            .expect("To header missing");
        let call_id = request.headers.iter().find(|h| h.to_string().starts_with("Call-ID:"))
            .expect("Call-ID header missing");
        let cseq = request.headers.iter().find(|h| h.to_string().starts_with("CSeq:"))
            .expect("CSeq header missing");
        let via = request.headers.iter().find(|h| h.to_string().starts_with("Via:"))
            .expect("Via header missing");
        
        // Verify content
        assert!(from.to_string().contains("Alice <sip:alice@example.com>"));
        assert!(from.to_string().contains("tag=1928301774"));
        assert!(to.to_string().contains("Bob <sip:bob@example.com>"));
        assert!(call_id.to_string().contains("a84b4c76e66710@pc33.atlanta.example.com"));
        assert!(cseq.to_string().contains("1 INVITE"));
        assert!(via.to_string().contains("SIP/2.0/UDP alice.example.com:5060"));
        assert!(via.to_string().contains("branch=z9hG4bK776asdhds"));
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
                CSeq: "1 INVITE",
                Via: "SIP/2.0/UDP alice.example.com:5060;branch=z9hG4bK776asdhds",
                ContentType: "application/sdp",
                ContentLength: "56"
            },
            body: sdp_body
        };

        // Check body content
        assert_eq!(String::from_utf8_lossy(&request.body), sdp_body);
        
        // Check Content-Type header
        let content_type = request.headers.iter().find(|h| h.to_string().starts_with("Content-Type:"))
            .expect("Content-Type header missing");
        assert!(content_type.to_string().contains("application/sdp"));
    }

    #[test]
    fn test_sip_request_register() {
        // Test REGISTER request
        let request = sip_request! {
            method: Method::Register,
            uri: "sip:registrar.example.com",
            headers: {
                From: "Alice <sip:alice@example.com>;tag=reg-tag",
                To: "Alice <sip:alice@example.com>",
                CallId: "register-123@example.com",
                CSeq: "1 REGISTER",
                Via: "SIP/2.0/UDP 192.168.1.2:5060;branch=z9hG4bK-reg",
                Contact: "<sip:alice@192.168.1.2:5060;transport=udp>"
            },
            max_forwards: 70
        };

        // Check method and URI
        assert_eq!(request.method, Method::Register);
        assert_eq!(request.uri.to_string(), "sip:registrar.example.com");
        
        // Check Contact header
        let contact = request.headers.iter().find(|h| h.to_string().starts_with("Contact:"))
            .expect("Contact header missing");
        assert!(contact.to_string().contains("sip:alice@192.168.1.2:5060"));
    }

    #[test]
    fn test_sip_request_with_custom_headers() {
        // Test adding custom headers
        let request = sip_request! {
            method: Method::Options,
            uri: "sip:server.example.com",
            headers: {
                From: "System <sip:system@example.com>",
                To: "Server <sip:server@example.com>",
                CallId: "options-4321@example.com",
                CSeq: "100 OPTIONS",
                Via: "SIP/2.0/TCP system.example.com:5060;branch=z9hG4bK-opts",
                Accept: "application/sdp",
                UserAgent: "Test Client/1.0"
            },
            max_forwards: 70
        };

        // Check custom headers
        let accept = request.headers.iter().find(|h| h.to_string().starts_with("Accept:"))
            .expect("Accept header missing");
        let user_agent = request.headers.iter().find(|h| h.to_string().starts_with("User-Agent:"))
            .expect("User-Agent header missing");
        
        assert!(accept.to_string().contains("application/sdp"));
        assert!(user_agent.to_string().contains("Test Client/1.0"));
    }
    
    #[test]
    fn test_sip_request_with_via_params() {
        // Test Via header with various parameters
        let request = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            headers: {
                From: "Alice <sip:alice@example.com>;tag=1928301774",
                To: "Bob <sip:bob@example.com>",
                CallId: "a84b4c76e66710@pc33.atlanta.example.com",
                CSeq: "1 INVITE",
                Via: "SIP/2.0/UDP alice.example.com:5060;branch=z9hG4bK776asdhds;received=192.168.1.1;rport=5060"
            },
            max_forwards: 70
        };

        // Check Via parameters
        let via = request.headers.iter().find(|h| h.to_string().starts_with("Via:"))
            .expect("Via header missing");
        
        assert!(via.to_string().contains("branch=z9hG4bK776asdhds"));
        assert!(via.to_string().contains("received=192.168.1.1"));
        assert!(via.to_string().contains("rport=5060"));
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
            via_branch: "z9hG4bK776asdhds"
        };

        // Check status and reason
        assert_eq!(response.status, StatusCode::Ok);
        assert_eq!(response.reason, Some("OK".to_string()));
        
        // Check From/To tags
        let from = response.headers.iter().find(|h| h.to_string().starts_with("From:"))
            .expect("From header missing");
        let to = response.headers.iter().find(|h| h.to_string().starts_with("To:"))
            .expect("To header missing");
        
        assert!(from.to_string().contains("tag=1928301774"));
        assert!(to.to_string().contains("tag=as83kd9bs"));
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
                ContentLength: "56"
            },
            body: sdp_body
        };

        // Check body
        assert_eq!(String::from_utf8_lossy(&response.body), sdp_body);
        
        // Check Content-Type
        let content_type = response.headers.iter().find(|h| h.to_string().starts_with("Content-Type:"))
            .expect("Content-Type header missing");
        assert!(content_type.to_string().contains("application/sdp"));
    }

    #[test]
    fn test_sip_response_error_codes() {
        // Test various error responses
        let response_400 = sip_response! {
            status: StatusCode::BadRequest,
            reason: "Bad Request",
            headers: {
                From: "Alice <sip:alice@example.com>;tag=1928301774",
                To: "Bob <sip:bob@example.com>",
                CallId: "a84b4c76e66710@pc33.atlanta.example.com",
                CSeq: "1 INVITE",
                Via: "SIP/2.0/UDP alice.example.com:5060;branch=z9hG4bK776asdhds"
            }
        };
        
        let response_404 = sip_response! {
            status: StatusCode::NotFound,
            reason: "Not Found",
            headers: {
                From: "Alice <sip:alice@example.com>;tag=1928301774",
                To: "Bob <sip:bob@example.com>",
                CallId: "a84b4c76e66710@pc33.atlanta.example.com",
                CSeq: "1 INVITE",
                Via: "SIP/2.0/UDP alice.example.com:5060;branch=z9hG4bK776asdhds"
            }
        };
        
        // Check status codes
        assert_eq!(response_400.status, StatusCode::BadRequest);
        assert_eq!(response_404.status, StatusCode::NotFound);
        
        // Check reason phrases
        assert_eq!(response_400.reason, Some("Bad Request".to_string()));
        assert_eq!(response_404.reason, Some("Not Found".to_string()));
    }

    #[test]
    fn test_sip_response_with_custom_headers() {
        // Test adding custom headers
        let response = sip_response! {
            status: StatusCode::Ok,
            reason: "OK",
            headers: {
                From: "Alice <sip:alice@example.com>;tag=1928301774",
                To: "Bob <sip:bob@example.com>;tag=as83kd9bs",
                CallId: "a84b4c76e66710@pc33.atlanta.example.com",
                CSeq: "1 INVITE",
                Via: "SIP/2.0/UDP alice.example.com:5060;branch=z9hG4bK776asdhds",
                Server: "Test Server/1.0",
                Warning: "399 example.com \"Incompatible parameters\""
            }
        };
        
        // Check custom headers
        let server = response.headers.iter().find(|h| h.to_string().starts_with("Server:"))
            .expect("Server header missing");
        let warning = response.headers.iter().find(|h| h.to_string().starts_with("Warning:"))
            .expect("Warning header missing");
        
        assert!(server.to_string().contains("Test Server/1.0"));
        assert!(warning.to_string().contains("399 example.com"));
    }

    #[test]
    fn test_multiple_via_headers() {
        // Test a response with multiple Via headers
        let response = sip_response! {
            status: StatusCode::Ok,
            headers: {
                From: "Alice <sip:alice@example.com>;tag=1928301774",
                To: "Bob <sip:bob@example.com>;tag=as83kd9bs",
                CallId: "a84b4c76e66710@pc33.atlanta.example.com",
                CSeq: "1 INVITE",
                Via: "SIP/2.0/UDP proxy1.example.com:5060;branch=z9hG4bK-p1",
                Via: "SIP/2.0/UDP proxy2.example.com:5060;branch=z9hG4bK-p2",
                Via: "SIP/2.0/UDP alice.example.com:5060;branch=z9hG4bK776asdhds"
            }
        };
        
        // Check for multiple Via headers
        let via_headers = response.headers.iter()
            .filter(|h| h.to_string().starts_with("Via:"))
            .collect::<Vec<_>>();
        
        // Should have 3 Via headers
        assert_eq!(via_headers.len(), 3);
        
        // First Via should be for proxy1 (top-most proxy)
        assert!(via_headers[0].to_string().contains("proxy1.example.com"));
        
        // Last Via should be for the original sender
        assert!(via_headers[2].to_string().contains("alice.example.com"));
    }

    #[test]
    fn test_flexible_param_syntax() {
        // Test different parameter syntax variations for From/To/Via
        let request1 = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            headers: {
                From: "Alice <sip:alice@example.com>;tag=1928301774",
                To: "Bob <sip:bob@example.com>",
                CallId: "abc123@example.com",
                CSeq: "1 INVITE",
                Via: "SIP/2.0/UDP example.com;branch=z9hG4bK1234;received=192.168.1.1"
            },
            max_forwards: 70
        };
        
        let request2 = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            headers: {
                From: "Alice <sip:alice@example.com>;tag=1928301774",
                To: "Bob <sip:bob@example.com>",
                CallId: "abc123@example.com",
                CSeq: "1 INVITE",
                Via: "SIP/2.0/UDP example.com;branch=z9hG4bK1234;received=192.168.1.1"
            },
            max_forwards: 70
        };
        
        let request3 = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            headers: {
                From: "Alice <sip:alice@example.com>;tag=1928301774",
                To: "Bob <sip:bob@example.com>",
                CallId: "abc123@example.com",
                CSeq: "1 INVITE",
                Via: "SIP/2.0/UDP example.com;branch=z9hG4bK1234;received=192.168.1.1"
            },
            max_forwards: 70
        };
        
        // Check the same tag values are present regardless of syntax
        assert!(request1.headers.iter().find(|h| h.to_string().starts_with("From:"))
            .unwrap().to_string().contains("tag=1928301774"));
        assert!(request2.headers.iter().find(|h| h.to_string().starts_with("From:"))
            .unwrap().to_string().contains("tag=1928301774"));
        assert!(request3.headers.iter().find(|h| h.to_string().starts_with("From:"))
            .unwrap().to_string().contains("tag=1928301774"));
    }

    #[test]
    fn test_complex_uri_params() {
        // Test URIs with parameters
        let request = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com;transport=tcp",
            headers: {
                From: "Alice <sip:alice@example.com;transport=tcp>;tag=1928301774",
                To: "Bob <sip:bob@example.com;transport=tcp>",
                CallId: "a84b4c76e66710@pc33.atlanta.example.com",
                CSeq: "1 INVITE",
                Via: "SIP/2.0/TCP alice.example.com:5060;branch=z9hG4bK776asdhds",
                Contact: "<sip:alice@alice.example.com;transport=tcp>"
            },
            max_forwards: 70
        };

        // Check params in URIs
        let uri = request.uri.to_string();
        let from = request.headers.iter().find(|h| h.to_string().starts_with("From:"))
            .expect("From header missing")
            .to_string();
        let to = request.headers.iter().find(|h| h.to_string().starts_with("To:"))
            .expect("To header missing")
            .to_string();
        let contact = request.headers.iter().find(|h| h.to_string().starts_with("Contact:"))
            .expect("Contact header missing")
            .to_string();
        
        assert!(uri.contains("transport=tcp"));
        assert!(from.contains("transport=tcp"));
        assert!(to.contains("transport=tcp"));
        assert!(contact.contains("transport=tcp"));
    }

    #[test]
    fn test_headers_in_different_order() {
        // Test that headers can be specified in any order in request macros
        // RFC 3261 does not require specific header ordering except for Via

        // Standard order
        let request1 = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            headers: {
                From: "Alice <sip:alice@example.com>;tag=1928301774",
                To: "Bob <sip:bob@example.com>",
                CallId: "a84b4c76e66710@pc33.atlanta.example.com",
                CSeq: "1 INVITE",
                Via: "SIP/2.0/UDP alice.example.com:5060;branch=z9hG4bK776asdhds",
                MaxForwards: "70",
                Contact: "<sip:alice@alice.example.com>",
                ContentType: "application/sdp"
            },
            body: "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n"
        };

        // Different order
        let request2 = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            headers: {
                To: "Bob <sip:bob@example.com>",
                From: "Alice <sip:alice@example.com>;tag=1928301774",
                Via: "SIP/2.0/UDP alice.example.com:5060;branch=z9hG4bK776asdhds",
                CallId: "a84b4c76e66710@pc33.atlanta.example.com",
                Contact: "<sip:alice@alice.example.com>",
                MaxForwards: "70",
                ContentType: "application/sdp",
                CSeq: "1 INVITE"
            },
            body: "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n"
        };
        
        // Another order with custom headers
        let request3 = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            headers: {
                CSeq: "1 INVITE",
                MaxForwards: "70",
                CallId: "a84b4c76e66710@pc33.atlanta.example.com",
                From: "Alice <sip:alice@example.com>;tag=1928301774",
                Via: "SIP/2.0/UDP alice.example.com:5060;branch=z9hG4bK776asdhds",
                To: "Bob <sip:bob@example.com>",
                Contact: "<sip:alice@alice.example.com>",
                ContentType: "application/sdp",
                Accept: "application/sdp"
            },
            body: "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n"
        };

        // Verify all requests have the same content
        let verify_request = |req: &Request| {
            assert_eq!(req.method, Method::Invite);
            assert_eq!(req.uri.to_string(), "sip:bob@example.com");
            
            // Check header presence
            let from = req.headers.iter().find(|h| h.to_string().starts_with("From:"))
                .expect("From header missing");
            let to = req.headers.iter().find(|h| h.to_string().starts_with("To:"))
                .expect("To header missing");
            let call_id = req.headers.iter().find(|h| h.to_string().starts_with("Call-ID:"))
                .expect("Call-ID header missing");
            let cseq = req.headers.iter().find(|h| h.to_string().starts_with("CSeq:"))
                .expect("CSeq header missing");
            let via = req.headers.iter().find(|h| h.to_string().starts_with("Via:"))
                .expect("Via header missing");
            
            // Check header content
            assert!(from.to_string().contains("Alice <sip:alice@example.com>"));
            assert!(from.to_string().contains("tag=1928301774"));
            assert!(to.to_string().contains("Bob <sip:bob@example.com>"));
            assert!(call_id.to_string().contains("a84b4c76e66710@pc33.atlanta.example.com"));
            assert!(cseq.to_string().contains("1 INVITE"));
            assert!(via.to_string().contains("SIP/2.0/UDP alice.example.com:5060"));
            assert!(via.to_string().contains("branch=z9hG4bK776asdhds"));
            
            // Check body
            assert_eq!(req.body.len(), 56);
        };
        
        verify_request(&request1);
        verify_request(&request2);
        verify_request(&request3);
        
        // Check custom header in request3
        let accept = request3.headers.iter().find(|h| h.to_string().starts_with("Accept:"))
            .expect("Accept header missing");
        assert!(accept.to_string().contains("application/sdp"));
        
        // Now test with responses in different order
        let response1 = sip_response! {
            status: StatusCode::Ok,
            reason: "OK",
            headers: {
                From: "Alice <sip:alice@example.com>;tag=1928301774",
                To: "Bob <sip:bob@example.com>;tag=as83kd9bs",
                CallId: "a84b4c76e66710@pc33.atlanta.example.com",
                CSeq: "1 INVITE",
                Via: "SIP/2.0/UDP alice.example.com:5060;branch=z9hG4bK776asdhds",
                Contact: "<sip:bob@192.168.1.2>"
            }
        };
        
        let response2 = sip_response! {
            status: StatusCode::Ok,
            reason: "OK",
            headers: {
                Via: "SIP/2.0/UDP alice.example.com:5060;branch=z9hG4bK776asdhds",
                CallId: "a84b4c76e66710@pc33.atlanta.example.com",
                From: "Alice <sip:alice@example.com>;tag=1928301774",
                To: "Bob <sip:bob@example.com>;tag=as83kd9bs",
                CSeq: "1 INVITE",
                Contact: "<sip:bob@192.168.1.2>"
            }
        };
        
        let response3 = sip_response! {
            status: StatusCode::Ok,
            reason: "OK",
            headers: {
                Via: "SIP/2.0/UDP alice.example.com:5060;branch=z9hG4bK776asdhds",
                To: "Bob <sip:bob@example.com>;tag=as83kd9bs",
                From: "Alice <sip:alice@example.com>;tag=1928301774",
                CallId: "a84b4c76e66710@pc33.atlanta.example.com",
                CSeq: "1 INVITE",
                Contact: "<sip:bob@192.168.1.2>",
                Server: "Test Server/1.0"
            }
        };
        
        // Verify all responses have the same content
        let verify_response = |resp: &Response| {
            assert_eq!(resp.status, StatusCode::Ok);
            assert_eq!(resp.reason, Some("OK".to_string()));
            
            // Check header presence
            let from = resp.headers.iter().find(|h| h.to_string().starts_with("From:"))
                .expect("From header missing");
            let to = resp.headers.iter().find(|h| h.to_string().starts_with("To:"))
                .expect("To header missing");
            let call_id = resp.headers.iter().find(|h| h.to_string().starts_with("Call-ID:"))
                .expect("Call-ID header missing");
            let cseq = resp.headers.iter().find(|h| h.to_string().starts_with("CSeq:"))
                .expect("CSeq header missing");
            let via = resp.headers.iter().find(|h| h.to_string().starts_with("Via:"))
                .expect("Via header missing");
            
            // Check header content
            assert!(from.to_string().contains("Alice <sip:alice@example.com>"));
            assert!(from.to_string().contains("tag=1928301774"));
            assert!(to.to_string().contains("Bob <sip:bob@example.com>"));
            assert!(to.to_string().contains("tag=as83kd9bs"));
            assert!(call_id.to_string().contains("a84b4c76e66710@pc33.atlanta.example.com"));
            assert!(cseq.to_string().contains("1 INVITE"));
            assert!(via.to_string().contains("SIP/2.0/UDP alice.example.com:5060"));
            assert!(via.to_string().contains("branch=z9hG4bK776asdhds"));
        };
        
        verify_response(&response1);
        verify_response(&response2);
        verify_response(&response3);
        
        // Check custom header in response3
        let server = response3.headers.iter().find(|h| h.to_string().starts_with("Server:"))
            .expect("Server header missing");
        assert!(server.to_string().contains("Test Server/1.0"));
    }

    // Helper function to extract parameter value from a header - now returns String to avoid lifetime issues
    fn find_header_value(headers: &[TypedHeader], header_name: &str, param_name: &str) -> Option<String> {
        for header in headers {
            if header.to_string().starts_with(&format!("{}:", header_name)) {
                let header_str = header.to_string();
                
                // Find parameter in header string - match both formats with and without spaces
                let param_pattern = format!(";{}=", param_name);
                if let Some(param_start) = header_str.find(&param_pattern) {
                    let param_value_start = param_start + param_name.len() + 2; // +2 for ;=
                    
                    // Find end of parameter value
                    let param_value_end = header_str[param_value_start..]
                        .find(|c: char| c == ';' || c == '>' || c == ' ' || c == '\r' || c == '\n')
                        .map(|pos| param_value_start + pos)
                        .unwrap_or(header_str.len());
                    
                    return Some(header_str[param_value_start..param_value_end].to_string());
                }
            }
        }
        None
    }
} 
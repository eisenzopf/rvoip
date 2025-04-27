/// Macro for creating SIP request messages with a more concise syntax.
///
/// # Examples
///
/// ```
/// # use rvoip_sip_core::sip_request;
/// # use rvoip_sip_core::types::Method;
/// let request = sip_request! {
///     method: Method::Invite,
///     uri: "sip:bob@example.com",
///     from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
///     to: ("Bob", "sip:bob@example.com"),
///     call_id: "a84b4c76e66710@pc33.atlanta.example.com",
///     cseq: 1,
///     via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds"),
///     contact: "sip:alice@alice.example.com",
///     max_forwards: 70,
///     content_type: "application/sdp",
///     body: "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n"
/// };
/// ```
#[macro_export]
macro_rules! sip_request {
    // Main macro pattern using token tree matching for parameters
    (
        method: $method:expr,
        uri: $uri:expr
        $(, from: ($from_name:expr, $from_uri:expr $(, $from_param_key:tt = $from_param_val:expr)*) )?
        $(, to: ($to_name:expr, $to_uri:expr $(, $to_param_key:tt = $to_param_val:expr)*) )?
        $(, call_id: $call_id:expr )?
        $(, cseq: $cseq:expr )?
        $(, via: ($via_host:expr, $via_transport:expr $(, $via_param_key:tt = $via_param_val:expr)*) )?
        $(, contact: $contact_uri:expr )?
        $(, contact_name: ($contact_name:expr, $contact_name_uri:expr) )?
        $(, max_forwards: $max_forwards:expr )?
        $(, content_type: $content_type:expr )?
        $(, body: $body:expr )?
    ) => {
        {
            use $crate::types::builder::RequestBuilder;
            let mut builder = RequestBuilder::new($method, $uri)
                .expect("URI parse error");

            $(
                let mut from_builder = builder.from($from_name, $from_uri);
                $(
                    match stringify!($from_param_key) {
                        "tag" => { from_builder = from_builder.with_tag($from_param_val); },
                        _ => { from_builder = from_builder.with_param(stringify!($from_param_key), Some($from_param_val)); }
                    }
                )*
                builder = from_builder.done();
            )?

            $(
                let mut to_builder = builder.to($to_name, $to_uri);
                $(
                    match stringify!($to_param_key) {
                        "tag" => { to_builder = to_builder.with_tag($to_param_val); },
                        _ => { to_builder = to_builder.with_param(stringify!($to_param_key), Some($to_param_val)); }
                    }
                )*
                builder = to_builder.done();
            )?

            $(
                builder = builder.call_id($call_id);
            )?

            $(
                builder = builder.cseq($cseq);
            )?

            $(
                let mut via_builder = builder.via($via_host, $via_transport);
                $(
                    match stringify!($via_param_key) {
                        "branch" => { via_builder = via_builder.with_branch($via_param_val); },
                        "received" => { 
                            // Parse IP address if possible, otherwise use generic param
                            if let Ok(ip) = $via_param_val.parse::<std::net::IpAddr>() {
                                via_builder = via_builder.with_received(ip);
                            } else {
                                via_builder = via_builder.with_param("received", Some($via_param_val));
                            }
                        },
                        "ttl" => { 
                            if let Ok(ttl) = $via_param_val.parse::<u8>() {
                                via_builder = via_builder.with_ttl(ttl);
                            } else {
                                via_builder = via_builder.with_param("ttl", Some($via_param_val));
                            }
                        },
                        "maddr" => { via_builder = via_builder.with_maddr($via_param_val); },
                        "rport" => {
                            if $via_param_val == "" || $via_param_val == "true" {
                                via_builder = via_builder.with_rport();
                            } else if let Ok(port) = $via_param_val.parse::<u16>() {
                                via_builder = via_builder.with_rport_value(port);
                            } else {
                                via_builder = via_builder.with_param("rport", Some($via_param_val));
                            }
                        },
                        _ => { via_builder = via_builder.with_param(stringify!($via_param_key), Some($via_param_val)); }
                    }
                )*
                builder = via_builder.done();
            )?

            $(
                builder = builder.contact($contact_uri)
                    .expect("Contact URI parse error");
            )?

            $(
                builder = builder.contact_with_name($contact_name, $contact_name_uri)
                    .expect("Contact URI parse error");
            )?

            $(
                builder = builder.max_forwards($max_forwards);
            )?

            $(
                builder = builder.content_type($content_type)
                    .expect("Content-Type parse error");
            )?

            $(
                builder = builder.body($body);
            )?
            
            builder.build()
        }
    };

    // Alternative pattern for "spaced" format - correctly handle same format as main pattern
    (
        method: $method:expr,
        uri: $uri:expr
        $(, from: ($from_name:expr, $from_uri:expr $(, $from_param_key:tt = $from_param_val:expr)*) )?
        $(, to: ($to_name:expr, $to_uri:expr $(, $to_param_key:tt = $to_param_val:expr)*) )?
        $(, call_id: $call_id:expr )?
        $(, cseq: $cseq:expr )?
        $(, via: ($via_host:expr, $via_transport:expr $(, $via_param_key:tt = $via_param_val:expr)*) )?
        $(, contact: $contact_uri:expr )?
        $(, contact_name: ($contact_name:expr, $contact_name_uri:expr) )?
        $(, max_forwards: $max_forwards:expr )?
        $(, content_type: $content_type:expr )?
        $(, body: $body:expr )?
    ) => {
        $crate::sip_request! {
            method: $method,
            uri: $uri
            $(, from: ($from_name, $from_uri $(, $from_param_key = $from_param_val)*) )?
            $(, to: ($to_name, $to_uri $(, $to_param_key = $to_param_val)*) )?
            $(, call_id: $call_id )?
            $(, cseq: $cseq )?
            $(, via: ($via_host, $via_transport $(, $via_param_key = $via_param_val)*) )?
            $(, contact: $contact_uri )?
            $(, contact_name: ($contact_name, $contact_name_uri) )?
            $(, max_forwards: $max_forwards )?
            $(, content_type: $content_type )?
            $(, body: $body )?
        }
    };
    
    // Handle custom headers with a dedicated "headers" field
    (
        method: $method:expr,
        uri: $uri:expr
        $(, from: ($from_name:expr, $from_uri:expr $(, $from_param_key:tt = $from_param_val:expr)*) )?
        $(, to: ($to_name:expr, $to_uri:expr $(, $to_param_key:tt = $to_param_val:expr)*) )?
        $(, call_id: $call_id:expr )?
        $(, cseq: $cseq:expr )?
        $(, via: ($via_host:expr, $via_transport:expr $(, $via_param_key:tt = $via_param_val:expr)*) )?
        $(, contact: $contact_uri:expr )?
        $(, contact_name: ($contact_name:expr, $contact_name_uri:expr) )?
        $(, max_forwards: $max_forwards:expr )?
        $(, content_type: $content_type:expr )?
        $(, body: $body:expr )?
        , headers: { $( $custom_header:ident: $custom_value:expr ),* }
    ) => {
        {
            use $crate::types::header::{HeaderName, HeaderValue};
            use $crate::types::TypedHeader;
            
            let mut request = $crate::sip_request! {
                method: $method,
                uri: $uri
                $(, from: ($from_name, $from_uri $(, $from_param_key = $from_param_val)*) )?
                $(, to: ($to_name, $to_uri $(, $to_param_key = $to_param_val)*) )?
                $(, call_id: $call_id )?
                $(, cseq: $cseq )?
                $(, via: ($via_host, $via_transport $(, $via_param_key = $via_param_val)*) )?
                $(, contact: $contact_uri )?
                $(, contact_name: ($contact_name, $contact_name_uri) )?
                $(, max_forwards: $max_forwards )?
                $(, content_type: $content_type )?
                $(, body: $body )?
            };
            
            // Add the custom headers
            $(
                request.headers.push(TypedHeader::Other(
                    HeaderName::Other(stringify!($custom_header).to_string()),
                    HeaderValue::text($custom_value.to_string())
                ));
            )*
            
            request
        }
    };
}

/// Macro for creating SIP response messages with a more concise syntax.
///
/// # Examples
///
/// ```
/// # use rvoip_sip_core::sip_response;
/// # use rvoip_sip_core::types::{Method, StatusCode};
/// let response = sip_response! {
///     status: StatusCode::Ok,
///     reason: "OK",
///     from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
///     to: ("Bob", "sip:bob@example.com", tag = "as83kd9bs"),
///     call_id: "a84b4c76e66710@pc33.atlanta.example.com",
///     cseq: (1, Method::Invite),
///     via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds"),
///     contact: "sip:bob@192.168.1.2",
///     content_type: "application/sdp",
///     body: "v=0\r\no=bob 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n"
/// };
/// ```
#[macro_export]
macro_rules! sip_response {
    (
        status: $status:expr
        $(, reason: $reason:expr )?
        $(, from: ($from_name:expr, $from_uri:expr $(, $from_param_key:tt = $from_param_val:expr)*) )?
        $(, to: ($to_name:expr, $to_uri:expr $(, $to_param_key:tt = $to_param_val:expr)*) )?
        $(, call_id: $call_id:expr )?
        $(, cseq: ($cseq:expr, $cseq_method:expr) )?
        $(, via: ($via_host:expr, $via_transport:expr $(, $via_param_key:tt = $via_param_val:expr)*) )?
        $(, contact: $contact_uri:expr )?
        $(, content_type: $content_type:expr )?
        $(, body: $body:expr )?
    ) => {
        {
            use $crate::types::builder::ResponseBuilder;
            let mut builder = ResponseBuilder::new($status);

            $(
                builder = builder.reason($reason);
            )?

            $(
                let mut from_builder = builder.from($from_name, $from_uri);
                $(
                    match stringify!($from_param_key) {
                        "tag" => { from_builder = from_builder.with_tag($from_param_val); },
                        _ => { from_builder = from_builder.with_param(stringify!($from_param_key), Some($from_param_val)); }
                    }
                )*
                builder = from_builder.done();
            )?

            $(
                let mut to_builder = builder.to($to_name, $to_uri);
                $(
                    match stringify!($to_param_key) {
                        "tag" => { to_builder = to_builder.with_tag($to_param_val); },
                        _ => { to_builder = to_builder.with_param(stringify!($to_param_key), Some($to_param_val)); }
                    }
                )*
                builder = to_builder.done();
            )?

            $(
                builder = builder.call_id($call_id);
            )?

            $(
                builder = builder.cseq($cseq, $cseq_method);
            )?

            $(
                let mut via_builder = builder.via($via_host, $via_transport);
                $(
                    match stringify!($via_param_key) {
                        "branch" => { via_builder = via_builder.with_branch($via_param_val); },
                        "received" => { 
                            // Parse IP address if possible, otherwise use generic param
                            if let Ok(ip) = $via_param_val.parse::<std::net::IpAddr>() {
                                via_builder = via_builder.with_received(ip);
                            } else {
                                via_builder = via_builder.with_param("received", Some($via_param_val));
                            }
                        },
                        "ttl" => { 
                            if let Ok(ttl) = $via_param_val.parse::<u8>() {
                                via_builder = via_builder.with_ttl(ttl);
                            } else {
                                via_builder = via_builder.with_param("ttl", Some($via_param_val));
                            }
                        },
                        "maddr" => { via_builder = via_builder.with_maddr($via_param_val); },
                        "rport" => {
                            if $via_param_val == "" || $via_param_val == "true" {
                                via_builder = via_builder.with_rport();
                            } else if let Ok(port) = $via_param_val.parse::<u16>() {
                                via_builder = via_builder.with_rport_value(port);
                            } else {
                                via_builder = via_builder.with_param("rport", Some($via_param_val));
                            }
                        },
                        _ => { via_builder = via_builder.with_param(stringify!($via_param_key), Some($via_param_val)); }
                    }
                )*
                builder = via_builder.done();
            )?

            $(
                builder = builder.contact($contact_uri)
                    .expect("Contact URI parse error");
            )?

            $(
                builder = builder.content_type($content_type)
                    .expect("Content-Type parse error");
            )?

            $(
                builder = builder.body($body);
            )?
            
            builder.build()
        }
    };

    // Alternative pattern for "spaced" format - correctly handle same format as main pattern
    (
        status: $status:expr
        $(, reason: $reason:expr )?
        $(, from: ($from_name:expr, $from_uri:expr $(, $from_param_key:tt = $from_param_val:expr)*) )?
        $(, to: ($to_name:expr, $to_uri:expr $(, $to_param_key:tt = $to_param_val:expr)*) )?
        $(, call_id: $call_id:expr )?
        $(, cseq: ($cseq:expr, $cseq_method:expr) )?
        $(, via: ($via_host:expr, $via_transport:expr $(, $via_param_key:tt = $via_param_val:expr)*) )?
        $(, contact: $contact_uri:expr )?
        $(, content_type: $content_type:expr )?
        $(, body: $body:expr )?
    ) => {
        $crate::sip_response! {
            status: $status
            $(, reason: $reason )?
            $(, from: ($from_name, $from_uri $(, $from_param_key = $from_param_val)*) )?
            $(, to: ($to_name, $to_uri $(, $to_param_key = $to_param_val)*) )?
            $(, call_id: $call_id )?
            $(, cseq: ($cseq, $cseq_method) )?
            $(, via: ($via_host, $via_transport $(, $via_param_key = $via_param_val)*) )?
            $(, contact: $contact_uri )?
            $(, content_type: $content_type )?
            $(, body: $body )?
        }
    };
    
    // Handle custom headers with a dedicated "headers" field
    (
        status: $status:expr
        $(, reason: $reason:expr )?
        $(, from: ($from_name:expr, $from_uri:expr $(, $from_param_key:tt = $from_param_val:expr)*) )?
        $(, to: ($to_name:expr, $to_uri:expr $(, $to_param_key:tt = $to_param_val:expr)*) )?
        $(, call_id: $call_id:expr )?
        $(, cseq: ($cseq:expr, $cseq_method:expr) )?
        $(, via: ($via_host:expr, $via_transport:expr $(, $via_param_key:tt = $via_param_val:expr)*) )?
        $(, contact: $contact_uri:expr )?
        $(, content_type: $content_type:expr )?
        $(, body: $body:expr )?
        , headers: { $( $custom_header:ident: $custom_value:expr ),* }
    ) => {
        {
            use $crate::types::header::{HeaderName, HeaderValue};
            use $crate::types::TypedHeader;
            
            let mut response = $crate::sip_response! {
                status: $status
                $(, reason: $reason )?
                $(, from: ($from_name, $from_uri $(, $from_param_key = $from_param_val)*) )?
                $(, to: ($to_name, $to_uri $(, $to_param_key = $to_param_val)*) )?
                $(, call_id: $call_id )?
                $(, cseq: ($cseq, $cseq_method) )?
                $(, via: ($via_host, $via_transport $(, $via_param_key = $via_param_val)*) )?
                $(, contact: $contact_uri )?
                $(, content_type: $content_type )?
                $(, body: $body )?
            };
            
            // Add the custom headers
            $(
                response.headers.push(TypedHeader::Other(
                    HeaderName::Other(stringify!($custom_header).to_string()),
                    HeaderValue::text($custom_value.to_string())
                ));
            )*
            
            response
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
        },
    };

    #[test]
    fn test_sip_request_basic() {
        // Test a basic INVITE request
        let request = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
            to: ("Bob", "sip:bob@example.com"),
            call_id: "a84b4c76e66710@pc33.atlanta.example.com",
            cseq: 1,
            via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds"),
            contact: "sip:alice@alice.example.com",
            max_forwards: 70
        };

        // Check that fields were properly set
        assert_eq!(request.method, Method::Invite);
        assert_eq!(request.uri.to_string(), "sip:bob@example.com");
        
        // Check headers
        let from = request.headers.iter().find(|h| h.to_string().starts_with("From:"))
            .expect("From header missing");
        let to = request.headers.iter().find(|h| h.to_string().starts_with("To:"))
            .expect("To header missing");
        let call_id = request.headers.iter().find(|h| h.to_string().starts_with("Call-ID:"))
            .expect("Call-ID header missing");
        
        assert!(from.to_string().contains("Alice"));
        assert!(from.to_string().contains("tag=1928301774"));
        assert!(to.to_string().contains("Bob"));
        assert!(call_id.to_string().contains("a84b4c76e66710@pc33.atlanta.example.com"));
    }

    #[test]
    fn test_sip_request_with_body() {
        // Test an INVITE with SDP body
        let sdp_body = "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n";
        
        let request = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
            to: ("Bob", "sip:bob@example.com"),
            call_id: "a84b4c76e66710@pc33.atlanta.example.com",
            cseq: 1,
            via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds"),
            content_type: "application/sdp",
            body: sdp_body
        };

        // Check body and content-type
        assert_eq!(String::from_utf8_lossy(&request.body), sdp_body);
        
        let content_type = request.headers.iter().find(|h| h.to_string().starts_with("Content-Type:"))
            .expect("Content-Type header missing");
        assert!(content_type.to_string().contains("application/sdp"));
    }

    #[test]
    fn test_sip_request_register() {
        // Test a REGISTER request
        let request = sip_request! {
            method: Method::Register,
            uri: "sip:registrar.example.com",
            from: ("Alice", "sip:alice@example.com", tag = "reg-tag"),
            to: ("Alice", "sip:alice@example.com"),
            call_id: "register-1234@example.com",
            cseq: 1,
            via: ("192.168.1.2:5060", "UDP", branch = "z9hG4bK-reg"),
            contact: "sip:alice@192.168.1.2:5060",
            max_forwards: 70
        };

        // Check method and URI
        assert_eq!(request.method, Method::Register);
        assert_eq!(request.uri.to_string(), "sip:registrar.example.com");
        
        // Check From/To has same value but From has tag
        let from = request.headers.iter().find(|h| h.to_string().starts_with("From:"))
            .expect("From header missing");
        let to = request.headers.iter().find(|h| h.to_string().starts_with("To:"))
            .expect("To header missing");
        
        assert!(from.to_string().contains("Alice"));
        assert!(from.to_string().contains("tag=reg-tag"));
        assert!(to.to_string().contains("Alice"));
        assert!(!to.to_string().contains("tag="));
    }

    #[test]
    fn test_sip_request_with_custom_headers() {
        // Test adding custom headers
        let request = sip_request! {
            method: Method::Options,
            uri: "*",
            from: ("System", "sip:system@example.com"),
            to: ("Server", "sip:server@example.com"),
            call_id: "options-4321@example.com",
            cseq: 100,
            via: ("system.example.com:5060", "TCP", branch="z9hG4bK-opts"),
            max_forwards: 70,
            headers: { 
                accept: "application/sdp",
                user_agent: "Test Client/1.0"
            }
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
            from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
            to: ("Bob", "sip:bob@example.com"),
            call_id: "a84b4c76e66710@pc33.atlanta.example.com",
            cseq: 1,
            via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds", received = "192.168.1.1", rport = "5060"),
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
            from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
            to: ("Bob", "sip:bob@example.com", tag = "as83kd9bs"),
            call_id: "a84b4c76e66710@pc33.atlanta.example.com",
            cseq: (1, Method::Invite),
            via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds")
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
            from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
            to: ("Bob", "sip:bob@example.com", tag = "as83kd9bs"),
            call_id: "a84b4c76e66710@pc33.atlanta.example.com",
            cseq: (1, Method::Invite),
            via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds"),
            content_type: "application/sdp",
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
        // Test 4xx response
        let error_response = sip_response! {
            status: StatusCode::BadRequest,
            reason: "Bad Request",
            from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
            to: ("Bob", "sip:bob@example.com"),
            call_id: "error-123@example.com",
            cseq: (42, Method::Message),
            via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds")
        };

        // Check status
        assert_eq!(error_response.status, StatusCode::BadRequest);
        assert_eq!(error_response.reason, Some("Bad Request".to_string()));
        
        // Check CSeq
        let cseq = error_response.headers.iter().find(|h| h.to_string().starts_with("CSeq:"))
            .expect("CSeq header missing");
        assert!(cseq.to_string().contains("42 MESSAGE"));
    }

    #[test]
    fn test_sip_response_with_custom_headers() {
        // Test adding custom headers to response
        let response = sip_response! {
            status: StatusCode::Ok,
            reason: "OK",
            from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
            to: ("Bob", "sip:bob@example.com", tag = "as83kd9bs"),
            call_id: "a84b4c76e66710@pc33.atlanta.example.com",
            cseq: (1, Method::Invite),
            via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds"),
            headers: {
                server: "Test Server/1.0",
                warning: "399 example.com \"Miscellaneous warning\""
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
        // Test multiple Via headers using header() method
        let base_request = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
            to: ("Bob", "sip:bob@example.com"),
            call_id: "a84b4c76e66710@pc33.atlanta.example.com",
            cseq: 1,
            via: ("proxy1.example.com:5060", "UDP", branch = "z9hG4bK-p1"),
            max_forwards: 70
        };
        
        // Add a second Via header
        let mut request = base_request;
        request.headers.push(TypedHeader::Other(
            HeaderName::Via, 
            HeaderValue::text("SIP/2.0/UDP proxy2.example.com:5060;branch=z9hG4bK-p2")
        ));
        
        // Check that we have two Via headers
        let via_headers: Vec<_> = request.headers.iter()
            .filter(|h| h.to_string().starts_with("Via:"))
            .collect();
        
        assert_eq!(via_headers.len(), 2);
        assert!(via_headers[0].to_string().contains("proxy1.example.com"));
        assert!(via_headers[1].to_string().contains("proxy2.example.com"));
    }

    #[test]
    fn test_complex_uri_params() {
        // Test URI with parameters in the from/to/contact fields
        let request = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com;transport=tcp",
            from: ("Alice", "sip:alice@example.com;transport=tcp", tag = "1928301774"),
            to: ("Bob", "sip:bob@example.com;transport=tcp"),
            call_id: "a84b4c76e66710@pc33.atlanta.example.com",
            cseq: 1,
            via: ("alice.example.com:5060", "TCP", branch = "z9hG4bK776asdhds"),
            contact: "sip:alice@alice.example.com;transport=tcp",
            max_forwards: 70
        };
        
        // Check that parameters were included in the URI
        assert!(request.uri.to_string().contains("transport=tcp"));
        
        // Check that parameters were included in the headers
        let from = request.headers.iter().find(|h| h.to_string().starts_with("From:"))
            .expect("From header missing");
        let to = request.headers.iter().find(|h| h.to_string().starts_with("To:"))
            .expect("To header missing");
        let contact = request.headers.iter().find(|h| h.to_string().starts_with("Contact:"))
            .expect("Contact header missing");
        
        assert!(from.to_string().contains("transport=tcp"));
        assert!(to.to_string().contains("transport=tcp"));
        assert!(contact.to_string().contains("transport=tcp"));
    }

    #[test]
    fn test_flexible_param_syntax() {
        // Test with no spaces around equals
        let request1 = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            from: ("Alice", "sip:alice@example.com", tag="1928301774"),
            to: ("Bob", "sip:bob@example.com"),
            via: ("example.com", "UDP", branch="z9hG4bK1234", received="192.168.1.1"),
            call_id: "abc123@example.com",
            cseq: 1
        };
        
        // Test with spaces around equals
        let request2 = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
            to: ("Bob", "sip:bob@example.com"),
            via: ("example.com", "UDP", branch = "z9hG4bK1234", received = "192.168.1.1"),
            call_id: "abc123@example.com",
            cseq: 1
        };
        
        // Test mixed styles
        let request3 = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            from: ("Alice", "sip:alice@example.com", tag="1928301774"),
            to: ("Bob", "sip:bob@example.com"),
            via: ("example.com", "UDP", branch = "z9hG4bK1234", received="192.168.1.1"),
            call_id: "abc123@example.com",
            cseq: 1
        };
        
        // Verify all requests are equivalent
        let tag1 = find_header_value(&request1.headers, "From", "tag");
        let tag2 = find_header_value(&request2.headers, "From", "tag");
        let tag3 = find_header_value(&request3.headers, "From", "tag");
        
        assert_eq!(tag1, Some("1928301774".to_string()));
        assert_eq!(tag2, Some("1928301774".to_string()));
        assert_eq!(tag3, Some("1928301774".to_string()));
        
        let branch1 = find_header_value(&request1.headers, "Via", "branch");
        let branch2 = find_header_value(&request2.headers, "Via", "branch");
        let branch3 = find_header_value(&request3.headers, "Via", "branch");
        
        assert_eq!(branch1, Some("z9hG4bK1234".to_string()));
        assert_eq!(branch2, Some("z9hG4bK1234".to_string()));
        assert_eq!(branch3, Some("z9hG4bK1234".to_string()));

        // Test response macro with both styles
        let response1 = sip_response! {
            status: StatusCode::Ok,
            reason: "OK",
            from: ("Alice", "sip:alice@example.com", tag="1928301774"),
            to: ("Bob", "sip:bob@example.com", tag="as83kd9bs"),
            call_id: "abc123@example.com",
            cseq: (1, Method::Invite),
            via: ("example.com", "UDP", branch="z9hG4bK1234")
        };
        
        let response2 = sip_response! {
            status: StatusCode::Ok,
            reason: "OK",
            from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
            to: ("Bob", "sip:bob@example.com", tag = "as83kd9bs"),
            call_id: "abc123@example.com",
            cseq: (1, Method::Invite),
            via: ("example.com", "UDP", branch = "z9hG4bK1234")
        };
        
        // Verify responses are equivalent
        let from_tag1 = find_header_value(&response1.headers, "From", "tag");
        let from_tag2 = find_header_value(&response2.headers, "From", "tag");
        
        assert_eq!(from_tag1, Some("1928301774".to_string()));
        assert_eq!(from_tag2, Some("1928301774".to_string()));
        
        let to_tag1 = find_header_value(&response1.headers, "To", "tag");
        let to_tag2 = find_header_value(&response2.headers, "To", "tag");
        
        assert_eq!(to_tag1, Some("as83kd9bs".to_string()));
        assert_eq!(to_tag2, Some("as83kd9bs".to_string()));
    }

    // Helper function to extract parameter value from a header
    fn find_header_value(headers: &[TypedHeader], header_name: &str, param_name: &str) -> Option<String> {
        for header in headers {
            if header.to_string().starts_with(&format!("{}:", header_name)) {
                let header_str = header.to_string();
                
                // Find parameter in header string
                if let Some(param_start) = header_str.find(&format!(";{}=", param_name)) {
                    let param_value_start = param_start + param_name.len() + 2; // +2 for ;=
                    
                    // Find end of parameter value
                    let param_value_end = header_str[param_value_start..]
                        .find(|c| c == ';' || c == '>' || c == ' ')
                        .map(|pos| param_value_start + pos)
                        .unwrap_or(header_str.len());
                    
                    // Return a cloned substring to avoid lifetime issues
                    return Some(header_str[param_value_start..param_value_end].to_string());
                }
            }
        }
        None
    }
} 
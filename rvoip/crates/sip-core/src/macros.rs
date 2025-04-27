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
///
/// The order of headers doesn't matter (except that method and uri must come first), complying with RFC 3261.
#[macro_export]
macro_rules! sip_request {
    (
        method: $method:expr,
        uri: $uri:expr
        $(, $key:ident : $value:tt)*
    ) => {
        {
            use $crate::types::builder::RequestBuilder;
            use $crate::types::header::{HeaderName, HeaderValue};
            use $crate::types::TypedHeader;
            use $crate::types::uri::{Uri, Host, Scheme};
            use $crate::types::Method;
            use $crate::types::sip_message::Request;
            use $crate::types::Version;
            
            let builder = if $method == Method::Options && $uri == "*" {
                let mut request = Request::new(Method::Options, Uri::sip("example.com"));
                request.uri.raw_uri = Some("*".to_string());
                RequestBuilder::from_request(request)
            } else {
                RequestBuilder::new($method, $uri).expect("Failed to create RequestBuilder")
            };
            
            let mut builder = builder;

            $(
                builder = $crate::sip_request!(@process_field builder, $key, $value);
            )*

            builder.build()
        }
    };

    // Process individual header fields based on key
    (@process_field $builder:ident, from, $from:tt) => {{
        $crate::sip_request!(@process_from $builder, $from)
    }};
    
    (@process_from $builder:ident, ($name:expr, $uri:expr)) => {{
        $builder.from($name, $uri).done()
    }};
    
    (@process_from $builder:ident, ($name:expr, $uri:expr, tag = $tag:expr)) => {{
        $builder.from($name, $uri).with_tag($tag).done()
    }};
    
    (@process_from $builder:ident, ($name:expr, $uri:expr, tag = $tag:expr, $($rest:tt)*)) => {{
        let mut builder = $builder.from($name, $uri).with_tag($tag);
        $crate::sip_request!(@process_params builder, $($rest)*);
        builder.done()
    }};
    
    (@process_from $builder:ident, ($name:expr, $uri:expr, $param_key:ident = $param_val:expr $(, $rest:tt)*)) => {{
        let mut builder = $builder.from($name, $uri);
        match stringify!($param_key) {
            "tag" => { builder = builder.with_tag($param_val); },
            _ => { builder = builder.with_param(stringify!($param_key), Some($param_val)); }
        }
        $(
            $crate::sip_request!(@process_params builder, $rest);
        )*
        builder.done()
    }};
    
    (@process_params $builder:ident, $key:ident = $val:expr $(, $rest:tt)*) => {{
        match stringify!($key) {
            "tag" => { $builder = $builder.with_tag($val); },
            _ => { $builder = $builder.with_param(stringify!($key), Some($val)); }
        }
        $(
            $crate::sip_request!(@process_params $builder, $rest);
        )*
    }};
    
    (@process_field $builder:ident, to, $to:tt) => {{
        $crate::sip_request!(@process_to $builder, $to)
    }};
    
    (@process_to $builder:ident, ($name:expr, $uri:expr)) => {{
        $builder.to($name, $uri).done()
    }};
    
    (@process_to $builder:ident, ($name:expr, $uri:expr, tag = $tag:expr)) => {{
        $builder.to($name, $uri).with_tag($tag).done()
    }};
    
    (@process_to $builder:ident, ($name:expr, $uri:expr, tag = $tag:expr, $($rest:tt)*)) => {{
        let mut builder = $builder.to($name, $uri).with_tag($tag);
        $crate::sip_request!(@process_params builder, $($rest)*);
        builder.done()
    }};
    
    (@process_to $builder:ident, ($name:expr, $uri:expr, $param_key:ident = $param_val:expr $(, $rest:tt)*)) => {{
        let mut builder = $builder.to($name, $uri);
        match stringify!($param_key) {
            "tag" => { builder = builder.with_tag($param_val); },
            _ => { builder = builder.with_param(stringify!($param_key), Some($param_val)); }
        }
        $(
            $crate::sip_request!(@process_params builder, $rest);
        )*
        builder.done()
    }};
    
    (@process_field $builder:ident, call_id, $call_id:expr) => {
        $builder.call_id($call_id)
    };
    
    (@process_field $builder:ident, cseq, $cseq:expr) => {
        $builder.cseq($cseq)
    };
    
    (@process_field $builder:ident, via, $via:tt) => {{
        $crate::sip_request!(@process_via $builder, $via)
    }};
    
    (@process_via $builder:ident, ($host:expr, $transport:expr)) => {{
        $builder.via($host, $transport).done()
    }};
    
    (@process_via $builder:ident, ($host:expr, $transport:expr, branch = $branch:expr)) => {{
        $builder.via($host, $transport).with_branch($branch).done()
    }};
    
    (@process_via $builder:ident, ($host:expr, $transport:expr, branch = $branch:expr, received = $received:expr, $($rest:tt)*)) => {{
        let mut builder = $builder.via($host, $transport).with_branch($branch);
        
        // Parse IP address if possible, otherwise use generic param
        if let Ok(ip) = $received.parse::<std::net::IpAddr>() {
            builder = builder.with_received(ip);
        } else {
            builder = builder.with_param("received", Some($received));
        }
        
        $crate::sip_request!(@process_via_params builder, $($rest)*);
        builder.done()
    }};
    
    (@process_via $builder:ident, ($host:expr, $transport:expr, branch = $branch:expr, $($rest:tt)*)) => {{
        let mut builder = $builder.via($host, $transport).with_branch($branch);
        $crate::sip_request!(@process_via_params builder, $($rest)*);
        builder.done()
    }};
    
    (@process_via $builder:ident, ($host:expr, $transport:expr, $param_key:ident = $param_val:expr $(, $rest:tt)*)) => {{
        let mut builder = $builder.via($host, $transport);
        
        $crate::sip_request!(@process_via_param builder, $param_key, $param_val);
        $(
            $crate::sip_request!(@process_via_params builder, $rest);
        )*
        
        builder.done()
    }};
    
    (@process_via_params $builder:ident, $key:ident = $val:expr $(, $rest:tt)*) => {{
        $crate::sip_request!(@process_via_param $builder, $key, $val);
        $(
            $crate::sip_request!(@process_via_params $builder, $rest);
        )*
    }};
    
    (@process_via_param $builder:ident, branch, $val:expr) => {{
        $builder = $builder.with_branch($val);
    }};
    
    (@process_via_param $builder:ident, received, $val:expr) => {{
        if let Ok(ip) = $val.parse::<std::net::IpAddr>() {
            $builder = $builder.with_received(ip);
        } else {
            $builder = $builder.with_param("received", Some($val));
        }
    }};
    
    (@process_via_param $builder:ident, ttl, $val:expr) => {{
        if let Ok(ttl) = $val.parse::<u8>() {
            $builder = $builder.with_ttl(ttl);
        } else {
            $builder = $builder.with_param("ttl", Some($val));
        }
    }};
    
    (@process_via_param $builder:ident, maddr, $val:expr) => {{
        $builder = $builder.with_maddr($val);
    }};
    
    (@process_via_param $builder:ident, rport, $val:expr) => {{
        if $val == "" || $val == "true" {
            $builder = $builder.with_rport();
        } else if let Ok(port) = $val.parse::<u16>() {
            $builder = $builder.with_rport_value(port);
        } else {
            $builder = $builder.with_param("rport", Some($val));
        }
    }};
    
    (@process_via_param $builder:ident, $key:ident, $val:expr) => {{
        $builder = $builder.with_param(stringify!($key), Some($val));
    }};
    
    (@process_field $builder:ident, contact, $contact_uri:expr) => {
        $builder.contact($contact_uri)
            .expect("Contact URI parse error")
    };
    
    (@process_field $builder:ident, contact_name, ($name:expr, $uri:expr)) => {
        $builder.contact_with_name($name, $uri)
            .expect("Contact URI parse error")
    };
    
    (@process_field $builder:ident, max_forwards, $max_forwards:expr) => {
        $builder.max_forwards($max_forwards)
    };
    
    (@process_field $builder:ident, content_type, $content_type:expr) => {
        $builder.content_type($content_type)
            .expect("Content-Type parse error")
    };
    
    (@process_field $builder:ident, body, $body:expr) => {
        $builder.body($body)
    };
    
    (@process_field $builder:ident, accept, $value:expr) => {
        $builder.header(TypedHeader::Other(
            HeaderName::Accept,
            HeaderValue::text($value)
        ))
    };
    
    (@process_field $builder:ident, user_agent, $user_agent:expr) => {
        $builder.header(TypedHeader::Other(
            HeaderName::UserAgent,
            HeaderValue::text($user_agent)
        ))
    };
    
    (@process_field $builder:ident, server, $server:expr) => {
        $builder.header(TypedHeader::Other(
            HeaderName::Server,
            HeaderValue::text($server)
        ))
    };
    
    (@process_field $builder:ident, warning, $warning:expr) => {
        $builder.header(TypedHeader::Other(
            HeaderName::Warning,
            HeaderValue::text($warning)
        ))
    };
    
    // Special case for headers block
    (@process_field $builder:ident, headers, { $($custom_header:ident : $custom_value:expr),* }) => {
        {
            let mut builder = $builder;
            $(
                match stringify!($custom_header) {
                    "accept" => {
                        builder = builder.header(TypedHeader::Other(
                            HeaderName::Accept,
                            HeaderValue::text($custom_value)
                        ));
                    },
                    "user_agent" => {
                        builder = builder.header(TypedHeader::Other(
                            HeaderName::UserAgent,
                            HeaderValue::text($custom_value)
                        ));
                    },
                    "server" => {
                        builder = builder.header(TypedHeader::Other(
                            HeaderName::Server,
                            HeaderValue::text($custom_value)
                        ));
                    },
                    "warning" => {
                        builder = builder.header(TypedHeader::Other(
                            HeaderName::Warning,
                            HeaderValue::text($custom_value)
                        ));
                    },
                    _ => {
                        // For other headers, capitalize the first letter of each word
                        let mut name = stringify!($custom_header).to_string();
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
                        
                        builder = builder.header(TypedHeader::Other(
                            HeaderName::Other(name),
                            HeaderValue::text($custom_value)
                        ));
                    }
                }
            )*
            builder
        }
    };

    // Add a new pattern to handle tuples for 'from' that include tag
    (@process_field $builder:ident, from, ($name:expr, $uri:expr, $tag:expr)) => {{
        $builder.from($name, $uri).with_tag($tag).done()
    }};

    // Add a new pattern to handle tuples for 'to'
    (@process_field $builder:ident, to, ($name:expr, $uri:expr)) => {{
        $builder.to($name, $uri).done()
    }};

    // Add a new pattern to handle tuples for 'via' that include branch param
    (@process_field $builder:ident, via, ($host:expr, $transport:expr, $branch:expr)) => {{
        $builder.via($host, $transport).with_branch($branch.split('=').nth(1).unwrap_or($branch)).done()
    }};
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
///
/// The order of headers doesn't matter (except that status must come first), complying with RFC 3261.
#[macro_export]
macro_rules! sip_response {
    (
        status: $status:expr
        $(, $key:ident : $value:tt)*
    ) => {
        {
            use $crate::types::builder::ResponseBuilder;
            use $crate::types::header::{HeaderName, HeaderValue};
            use $crate::types::TypedHeader;
            use $crate::types::uri::{Uri, Host, Scheme};
            use $crate::types::Method;
            use $crate::types::StatusCode;
            
            let mut builder = ResponseBuilder::new($status);

            $(
                builder = $crate::sip_response!(@process_field builder, $key, $value);
            )*

            builder.build()
        }
    };
    
    // Process individual header fields based on key
    (@process_field $builder:ident, reason, $reason:expr) => {
        $builder.reason($reason)
    };
    
    (@process_field $builder:ident, from, $from:tt) => {{
        $crate::sip_response!(@process_from $builder, $from)
    }};
    
    (@process_from $builder:ident, ($name:expr, $uri:expr)) => {{
        $builder.from($name, $uri).done()
    }};
    
    (@process_from $builder:ident, ($name:expr, $uri:expr, tag = $tag:expr)) => {{
        $builder.from($name, $uri).with_tag($tag).done()
    }};
    
    (@process_from $builder:ident, ($name:expr, $uri:expr, tag = $tag:expr, $($rest:tt)*)) => {{
        let mut builder = $builder.from($name, $uri).with_tag($tag);
        $crate::sip_response!(@process_params builder, $($rest)*);
        builder.done()
    }};
    
    (@process_from $builder:ident, ($name:expr, $uri:expr, $param_key:ident = $param_val:expr $(, $rest:tt)*)) => {{
        let mut builder = $builder.from($name, $uri);
        match stringify!($param_key) {
            "tag" => { builder = builder.with_tag($param_val); },
            _ => { builder = builder.with_param(stringify!($param_key), Some($param_val)); }
        }
        $(
            $crate::sip_response!(@process_params builder, $rest);
        )*
        builder.done()
    }};
    
    (@process_params $builder:ident, $key:ident = $val:expr $(, $rest:tt)*) => {{
        match stringify!($key) {
            "tag" => { $builder = $builder.with_tag($val); },
            _ => { $builder = $builder.with_param(stringify!($key), Some($val)); }
        }
        $(
            $crate::sip_response!(@process_params $builder, $rest);
        )*
    }};
    
    (@process_field $builder:ident, to, $to:tt) => {{
        $crate::sip_response!(@process_to $builder, $to)
    }};
    
    (@process_to $builder:ident, ($name:expr, $uri:expr)) => {{
        $builder.to($name, $uri).done()
    }};
    
    (@process_to $builder:ident, ($name:expr, $uri:expr, tag = $tag:expr)) => {{
        $builder.to($name, $uri).with_tag($tag).done()
    }};
    
    (@process_to $builder:ident, ($name:expr, $uri:expr, tag = $tag:expr, $($rest:tt)*)) => {{
        let mut builder = $builder.to($name, $uri).with_tag($tag);
        $crate::sip_response!(@process_params builder, $($rest)*);
        builder.done()
    }};
    
    (@process_to $builder:ident, ($name:expr, $uri:expr, $param_key:ident = $param_val:expr $(, $rest:tt)*)) => {{
        let mut builder = $builder.to($name, $uri);
        match stringify!($param_key) {
            "tag" => { builder = builder.with_tag($param_val); },
            _ => { builder = builder.with_param(stringify!($param_key), Some($param_val)); }
        }
        $(
            $crate::sip_response!(@process_params builder, $rest);
        )*
        builder.done()
    }};
    
    (@process_field $builder:ident, call_id, $call_id:expr) => {
        $builder.call_id($call_id)
    };
    
    (@process_field $builder:ident, cseq, $cseq:tt) => {
        $builder.cseq($cseq.0, $cseq.1)
    };
    
    (@process_field $builder:ident, via, $via:tt) => {{
        $crate::sip_response!(@process_via $builder, $via)
    }};
    
    (@process_via $builder:ident, ($host:expr, $transport:expr)) => {{
        $builder.via($host, $transport).done()
    }};
    
    (@process_via $builder:ident, ($host:expr, $transport:expr, branch = $branch:expr)) => {{
        $builder.via($host, $transport).with_branch($branch).done()
    }};
    
    (@process_via $builder:ident, ($host:expr, $transport:expr, branch = $branch:expr, received = $received:expr, $($rest:tt)*)) => {{
        let mut builder = $builder.via($host, $transport).with_branch($branch);
        
        // Parse IP address if possible, otherwise use generic param
        if let Ok(ip) = $received.parse::<std::net::IpAddr>() {
            builder = builder.with_received(ip);
        } else {
            builder = builder.with_param("received", Some($received));
        }
        
        $crate::sip_response!(@process_via_params builder, $($rest)*);
        builder.done()
    }};
    
    (@process_via $builder:ident, ($host:expr, $transport:expr, branch = $branch:expr, $($rest:tt)*)) => {{
        let mut builder = $builder.via($host, $transport).with_branch($branch);
        $crate::sip_response!(@process_via_params builder, $($rest)*);
        builder.done()
    }};
    
    (@process_via $builder:ident, ($host:expr, $transport:expr, $param_key:ident = $param_val:expr $(, $rest:tt)*)) => {{
        let mut builder = $builder.via($host, $transport);
        
        $crate::sip_response!(@process_via_param builder, $param_key, $param_val);
        $(
            $crate::sip_response!(@process_via_params builder, $rest);
        )*
        
        builder.done()
    }};
    
    (@process_via_params $builder:ident, $key:ident = $val:expr $(, $rest:tt)*) => {{
        $crate::sip_response!(@process_via_param $builder, $key, $val);
        $(
            $crate::sip_response!(@process_via_params $builder, $rest);
        )*
    }};
    
    (@process_via_param $builder:ident, branch, $val:expr) => {{
        $builder = $builder.with_branch($val);
    }};
    
    (@process_via_param $builder:ident, received, $val:expr) => {{
        if let Ok(ip) = $val.parse::<std::net::IpAddr>() {
            $builder = $builder.with_received(ip);
        } else {
            $builder = $builder.with_param("received", Some($val));
        }
    }};
    
    (@process_via_param $builder:ident, ttl, $val:expr) => {{
        if let Ok(ttl) = $val.parse::<u8>() {
            $builder = $builder.with_ttl(ttl);
        } else {
            $builder = $builder.with_param("ttl", Some($val));
        }
    }};
    
    (@process_via_param $builder:ident, maddr, $val:expr) => {{
        $builder = $builder.with_maddr($val);
    }};
    
    (@process_via_param $builder:ident, rport, $val:expr) => {{
        if $val == "" || $val == "true" {
            $builder = $builder.with_rport();
        } else if let Ok(port) = $val.parse::<u16>() {
            $builder = $builder.with_rport_value(port);
        } else {
            $builder = $builder.with_param("rport", Some($val));
        }
    }};
    
    (@process_via_param $builder:ident, $key:ident, $val:expr) => {{
        $builder = $builder.with_param(stringify!($key), Some($val));
    }};
    
    (@process_field $builder:ident, contact, $contact_uri:expr) => {
        $builder.contact($contact_uri)
            .expect("Contact URI parse error")
    };
    
    (@process_field $builder:ident, contact_name, ($name:expr, $uri:expr)) => {
        $builder.contact_with_name($name, $uri)
            .expect("Contact URI parse error")
    };
    
    (@process_field $builder:ident, content_type, $content_type:expr) => {
        $builder.content_type($content_type)
            .expect("Content-Type parse error")
    };
    
    (@process_field $builder:ident, body, $body:expr) => {
        $builder.body($body)
    };
    
    (@process_field $builder:ident, accept, $accept:expr) => {
        $builder.header(TypedHeader::Other(
            HeaderName::Accept,
            HeaderValue::text($accept)
        ))
    };
    
    (@process_field $builder:ident, user_agent, $user_agent:expr) => {
        $builder.header(TypedHeader::Other(
            HeaderName::UserAgent,
            HeaderValue::text($user_agent)
        ))
    };
    
    (@process_field $builder:ident, server, $server:expr) => {
        $builder.header(TypedHeader::Other(
            HeaderName::Server,
            HeaderValue::text($server)
        ))
    };
    
    (@process_field $builder:ident, warning, $warning:expr) => {
        $builder.header(TypedHeader::Other(
            HeaderName::Warning,
            HeaderValue::text($warning)
        ))
    };
    
    // Special case for headers block
    (@process_field $builder:ident, headers, { $($custom_header:ident : $custom_value:expr),* }) => {
        {
            let mut builder = $builder;
            $(
                match stringify!($custom_header) {
                    "accept" => {
                        builder = builder.header(TypedHeader::Other(
                            HeaderName::Accept,
                            HeaderValue::text($custom_value)
                        ));
                    },
                    "user_agent" => {
                        builder = builder.header(TypedHeader::Other(
                            HeaderName::UserAgent,
                            HeaderValue::text($custom_value)
                        ));
                    },
                    "server" => {
                        builder = builder.header(TypedHeader::Other(
                            HeaderName::Server,
                            HeaderValue::text($custom_value)
                        ));
                    },
                    "warning" => {
                        builder = builder.header(TypedHeader::Other(
                            HeaderName::Warning,
                            HeaderValue::text($custom_value)
                        ));
                    },
                    _ => {
                        // For other headers, capitalize the first letter of each word
                        let mut name = stringify!($custom_header).to_string();
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
                        
                        builder = builder.header(TypedHeader::Other(
                            HeaderName::Other(name),
                            HeaderValue::text($custom_value)
                        ));
                    }
                }
            )*
            builder
        }
    };

    // Also add similar patterns for the sip_response macro:
    (@process_field $builder:ident, from, ($name:expr, $uri:expr, $tag:expr)) => {{
        $builder.from($name, $uri).with_tag($tag).done()
    }};

    (@process_field $builder:ident, to, ($name:expr, $uri:expr)) => {{
        $builder.to($name, $uri).done()
    }};

    (@process_field $builder:ident, via, ($host:expr, $transport:expr, $branch:expr)) => {{
        $builder.via($host, $transport).with_branch($branch.split('=').nth(1).unwrap_or($branch)).done()
    }};
}

#[cfg(test)]
mod tests {
    use crate::{
        sip_request,
        sip_response,
        types::{
            Method, StatusCode, uri::Uri, uri::Scheme,
            TypedHeader, header::{HeaderName, HeaderValue},
            sip_message::{Request, Response},
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
            uri: "sip:server.example.com",
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
            call_id: "abc123@example.com",
            cseq: 1,
            via: ("example.com", "UDP", branch="z9hG4bK1234", received="192.168.1.1")
        };
        
        // Test with spaces around equals
        let request2 = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
            to: ("Bob", "sip:bob@example.com"),
            call_id: "abc123@example.com",
            cseq: 1,
            via: ("example.com", "UDP", branch = "z9hG4bK1234", received = "192.168.1.1")
        };
        
        // Test mixed styles
        let request3 = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            from: ("Alice", "sip:alice@example.com", tag="1928301774"),
            to: ("Bob", "sip:bob@example.com"),
            call_id: "abc123@example.com",
            cseq: 1,
            via: ("example.com", "UDP", branch = "z9hG4bK1234", received="192.168.1.1")
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

    #[test]
    fn test_headers_in_different_order() {
        // Test that headers can be specified in any order in request macros
        // RFC 3261 does not require specific header ordering except for Via

        // Standard order
        let request1 = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
            to: ("Bob", "sip:bob@example.com"),
            call_id: "a84b4c76e66710@pc33.atlanta.example.com",
            cseq: 1,
            via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds"),
            max_forwards: 70,
            contact: "sip:alice@alice.example.com",
            content_type: "application/sdp",
            body: "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n"
        };

        // Different order
        let request2 = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            to: ("Bob", "sip:bob@example.com"),
            from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
            via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds"),
            call_id: "a84b4c76e66710@pc33.atlanta.example.com",
            contact: "sip:alice@alice.example.com",
            max_forwards: 70,
            content_type: "application/sdp",
            cseq: 1,
            body: "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n"
        };
        
        // Another order with custom headers
        let request3 = sip_request! {
            method: Method::Invite,
            uri: "sip:bob@example.com",
            cseq: 1,
            max_forwards: 70,
            call_id: "a84b4c76e66710@pc33.atlanta.example.com",
            from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
            via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds"),
            to: ("Bob", "sip:bob@example.com"),
            contact: "sip:alice@alice.example.com",
            content_type: "application/sdp",
            body: "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n",
            accept: "application/sdp"
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
            from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
            to: ("Bob", "sip:bob@example.com", tag = "as83kd9bs"),
            call_id: "a84b4c76e66710@pc33.atlanta.example.com",
            cseq: (1, Method::Invite),
            via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds"),
            contact: "sip:bob@192.168.1.2"
        };
        
        let response2 = sip_response! {
            status: StatusCode::Ok,
            reason: "OK",
            via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds"),
            call_id: "a84b4c76e66710@pc33.atlanta.example.com",
            from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
            to: ("Bob", "sip:bob@example.com", tag = "as83kd9bs"),
            cseq: (1, Method::Invite),
            contact: "sip:bob@192.168.1.2"
        };
        
        let response3 = sip_response! {
            status: StatusCode::Ok,
            via: ("alice.example.com:5060", "UDP", branch = "z9hG4bK776asdhds"),
            to: ("Bob", "sip:bob@example.com", tag = "as83kd9bs"),
            from: ("Alice", "sip:alice@example.com", tag = "1928301774"),
            call_id: "a84b4c76e66710@pc33.atlanta.example.com",
            cseq: (1, Method::Invite),
            reason: "OK",
            contact: "sip:bob@192.168.1.2",
            server: "Test Server/1.0"
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
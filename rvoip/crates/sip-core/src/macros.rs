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
///     from: ("Alice", "sip:alice@example.com", tag="1928301774"),
///     to: ("Bob", "sip:bob@example.com"),
///     call_id: "a84b4c76e66710@pc33.atlanta.example.com",
///     cseq: 1,
///     via: ("alice.example.com:5060", "UDP", branch="z9hG4bK776asdhds"),
///     contact: "sip:alice@alice.example.com",
///     max_forwards: 70,
///     content_type: "application/sdp",
///     body: "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n"
/// };
/// ```
#[macro_export]
macro_rules! sip_request {
    (
        method: $method:expr,
        uri: $uri:expr
        $(, from: ($from_name:expr, $from_uri:expr $(, tag=$from_tag:expr)? $(, $from_param_name:ident=$from_param_val:expr)* ) )?
        $(, to: ($to_name:expr, $to_uri:expr $(, tag=$to_tag:expr)? $(, $to_param_name:ident=$to_param_val:expr)* ) )?
        $(, call_id: $call_id:expr )?
        $(, cseq: $cseq:expr )?
        $(, via: ($via_host:expr, $via_transport:expr $(, branch=$branch:expr)? $(, $via_param_name:ident=$via_param_val:expr)* ) )?
        $(, contact: $contact_uri:expr )?
        $(, contact_name: ($contact_name:expr, $contact_name_uri:expr) )?
        $(, max_forwards: $max_forwards:expr )?
        $(, content_type: $content_type:expr )?
        $(, body: $body:expr )?
        $(, $custom_header:tt: $custom_value:expr )*
    ) => {
        {
            use $crate::types::builder::RequestBuilder;
            let mut builder = RequestBuilder::new($method, $uri)
                .expect("URI parse error");

            $(
                let from_builder = builder.from($from_name, $from_uri);
                $(
                    let from_builder = from_builder.with_tag($from_tag);
                )?
                $(
                    let from_builder = from_builder.with_param(stringify!($from_param_name), Some($from_param_val));
                )*
                builder = from_builder.done();
            )?

            $(
                let to_builder = builder.to($to_name, $to_uri);
                $(
                    let to_builder = to_builder.with_tag($to_tag);
                )?
                $(
                    let to_builder = to_builder.with_param(stringify!($to_param_name), Some($to_param_val));
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
                let via_builder = builder.via($via_host, $via_transport);
                $(
                    let via_builder = via_builder.with_branch($branch);
                )?
                $(
                    let via_builder = match stringify!($via_param_name) {
                        "ttl" => via_builder.with_ttl($via_param_val),
                        "maddr" => via_builder.with_maddr($via_param_val),
                        "rport" => via_builder.with_rport_value($via_param_val),
                        _ => via_builder,
                    };
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

            // Custom headers would go here...
            
            builder.build()
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
///     from: ("Alice", "sip:alice@example.com", tag="1928301774"),
///     to: ("Bob", "sip:bob@example.com", tag="as83kd9bs"),
///     call_id: "a84b4c76e66710@pc33.atlanta.example.com",
///     cseq: (1, Method::Invite),
///     via: ("alice.example.com:5060", "UDP", branch="z9hG4bK776asdhds"),
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
        $(, from: ($from_name:expr, $from_uri:expr $(, tag=$from_tag:expr)? $(, $from_param_name:ident=$from_param_val:expr)* ) )?
        $(, to: ($to_name:expr, $to_uri:expr $(, tag=$to_tag:expr)? $(, $to_param_name:ident=$to_param_val:expr)* ) )?
        $(, call_id: $call_id:expr )?
        $(, cseq: ($cseq:expr, $cseq_method:expr) )?
        $(, via: ($via_host:expr, $via_transport:expr $(, branch=$branch:expr)? $(, $via_param_name:ident=$via_param_val:expr)* ) )?
        $(, contact: $contact_uri:expr )?
        $(, content_type: $content_type:expr )?
        $(, body: $body:expr )?
        $(, $custom_header:tt: $custom_value:expr )*
    ) => {
        {
            use $crate::types::builder::ResponseBuilder;
            let mut builder = ResponseBuilder::new($status);

            $(
                builder = builder.reason($reason);
            )?

            $(
                let from_builder = builder.from($from_name, $from_uri);
                $(
                    let from_builder = from_builder.with_tag($from_tag);
                )?
                $(
                    let from_builder = from_builder.with_param(stringify!($from_param_name), Some($from_param_val));
                )*
                builder = from_builder.done();
            )?

            $(
                let to_builder = builder.to($to_name, $to_uri);
                $(
                    let to_builder = to_builder.with_tag($to_tag);
                )?
                $(
                    let to_builder = to_builder.with_param(stringify!($to_param_name), Some($to_param_val));
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
                let via_builder = builder.via($via_host, $via_transport);
                $(
                    let via_builder = via_builder.with_branch($branch);
                )?
                $(
                    let via_builder = match stringify!($via_param_name) {
                        "ttl" => via_builder.with_ttl($via_param_val),
                        "maddr" => via_builder.with_maddr($via_param_val),
                        "rport" => via_builder.with_rport_value($via_param_val),
                        _ => via_builder,
                    };
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

            // Custom headers would go here...
            
            builder.build()
        }
    };
} 
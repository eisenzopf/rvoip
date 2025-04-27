use rvoip_sip_core::{
    types::{
        Message, 
        StatusCode, 
        Method,
        Version,
        to::To,
        from::From,
        call_id::CallId,
        cseq::CSeq,
        content_type::ContentType,
        content_length::ContentLength,
        via::{Via, ViaHeader, SentProtocol},
        uri::Uri,
        sip_message::{Request, Response},
        Address,
        TypedHeader,
        Param,
        header::{HeaderName, HeaderValue},
        builder::{RequestBuilder, ResponseBuilder},
    },
    error::Error,
    sip_request,
    sip_response,
};

use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
use bytes::Bytes;

/// Builds a simple SIP INVITE request
pub fn build_invite_request() -> Result<Message, Error> {
    // Using the builder pattern
    let sdp_body = "\
v=0
o=alice 2890844526 2890844526 IN IP4 alice.example.com
s=Session SDP
c=IN IP4 alice.example.com
t=0 0
m=audio 49172 RTP/AVP 0
a=rtpmap:0 PCMU/8000
";

    // Builder pattern approach
    let request = RequestBuilder::invite("sip:bob@example.com").expect("URI parse error")
        .from("Alice", "sip:alice@example.com")
            .with_tag("1928301774")
            .done()
        .to("Bob", "sip:bob@example.com")
            .done()
        .call_id("a84b4c76e66710@pc33.atlanta.example.com")
        .via("alice.example.com:5060", "UDP")
            .with_branch("z9hG4bK776asdhds")
            .done()
        .cseq(1)
        .contact("sip:alice@alice.example.com").expect("Contact URI parse error")
        .max_forwards(70)
        .content_type("application/sdp").expect("Content-Type parse error")
        .body(sdp_body)
        .build();
    
    // Convert to a generic Message
    Ok(Message::Request(request))
}

/// Alternative version using the macro
pub fn build_invite_request_using_macro() -> Result<Message, Error> {
    let sdp_body = "\
v=0
o=alice 2890844526 2890844526 IN IP4 alice.example.com
s=Session SDP
c=IN IP4 alice.example.com
t=0 0
m=audio 49172 RTP/AVP 0
a=rtpmap:0 PCMU/8000
";

    // Macro approach
    let request = sip_request! {
        method: Method::Invite,
        uri: "sip:bob@example.com",
        from: ("Alice", "sip:alice@example.com", tag="1928301774"),
        to: ("Bob", "sip:bob@example.com"),
        call_id: "a84b4c76e66710@pc33.atlanta.example.com",
        cseq: 1,
        via: ("alice.example.com:5060", "UDP", branch="z9hG4bK776asdhds"),
        contact: "sip:alice@alice.example.com",
        max_forwards: 70,
        content_type: "application/sdp",
        body: sdp_body
    };
    
    // Convert to a generic Message
    Ok(Message::Request(request))
}

/// Builds a 200 OK response to an INVITE request
pub fn build_200_ok_response(invite_request: &Message) -> Result<Message, Error> {
    let sdp_body = "\
v=0
o=bob 2890844527 2890844527 IN IP4 bob.example.com
s=Session SDP
c=IN IP4 bob.example.com
t=0 0
m=audio 49174 RTP/AVP 0
a=rtpmap:0 PCMU/8000
";

    // First, extract necessary information from the request
    let (from, to, call_id, cseq) = if let Message::Request(req) = invite_request {
        (
            req.header(&HeaderName::From).cloned(),
            req.header(&HeaderName::To).cloned(),
            req.header(&HeaderName::CallId).cloned(),
            req.header(&HeaderName::CSeq).cloned(),
        )
    } else {
        return Err(Error::Other("Input is not a request".to_string()));
    };

    // Start with a 200 OK response
    let mut response = ResponseBuilder::ok()
        .reason("OK");
    
    // Add headers from the request
    if let Some(TypedHeader::From(from_header)) = from {
        response = response.header(TypedHeader::From(from_header));
    }
    
    if let Some(TypedHeader::To(to_header)) = to {
        // Clone the To header and add tag
        let mut to_value = to_header.0.clone();
        to_value.set_tag("as83kd9bs");
        response = response.header(TypedHeader::To(To(to_value)));
    }
    
    if let Some(TypedHeader::CallId(call_id_header)) = call_id {
        response = response.header(TypedHeader::CallId(call_id_header));
    }
    
    if let Some(TypedHeader::CSeq(cseq_header)) = cseq {
        response = response.header(TypedHeader::CSeq(cseq_header));
    }
    
    if let Some(TypedHeader::Via(via_headers)) = 
        if let Message::Request(req) = invite_request {
            req.header(&HeaderName::Via).cloned()
        } else {
            None
        } 
    {
        response = response.header(TypedHeader::Via(via_headers));
    }
    
    // Add additional headers
    let response = response
        .contact("sip:bob@192.168.1.2").expect("Contact URI parse error")
        .header(TypedHeader::Other(HeaderName::Server, HeaderValue::text("rvoip-sip-demo/1.0".to_string())))
        .content_type("application/sdp").expect("Content-Type parse error")
        .body(sdp_body)
        .build();
    
    // Convert to a generic Message
    Ok(Message::Response(response))
}

/// Alternative version using the macro
pub fn build_200_ok_response_using_macro(invite_request: &Message) -> Result<Message, Error> {
    // First, extract necessary information from the request
    if let Message::Request(req) = invite_request {
        let from = if let Some(TypedHeader::From(from)) = req.header(&HeaderName::From) {
            from.0.clone()
        } else {
            return Err(Error::Other("Missing From header".to_string()));
        };
        
        let to = if let Some(TypedHeader::To(to)) = req.header(&HeaderName::To) {
            to.0.clone()
        } else {
            return Err(Error::Other("Missing To header".to_string()));
        };
        
        let call_id = if let Some(TypedHeader::CallId(call_id)) = req.header(&HeaderName::CallId) {
            call_id.0.clone()
        } else {
            return Err(Error::Other("Missing Call-ID header".to_string()));
        };
        
        let cseq = if let Some(TypedHeader::CSeq(cseq)) = req.header(&HeaderName::CSeq) {
            (cseq.seq, cseq.method.clone())
        } else {
            return Err(Error::Other("Missing CSeq header".to_string()));
        };
        
        let via = if let Some(TypedHeader::Via(via)) = req.header(&HeaderName::Via) {
            if !via.0.is_empty() {
                let vh = &via.0[0];
                let branch = vh.params.iter()
                    .find_map(|p| if let Param::Branch(b) = p { Some(b.clone()) } else { None })
                    .unwrap_or_else(|| "z9hG4bK-invalid".to_string());
                Some((vh.sent_by_host.to_string(), vh.sent_protocol.transport.clone(), branch))
            } else {
                None
            }
        } else {
            None
        };
        
        let sdp_body = "\
v=0
o=bob 2890844527 2890844527 IN IP4 bob.example.com
s=Session SDP
c=IN IP4 bob.example.com
t=0 0
m=audio 49174 RTP/AVP 0
a=rtpmap:0 PCMU/8000
";

        // Create response using the macro
        if let Some((via_host, via_transport, branch)) = via {
            let response = sip_response! {
                status: StatusCode::Ok,
                reason: "OK",
                from: (from.display_name.unwrap_or_default(), from.uri.to_string(), tag=from.tag().unwrap_or_default()),
                to: (to.display_name.unwrap_or_default(), to.uri.to_string(), tag="as83kd9bs"),
                call_id: call_id,
                cseq: (cseq.0, cseq.1),
                via: (via_host, via_transport, branch=branch),
                contact: "sip:bob@192.168.1.2",
                content_type: "application/sdp",
                body: sdp_body
            };
            
            return Ok(Message::Response(response));
        } else {
            return Err(Error::Other("Missing Via header".to_string()));
        };
    }
    
    Err(Error::Other("Input is not a request".to_string()))
}

/// Builds a SIP REGISTER request
pub fn build_register_request() -> Result<Message, Error> {
    // Generate a random branch parameter for Via
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let branch = format!("z9hG4bK-{}", timestamp % 10000);
    let call_id = format!("reg-{}-{}", timestamp, timestamp % 1000);
    
    // Using the builder pattern
    let request = RequestBuilder::register("sip:registrar.example.com").expect("URI parse error")
        .from("Alice", "sip:alice@example.com")
            .with_tag("reg-tag")
            .done()
        .to("Alice", "sip:alice@example.com")
            .done()
        .call_id(&call_id)
        .via("192.168.1.2:5060", "UDP")
            .with_branch(&branch)
            .done()
        .cseq(1)
        .contact("sip:alice@192.168.1.2:5060").expect("Contact URI parse error")
        .max_forwards(70)
        .header(TypedHeader::Other(HeaderName::Expires, HeaderValue::integer(3600)))
        .header(TypedHeader::Other(HeaderName::UserAgent, HeaderValue::text("rvoip-sip-demo/1.0".to_string())))
        .build();
    
    // Convert to a generic Message
    Ok(Message::Request(request))
}

/// Alternative version using the macro
pub fn build_register_request_using_macro() -> Result<Message, Error> {
    // Generate a random branch parameter for Via
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let branch = format!("z9hG4bK-{}", timestamp % 10000);
    let call_id = format!("reg-{}-{}", timestamp, timestamp % 1000);
    
    // Using the macro
    let request = sip_request! {
        method: Method::Register,
        uri: "sip:registrar.example.com",
        from: ("Alice", "sip:alice@example.com", tag="reg-tag"),
        to: ("Alice", "sip:alice@example.com"),
        call_id: call_id,
        cseq: 1,
        via: ("192.168.1.2:5060", "UDP", branch=branch),
        contact: "sip:alice@192.168.1.2:5060",
        max_forwards: 70
    };
    
    // Add additional headers (would be nice to add to the macro in the future)
    let mut request = request;
    request.headers.push(TypedHeader::Other(HeaderName::Expires, HeaderValue::integer(3600)));
    request.headers.push(TypedHeader::Other(HeaderName::UserAgent, HeaderValue::text("rvoip-sip-demo/1.0".to_string())));
    
    // Convert to a generic Message
    Ok(Message::Request(request))
}

/// Converts a Message to a wire-format SIP message string
pub fn message_to_string(message: &Message) -> String {
    // Simple debug version - return a placeholder string 
    // to avoid potential recursion
    match message {
        Message::Request(req) => {
            format!("{} {} {}\r\n\
                    Via: SIP/2.0/UDP example.com;branch=z9hG4bK123\r\n\
                    To: <{}>\r\n\
                    From: <sip:user@example.com>;tag=abcdef\r\n\
                    Call-ID: 12345@example.com\r\n\
                    CSeq: 1 {}\r\n\
                    Content-Length: {}\r\n\
                    \r\n{}",
                    req.method, 
                    req.uri,
                    req.version,
                    req.uri,
                    req.method,
                    req.body.len(),
                    if !req.body.is_empty() { String::from_utf8_lossy(&req.body) } else { "".into() }
            )
        },
        Message::Response(resp) => {
            let status_code = match resp.status {
                StatusCode::Ok => 200,
                StatusCode::Trying => 100,
                _ => 200, // Default to 200 for demo
            };
            
            format!("{} {} {}\r\n\
                    Via: SIP/2.0/UDP example.com;branch=z9hG4bK123\r\n\
                    To: <sip:user@example.com>;tag=abcdef\r\n\
                    From: <sip:user@example.com>;tag=123456\r\n\
                    Call-ID: 12345@example.com\r\n\
                    CSeq: 1 INVITE\r\n\
                    Content-Length: {}\r\n\
                    \r\n{}",
                    resp.version,
                    status_code,
                    resp.reason.as_deref().unwrap_or(""),
                    resp.body.len(),
                    if !resp.body.is_empty() { String::from_utf8_lossy(&resp.body) } else { "".into() }
            )
        }
    }
} 
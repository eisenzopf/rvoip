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
        header::{HeaderName, HeaderValue, Header},
        contact::{Contact, ContactParamInfo},
    },
    error::Error,
};

use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
use bytes::Bytes;

/// Builds a simple SIP INVITE request
pub fn build_invite_request() -> Result<Message, Error> {
    // Create a new Request object
    let mut request = Request::new(
        Method::Invite,
        Uri::from_str("sip:bob@example.com")?,
    );
    
    // Set the version
    request.version = Version::new(2, 0);
    
    // Add Via header with branch parameter
    let sent_protocol = SentProtocol {
        name: "SIP".to_string(),
        version: "2.0".to_string(), 
        transport: "UDP".to_string()
    };
    
    let via_header = ViaHeader {
        sent_protocol,
        sent_by_host: "alice.example.com:5060".parse().unwrap(),
        sent_by_port: None,
        params: vec![Param::Branch("z9hG4bK776asdhds".to_string())],
    };
    
    request = request.with_header(TypedHeader::Via(Via(vec![via_header])));
    
    // Create From header with tag parameter
    let from_addr = Address {
        display_name: Some("Alice".to_string()),
        uri: Uri::from_str("sip:alice@example.com")?,
        params: vec![Param::Tag("1928301774".to_string())],
    };
    
    request = request.with_header(TypedHeader::From(From(from_addr)));
    
    // Create To header
    let to_addr = Address {
        display_name: Some("Bob".to_string()),
        uri: Uri::from_str("sip:bob@example.com")?,
        params: vec![],
    };
    
    request = request.with_header(TypedHeader::To(To(to_addr)));
    
    // Add Call-ID header
    request = request.with_header(TypedHeader::CallId(
        CallId("a84b4c76e66710@pc33.atlanta.example.com".to_string())
    ));
    
    // Add CSeq header
    request = request.with_header(TypedHeader::CSeq(
        CSeq::new(1, Method::Invite)
    ));
    
    // Add Contact header
    let contact_uri = Uri::from_str("sip:alice@alice.example.com")?;
    let contact_address = Address {
        display_name: None,
        uri: contact_uri,
        params: vec![],
    };
    
    // Import what we need for Contact header
    let contact_param = ContactParamInfo { address: contact_address };
    request = request.with_header(TypedHeader::Contact(
        Contact::new_params(vec![contact_param])
    ));
    
    // Add Max-Forwards header
    use rvoip_sip_core::types::max_forwards::MaxForwards;
    request = request.with_header(TypedHeader::MaxForwards(
        MaxForwards(70)
    ));
    
    // Set Content-Type and Content-Length for the SDP body
    let sdp_body = "\
v=0
o=alice 2890844526 2890844526 IN IP4 alice.example.com
s=Session SDP
c=IN IP4 alice.example.com
t=0 0
m=audio 49172 RTP/AVP 0
a=rtpmap:0 PCMU/8000
";
    
    // Add Content-Type header
    // Create the content type value
    let content_type = ContentType::from_str("application/sdp").unwrap();
    request = request.with_header(TypedHeader::ContentType(content_type));
    
    // Add Content-Length header and body
    request = request.with_header(TypedHeader::ContentLength(
        ContentLength(sdp_body.len() as u32)
    ));
    
    request.body = Bytes::from(sdp_body);
    
    // Convert to a generic Message
    Ok(Message::Request(request))
}

/// Builds a 200 OK response to an INVITE request
pub fn build_200_ok_response(invite_request: &Message) -> Result<Message, Error> {
    // Create a new Response object with OK status
    let mut response = Response::new(StatusCode::Ok);
    
    // Set the version and reason phrase
    response.version = Version::new(2, 0);
    response.reason = Some("OK".to_string());
    
    // Copy headers from the request as needed
    if let Message::Request(req) = invite_request {
        // Get Via headers from the request
        if let Some(TypedHeader::Via(via_headers)) = req.header(&HeaderName::Via) {
            response = response.with_header(TypedHeader::Via(via_headers.clone()));
        }
        
        // Get From header from the request
        if let Some(TypedHeader::From(from)) = req.header(&HeaderName::From) {
            response = response.with_header(TypedHeader::From(from.clone()));
        }
        
        // Get To header from the request and add a tag
        if let Some(TypedHeader::To(to)) = req.header(&HeaderName::To) {
            let mut to_with_tag = to.clone();
            
            // Make a new Address with the tag parameter
            let mut params = to_with_tag.0.params.clone();
            params.push(Param::Tag("as83kd9bs".to_string()));
            
            let to_addr = Address {
                display_name: to_with_tag.0.display_name.clone(),
                uri: to_with_tag.0.uri.clone(),
                params,
            };
            
            response = response.with_header(TypedHeader::To(To(to_addr)));
        }
        
        // Get Call-ID header from the request
        if let Some(TypedHeader::CallId(call_id)) = req.header(&HeaderName::CallId) {
            response = response.with_header(TypedHeader::CallId(call_id.clone()));
        }
        
        // Get CSeq header from the request
        if let Some(TypedHeader::CSeq(cseq)) = req.header(&HeaderName::CSeq) {
            response = response.with_header(TypedHeader::CSeq(cseq.clone()));
        }
    }
    
    // Add Contact header
    let contact_uri = Uri::from_str("sip:bob@192.168.1.2")?;
    let contact_address = Address {
        display_name: None,
        uri: contact_uri,
        params: vec![],
    };
    
    // Import what we need for Contact header
    let contact_param = ContactParamInfo { address: contact_address };
    response = response.with_header(TypedHeader::Contact(
        Contact::new_params(vec![contact_param])
    ));
    
    // Add a Server header as a raw header
    let server_value = HeaderValue::text("rvoip-sip-demo/1.0".to_string());
    
    // Add the header using with_header method with TypedHeader::Other
    response = response.with_header(TypedHeader::Other(HeaderName::Server, server_value));
    
    // Add SDP body for the 200 OK (accept the call)
    let sdp_body = "\
v=0
o=bob 2890844527 2890844527 IN IP4 bob.example.com
s=Session SDP
c=IN IP4 bob.example.com
t=0 0
m=audio 49174 RTP/AVP 0
a=rtpmap:0 PCMU/8000
";
    
    // Add Content-Type header
    let content_type = ContentType::from_str("application/sdp").unwrap();
    response = response.with_header(TypedHeader::ContentType(content_type));
    
    // Add Content-Length header and body
    response = response.with_header(TypedHeader::ContentLength(
        ContentLength(sdp_body.len() as u32)
    ));
    
    response.body = Bytes::from(sdp_body);
    
    // Convert to a generic Message
    Ok(Message::Response(response))
}

/// Builds a SIP REGISTER request
pub fn build_register_request() -> Result<Message, Error> {
    // Create a new Request object
    let mut request = Request::new(
        Method::Register,
        Uri::from_str("sip:registrar.example.com")?,
    );
    
    // Set the version
    request.version = Version::new(2, 0);
    
    // Generate a random branch parameter for Via
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let branch = format!("z9hG4bK-{}", timestamp % 10000);
    
    // Add Via header
    let sent_protocol = SentProtocol {
        name: "SIP".to_string(),
        version: "2.0".to_string(), 
        transport: "UDP".to_string()
    };
    
    let via_header = ViaHeader {
        sent_protocol,
        sent_by_host: "192.168.1.2:5060".parse().unwrap(),
        sent_by_port: None,
        params: vec![Param::Branch(branch)],
    };
    
    request = request.with_header(TypedHeader::Via(Via(vec![via_header])));
    
    // Create user URI for From/To headers
    let user_uri = Uri::from_str("sip:alice@example.com")?;
    
    // Create From header with tag
    let from_addr = Address {
        display_name: Some("Alice".to_string()),
        uri: user_uri.clone(),
        params: vec![Param::Tag("reg-tag".to_string())],
    };
    
    request = request.with_header(TypedHeader::From(From(from_addr)));
    
    // Create To header (same as From but without tag)
    let to_addr = Address {
        display_name: Some("Alice".to_string()),
        uri: user_uri.clone(),
        params: vec![],
    };
    
    request = request.with_header(TypedHeader::To(To(to_addr)));
    
    // Generate a random Call-ID
    let call_id = format!("reg-{}-{}", timestamp, timestamp % 1000);
    request = request.with_header(TypedHeader::CallId(
        CallId(call_id)
    ));
    
    // Add CSeq header
    request = request.with_header(TypedHeader::CSeq(
        CSeq::new(1, Method::Register)
    ));
    
    // Add Contact header
    let contact_uri = Uri::from_str("sip:alice@192.168.1.2:5060")?;
    let contact_address = Address {
        display_name: None,
        uri: contact_uri,
        params: vec![],
    };
    
    // Import what we need for Contact header
    let contact_param = ContactParamInfo { address: contact_address };
    request = request.with_header(TypedHeader::Contact(
        Contact::new_params(vec![contact_param])
    ));
    
    // Add Max-Forwards header
    use rvoip_sip_core::types::max_forwards::MaxForwards;
    request = request.with_header(TypedHeader::MaxForwards(
        MaxForwards(70)
    ));
    
    // Add Expires header as a raw header
    let expires_value = HeaderValue::integer(3600);
    
    // Add the header using with_header method with TypedHeader::Other
    request = request.with_header(TypedHeader::Other(HeaderName::Expires, expires_value));
    
    // Add User-Agent header
    let ua_value = HeaderValue::text("rvoip-sip-demo/1.0".to_string());
    
    // Add the header using with_header method with TypedHeader::Other
    request = request.with_header(TypedHeader::Other(HeaderName::UserAgent, ua_value));
    
    // Add Content-Length header (no body)
    request = request.with_header(TypedHeader::ContentLength(
        ContentLength(0)
    ));
    
    // Convert to a generic Message
    Ok(Message::Request(request))
}

/// Converts a Message to a wire-format SIP message string
pub fn message_to_string(message: &Message) -> String {
    let mut result = String::new();
    
    // Add request or status line
    match message {
        Message::Request(request) => {
            result.push_str(&format!("{} {} {}\r\n", 
                request.method,
                request.uri,
                request.version));
        },
        Message::Response(response) => {
            // Format status code as a number
            let status_code = match response.status {
                StatusCode::Ok => 200,
                StatusCode::Trying => 100,
                _ => 200, // Default to 200 for demo
            };
            
            result.push_str(&format!("{} {} {}\r\n", 
                response.version,
                status_code,
                response.reason.as_deref().unwrap_or("")));
        }
    }
    
    // Add headers
    let headers = match message {
        Message::Request(req) => &req.headers,
        Message::Response(resp) => &resp.headers,
    };
    
    for header in headers {
        // Convert TypedHeader to raw string format
        let header_str = header.to_string();
        result.push_str(&format!("{}\r\n", header_str));
    }
    
    // Empty line separator
    result.push_str("\r\n");
    
    // Add body if any
    let body = match message {
        Message::Request(req) => &req.body,
        Message::Response(resp) => &resp.body,
    };
    
    if !body.is_empty() {
        result.push_str(&String::from_utf8_lossy(body));
    }
    
    result
} 
use rvoip_sip_core::{
    Request, Response, Header, HeaderName, Uri
};

/// Add common headers to a response based on a request
pub fn add_response_headers(request: &Request, response: &mut Response) {
    // Copy headers from request
    for header in &request.headers {
        match header.name {
            HeaderName::Via | HeaderName::From | HeaderName::CallId | HeaderName::CSeq => {
                response.headers.push(header.clone());
            },
            _ => {},
        }
    }
    
    // Add Content-Length if not present
    if !response.headers.iter().any(|h| h.name == HeaderName::ContentLength) {
        response.headers.push(Header::text(HeaderName::ContentLength, "0"));
    }
}

/// Helper function to extract URI from a SIP header
pub fn extract_uri_from_header(request: &Request, header_name: HeaderName) -> Option<Uri> {
    let header = request.headers.iter()
        .find(|h| h.name == header_name)?;
    
    let value = header.value.as_text()?;
    
    // Extract URI from the header value
    let uri_str = if let Some(uri_end) = value.find('>') {
        if let Some(uri_start) = value.find('<') {
            &value[uri_start + 1..uri_end]
        } else {
            value
        }
    } else {
        value
    };
    
    // Parse the URI
    match uri_str.parse() {
        Ok(uri) => Some(uri),
        Err(_) => None,
    }
} 
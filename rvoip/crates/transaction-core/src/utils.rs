use rand::{thread_rng, Rng};
use rvoip_sip_core::{Header, HeaderName, Message, Method, Request, Response, StatusCode};
use std::str::FromStr;

/// Generate a random branch parameter for Via header
pub fn generate_branch() -> String {
    let mut rng = thread_rng();
    let random: u64 = rng.gen();
    format!("z9hG4bK-{:x}", random)
}

/// Extract the branch parameter from a message
pub fn extract_branch(message: &Message) -> Option<String> {
    message
        .header(&HeaderName::Via)
        .and_then(|via| {
            let value = via.value.to_string_value();
            // Extract branch parameter (very basic implementation)
            if let Some(branch_pos) = value.find("branch=") {
                let branch_start = branch_pos + 7; // "branch=" length
                let branch_end = value[branch_start..]
                    .find(|c: char| c == ';' || c == ',' || c.is_whitespace())
                    .map(|pos| branch_start + pos)
                    .unwrap_or(value.len());
                Some(value[branch_start..branch_end].to_string())
            } else {
                None
            }
        })
}

/// Extract the Call-ID from a message
pub fn extract_call_id(message: &Message) -> Option<String> {
    message
        .header(&HeaderName::CallId)
        .map(|h| h.value.to_string_value())
}

/// Extract the CSeq from a message
pub fn extract_cseq(message: &Message) -> Option<(u32, Method)> {
    message
        .header(&HeaderName::CSeq)
        .and_then(|h| {
            let value = h.value.to_string_value();
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() != 2 {
                return None;
            }
            
            let seq_num = parts[0].parse::<u32>().ok()?;
            let method = Method::from_str(parts[1]).ok()?;
            Some((seq_num, method))
        })
}

/// Create a general response to a request
pub fn create_response(request: &Request, status: StatusCode) -> Response {
    let mut response = Response::new(status);
    
    // Copy headers from request that should be in the response
    for header in &request.headers {
        if matches!(header.name, 
            HeaderName::Via | 
            HeaderName::From | 
            HeaderName::To | 
            HeaderName::CallId | 
            HeaderName::CSeq
        ) {
            response = response.with_header(header.clone());
        }
    }
    
    // Add Content-Length header (empty body)
    response = response.with_header(Header::integer(HeaderName::ContentLength, 0));
    
    response
}

/// Create a TRYING (100) response for an INVITE request
pub fn create_trying_response(request: &Request) -> Response {
    create_response(request, StatusCode::Trying)
}

/// Create a RINGING (180) response for an INVITE request
pub fn create_ringing_response(request: &Request) -> Response {
    create_response(request, StatusCode::Ringing)
}

/// Create an OK (200) response for a request
pub fn create_ok_response(request: &Request) -> Response {
    create_response(request, StatusCode::Ok)
} 
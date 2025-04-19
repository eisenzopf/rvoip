use std::str::FromStr;

use bytes::Bytes;

use crate::error::{Error, Result};
use crate::header::{Header, HeaderName, HeaderValue};
use crate::message::{Message, Request, Response, StatusCode};
use crate::method::Method;
use crate::uri::Uri;
use crate::version::Version;

/// Parses a SIP message from raw bytes
pub fn parse_message(data: &Bytes) -> Result<Message> {
    // Convert bytes to string
    let data_str = std::str::from_utf8(data).map_err(|_| {
        Error::InvalidFormat("Message contains invalid UTF-8".to_string())
    })?;
    
    // Split message into lines
    let mut lines = data_str.lines();
    
    // Parse the first line (request-line or status-line)
    let first_line = lines.next().ok_or_else(|| {
        Error::InvalidFormat("Empty message".to_string())
    })?;
    
    // Determine if this is a request or response
    if first_line.starts_with("SIP/") {
        // This is a response
        parse_response(first_line, lines)
    } else {
        // This is a request
        parse_request(first_line, lines)
    }
}

/// Parses a SIP request from the given lines
fn parse_request<'a, I>(request_line: &str, lines: I) -> Result<Message>
where
    I: Iterator<Item = &'a str>,
{
    // Parse the request line: METHOD URI SIP/VERSION
    let mut parts = request_line.split_whitespace();
    
    let method_str = parts.next().ok_or_else(|| {
        Error::InvalidFormat("Missing method in request line".to_string())
    })?;
    
    let uri_str = parts.next().ok_or_else(|| {
        Error::InvalidFormat("Missing URI in request line".to_string())
    })?;
    
    let version_str = parts.next().ok_or_else(|| {
        Error::InvalidFormat("Missing version in request line".to_string())
    })?;
    
    // Parse the components
    let method = Method::from_str(method_str)?;
    let uri = Uri::from_str(uri_str)?;
    let version = Version::from_str(version_str)?;
    
    // Parse headers and body
    let (headers, body) = parse_headers_and_body(lines)?;
    
    // Create the request
    let request = Request {
        method,
        uri,
        version,
        headers,
        body,
    };
    
    Ok(Message::Request(request))
}

/// Parses a SIP response from the given lines
fn parse_response<'a, I>(status_line: &str, lines: I) -> Result<Message>
where
    I: Iterator<Item = &'a str>,
{
    // Parse the status line: SIP/VERSION STATUS REASON
    let mut parts = status_line.split_whitespace();
    
    let version_str = parts.next().ok_or_else(|| {
        Error::InvalidFormat("Missing version in status line".to_string())
    })?;
    
    let status_str = parts.next().ok_or_else(|| {
        Error::InvalidFormat("Missing status code in status line".to_string())
    })?;
    
    // The reason phrase can contain spaces, so we need to join the remaining parts
    let reason = parts.collect::<Vec<&str>>().join(" ");
    
    // Parse the components
    let version = Version::from_str(version_str)?;
    let status = StatusCode::from_str(status_str)?;
    
    // Parse headers and body
    let (headers, body) = parse_headers_and_body(lines)?;
    
    // Create the response
    let response = Response {
        version,
        status,
        reason: if reason.is_empty() { None } else { Some(reason) },
        headers,
        body,
    };
    
    Ok(Message::Response(response))
}

/// Parses headers and body from the given lines
fn parse_headers_and_body<'a, I>(lines: I) -> Result<(Vec<Header>, Bytes)>
where
    I: Iterator<Item = &'a str>,
{
    let mut headers = Vec::new();
    let mut body_lines = Vec::new();
    let mut in_body = false;
    
    // Process each line
    for line in lines {
        if in_body {
            // We're in the body, collect all remaining lines
            body_lines.push(line);
        } else if line.is_empty() {
            // Empty line marks the end of headers
            in_body = true;
        } else {
            // This is a header line
            let header = parse_header(line)?;
            headers.push(header);
        }
    }
    
    // Convert body lines to bytes
    let body = if body_lines.is_empty() {
        Bytes::new()
    } else {
        Bytes::from(body_lines.join("\r\n"))
    };
    
    Ok((headers, body))
}

/// Parses a single header line
fn parse_header(line: &str) -> Result<Header> {
    // Split the line at the first colon
    let parts: Vec<&str> = line.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(Error::InvalidHeader(format!("Invalid header format: {}", line)));
    }
    
    let name_str = parts[0].trim();
    let value_str = parts[1].trim();
    
    // Parse header name and value
    let name = HeaderName::from_str(name_str)?;
    let value = HeaderValue::from_str(value_str)?;
    
    Ok(Header::new(name, value))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_request() {
        let message = "INVITE sip:bob@example.com SIP/2.0\r\n\
                      Via: SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds\r\n\
                      Max-Forwards: 70\r\n\
                      To: Bob <sip:bob@example.com>\r\n\
                      From: Alice <sip:alice@example.com>;tag=1928301774\r\n\
                      Call-ID: a84b4c76e66710@pc33.example.com\r\n\
                      CSeq: 314159 INVITE\r\n\
                      Contact: <sip:alice@pc33.example.com>\r\n\
                      Content-Type: application/sdp\r\n\
                      Content-Length: 0\r\n\
                      \r\n";
        
        let message_bytes = Bytes::from(message);
        let parsed = parse_message(&message_bytes).unwrap();
        
        assert!(parsed.is_request());
        if let Message::Request(req) = parsed {
            assert_eq!(req.method, Method::Invite);
            assert_eq!(req.uri.to_string(), "sip:bob@example.com");
            assert_eq!(req.version, Version::sip_2_0());
            assert_eq!(req.headers.len(), 9);
            
            // Check a few headers
            let call_id = req.header(&HeaderName::CallId).unwrap();
            assert_eq!(call_id.value.as_text().unwrap(), "a84b4c76e66710@pc33.example.com");
            
            let from = req.header(&HeaderName::From).unwrap();
            assert_eq!(from.value.as_text().unwrap(), "Alice <sip:alice@example.com>;tag=1928301774");
        } else {
            panic!("Expected request, got response");
        }
    }
    
    #[test]
    fn test_parse_response() {
        let message = "SIP/2.0 200 OK\r\n\
                      Via: SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds\r\n\
                      To: Bob <sip:bob@example.com>;tag=a6c85cf\r\n\
                      From: Alice <sip:alice@example.com>;tag=1928301774\r\n\
                      Call-ID: a84b4c76e66710@pc33.example.com\r\n\
                      CSeq: 314159 INVITE\r\n\
                      Contact: <sip:bob@192.168.0.2>\r\n\
                      Content-Length: 0\r\n\
                      \r\n";
        
        let message_bytes = Bytes::from(message);
        let parsed = parse_message(&message_bytes).unwrap();
        
        assert!(parsed.is_response());
        if let Message::Response(resp) = parsed {
            assert_eq!(resp.status, StatusCode::Ok);
            assert_eq!(resp.version, Version::sip_2_0());
            assert_eq!(resp.reason_phrase(), "OK");
            assert_eq!(resp.headers.len(), 7);
            
            // Check a few headers
            let to = resp.header(&HeaderName::To).unwrap();
            assert_eq!(to.value.as_text().unwrap(), "Bob <sip:bob@example.com>;tag=a6c85cf");
            
            let cseq = resp.header(&HeaderName::CSeq).unwrap();
            assert_eq!(cseq.value.as_text().unwrap(), "314159 INVITE");
        } else {
            panic!("Expected response, got request");
        }
    }
} 
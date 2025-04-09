use std::collections::HashMap;
use std::str::FromStr;

use bytes::Bytes;

use crate::error::{Error, Result};
use crate::header::{Header, HeaderName, HeaderValue};

/// A single part in a multipart MIME body
#[derive(Debug, Clone)]
pub struct MimePart {
    /// Headers for this MIME part
    pub headers: Vec<Header>,
    /// Body content of this MIME part
    pub body: Bytes,
}

impl MimePart {
    /// Create a new MIME part
    pub fn new(headers: Vec<Header>, body: impl Into<Bytes>) -> Self {
        MimePart {
            headers,
            body: body.into(),
        }
    }

    /// Get a header from this MIME part
    pub fn header(&self, name: &HeaderName) -> Option<&Header> {
        self.headers.iter().find(|h| &h.name == name)
    }

    /// Get the Content-Type of this MIME part
    pub fn content_type(&self) -> Option<&str> {
        self.header(&HeaderName::ContentType)
            .and_then(|h| h.value.as_text())
    }

    /// Get the content as a string if it's valid UTF-8
    pub fn body_str(&self) -> Result<&str> {
        std::str::from_utf8(&self.body)
            .map_err(|_| Error::InvalidFormat("MIME part contains invalid UTF-8".to_string()))
    }
}

/// A multipart MIME message body
#[derive(Debug, Clone)]
pub struct MultipartBody {
    /// The boundary string that separates MIME parts
    pub boundary: String,
    /// The individual MIME parts
    pub parts: Vec<MimePart>,
}

impl MultipartBody {
    /// Create a new multipart body with the given boundary
    pub fn new(boundary: impl Into<String>) -> Self {
        MultipartBody {
            boundary: boundary.into(),
            parts: Vec::new(),
        }
    }

    /// Add a new part to this multipart body
    pub fn add_part(&mut self, part: MimePart) {
        self.parts.push(part);
    }

    /// Parse a multipart body from raw bytes
    pub fn parse(content_type: &str, body: &Bytes) -> Result<Self> {
        // Extract boundary from content-type
        let boundary = extract_boundary(content_type)?;
        
        // Convert body to string for parsing
        let body_str = std::str::from_utf8(body)
            .map_err(|_| Error::InvalidFormat("Multipart body contains invalid UTF-8".to_string()))?;
        
        // Create the multipart container
        let mut multipart = MultipartBody::new(boundary.clone());
        
        // Construct the boundary markers
        let boundary_start = format!("--{}", boundary);
        let boundary_end = format!("--{}--", boundary);
        
        // Split the body into parts using the boundary
        let parts: Vec<&str> = body_str.split(&boundary_start).collect();
        
        // The first part is usually empty or contains preamble text, skip it
        for part in parts.iter().skip(1) {
            // Skip empty parts
            if part.trim().is_empty() {
                continue;
            }
            
            // Check if this is the end boundary
            if part.trim() == "--" || part.trim().starts_with("--\r\n") {
                continue;
            }
            
            // Find the headers/body separator (empty line)
            let part = part.trim_start_matches(|c| c == '\r' || c == '\n');
            
            if let Some(separator_pos) = find_headers_end(part) {
                let (headers_text, body_text) = part.split_at(separator_pos);
                
                // Skip the separator itself to get body
                let body_start = body_text.find(|c: char| c != '\r' && c != '\n')
                    .unwrap_or(0);
                let body_text = &body_text[body_start..];
                
                // Parse headers
                let headers = parse_headers(headers_text)?;
                
                // Create MIME part - Make sure to convert body_text to owned type (Vec<u8>)
                let body_bytes = body_text.as_bytes().to_vec();
                multipart.add_part(MimePart::new(headers, body_bytes));
            } else {
                return Err(Error::InvalidFormat("Invalid MIME part format".to_string()));
            }
        }
        
        Ok(multipart)
    }

    /// Serialize this multipart body to bytes
    pub fn to_bytes(&self) -> Bytes {
        let mut output = Vec::new();
        
        for part in &self.parts {
            // Add boundary
            output.extend_from_slice(format!("--{}\r\n", self.boundary).as_bytes());
            
            // Add headers
            for header in &part.headers {
                output.extend_from_slice(format!("{}\r\n", header).as_bytes());
            }
            
            // Add separator
            output.extend_from_slice(b"\r\n");
            
            // Add body
            output.extend_from_slice(&part.body);
            output.extend_from_slice(b"\r\n");
        }
        
        // Add final boundary
        output.extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        
        Bytes::from(output)
    }
}

/// Helper function to find where headers end and body begins
fn find_headers_end(text: &str) -> Option<usize> {
    // Look for double CRLF or double LF
    if let Some(pos) = text.find("\r\n\r\n") {
        Some(pos + 2) // +2 to include the first \r\n
    } else if let Some(pos) = text.find("\n\n") {
        Some(pos + 1) // +1 to include the first \n
    } else {
        None
    }
}

/// Helper function to parse headers from text
fn parse_headers(text: &str) -> Result<Vec<Header>> {
    let mut headers = Vec::new();
    let mut current_header: Option<(HeaderName, String)> = None;
    
    for line in text.lines() {
        // Check if this is a continuation line (starts with whitespace)
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some((_, ref mut value)) = current_header {
                // Append to current header value
                value.push(' ');
                value.push_str(line.trim());
            }
        } else if !line.is_empty() {
            // Process any current header before starting a new one
            if let Some((name, value)) = current_header.take() {
                let header_value = HeaderValue::from_str(&value)?;
                headers.push(Header::new(name, header_value));
            }
            
            // Parse new header
            if let Some(colon_pos) = line.find(':') {
                let (name, value) = line.split_at(colon_pos);
                let name = HeaderName::from_str(name.trim())?;
                let value = value[1..].trim().to_string(); // Skip the colon
                current_header = Some((name, value));
            }
        }
    }
    
    // Process final header
    if let Some((name, value)) = current_header {
        let header_value = HeaderValue::from_str(&value)?;
        headers.push(Header::new(name, header_value));
    }
    
    Ok(headers)
}

/// Extract boundary parameter from Content-Type header
fn extract_boundary(content_type: &str) -> Result<String> {
    let parts: Vec<&str> = content_type.split(';').collect();
    
    // Verify this is a multipart/* content type
    if !parts[0].trim().to_lowercase().starts_with("multipart/") {
        return Err(Error::InvalidFormat(format!(
            "Content-Type is not multipart: {}", content_type
        )));
    }
    
    // Find boundary parameter
    for part in parts.iter().skip(1) {
        let param = part.trim();
        if param.to_lowercase().starts_with("boundary=") {
            let value = param["boundary=".len()..].trim();
            // Remove quotes if present
            let boundary = value.trim_matches('"');
            return Ok(boundary.to_string());
        }
    }
    
    Err(Error::InvalidFormat(format!(
        "No boundary parameter in Content-Type: {}", content_type
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_extract_boundary() {
        // Test simple boundary
        let content_type = "multipart/mixed; boundary=boundary1";
        let boundary = extract_boundary(content_type).unwrap();
        assert_eq!(boundary, "boundary1");
        
        // Test quoted boundary
        let content_type = "multipart/mixed; boundary=\"complex-boundary-123\"";
        let boundary = extract_boundary(content_type).unwrap();
        assert_eq!(boundary, "complex-boundary-123");
        
        // Test with other parameters
        let content_type = "multipart/related; type=\"text/html\"; boundary=boundary2";
        let boundary = extract_boundary(content_type).unwrap();
        assert_eq!(boundary, "boundary2");
        
        // Test invalid content type (not multipart)
        let content_type = "text/plain; charset=utf-8";
        assert!(extract_boundary(content_type).is_err());
        
        // Test missing boundary
        let content_type = "multipart/mixed; charset=utf-8";
        assert!(extract_boundary(content_type).is_err());
    }
    
    #[test]
    fn test_parse_multipart() {
        let boundary = "boundary1";
        let content_type = format!("multipart/mixed; boundary={}", boundary);
        
        let body = format!(
            "--{boundary}\r\n\
            Content-Type: text/plain\r\n\
            Content-Length: 13\r\n\
            \r\n\
            Hello, world!\r\n\
            --{boundary}\r\n\
            Content-Type: application/sdp\r\n\
            Content-Length: 21\r\n\
            \r\n\
            v=0\r\n\
            o=- 123 456 IN IP4\r\n\
            --{boundary}--\r\n", 
            boundary = boundary
        );
        
        let body_bytes = Bytes::from(body);
        let multipart = MultipartBody::parse(&content_type, &body_bytes).unwrap();
        
        assert_eq!(multipart.boundary, boundary);
        assert_eq!(multipart.parts.len(), 2);
        
        // Check first part
        let part1 = &multipart.parts[0];
        assert_eq!(part1.content_type().unwrap(), "text/plain");
        assert_eq!(part1.body_str().unwrap(), "Hello, world!");
        
        // Check second part
        let part2 = &multipart.parts[1];
        assert_eq!(part2.content_type().unwrap(), "application/sdp");
        assert_eq!(part2.body_str().unwrap(), "v=0\r\no=- 123 456 IN IP4");
    }
    
    #[test]
    fn test_serialize_multipart() {
        let boundary = "boundary1";
        let mut multipart = MultipartBody::new(boundary);
        
        // Add first part
        let mut part1_headers = Vec::new();
        part1_headers.push(Header::text(HeaderName::ContentType, "text/plain"));
        let part1 = MimePart::new(part1_headers, "Hello, world!");
        multipart.add_part(part1);
        
        // Add second part
        let mut part2_headers = Vec::new();
        part2_headers.push(Header::text(HeaderName::ContentType, "application/sdp"));
        let part2 = MimePart::new(part2_headers, "v=0\r\no=- 123 456 IN IP4");
        multipart.add_part(part2);
        
        // Serialize
        let bytes = multipart.to_bytes();
        let text = std::str::from_utf8(&bytes).unwrap();
        
        // Verify parts are included
        assert!(text.contains("--boundary1\r\n"));
        assert!(text.contains("Content-Type: text/plain\r\n"));
        assert!(text.contains("Hello, world!"));
        assert!(text.contains("Content-Type: application/sdp\r\n"));
        assert!(text.contains("v=0\r\no=- 123 456 IN IP4"));
        assert!(text.contains("--boundary1--\r\n"));
        
        // Parse the serialized body back
        let content_type = format!("multipart/mixed; boundary={}", boundary);
        let parsed = MultipartBody::parse(&content_type, &bytes).unwrap();
        
        assert_eq!(parsed.parts.len(), 2);
        assert_eq!(parsed.parts[0].body_str().unwrap(), "Hello, world!");
        assert_eq!(parsed.parts[1].body_str().unwrap(), "v=0\r\no=- 123 456 IN IP4");
    }
} 
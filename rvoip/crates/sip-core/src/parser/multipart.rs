use std::collections::HashMap;
use std::str::{FromStr, from_utf8_lossy};

use nom::{
    branch::alt,
    bytes::complete::{tag, take_till, take_until, take_while, take_while1, crlf as bytes_crlf},
    character::complete::{char, line_ending, multispace0, space0, space1},
    combinator::{map, map_res, opt, recognize, value, verify, eof},
    multi::{many0, many1, many_till, separated_list0, separated_list1},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    IResult,
};
use bytes::Bytes;

use crate::error::{Error, Result};
use crate::header::{Header, HeaderName, HeaderValue}; // Use core Header type
use crate::types::SdpSession; // Placeholder for parsed SDP
use super::utils::{crlf};
use super::headers::header_parser; // Use the header parser

/// Represents potentially parsed body content types.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedBody {
    Sdp(SdpSession),
    Text(String),
    Other(Bytes), // Fallback for unknown/binary
}

/// A single part in a multipart MIME body
#[derive(Debug, Clone, PartialEq)]
pub struct MimePart {
    /// Headers for this part
    pub headers: Vec<Header>,
    /// Raw content of this part
    pub raw_content: Bytes,
    /// Optionally parsed content based on Content-Type
    pub parsed_content: Option<ParsedBody>, 
}

impl MimePart {
    /// Create a new MIME part
    pub fn new() -> Self {
        Self {
            headers: Vec::new(),
            raw_content: Bytes::new(),
            parsed_content: None, 
        }
    }
    
    /// Get the first header value by name, case-insensitive
    pub fn get_header(&self, name: &HeaderName) -> Option<&HeaderValue> {
        self.headers.iter().find(|h| &h.name == name).map(|h| &h.value)
    }

    /// Get the first header value as text by name, case-insensitive
    pub fn get_header_text(&self, name: &HeaderName) -> Option<&str> {
        self.get_header(name).and_then(|v| v.as_text())
    }
    
    /// Get the content-type of this part as text
    pub fn content_type(&self) -> Option<&str> {
        self.get_header_text(&HeaderName::ContentType)
    }
    
    /// Get the content-disposition of this part as text
    pub fn content_disposition(&self) -> Option<&str> {
        self.get_header_text(&HeaderName::ContentDisposition)
    }

    /// Get the raw content as lossy UTF-8 string
    pub fn content_as_str_lossy(&self) -> std::borrow::Cow<str> {
        from_utf8_lossy(&self.raw_content)
    }
}

/// A parsed multipart MIME body
#[derive(Debug, Clone, PartialEq)]
pub struct MultipartBody {
    /// The boundary string that separates parts
    pub boundary: String,
    /// The parts in this multipart body
    pub parts: Vec<MimePart>,
}

impl MultipartBody {
    /// Create a new multipart body with the given boundary
    pub fn new(boundary: impl Into<String>) -> Self {
        Self {
            boundary: boundary.into(),
            parts: Vec::new(),
        }
    }
    
    /// Add a part to this multipart body
    pub fn add_part(&mut self, part: MimePart) {
        self.parts.push(part);
    }
    
    /// Find the first part by its content-type (exact match on type/subtype)
    pub fn find_by_content_type(&self, content_type: &str) -> Option<&MimePart> {
        self.parts.iter().find(|part| {
            part.content_type()
                .map(|ct| ct.trim().starts_with(content_type))
                .unwrap_or(false)
        })
    }
    
    /// Get the first SDP part if present
    pub fn sdp_part(&self) -> Option<&MimePart> {
        self.find_by_content_type("application/sdp")
    }
    
    /// Get the parsed SDP content if present
    pub fn sdp_session(&self) -> Option<&SdpSession> {
        self.sdp_part().and_then(|part| match &part.parsed_content {
            Some(ParsedBody::Sdp(session)) => Some(session),
            _ => None,
        })
    }

    /// Get the raw SDP content as a string if present
     pub fn sdp_content_raw(&self) -> Option<&str> {
        self.sdp_part().and_then(|part| std::str::from_utf8(&part.raw_content).ok())
    }
}

/// Parse a multipart body with the given boundary
pub fn parse_multipart(content: &[u8], boundary: &str) -> Result<MultipartBody> {
    let full_boundary = format!("--{}", boundary);
    let end_boundary = format!("--{}--", boundary);
    
    // Use slice directly, avoid lossy conversion if possible
    match multipart_parser(content, &full_boundary, &end_boundary) {
        Ok((_, mut body)) => {
            body.boundary = boundary.to_string();
            Ok(body)
        },
        // Provide more context on error
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => Err(Error::Parser(format!(
            "Failed to parse multipart body near '{}': {:?}", 
            from_utf8_lossy(e.input),
            e.code
        ))),
        Err(nom::Err::Incomplete(needed)) => Err(Error::Parser(format!(
            "Incomplete multipart body, needed: {:?}", needed
        ))),
    }
}

// Helper to find the next occurrence of boundary or end_boundary
fn find_next_boundary<'a>(input: &'a [u8], boundary: &[u8], end_boundary: &[u8]) -> Option<(usize, usize)> {
    input.windows(boundary.len()).position(|window| window == boundary)
        .map(|pos| (pos, boundary.len()))
        .or_else(|| {
            input.windows(end_boundary.len()).position(|window| window == end_boundary)
                 .map(|pos| (pos, end_boundary.len()))
        })
}

/// nom parser for a multipart body using byte slices
fn multipart_parser<'a>(mut input: &'a [u8], boundary: &str, end_boundary: &str) -> IResult<&'a [u8], MultipartBody> {
    let boundary_bytes = boundary.as_bytes();
    let end_boundary_bytes = end_boundary.as_bytes();

    // Skip preamble: Find the first boundary
    if let Some((pos, len)) = find_next_boundary(input, boundary_bytes, end_boundary_bytes) {
        if pos > 0 { // Only advance if there's preamble
           input = &input[pos..];
        }
    } else {
         // No boundary found at all
        return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::TakeUntil)));
    }

    // Create the multipart body
    let mut body = MultipartBody::new(boundary.trim_start_matches("--"));

    loop {
        // Expect the boundary prefix
        let (i, _) = tag(boundary_bytes)(input)?;
        input = i;

        // Check for the end boundary immediately after the normal boundary
        if let Ok((i, _)) = tag::<_, _, nom::error::Error<&[u8]>>(b"--")(input) {
             // This is the final boundary marker
            let (i, _) = alt((bytes_crlf, eof))(i)?; // Consume trailing CRLF or EOF, use bytes_crlf
            input = i;
            break; // End of multipart body
        }

        // Expect CRLF after boundary
        let (i, _) = bytes_crlf(input)?;// use bytes_crlf
        input = i;

        // Parse headers for the part
        let mut headers = Vec::new();
        let mut header_input = input;
        loop {
            match header_parser(from_utf8_lossy(header_input).as_ref()) { // Use lossy for header parsing
                Ok((rest_str, hdr)) => {
                     let consumed_bytes = header_input.len() - rest_str.len();
                     headers.push(hdr);
                     header_input = &header_input[consumed_bytes..];
                },
                Err(_) => break, // Error or no more headers
            }
            // Check for the empty line (end of headers)
            if let Ok((rest_bytes, _)) = bytes_crlf(header_input) {
                header_input = rest_bytes;
                break;
            }
        }
        input = header_input; // Update main input position after headers

        // Parse content until the next boundary
        let (content_bytes, next_boundary_len) = 
            match find_next_boundary(input, boundary_bytes, end_boundary_bytes) {
                Some((pos, len)) => { 
                    // Need to potentially backtrack CRLF before the boundary
                    let content_end = if pos >= 2 && &input[pos-2..pos] == b"\r\n" {
                        pos - 2
                    } else if pos >= 1 && input[pos-1] == b'\n' {
                        pos - 1
                    } else {
                        pos
                    };
                    (&input[..content_end], len)
                }
                None => return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::TakeUntil))), // Missing closing boundary
            };

        let mut part = MimePart::new();
        part.headers = headers;
        part.raw_content = Bytes::copy_from_slice(content_bytes);

        // Attempt to parse known content types
        if let Some(content_type) = part.content_type() {
            if content_type.trim().starts_with("application/sdp") {
                 match crate::sdp::parser::parse_sdp(&part.raw_content) {
                    Ok(sdp_session) => {
                        part.parsed_content = Some(ParsedBody::Sdp(sdp_session));
                    }
                    Err(e) => {
                        // Failed to parse SDP, maybe log it?
                        // Keep raw content, parsed_content remains None
                        println!("Multipart Parser: Failed to parse SDP content: {}", e);
                        part.parsed_content = Some(ParsedBody::Other(part.raw_content.clone())); // Store as Other if SDP parsing fails
                    }
                }
            } else if content_type.trim().starts_with("text/") {
                // Attempt to parse as text
                match String::from_utf8(part.raw_content.to_vec()) {
                    Ok(text) => part.parsed_content = Some(ParsedBody::Text(text)),
                    Err(_) => part.parsed_content = Some(ParsedBody::Other(part.raw_content.clone())),
                }
            }
            else {
                 // Default to Other/Binary
                 part.parsed_content = Some(ParsedBody::Other(part.raw_content.clone()));
            }
        } else {
             // No content type, treat as Other/Binary
             part.parsed_content = Some(ParsedBody::Other(part.raw_content.clone()));
        }
        
        body.add_part(part);

        // Advance input past the content and the boundary/end_boundary marker found
        input = &input[content_bytes.len()..]; // Move past content
        if input.len() >= next_boundary_len {
            input = &input[next_boundary_len..]; // Move past boundary marker, ready for next loop/end check
        } else {
             // Should not happen if find_next_boundary succeeded
             return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Complete)));
        } 
    }

    Ok((input, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    // #[test]
    // fn test_parse_simple_multipart() { ... }
    // #[test]
    // fn test_parse_complex_multipart() { ... }
} 
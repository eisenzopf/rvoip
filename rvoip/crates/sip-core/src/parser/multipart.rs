// Parser for multipart MIME bodies (RFC 2046)

use std::collections::HashMap;
use std::str;
use std::borrow::Cow;

use nom::{
    branch::alt,
    bytes::complete::{tag, take_until},
    character::complete::{crlf, space0, space1},
    combinator::{map, map_res, opt, recognize, eof},
    error::{Error as NomError, ErrorKind, ParseError},
    multi::{many0},
    sequence::{pair, preceded},
    IResult,
};
use bytes::Bytes;

use crate::error::{Error, Result};
use crate::header::{Header, HeaderName, HeaderValue};
use crate::types::sdp::SdpSession; 
// Import the structures from the types module
use crate::types::multipart::{MultipartBody, MimePart, ParsedBody};
use crate::parser::ParseResult;
use crate::parser::message::parse_known_header; // Use the main header dispatcher
use crate::parser::whitespace::crlf as parse_crlf; // Alias to avoid clash


/// Parses headers for a MIME part until a blank line (CRLF) is encountered.
/// Handles line folding.
fn parse_part_headers(input: &[u8]) -> IResult<&[u8], Vec<Header>> {
    let mut headers = Vec::new();
    let mut current_input = input;
    let mut current_header_name: Option<HeaderName> = None;
    let mut current_header_value_bytes: Vec<u8> = Vec::new();

    loop {
        // Peek for the end of headers (CRLF)
        if current_input.starts_with(b"\r\n") {
            current_input = &current_input[2..];
            break;
        } else if current_input.starts_with(b"\n") {
            current_input = &current_input[1..];
            break;
        } else if current_input.is_empty() {
             // Reached end of input unexpectedly before blank line
             return Err(nom::Err::Error(NomError::from_error_kind(input, ErrorKind::CrLf)));
        }

        // Read one logical line (including folding)
        // Find the next non-folded line break
        let mut line_end = 0;
        let mut next_line_start = 0;
        let mut current_pos = 0;
        let mut found_line_end = false;
        while current_pos < current_input.len() {
            if let Some(idx) = current_input[current_pos..].iter().position(|&b| b == b'\n') {
                let line_break_pos = current_pos + idx;
                let cr = line_break_pos > 0 && current_input[line_break_pos - 1] == b'\r';
                line_end = if cr { line_break_pos - 1 } else { line_break_pos };
                next_line_start = line_break_pos + 1;

                // Check for folding
                if next_line_start < current_input.len() && 
                   (current_input[next_line_start] == b' ' || current_input[next_line_start] == b'\t') {
                    // It folds, continue search from after the newline
                    current_pos = next_line_start;
                    continue;
                } else {
                    // Not folded, this is the end of our logical line
                    found_line_end = true;
                    break;
                }
            } else {
                // No more newlines found, treat rest of input as the line
                line_end = current_input.len();
                next_line_start = current_input.len();
                found_line_end = true; // Or should this be an error if no CRLF?
                break;
            }
        }
        
        if !found_line_end {
            // Should be impossible if input isn't empty, but handle anyway
            return Err(nom::Err::Error(NomError::from_error_kind(input, ErrorKind::CrLf)));
        }

        let logical_line = &current_input[..line_end];
        let remaining_input_after_line = &current_input[next_line_start..];

        // Process the logical line
        // Unfold (replace CRLF LWS with SP)
        let unfolded_line = logical_line.split(|b| *b == b'\r' || *b == b'\n')
                                    .enumerate()
                                    .map(|(i, segment)| {
                                        if i > 0 {
                                            // For segments after the first, trim leading whitespace
                                            segment.iter().skip_while(|&&b| b == b' ' || b == b'\t').cloned().collect::<Vec<u8>>()
                                        } else {
                                            segment.to_vec()
                                        }
                                    })
                                    .collect::<Vec<Vec<u8>>>()
                                    .join(&b" "[..]);

        // Check if it's a new header or continuation (already handled by unfold?) No, unfold handles value folding.
        // Need to check first char of *original* logical line.
        if logical_line.starts_with(b" ") || logical_line.starts_with(b"\t") {
            // This case should be handled by the unfolding logic above. If we reach here with leading space, 
            // it implies a folded line without a preceding header, which is invalid.
            // We could ignore it or return an error.
            // For robustness, maybe ignore and just advance.
             current_input = remaining_input_after_line;
             continue;
        } else {
             // Process previous header (if any)
            if let Some(name) = current_header_name.take() {
                let value_bytes_trimmed = crate::parser::message::trim_bytes(&current_header_value_bytes);
                let parsed_value = parse_known_header(&name, value_bytes_trimmed)
                                       .unwrap_or_else(|_| HeaderValue::Raw(value_bytes_trimmed.to_vec()));
                headers.push(Header::new(name, parsed_value));
            }
            current_header_value_bytes.clear();

            // Parse new header line (name: value)
            if let Some(colon_pos) = unfolded_line.iter().position(|&b| b == b':') {
                let name_bytes = crate::parser::message::trim_bytes(&unfolded_line[..colon_pos]);
                current_header_value_bytes.extend_from_slice(&unfolded_line[colon_pos + 1..]); 
                
                use std::str::FromStr;
                match str::from_utf8(name_bytes) {
                    Ok(name_str) => {
                         current_header_name = Some(HeaderName::from_str(name_str).unwrap_or_else(|_| HeaderName::Other(name_str.to_string())));
                    }
                    Err(_) => { 
                        current_header_name = Some(HeaderName::Other(String::from_utf8_lossy(name_bytes).into_owned()));
                    }
                };
            } // Ignore lines without a colon
        }
        current_input = remaining_input_after_line;
    }

    // Process the very last header
    if let Some(name) = current_header_name.take() {
        let value_bytes_trimmed = crate::parser::message::trim_bytes(&current_header_value_bytes);
        let parsed_value = parse_known_header(&name, value_bytes_trimmed)
                               .unwrap_or_else(|_| HeaderValue::Raw(value_bytes_trimmed.to_vec()));
        headers.push(Header::new(name, parsed_value));
    }

    Ok((current_input, headers))
}


/// Tries to parse the raw content bytes based on Content-Type header.
fn parse_part_content(headers: &[Header], raw_content: &Bytes) -> Option<ParsedBody> {
     let content_type = headers.iter()
        .find(|h| h.name == HeaderName::ContentType)
        .and_then(|h| match &h.value { // Match HeaderValue
            HeaderValue::Raw(bytes) => str::from_utf8(bytes).ok(), // Use raw bytes
            _ => None
        });

    if let Some(ct) = content_type {
        if ct.trim().starts_with("application/sdp") {
             match crate::sdp::parser::parse_sdp(raw_content) {
                Ok(sdp_session) => Some(ParsedBody::Sdp(sdp_session)),
                Err(e) => {
                    // Failed to parse SDP, maybe log it?
                    // Keep raw content, return Other
                    println!("Multipart Parser: Failed to parse SDP content: {}", e);
                    Some(ParsedBody::Other(raw_content.clone()))
                }
            }
        } else if ct.trim().starts_with("text/") {
            match String::from_utf8(raw_content.to_vec()) {
                Ok(text) => Some(ParsedBody::Text(text)),
                Err(_) => Some(ParsedBody::Other(raw_content.clone())),
            }
        }
        else {
             Some(ParsedBody::Other(raw_content.clone()))
        }
    } else {
         Some(ParsedBody::Other(raw_content.clone()))
    }
}

/// Helper to find the next occurrence of boundary or end_boundary
fn find_next_boundary<'a>(input: &'a [u8], boundary: &[u8], end_boundary: &[u8]) -> Option<(usize, usize, bool)> {
    input.windows(boundary.len()).position(|window| window == boundary)
        .map(|pos| (pos, boundary.len(), false)) // Found normal boundary
        .or_else(|| {
            input.windows(end_boundary.len()).position(|window| window == end_boundary)
                 .map(|pos| (pos, end_boundary.len(), true)) // Found end boundary
        })
}

/// nom parser for a multipart body using byte slices
fn multipart_parser<'a>(mut input: &'a [u8], boundary: &str, end_boundary: &str) -> IResult<&'a [u8], MultipartBody> {
    let boundary_bytes = boundary.as_bytes();
    let end_boundary_bytes = end_boundary.as_bytes();

    // Skip preamble: Find the first boundary
    if let Some((pos, _len, is_end)) = find_next_boundary(input, boundary_bytes, end_boundary_bytes) {
        if is_end { // Found end boundary immediately
             return Err(nom::Err::Error(NomError::from_error_kind(input, ErrorKind::Tag)));
        }
        input = &input[pos..]; // Move input to the start of the boundary
    } else {
        return Err(nom::Err::Error(NomError::from_error_kind(input, ErrorKind::TakeUntil)));
    }

    let mut body = MultipartBody::new(boundary.trim_start_matches("--"));

    loop {
        // Consume the boundary marker
        let (i, _) = tag(boundary_bytes)(input)?;
        input = i;

        // Expect CRLF after boundary
        let (i, _) = parse_crlf(input)?;
        input = i;

        // Parse headers for the part
        let (i, headers) = parse_part_headers(input)?;
        input = i;

        // Find the next boundary marker
        let (content_bytes, boundary_pos, boundary_len, is_end_boundary) = 
            match find_next_boundary(input, boundary_bytes, end_boundary_bytes) {
                Some((pos, len, is_end)) => {
                    // Backtrack CRLF before the boundary
                    let content_end = if pos >= 2 && &input[pos-2..pos] == b"\r\n" { pos - 2 }
                                    else if pos >= 1 && input[pos-1] == b'\n' { pos - 1 }
                                    else { pos };
                    (&input[..content_end], pos, len, is_end)
                }
                None => return Err(nom::Err::Error(NomError::from_error_kind(input, ErrorKind::TakeUntil))),
            };

        let raw_content = Bytes::copy_from_slice(content_bytes);
        let parsed_content = parse_part_content(&headers, &raw_content);

        let mut part = MimePart::new();
        part.headers = headers;
        part.raw_content = raw_content;
        part.parsed_content = parsed_content;
        
        body.add_part(part);

        // Advance input past the content
        input = &input[content_bytes.len()..]; 

        // Check if the boundary we found was the end boundary
        if is_end_boundary {
            // Consume the end boundary marker itself (e.g., "--boundary--")
            let (i, _) = tag(end_boundary_bytes)(input)?;
            // Consume trailing CRLF or EOF
            let (i, _) = alt((parse_crlf, eof))(i)?;
            input = i;
            break; // Exit loop, parsing is complete
        } else {
            // It was a normal boundary, consume it to prepare for next part
             let (i, _) = tag(boundary_bytes)(input)?;
            input = i;
            // Loop continues to parse next part
        }
    } // End main loop

    Ok((input, body))
}

/// Public entry point to parse a multipart body
pub fn parse_multipart(content: &[u8], boundary: &str) -> Result<MultipartBody> {
    // Construct the boundary markers expected by the parser
    let full_boundary = format!("--{}", boundary);
    let end_boundary = format!("--{}--", boundary);
    
    // Call the internal nom parser
    match all_consuming(|i| multipart_parser(i, &full_boundary, &end_boundary))(content) {
        Ok((_, body)) => Ok(body),
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => {
            let offset = content.len() - e.input.len(); // Calculate offset
            Err(Error::ParsingError { 
                message: format!("Failed to parse multipart body near offset {}: {:?}", offset, e.code),
                source: None 
            })
        },
        Err(nom::Err::Incomplete(needed)) => {
            Err(Error::ParsingError{ message: format!("Incomplete multipart body: Needed {:?}", needed), source: None })
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    // #[test]
    // fn test_parse_simple_multipart() { ... }
    // #[test]
    // fn test_parse_complex_multipart() { ... }
} 
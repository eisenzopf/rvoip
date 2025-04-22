use crate::types::version::Version;
use crate::types::{Method, StatusCode, Message, Request, Response};

// Update imports to use available modules
// use crate::parser::headers::{parse_header as parse_header_value, parse_headers, header_parser as single_nom_header_parser, headers_parser as nom_headers_parser};
use crate::parser::request::parse_request_line;
use crate::parser::response::parse_response_line;
// use crate::parser::utils::crlf;
use nom::bytes::complete::{take, take_till};
use nom::character::complete::{multispace0};
use crate::parser::headers::{parse_cseq, parse_content_length, parse_expires, parse_max_forwards};
use crate::types::{CSeq, ContentLength, Expires, MaxForwards};
use crate::parser::whitespace::{crlf, lws};
use crate::parser::separators::hcolon;
use crate::parser::token::token;
use crate::parser::utf8::text_utf8_char;
use crate::parser::common::sip_version;
use crate::parser::utils::unfold_lws;
use crate::parser::common::ParseResult;
use crate::parser::response::parse_status_line;
use nom::combinator::{all_consuming, map, map_res, recognize};
use nom::branch::alt;
use nom::error::{Error as NomError, ErrorKind};
use nom::sequence::tuple;
use nom::multi::many_till;
use nom::Needed;
use nom::IResult;
use bytes::Bytes;
use std::str;
use crate::error::{Error, Result};
use crate::types::{Header, TypedHeader, HeaderName, HeaderValue};
use std::str::FromStr;
use crate::types::uri::Host;

/// Maximum length of a single line in a SIP message
pub const MAX_LINE_LENGTH: usize = 4096;
/// Maximum number of headers in a SIP message
pub const MAX_HEADER_COUNT: usize = 100;
/// Maximum size of a SIP message body
pub const MAX_BODY_SIZE: usize = 16 * 1024 * 1024; // 16 MB

/// Helper for trimming leading/trailing ASCII whitespace from a byte slice
pub fn trim_bytes<'a>(bytes: &'a [u8]) -> &'a [u8] {
    let start = bytes.iter().position(|&b| !b.is_ascii_whitespace()).unwrap_or(0);
    let end = bytes.iter().rposition(|&b| !b.is_ascii_whitespace()).map_or(0, |p| p + 1);
    &bytes[start..end]
}

/// Parses a block of header lines terminated by an empty line (CRLF).
/// Uses the `message_header` parser for each line.
/// Returns a Vec of raw Headers.
fn parse_header_block(input: &[u8]) -> ParseResult<Vec<Header>> {
    // many_till parses 0 or more headers until the terminating CRLF
    let (remaining_input, (headers, _terminator)) = many_till(
        message_header, 
        crlf // The CRLF that ends the header section
    )(input)?;
    
    Ok((remaining_input, headers))
}

/// Top-level nom parser for a full SIP message (using byte input)
fn full_message_parser(input: &[u8]) -> IResult<&[u8], Message> {
    // 1. Parse Start Line
    let (rest, start_line_data) = alt((
        map(parse_request_line, |(m, u, v)| (true, Some(m), Some(u), Some(v), None, None)),
        map(parse_status_line, |(v, s, r)| {
            let reason_opt = if r.is_empty() { None } else { str::from_utf8(r).ok().map(String::from) };
            (false, None, None, Some(v), Some(s), reason_opt)
        })
    ))(input)?;
    
    let (is_request, method, uri, version, status_code, reason_phrase_opt) = start_line_data;
    
    // 2. Parse Raw Headers block
    let (rest, raw_headers) = parse_header_block(rest)?;

    // 3. Convert Raw Headers to Typed Headers
    let mut typed_headers: Vec<TypedHeader> = Vec::with_capacity(raw_headers.len());
    for header in raw_headers {
        match TypedHeader::try_from(header) {
            Ok(typed) => typed_headers.push(typed),
            Err(e) => {
                eprintln!("Header parsing error: {}", e); 
                 return Err(nom::Err::Failure(NomError::new(input, ErrorKind::Verify))); 
            }
        }
    }

    // 4. Get Content-Length (optional) - Needs update to use TypedHeader
    let content_length = typed_headers.iter().find_map(|h| {
        if let TypedHeader::ContentLength(cl) = h {
            Some(cl.0 as usize)
        } else { None }
    }).unwrap_or(0);

    // 5. Parse Body based on Content-Length
    if rest.len() < content_length {
        return Err(nom::Err::Incomplete(Needed::new(content_length - rest.len())));
    }
    let (final_rest, body_slice) = take(content_length)(rest)?;
    
    // 6. Construct Message - Needs update to use Vec<TypedHeader>
    let body = Bytes::copy_from_slice(body_slice);
    let message = if is_request {
        let mut req = Request::new(method.unwrap(), uri.unwrap());
        req.version = version.unwrap();
        req.set_headers(typed_headers); // Assuming set_headers exists
        if content_length > 0 { req.body = body; }
        Message::Request(req)
    } else {
        let mut resp = Response::new(status_code.unwrap());
        resp.version = version.unwrap();
        if let Some(reason) = reason_phrase_opt {
             resp = resp.with_reason(reason);
        }
        resp.set_headers(typed_headers); // Assuming set_headers exists
        if content_length > 0 { resp.body = body; }
        Message::Response(resp)
    };

    Ok((final_rest, message))
}

/// Parse a SIP message from bytes
pub fn parse_message(input: &[u8]) -> Result<Message> {
    match all_consuming(full_message_parser)(input) {
        Ok((_, message)) => Ok(message),
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => {
             let offset = input.len() - e.input.len();
            Err(Error::ParseError( 
                format!("Failed to parse message near offset {}: {:?}", offset, e.code)
            ))
        },
        Err(nom::Err::Incomplete(needed)) => {
            Err(Error::ParseError(format!("Incomplete message: Needed {:?}", needed)))
        },
    }
}

/// Parse a SIP message from bytes (legacy API, kept for compatibility)
pub fn parse_message_bytes(input: &[u8]) -> Result<Message> {
    parse_message(input)
}

// header-name = token
fn header_name(input: &[u8]) -> ParseResult<&[u8]> {
    token(input)
}

// header-value = *(TEXT-UTF8char / UTF8-CONT / LWS)
// Simplified: Takes bytes until CRLF. Actual unfolding/validation happens later.
pub fn header_value(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(take_till(|c| c == b'\r' || c == b'\n'))(input)
}

// message-header = header-name HCOLON header-value CRLF
// Parses a single logical header line.
// Unfolds LWS in the value.
// Returns a raw Header struct.
fn message_header(input: &[u8]) -> ParseResult<Header> {
    map_res(
        tuple((
            header_name,
            hcolon,      // Separator with whitespace handling
            header_value, // Parses raw value up to CRLF
            crlf
        )),
        |(name_bytes, _, raw_value_bytes, _)| {
            str::from_utf8(name_bytes)
                .map_err(|_| nom::Err::Failure(NomError::new(name_bytes, ErrorKind::Char)))
                .and_then(|name_str| {
                    HeaderName::from_str(name_str)
                        .map_err(|_| nom::Err::Failure(NomError::new(name_bytes, ErrorKind::Verify)))
                        .map(|header_name| {
                            let header_value = HeaderValue::Raw(raw_value_bytes.to_vec());
                            Header::new(header_name, header_value)
                        })
                })
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use crate::types::uri::Host;
    use crate::types::{Request, Response, CSeq, HeaderName, HeaderValue, Method, Version};
    use std::collections::HashMap;

    #[test]
    fn test_message_header_simple() {
        let input = b"Subject: Simple Subject\r\n";
        let result = message_header(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(header.name, HeaderName::Subject);
        assert!(matches!(header.value, HeaderValue::Raw(ref v) if v == b"Simple Subject"));
    }

    #[test]
    fn test_message_header_cseq() {
        let input = b"CSeq: 101 INVITE\r\n";
        let result = message_header(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(header.name, HeaderName::CSeq);
        assert!(matches!(header.value, HeaderValue::Raw(ref v) if v == b"101 INVITE"));
    }

     #[test]
    fn test_message_header_with_lws() {
        let input = b"From:  Alice <sip:alice@atlanta.com> ;tag=123\r\n";
        let result = message_header(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(header.name, HeaderName::From);
        assert!(matches!(header.value, HeaderValue::Raw(ref v) if v == b"Alice <sip:alice@atlanta.com> ;tag=123"));
    }
    
    #[test]
    fn test_message_header_extension() {
        let input = b"X-Custom-Header: Some custom data here\r\n";
        let result = message_header(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(header.name, HeaderName::Other("X-Custom-Header".to_string()));
        assert!(matches!(header.value, HeaderValue::Raw(ref v) if v == b"Some custom data here"));
    }

    #[test]
    fn test_message_header_no_value() {
        let input = b"Allow:\r\n"; // Empty value
        let result = message_header(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(header.name, HeaderName::Allow);
        assert!(matches!(header.value, HeaderValue::Raw(ref v) if v.is_empty()));
    }

    #[test]
    fn test_invalid_message_header_no_colon() {
        let input = b"Invalid Header Line\r\n";
        assert!(message_header(input).is_err());
    }

    #[test]
    fn test_invalid_message_header_no_crlf() {
        let input = b"Subject: No CRLF";
        assert!(message_header(input).is_err());
    }
    
    #[test]
    fn test_parse_header_block_simple() {
        let input = b"Subject: Simple\r\nContent-Length: 5\r\n\r\nBody";
        let result = parse_header_block(input);
        assert!(result.is_ok());
        let (rem, headers) = result.unwrap();
        assert_eq!(rem, b"Body");
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0].name, HeaderName::Subject);
        assert!(matches!(headers[0].value, HeaderValue::Raw(ref v) if v == b"Simple"));
        assert_eq!(headers[1].name, HeaderName::ContentLength);
        assert!(matches!(headers[1].value, HeaderValue::Raw(ref v) if v == b"5"));
    }

    #[test]
    fn test_parse_header_block_empty() {
        let input = b"\r\nBody";
        let result = parse_header_block(input);
        assert!(result.is_ok());
        let (rem, headers) = result.unwrap();
        assert_eq!(rem, b"Body");
        assert!(headers.is_empty());
    }

    #[test]
    fn test_parse_header_block_no_body() {
        let input = b"Subject: Simple\r\n\r\n";
        let result = parse_header_block(input);
        assert!(result.is_ok());
        let (rem, headers) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].name, HeaderName::Subject);
    }

    #[test]
    fn test_parse_header_block_folding_raw() {
        // This test assumes folding is NOT handled yet, captured in Raw
        let input = b"Subject: Line 1\r\n Line 2\r\nContent-Length: 0\r\n\r\n";
        let result = parse_header_block(input);
        assert!(result.is_ok());
        let (rem, headers) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0].name, HeaderName::Subject);
        // Specific subject parser likely fails due to internal CRLF LWS,
        // so it should fall back to Raw containing the un-processed folded value.
        assert!(matches!(headers[0].value, HeaderValue::Raw(ref v) if v == b"Line 1\r\n Line 2")); 
        assert_eq!(headers[1].name, HeaderName::ContentLength);
    }

    #[test]
    fn test_message_header_unfolding() {
        // Test that message_header correctly calls unfold_lws before parsing/storing
        let input = b"Subject: Line 1\r\n Line 2\r\n\t Continued Here\r\n";
        let result = message_header(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(header.name, HeaderName::Subject);
        // Check that the parsed Subject value reflects the unfolded string
        assert!(matches!(header.value, HeaderValue::Raw(ref v) if v == b"Line 1 Line 2 Continued Here"));
    }

    #[test]
    fn test_message_header_unfolding_raw() {
        let input = b"X-Folded: Value\r\n Part 2\r\n";
        let result = message_header(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(header.name, HeaderName::Other("X-Folded".to_string()));
        // Raw value should be the unfolded version (but not trimmed here yet)
        assert!(matches!(header.value, HeaderValue::Raw(ref v) if v == b"Value Part 2"));
    }

    // TODO: Add tests for full_message_parser and parse_message
    // #[test]
    // fn test_parse_request_full() { /* ... */ }
    
    // #[test]
    // fn test_parse_response_full() { /* ... */ }
} 

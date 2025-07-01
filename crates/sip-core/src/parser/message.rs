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

/// Mode for parsing SIP messages
/// - Strict: Rejects messages that don't conform to RFC 3261 strictly
/// - Lenient: Attempts to recover from common errors like mismatched Content-Length
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseMode {
    Strict,
    Lenient,
}

impl Default for ParseMode {
    fn default() -> Self {
        ParseMode::Lenient
    }
}

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

/// Parse a header value with support for line folding according to RFC 3261 Section 7.3.1.
/// 
/// Line folding occurs when a line ending CRLF is followed by whitespace (SP or HTAB).
/// According to the RFC:
/// "A line break followed by folding whitespace is equivalent to a SP character."
/// 
/// This parser correctly handles these folded lines and returns a slice that includes
/// the entire logical header value across multiple physical lines.
pub fn header_value_better(input: &[u8]) -> ParseResult<&[u8]> {
    // Find the next non-continuation line ending (i.e., CRLF not followed by SP/HTAB)
    let mut i = 0;
    let len = input.len();
    let start = input;
    
    while i + 1 < len {
        // Look for CRLF
        if input[i] == b'\r' && input[i + 1] == b'\n' {
            // Check if followed by whitespace (folding)
            if i + 2 < len && (input[i + 2] == b' ' || input[i + 2] == b'\t') {
                // This is a folded line, continue scanning
                i += 2; // Skip the CR LF
                
                // Skip all whitespace in this folded line
                while i < len && (input[i] == b' ' || input[i] == b'\t') {
                    i += 1;
                }
            } else {
                // This is a non-folded line ending, end of header value
                break;
            }
        } else {
            // Regular character
            i += 1;
        }
    }
    
    // Return the span from start to the end of the value
    Ok((&input[i..], &start[..i]))
}

/// Top-level nom parser for a full SIP message (using byte input)
/// 
/// This parser follows a resilient approach to header parsing:
/// 1. It will parse the start line (request-line or status-line)
/// 2. It will parse all headers and attempt to convert them to typed headers
/// 3. If a header can't be parsed into its typed form, it will be retained as a raw header
///    instead of failing the entire message parse
/// 4. Content-Length is extracted to determine body size
/// 5. The message body is parsed based on the Content-Length header (or 0 if absent)
fn full_message_parser(input: &[u8], mode: ParseMode) -> IResult<&[u8], Message> {
    // 1. Parse Start Line
    let (rest, start_line_data) = alt((
        map(parse_request_line, |(m, u, v)| (true, Some(m), Some(u), Some(v), None, None)),
        map(parse_status_line, |(v, s, r)| {
            // Always include the reason phrase, even if it's empty
            let reason_opt = str::from_utf8(r).ok().map(String::from).or(Some(String::new()));
            (false, None, None, Some(v), Some(s), reason_opt)
        })
    ))(input)?;
    
    let (is_request, method, uri, version, status_code, reason_phrase_opt) = start_line_data;
    
    // 2. Parse Raw Headers block
    let (rest, raw_headers) = parse_header_block(rest)?;

    // 3. Convert Raw Headers to Typed Headers - with error tolerance
    let mut typed_headers: Vec<TypedHeader> = Vec::with_capacity(raw_headers.len());
    
    // Collect all Content-Length headers (to use the last one per RFC 3261)
    let mut content_length_values = Vec::new();
    
    for header in raw_headers {
        // Track Content-Length headers separately
        if header.name == HeaderName::ContentLength {
            if let HeaderValue::Raw(bytes) = &header.value {
                if let Ok(s) = str::from_utf8(bytes) {
                    if let Ok(cl) = s.trim().parse::<usize>() {
                        content_length_values.push(cl);
                    }
                }
            }
        }
        
        match TypedHeader::try_from(header.clone()) {
            Ok(typed) => typed_headers.push(typed),
            Err(e) => {
                eprintln!("Warning: Header parsing error (skipping): {}", e); 
                // Add a fallback for unparseable headers
                typed_headers.push(TypedHeader::Other(header.name, header.value));
            }
        }
    }

    // 4. Get Content-Length - use the last one per RFC 3261 section 20.14
    // "If there are multiple Content-Length headers, use the last value"
    let content_length = if !content_length_values.is_empty() {
        *content_length_values.last().unwrap()
    } else {
        // Fallback to typed header if no raw Content-Length found
        typed_headers.iter().find_map(|h| {
            if let TypedHeader::ContentLength(cl) = h {
                Some(cl.0 as usize)
            } else { None }
        }).unwrap_or(0)
    };
    
    // 5. Parse Body based on Content-Length
    if rest.len() < content_length {
        // In lenient mode, be more forgiving with incomplete bodies
        if mode == ParseMode::Lenient {
            let actual_length = rest.len();
            eprintln!("Warning: Content-Length ({}) exceeds available body data ({}). Using available data.", 
                    content_length, actual_length);
            let (final_rest, body_slice) = take(actual_length)(rest)?;
            let body = Bytes::copy_from_slice(body_slice);
            
            // Construct the message with what we have
            let message = if is_request {
                let mut req = Request::new(method.unwrap(), uri.unwrap());
                req.version = version.unwrap();
                req.set_headers(typed_headers);
                if actual_length > 0 { req.body = body; }
                Message::Request(req)
            } else {
                let mut resp = Response::new(status_code.unwrap());
                resp.version = version.unwrap();
                if let Some(reason) = reason_phrase_opt {
                     resp = resp.with_reason(reason);
                }
                resp.set_headers(typed_headers);
                if actual_length > 0 { resp.body = body; }
                Message::Response(resp)
            };
            
            return Ok((final_rest, message));
        } else {
            // In strict mode, reject messages with mismatched Content-Length
            return Err(nom::Err::Incomplete(Needed::new(content_length - rest.len())));
        }
    }
    
    // Take exactly content_length bytes for the body
    let (final_rest, body_slice) = take(content_length)(rest)?;

    // In lenient mode, if there's extra data after consuming content_length bytes, discard it with a warning
    if mode == ParseMode::Lenient && !final_rest.is_empty() {
        eprintln!("Warning: Message has {} extra bytes after Content-Length: {}. Ignoring excess data.", 
                  final_rest.len(), content_length);
    }

    // 6. Construct Message
    let body = Bytes::copy_from_slice(body_slice);
    let message = if is_request {
        let mut req = Request::new(method.unwrap(), uri.unwrap());
        req.version = version.unwrap();
        req.set_headers(typed_headers);
        if content_length > 0 { req.body = body; }
        Message::Request(req)
    } else {
        let mut resp = Response::new(status_code.unwrap());
        resp.version = version.unwrap();
        if let Some(reason) = reason_phrase_opt {
             resp = resp.with_reason(reason);
        }
        resp.set_headers(typed_headers);
        if content_length > 0 { resp.body = body; }
        Message::Response(resp)
    };

    Ok((final_rest, message))
}

/// Parse a SIP message from bytes
pub fn parse_message(input: &[u8]) -> Result<Message> {
    parse_message_with_mode(input, ParseMode::Lenient)
}

/// Parse a SIP message from bytes with specific parsing mode
pub fn parse_message_with_mode(input: &[u8], mode: ParseMode) -> Result<Message> {
    // Add detailed debug logging to capture exact messages being parsed
    eprintln!("=== PARSING SIP MESSAGE ===");
    eprintln!("Input length: {} bytes", input.len());
    eprintln!("Input as string: {:?}", String::from_utf8_lossy(input));
    eprintln!("Input as hex: {:02x?}", input);
    eprintln!("===========================");
    
    // In strict mode, use all_consuming to ensure the entire input is consumed
    // In lenient mode, don't use all_consuming to allow for excess input after valid message
    let parser_result = if mode == ParseMode::Strict {
        all_consuming(|i| full_message_parser(i, mode))(input)
    } else {
        full_message_parser(input, mode)
    };

    match parser_result {
        Ok((_, message)) => {
            eprintln!("=== PARSE SUCCESS ===");
            eprintln!("Successfully parsed message");
            eprintln!("=====================");
            Ok(message)
        },
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => {
            let offset = input.len() - e.input.len();
            eprintln!("=== PARSE ERROR ===");
            eprintln!("Error at offset: {}", offset);
            eprintln!("Error code: {:?}", e.code);
            eprintln!("Remaining input: {:?}", String::from_utf8_lossy(e.input));
            eprintln!("Remaining input as hex: {:02x?}", e.input);
            eprintln!("==================");
            Err(Error::ParseError(
                format!("Failed to parse message near offset {}: {:?}", offset, e.code)
            ))
        }
        Err(nom::Err::Incomplete(_)) => {
            eprintln!("=== PARSE INCOMPLETE ===");
            eprintln!("Incomplete message");
            eprintln!("========================");
            Err(Error::ParseError(
                "Incomplete message".to_string()
            ))
        }
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
// Parses a single logical header line including folded lines
// Returns a raw Header struct with the value already unfolded
fn message_header(input: &[u8]) -> ParseResult<Header> {
    map_res(
        tuple((
            header_name,
            hcolon,             // Separator with whitespace handling
            header_value_better, // Handles line folding
            crlf                // Final CRLF
        )),
        |(name_bytes, _, raw_value_bytes, _)| {
            str::from_utf8(name_bytes)
                .map_err(|_| nom::Err::Failure(NomError::new(name_bytes, ErrorKind::Char)))
                .and_then(|name_str| {
                    HeaderName::from_str(name_str)
                        .map_err(|_| nom::Err::Failure(NomError::new(name_bytes, ErrorKind::Verify)))
                        .map(|header_name| {
                            // Unfold the raw value to normalize it
                            let unfolded_value = unfold_lws(raw_value_bytes);
                            let header_value = HeaderValue::Raw(unfolded_value);
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

    // RFC 3261 Section 7.3 - Headers
    // Test parsing of message headers according to the ABNF grammar:
    // message-header = header-name HCOLON header-value CRLF

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
    fn test_message_header_case_insensitivity() {
        // RFC 3261 Section 7.3.1: Field names are case-insensitive
        let input = b"SUBJECT: Case Insensitive Test\r\n";
        let result = message_header(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(header.name, HeaderName::Subject);
        assert!(matches!(header.value, HeaderValue::Raw(ref v) if v == b"Case Insensitive Test"));
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
    fn test_header_with_utf8() {
        // RFC 3261 allows UTF-8 characters in header values
        let input = "Subject: UTF-8 Chars - こんにちは\r\n".as_bytes();
        let result = message_header(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(header.name, HeaderName::Subject);
        assert!(matches!(header.value, HeaderValue::Raw(ref v) if 
            std::str::from_utf8(v).unwrap() == "UTF-8 Chars - こんにちは"));
    }

    #[test]
    fn test_message_header_with_quotes() {
        // RFC 3261 allows quoted strings in header values
        let input = b"Subject: \"Quoted Value\" with more text\r\n";
        let result = message_header(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(header.name, HeaderName::Subject);
        assert!(matches!(header.value, HeaderValue::Raw(ref v) if v == b"\"Quoted Value\" with more text"));
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
    fn test_line_folding_header() {
        // RFC 3261 Section 7.3.1 - Line folding
        // A line break and any amount of whitespace are equivalent to a single SP
        let input = b"Subject: Line 1\r\n Line 2\r\n\t Continued Here\r\n";
        let result = message_header(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        assert!(rem.is_empty(), "Expected empty remainder");
        assert_eq!(header.name, HeaderName::Subject);
        
        // Verify that the value was properly unfolded
        if let HeaderValue::Raw(value) = &header.value {
            assert_eq!(String::from_utf8_lossy(value), "Line 1 Line 2 Continued Here");
        } else {
            panic!("Expected Raw header value");
        }
    }

    #[test]
    fn test_multiple_line_folding() {
        // Test multiple line foldings in a single header
        let input = b"Via: SIP/2.0/UDP pc33.atlanta.com\r\n ;branch=z9hG4bK776asdhds\r\n ;received=192.0.2.1\r\n";
        let result = message_header(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(header.name, HeaderName::Via);
        
        // Verify that the value was properly unfolded
        if let HeaderValue::Raw(value) = &header.value {
            assert_eq!(
                String::from_utf8_lossy(value), 
                "SIP/2.0/UDP pc33.atlanta.com ;branch=z9hG4bK776asdhds ;received=192.0.2.1"
            );
        } else {
            panic!("Expected Raw header value");
        }
    }

    #[test]
    fn test_multiple_header_lines() {
        // RFC 3261 Section 7.3.1 - Multiple headers with same field name
        let input = b"Via: SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds\r\n\
                     Via: SIP/2.0/UDP bigbox3.site3.atlanta.com\r\n\
                     \r\n";
                     
        let result = parse_header_block(input);
        assert!(result.is_ok());
        let (rem, headers) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0].name, HeaderName::Via);
        assert_eq!(headers[1].name, HeaderName::Via);
    }

    #[test]
    fn test_compact_form_headers() {
        // RFC 3261 Section 7.3.3 - Compact Form
        let input = b"i: a84b4c76e66710@pc33.atlanta.com\r\n"; // Call-ID compact form
        let result = message_header(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(header.name, HeaderName::CallId); 
        assert!(matches!(header.value, HeaderValue::Raw(ref v) if v == b"a84b4c76e66710@pc33.atlanta.com"));
        
        // Test more compact forms
        let input = b"m: audio 49170 RTP/AVP 0\r\n"; // Contact compact form
        let result = message_header(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        assert_eq!(header.name, HeaderName::Contact);
    }

    #[test]
    fn test_header_values_with_special_chars() {
        // RFC 3261 allows various special characters in header values
        let input = b"User-Agent: SIP-Client/5.0 (Special:Ch@rs; \"Quoted\"; v=1.0)\r\n";
        let result = message_header(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(header.name, HeaderName::UserAgent);
        assert!(matches!(header.value, HeaderValue::Raw(ref v) if v == b"SIP-Client/5.0 (Special:Ch@rs; \"Quoted\"; v=1.0)"));
    }

    #[test]
    fn test_header_values_with_separators() {
        // Test header with various separator characters
        let input = b"Supported: timer, 100rel, path, gruu\r\n";
        let result = message_header(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(header.name, HeaderName::Supported);
        assert!(matches!(header.value, HeaderValue::Raw(ref v) if v == b"timer, 100rel, path, gruu"));
    }

    // ABNF tests for specific header types - ensuring the raw parser can handle them
    
    #[test]
    fn test_via_header_format() {
        // Via = ( "Via" / "v" ) HCOLON via-parm *(COMMA via-parm)
        let input = b"Via: SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds, SIP/2.0/TCP bigbox3.site3.atlanta.com\r\n";
        let result = message_header(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(header.name, HeaderName::Via);
        assert!(matches!(header.value, HeaderValue::Raw(ref v) if 
            v == b"SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds, SIP/2.0/TCP bigbox3.site3.atlanta.com"));
    }

    #[test]
    fn test_full_from_header() {
        // From = ( "From" / "f" ) HCOLON from-spec
        // from-spec = ( name-addr / addr-spec ) *( SEMI from-param )
        let input = b"From: \"Caller\" <sip:caller@atlanta.example.com>;tag=958465702;param=val\r\n";
        let result = message_header(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(header.name, HeaderName::From);
        assert!(matches!(header.value, HeaderValue::Raw(ref v) if 
            v == b"\"Caller\" <sip:caller@atlanta.example.com>;tag=958465702;param=val"));
    }

    // Integration tests for overall message structure (request line + headers + body)

    #[test]
    fn test_parse_full_request() {
        // Full SIP INVITE message as per RFC 3261 examples
        let input = b"INVITE sip:bob@biloxi.com SIP/2.0\r\n\
                     Via: SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds\r\n\
                     Max-Forwards: 70\r\n\
                     To: Bob <sip:bob@biloxi.com>\r\n\
                     From: Alice <sip:alice@atlanta.com>;tag=1928301774\r\n\
                     Call-ID: a84b4c76e66710@pc33.atlanta.com\r\n\
                     CSeq: 314159 INVITE\r\n\
                     Contact: <sip:alice@pc33.atlanta.com>\r\n\
                     Content-Type: application/sdp\r\n\
                     Content-Length: 4\r\n\
                     \r\n\
                     Test";

        let result = full_message_parser(input, ParseMode::Lenient);
        assert!(result.is_ok(), "Failed to parse valid SIP message");
        
        let (rem, msg) = result.unwrap();
        assert!(rem.is_empty(), "Parser should consume the entire input");
        
        match msg {
            Message::Request(req) => {
                assert_eq!(req.method, Method::Invite);
                assert_eq!(req.uri.to_string(), "sip:bob@biloxi.com");
                assert_eq!(req.headers.len(), 9);
                assert_eq!(req.body.len(), 4);
                assert_eq!(&req.body[..], b"Test");
            },
            _ => panic!("Expected Request, got Response")
        }
    }

    #[test]
    fn test_parse_full_response() {
        // Full SIP 200 OK response as per RFC 3261 examples
        let input = b"SIP/2.0 200 OK\r\n\
                     Via: SIP/2.0/UDP server10.biloxi.com;branch=z9hG4bKnashds8\r\n\
                     Via: SIP/2.0/UDP bigbox3.site3.atlanta.com\r\n\
                     To: Bob <sip:bob@biloxi.com>;tag=a6c85cf\r\n\
                     From: Alice <sip:alice@atlanta.com>;tag=1928301774\r\n\
                     Call-ID: a84b4c76e66710@pc33.atlanta.com\r\n\
                     CSeq: 314159 INVITE\r\n\
                     Contact: <sip:bob@192.0.2.4>\r\n\
                     Content-Type: application/sdp\r\n\
                     Content-Length: 4\r\n\
                     \r\n\
                     Body";

        let result = full_message_parser(input, ParseMode::Lenient);
        assert!(result.is_ok(), "Failed to parse valid SIP response");
        
        let (rem, msg) = result.unwrap();
        assert!(rem.is_empty(), "Parser should consume the entire input");
        
        match msg {
            Message::Response(resp) => {
                assert_eq!(resp.status, StatusCode::Ok);
                assert_eq!(resp.reason_phrase(), "OK");
                assert_eq!(resp.headers.len(), 9);
                assert_eq!(resp.body.len(), 4);
                assert_eq!(&resp.body[..], b"Body");
            },
            _ => panic!("Expected Response, got Request")
        }
    }

    #[test]
    fn test_message_missing_content_length() {
        // RFC 3261 Section 20.14: If Content-Length header is missing, body is empty
        let input = b"INVITE sip:bob@biloxi.com SIP/2.0\r\n\
                     Via: SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds\r\n\
                     To: Bob <sip:bob@biloxi.com>\r\n\
                     From: Alice <sip:alice@atlanta.com>;tag=1928301774\r\n\
                     Call-ID: a84b4c76e66710@pc33.atlanta.com\r\n\
                     CSeq: 314159 INVITE\r\n\
                     \r\n\
                     This content should be ignored";

        let result = full_message_parser(input, ParseMode::Lenient);
        assert!(result.is_ok(), "Failed to parse message without Content-Length");
        
        let (_, msg) = result.unwrap();
        match msg {
            Message::Request(req) => {
                assert_eq!(req.body.len(), 0, "Body should be empty when Content-Length is missing");
            },
            _ => panic!("Expected Request, got Response")
        }
    }

    #[test]
    fn test_message_with_folded_headers() {
        // Test with properly formatted line folding in headers (according to RFC 3261)
        let input = b"INVITE sip:bob@biloxi.com SIP/2.0\r\n\
                     Subject: This is a very long subject header that\r\n \
                      spans multiple lines and uses line\r\n \
                      folding as described in RFC 3261\r\n\
                     Content-Length: 0\r\n\
                     \r\n";
        
        let result = full_message_parser(input, ParseMode::Lenient);
        
        // Print detailed error info if it fails
        if let Err(e) = &result {
            println!("Parsing error: {:?}", e);
            
            // If it's a failure error, try to get more info
            if let nom::Err::Failure(ref ne) = e {
                println!("Failure input: {:?}", String::from_utf8_lossy(ne.input));
                println!("Failure code: {:?}", ne.code);
            }
        }
        
        assert!(result.is_ok(), "Failed to parse message with folded headers");
        
        // If parsing succeeded, check that the subject header was properly unfolded
        if let Ok((_, message)) = result {
            if let Message::Request(request) = message {
                let subject_header = request.headers.iter().find(|h| matches!(h, TypedHeader::Subject(_)));
                assert!(subject_header.is_some(), "Subject header not found in parsed message");
                if let Some(TypedHeader::Subject(subject)) = subject_header {
                    assert_eq!(subject.text(), "This is a very long subject header that spans multiple lines and uses line folding as described in RFC 3261");
                }
            } else {
                panic!("Expected Request, got Response");
            }
        }
    }

    #[test]
    fn test_abnf_invalid_messages() {
        // Test rejection of invalid messages
        
        // 1. Missing start line
        let input = b"Via: SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds\r\n\r\n";
        assert!(full_message_parser(input, ParseMode::Strict).is_err(), "Should reject message without start line");
        
        // 2. Invalid request-line format
        let input = b"INVITE\r\n\r\n"; // Missing URI and version
        assert!(full_message_parser(input, ParseMode::Strict).is_err(), "Should reject invalid request-line");
        
        // 3. Invalid status-line format
        let input = b"SIP/2.0 200\r\n\r\n"; // Missing reason phrase
        assert!(full_message_parser(input, ParseMode::Strict).is_err(), "Should reject invalid status-line");
        
        // 4. Content-Length mismatch - strict mode rejects mismatched Content-Length
        let input = b"INVITE sip:bob@biloxi.com SIP/2.0\r\n\
                     Content-Length: 10\r\n\
                     \r\n\
                     Test"; // Only 4 bytes, but Content-Length said 10
        assert!(full_message_parser(input, ParseMode::Strict).is_err(), "Should reject message with Content-Length mismatch");
        
        // 5. But lenient mode accepts mismatched Content-Length and uses available data
        let input = b"INVITE sip:bob@biloxi.com SIP/2.0\r\n\
                     Content-Length: 10\r\n\
                     \r\n\
                     Test"; // Only 4 bytes, but Content-Length said 10
        let result = full_message_parser(input, ParseMode::Lenient);
        assert!(result.is_ok(), "Lenient mode should accept message with Content-Length mismatch");
        if let Ok((_, Message::Request(req))) = result {
            assert_eq!(req.body.len(), 4, "Lenient mode should use available body data");
        } else {
            panic!("Expected Request in lenient parsing mode");
        }
        
        // 6. Body longer than Content-Length - lenient mode should truncate to Content-Length
        let input = b"INVITE sip:bob@biloxi.com SIP/2.0\r\n\
                     Content-Length: 4\r\n\
                     \r\n\
                     TestExtraDataThatShouldBeIgnored";
        let result = full_message_parser(input, ParseMode::Lenient);
        assert!(result.is_ok(), "Lenient mode should accept message with body longer than Content-Length");
        if let Ok((_, Message::Request(req))) = result {
            assert_eq!(req.body.len(), 4, "Lenient mode should use only Content-Length bytes");
            assert_eq!(&req.body[..], b"Test", "Lenient mode should use correct body part");
        } else {
            panic!("Expected Request in lenient parsing mode");
        }
    }
} 

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::str::FromStr;

use bytes::Bytes;
use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_till, take_until, take_while, take_while1},
    character::complete::{char, digit1, line_ending, space0, space1},
    combinator::{map, map_res, opt, recognize, value, verify},
    multi::{many0, many1, many_till},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    Err, IResult, Needed,
};

use crate::error::{Error, Result};
use crate::header::{Header, HeaderName, HeaderValue};
use crate::message::{Message, Request, Response, StatusCode};
use crate::method::Method;
use crate::uri::{Host, Uri};
use crate::version::Version;

/// Maximum line length for SIP messages
pub const MAX_LINE_LENGTH: usize = 8192;

/// Maximum header count for SIP messages
pub const MAX_HEADER_COUNT: usize = 100;

/// Maximum body size for SIP messages
pub const MAX_BODY_SIZE: usize = 16 * 1024 * 1024; // 16MB

/// State for incremental parsing
#[derive(Debug, Clone)]
pub enum ParseState {
    /// Initial state, expecting request/response line
    Initial,
    /// Parsed the start line, now reading headers
    Headers { 
        /// The partially parsed message
        message: PartialMessage, 
        /// Headers parsed so far
        headers: Vec<Header> 
    },
    /// Read all headers, now reading the body
    Body { 
        /// The partially parsed message
        message: PartialMessage, 
        /// Headers parsed
        headers: Vec<Header>,
        /// Content length indicated in headers
        content_length: usize 
    },
    /// Parsing complete
    Complete(Message),
    /// Error encountered
    Error(Error),
}

impl Default for ParseState {
    fn default() -> Self {
        ParseState::Initial
    }
}

/// Partially parsed message during incremental parsing
#[derive(Debug, Clone)]
pub enum PartialMessage {
    /// Partially parsed request
    Request {
        /// Request method
        method: Method,
        /// Request URI
        uri: Uri,
        /// SIP version
        version: Version,
    },
    /// Partially parsed response
    Response {
        /// SIP version
        version: Version,
        /// Status code
        status: StatusCode,
        /// Reason phrase
        reason: Option<String>,
    },
}

impl PartialMessage {
    /// Convert to a complete Message with the given headers and body
    fn into_message(self, headers: Vec<Header>, body: Bytes) -> Message {
        match self {
            PartialMessage::Request { method, uri, version } => {
                Message::Request(Request {
                    method,
                    uri,
                    version,
                    headers,
                    body,
                })
            },
            PartialMessage::Response { version, status, reason } => {
                Message::Response(Response {
                    version,
                    status,
                    reason,
                    headers,
                    body,
                })
            },
        }
    }
}

/// Incremental parser for SIP messages
///
/// This parser can incrementally parse SIP messages, handling cases where
/// the full message is not yet available in a single buffer.
#[derive(Debug, Default)]
pub struct IncrementalParser {
    /// Current parse state
    state: ParseState,
    /// Buffer for accumulating message data
    buffer: Vec<u8>,
}

impl IncrementalParser {
    /// Create a new incremental parser
    pub fn new() -> Self {
        Self {
            state: ParseState::Initial,
            buffer: Vec::new(),
        }
    }

    /// Feed more data to the parser
    pub fn feed(&mut self, data: &[u8]) -> Result<ParseState> {
        // Append the new data to our buffer
        self.buffer.extend_from_slice(data);
        
        // Process the data based on current state
        match &self.state {
            ParseState::Initial => self.parse_initial(),
            ParseState::Headers { message, headers } => {
                let message = message.clone();
                let headers = headers.clone();
                self.parse_headers(message, headers)
            },
            ParseState::Body { message, headers, content_length } => {
                let message = message.clone();
                let headers = headers.clone();
                let content_length = *content_length;
                self.parse_body(message, headers, content_length)
            },
            ParseState::Complete(_) | ParseState::Error(_) => {
                // Already complete or error state, do nothing
                Ok(self.state.clone())
            }
        }
    }

    /// Parse the initial line of a SIP message
    fn parse_initial(&mut self) -> Result<ParseState> {
        // Convert buffer to string to parse the initial line
        let data = match std::str::from_utf8(&self.buffer) {
            Ok(data) => data,
            Err(_) => {
                // Not valid UTF-8
                let error = Error::InvalidFormat("Message contains invalid UTF-8".to_string());
                self.state = ParseState::Error(error.clone());
                return Err(error);
            }
        };

        // Try to parse the first line
        let parsed_result = alt((
            map(sip_request_line, |(method, uri, version)| {
                PartialMessage::Request { method, uri, version }
            }),
            map(sip_response_line, |(version, status, reason)| {
                PartialMessage::Response { version, status, reason }
            }),
        ))(data);

        match parsed_result {
            Ok((remainder, message)) => {
                // Success! Update buffer to remaining data and move to Headers state
                let consumed = data.len() - remainder.len();
                let new_buffer = self.buffer[consumed..].to_vec();
                self.buffer = new_buffer;
                
                self.state = ParseState::Headers { 
                    message, 
                    headers: Vec::new() 
                };
                Ok(self.state.clone())
            },
            Err(Err::Incomplete(_)) => {
                // Need more data
                Ok(ParseState::Initial)
            },
            Err(e) => {
                // Error in parsing
                let error = Error::Parser(format!("Failed to parse initial line: {:?}", e));
                self.state = ParseState::Error(error.clone());
                Err(error)
            }
        }
    }

    /// Parse headers from the buffer
    fn parse_headers(&mut self, message: PartialMessage, mut existing_headers: Vec<Header>) -> Result<ParseState> {
        // Convert buffer to string to parse headers
        let data = match std::str::from_utf8(&self.buffer) {
            Ok(data) => data,
            Err(_) => {
                // Not valid UTF-8
                let error = Error::InvalidFormat("Message contains invalid UTF-8".to_string());
                self.state = ParseState::Error(error.clone());
                return Err(error);
            }
        };

        // Try to parse headers
        match headers_with_content_length(data) {
            Ok((remainder, (headers, content_length))) => {
                // Add new headers to existing
                existing_headers.extend(headers);
                
                // Update buffer to remaining data
                let consumed = data.len() - remainder.len();
                let new_buffer = self.buffer[consumed..].to_vec();
                self.buffer = new_buffer;
                
                // Move to Body state
                let message_clone = message.clone();
                let headers_clone = existing_headers.clone();
                
                self.state = ParseState::Body { 
                    message, 
                    headers: existing_headers, 
                    content_length 
                };
                
                // If we already have enough data for the body, parse it now
                if self.buffer.len() >= content_length {
                    return self.parse_body(message_clone, headers_clone, content_length);
                }
                
                Ok(self.state.clone())
            },
            Err(Err::Incomplete(_)) => {
                // Need more data
                self.state = ParseState::Headers { message, headers: existing_headers };
                Ok(self.state.clone())
            },
            Err(e) => {
                // Error in parsing
                let error = Error::Parser(format!("Failed to parse headers: {:?}", e));
                self.state = ParseState::Error(error.clone());
                Err(error)
            }
        }
    }

    /// Parse the message body
    fn parse_body(&mut self, message: PartialMessage, headers: Vec<Header>, content_length: usize) -> Result<ParseState> {
        // Check if we have enough data for the body
        if self.buffer.len() >= content_length {
            // Extract the body data
            let body_data = self.buffer[..content_length].to_vec();
            
            // Update buffer to remove consumed data
            self.buffer = self.buffer[content_length..].to_vec();
            
            // Create the complete message
            let complete_message = message.into_message(headers, Bytes::from(body_data));
            
            // Update state
            self.state = ParseState::Complete(complete_message.clone());
            
            Ok(self.state.clone())
        } else {
            // Need more data
            self.state = ParseState::Body { message, headers, content_length };
            Ok(self.state.clone())
        }
    }

    /// Get the current state of the parser
    pub fn state(&self) -> &ParseState {
        &self.state
    }

    /// Reset the parser to initial state
    pub fn reset(&mut self) {
        self.state = ParseState::Initial;
        self.buffer.clear();
    }

    /// Take the completed message if available
    pub fn take_message(&mut self) -> Option<Message> {
        if let ParseState::Complete(message) = &self.state {
            let message = message.clone();
            self.reset();
            Some(message)
        } else {
            None
        }
    }
}

/// Parse a SIP message from raw bytes
pub fn parse_message(data: &Bytes) -> Result<Message> {
    // Convert bytes to string for parsing
    let data_str = std::str::from_utf8(data).map_err(|_| {
        Error::InvalidFormat("Message contains invalid UTF-8".to_string())
    })?;
    
    // Use nom to parse the full message
    match sip_message(data_str) {
        Ok((_, message)) => Ok(message),
        Err(e) => Err(Error::from(e)),
    }
}

// Parser for a SIP request line
fn sip_request_line(input: &str) -> IResult<&str, (Method, Uri, Version)> {
    // Parse method
    let (input, method) = method_parser(input)?;
    // Parse space
    let (input, _) = space1(input)?;
    // Parse URI
    let (input, uri) = uri_parser(input)?;
    // Parse space
    let (input, _) = space1(input)?;
    // Parse version
    let (input, version) = version_parser(input)?;
    
    let (input, _) = crlf(input)?;
    
    Ok((input, (method, uri, version)))
}

// Parser for a SIP response line
fn sip_response_line(input: &str) -> IResult<&str, (Version, StatusCode, Option<String>)> {
    // Parse version
    let (input, version) = version_parser(input)?;
    let (input, _) = space1(input)?;
    let (input, status) = status_code_parser(input)?;
    let (input, _) = space1(input)?;
    let (input, reason) = reason_phrase_parser(input)?;
    
    let (input, _) = crlf(input)?;
    
    let reason_opt = if reason.is_empty() { 
        None 
    } else { 
        Some(reason.to_string()) 
    };
    
    Ok((input, (version, status, reason_opt)))
}

// Parser for a complete SIP message
fn sip_message(input: &str) -> IResult<&str, Message> {
    alt((
        map(sip_request, Message::Request),
        map(sip_response, Message::Response),
    ))(input)
}

// Parser for a SIP request
fn sip_request(input: &str) -> IResult<&str, Request> {
    let (input, (method, uri, version)) = sip_request_line(input)?;
    let (input, headers) = headers_parser(input)?;
    let (input, body) = body_parser_with_content_length(&headers, input)?;
    
    Ok((
        input,
        Request {
            method,
            uri,
            version,
            headers,
            body: Bytes::from(body),
        },
    ))
}

// Parser for a SIP response
fn sip_response(input: &str) -> IResult<&str, Response> {
    let (input, (version, status, reason)) = sip_response_line(input)?;
    let (input, headers) = headers_parser(input)?;
    let (input, body) = body_parser_with_content_length(&headers, input)?;
    
    Ok((
        input,
        Response {
            version,
            status,
            reason,
            headers,
            body: Bytes::from(body),
        },
    ))
}

// Parser for SIP method
fn method_parser(input: &str) -> IResult<&str, Method> {
    map_res(
        take_while1(|c: char| c.is_ascii_alphabetic()),
        |s: &str| Method::from_str(s)
    )(input)
}

// Parser for SIP URI
fn uri_parser(input: &str) -> IResult<&str, Uri> {
    // For now, we'll just capture the URI as a string and parse it with FromStr
    map_res(
        take_while1(|c: char| !c.is_whitespace()),
        |s: &str| Uri::from_str(s)
    )(input)
}

// Parser for SIP version
fn version_parser(input: &str) -> IResult<&str, Version> {
    map_res(
        recognize(tuple((
            tag_no_case("SIP/"),
            digit1,
            char('.'),
            digit1,
        ))),
        |s: &str| Version::from_str(s)
    )(input)
}

// Parser for status code
fn status_code_parser(input: &str) -> IResult<&str, StatusCode> {
    map_res(
        digit1,
        |s: &str| {
            let code = s.parse::<u16>().unwrap_or(0);
            StatusCode::from_u16(code)
        }
    )(input)
}

// Parser for reason phrase
fn reason_phrase_parser(input: &str) -> IResult<&str, &str> {
    take_till(|c| c == '\r' || c == '\n')(input)
}

// Parser for headers
fn headers_parser(input: &str) -> IResult<&str, Vec<Header>> {
    terminated(
        verify(
            many0(header_parser_with_continuation),
            |headers: &Vec<Header>| headers.len() <= MAX_HEADER_COUNT
        ),
        crlf
    )(input)
}

// Parse headers and extract Content-Length
fn headers_with_content_length(input: &str) -> IResult<&str, (Vec<Header>, usize)> {
    let (input, headers) = headers_parser(input)?;
    
    // Extract Content-Length header if present
    let content_length = headers.iter()
        .find(|h| h.name == HeaderName::ContentLength)
        .and_then(|h| h.value.as_integer())
        .map(|i| i as usize)
        .unwrap_or(0);
    
    Ok((input, (headers, content_length)))
}

// Parser for a single header with support for line continuation
fn header_parser_with_continuation(input: &str) -> IResult<&str, Header> {
    // Parse the initial header line
    let (mut remainder, header) = header_parser(input)?;
    
    // Local mutable value to build our header
    let mut value = header.value;
    
    // Look for continuation lines
    loop {
        let continuation = tuple((
            crlf,
            space1, // Must start with whitespace for continuation
            take_till(|c| c == '\r' || c == '\n')
        ))(remainder);
        
        match continuation {
            Ok((new_remainder, (_, _, continuation_text))) => {
                // Append the continuation text to the value
                let new_value = match value {
                    HeaderValue::Text(text) => {
                        HeaderValue::Text(format!("{} {}", text, continuation_text))
                    },
                    HeaderValue::Raw(text) => {
                        HeaderValue::Raw(format!("{} {}", text, continuation_text))
                    },
                    // For other types, convert to raw and append
                    _ => HeaderValue::Raw(format!("{} {}", value.to_string_value(), continuation_text)),
                };
                
                value = new_value;
                remainder = new_remainder;
            },
            _ => break,
        }
    }
    
    // Create the final header with the complete value
    let (_, remainder) = crlf(remainder)?;
    
    Ok((remainder, Header::new(header.name, value)))
}

// Parser for a single header
fn header_parser(input: &str) -> IResult<&str, Header> {
    let (input, (name, value)) = separated_pair(
        map_res(
            take_till(|c| c == ':'),
            |s: &str| HeaderName::from_str(s.trim())
        ),
        tuple((char(':'), space0)),
        map_res(
            take_till(|c| c == '\r' || c == '\n'),
            |s: &str| Ok::<_, Error>(HeaderValue::from_str(s.trim())?)
        )
    )(input)?;
    
    Ok((input, Header::new(name, value)))
}

// Parse the body of the message using Content-Length
fn body_parser_with_content_length<'a>(headers: &[Header], input: &'a str) -> IResult<&'a str, String> {
    // Extract Content-Length header if present
    let content_length = headers.iter()
        .find(|h| h.name == HeaderName::ContentLength)
        .and_then(|h| h.value.as_integer())
        .map(|i| i as usize)
        .unwrap_or(0);
    
    // If Content-Length is 0, return empty body
    if content_length == 0 {
        return Ok((input, String::new()));
    }
    
    // Verify we have enough bytes
    if input.len() < content_length {
        return Err(Err::Incomplete(Needed::Size(NonZeroUsize::new(content_length).unwrap())));
    }
    
    // Extract the exact number of bytes specified by Content-Length
    let (remainder, body) = input.split_at(content_length);
    
    Ok((remainder, body.to_string()))
}

// Parse the body of the message
fn body_parser(input: &str) -> IResult<&str, String> {
    Ok((input, input.to_string()))  // Convert to owned String
}

// Parser for CRLF
fn crlf(input: &str) -> IResult<&str, &str> {
    alt((tag("\r\n"), tag("\n")))(input)
}

/// Parse a Via header value
pub fn parse_via(value: &str) -> Result<HashMap<String, String>> {
    let mut components = HashMap::new();
    
    // Via header format: SIP/2.0/UDP host:port;branch=xxx;other=params
    let parts: Vec<&str> = value.split(';').collect();
    
    if parts.is_empty() {
        return Err(Error::InvalidHeader(format!("Invalid Via header: {}", value)));
    }
    
    // Parse protocol part: SIP/2.0/UDP
    let protocol_parts: Vec<&str> = parts[0].trim().split('/').collect();
    if protocol_parts.len() < 3 {
        return Err(Error::InvalidHeader(format!("Invalid Via protocol: {}", parts[0])));
    }
    
    components.insert("protocol".to_string(), protocol_parts[0].to_string());
    components.insert("version".to_string(), protocol_parts[1].to_string());
    
    // Extract transport and host:port
    let transport_and_host = protocol_parts[2].trim().split_whitespace().collect::<Vec<&str>>();
    
    if transport_and_host.is_empty() {
        return Err(Error::InvalidHeader(format!("Missing transport in Via: {}", parts[0])));
    }
    
    components.insert("transport".to_string(), transport_and_host[0].to_string());
    
    // Extract sent-by (host:port)
    if transport_and_host.len() > 1 {
        let sent_by = transport_and_host[1];
        if sent_by.contains(':') {
            let host_port: Vec<&str> = sent_by.split(':').collect();
            components.insert("host".to_string(), host_port[0].to_string());
            if host_port.len() > 1 {
                components.insert("port".to_string(), host_port[1].to_string());
            }
        } else {
            components.insert("host".to_string(), sent_by.to_string());
        }
    }
    
    // Parse parameters
    for i in 1..parts.len() {
        let param = parts[i].trim();
        if param.contains('=') {
            let param_parts: Vec<&str> = param.split('=').collect();
            if param_parts.len() >= 2 {
                components.insert(param_parts[0].to_string(), param_parts[1].to_string());
            }
        } else {
            components.insert(param.to_string(), "".to_string());
        }
    }
    
    Ok(components)
}

/// Parse multiple Via headers
pub fn parse_multiple_vias(input: &str) -> Result<Vec<HashMap<String, String>>> {
    let via_parts = crate::header_parsers::parse_comma_separated_list(input);
    
    let mut result = Vec::new();
    for part in via_parts {
        result.push(parse_via(&part)?);
    }
    
    Ok(result)
}

/// Parse a Contact header value
pub fn parse_contact(input: &str) -> Result<Vec<HashMap<String, String>>> {
    let mut contacts = Vec::new();
    
    // Split by commas for multiple contacts, but respect < > pairs
    let contact_parts = crate::header_parsers::parse_comma_separated_list(input);
    
    // Parse each contact
    for contact in contact_parts {
        let mut components = HashMap::new();
        
        // Check if we have a display name
        if contact.contains('<') && contact.contains('>') {
            let display_name = contact[..contact.find('<').unwrap()].trim();
            if !display_name.is_empty() {
                components.insert("display_name".to_string(), 
                                  display_name.trim_matches('"').to_string());
            }
            
            // Extract URI and parameters
            let rest = &contact[contact.find('<').unwrap()..];
            if let Some(uri_end) = rest.find('>') {
                // Extract URI
                let uri = &rest[1..uri_end];
                components.insert("uri".to_string(), uri.to_string());
                
                // Extract parameters after >
                if uri_end + 1 < rest.len() {
                    let params = &rest[uri_end+1..];
                    crate::header_parsers::parse_parameters(params, &mut components);
                }
            }
        } else {
            // Just a URI, possibly with parameters
            let parts: Vec<&str> = contact.split(';').collect();
            components.insert("uri".to_string(), parts[0].to_string());
            
            // Parse parameters
            for i in 1..parts.len() {
                let param = parts[i].trim();
                crate::header_parsers::parse_parameter(param, &mut components);
            }
        }
        
        contacts.push(components);
    }
    
    Ok(contacts)
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

    #[test]
    fn test_parse_message_with_body() {
        let body = "v=0\r\n\
                    o=alice 2890844526 2890844526 IN IP4 pc33.example.com\r\n\
                    s=Session SDP\r\n\
                    c=IN IP4 pc33.example.com\r\n\
                    t=0 0\r\n\
                    m=audio 49172 RTP/AVP 0\r\n\
                    a=rtpmap:0 PCMU/8000\r\n";
        
        let message = format!("INVITE sip:bob@example.com SIP/2.0\r\n\
                              Via: SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds\r\n\
                              Max-Forwards: 70\r\n\
                              To: Bob <sip:bob@example.com>\r\n\
                              From: Alice <sip:alice@example.com>;tag=1928301774\r\n\
                              Call-ID: a84b4c76e66710@pc33.example.com\r\n\
                              CSeq: 314159 INVITE\r\n\
                              Contact: <sip:alice@pc33.example.com>\r\n\
                              Content-Type: application/sdp\r\n\
                              Content-Length: {}\r\n\
                              \r\n\
                              {}", body.len(), body);
        
        let message_bytes = Bytes::from(message);
        let parsed = parse_message(&message_bytes).unwrap();
        
        if let Message::Request(req) = parsed {
            assert!(!req.body.is_empty());
            assert_eq!(req.body.len(), body.len());
            
            let body_str = std::str::from_utf8(&req.body).unwrap();
            assert_eq!(body_str, body);
            
            // Check Content-Length header
            let content_length = req.header(&HeaderName::ContentLength).unwrap();
            assert_eq!(content_length.value.as_integer().unwrap() as usize, body.len());
        } else {
            panic!("Expected request, got response");
        }
    }
    
    #[test]
    fn test_header_continuation() {
        let message = "INVITE sip:bob@example.com SIP/2.0\r\n\
                      Via: SIP/2.0/UDP pc33.example.com:5060;\r\n\
                       branch=z9hG4bK776asdhds\r\n\
                      To: Bob <sip:bob@example.com>\r\n\
                      From: Alice <sip:alice@example.com>;\r\n\
                       tag=1928301774\r\n\
                      Call-ID: a84b4c76e66710@pc33.example.com\r\n\
                      Content-Length: 0\r\n\
                      \r\n";
        
        let message_bytes = Bytes::from(message);
        let parsed = parse_message(&message_bytes).unwrap();
        
        if let Message::Request(req) = parsed {
            // Check folded headers are correctly parsed
            let via = req.header(&HeaderName::Via).unwrap();
            assert_eq!(via.value.as_text().unwrap(), "SIP/2.0/UDP pc33.example.com:5060; branch=z9hG4bK776asdhds");
            
            let from = req.header(&HeaderName::From).unwrap();
            assert_eq!(from.value.as_text().unwrap(), "Alice <sip:alice@example.com>; tag=1928301774");
        } else {
            panic!("Expected request, got response");
        }
    }
    
    #[test]
    fn test_compact_header_forms() {
        let message = "INVITE sip:bob@example.com SIP/2.0\r\n\
                      v: SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds\r\n\
                      t: Bob <sip:bob@example.com>\r\n\
                      f: Alice <sip:alice@example.com>;tag=1928301774\r\n\
                      i: a84b4c76e66710@pc33.example.com\r\n\
                      c: application/sdp\r\n\
                      l: 0\r\n\
                      \r\n";
        
        let message_bytes = Bytes::from(message);
        let parsed = parse_message(&message_bytes).unwrap();
        
        if let Message::Request(req) = parsed {
            // Verify compact headers are correctly parsed
            assert!(req.header(&HeaderName::Via).is_some());
            assert!(req.header(&HeaderName::To).is_some());
            assert!(req.header(&HeaderName::From).is_some());
            assert!(req.header(&HeaderName::CallId).is_some());
            assert!(req.header(&HeaderName::ContentType).is_some());
            assert!(req.header(&HeaderName::ContentLength).is_some());
        } else {
            panic!("Expected request, got response");
        }
    }
    
    #[test]
    fn test_incremental_parser() {
        let message = "INVITE sip:bob@example.com SIP/2.0\r\n\
                      Via: SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds\r\n\
                      Max-Forwards: 70\r\n\
                      To: Bob <sip:bob@example.com>\r\n\
                      From: Alice <sip:alice@example.com>;tag=1928301774\r\n\
                      Call-ID: a84b4c76e66710@pc33.example.com\r\n\
                      CSeq: 314159 INVITE\r\n\
                      Content-Length: 0\r\n\
                      \r\n";
        
        // Split into chunks to simulate incremental receipt
        let chunks = [
            &message[0..30],
            &message[30..80],
            &message[80..150],
            &message[150..],
        ];
        
        let mut parser = IncrementalParser::new();
        
        // Feed each chunk
        for (i, chunk) in chunks.iter().enumerate() {
            let state = parser.feed(chunk.as_bytes()).unwrap();
            
            match i {
                0 => assert!(matches!(state, ParseState::Initial)),
                1 => assert!(matches!(state, ParseState::Headers { .. })),
                2 => assert!(matches!(state, ParseState::Headers { .. })),
                3 => assert!(matches!(state, ParseState::Complete(_))),
                _ => unreachable!(),
            }
        }
        
        // Get the completed message
        let message = parser.take_message().unwrap();
        assert!(message.is_request());
        
        if let Message::Request(req) = message {
            assert_eq!(req.method, Method::Invite);
            assert_eq!(req.uri.to_string(), "sip:bob@example.com");
        } else {
            panic!("Expected request, got response");
        }
    }
    
    #[test]
    fn test_incremental_parser_with_body() {
        let body = "v=0\r\n\
                    o=alice 2890844526 2890844526 IN IP4 pc33.example.com\r\n\
                    s=Session SDP\r\n\
                    c=IN IP4 pc33.example.com\r\n\
                    t=0 0\r\n\
                    m=audio 49172 RTP/AVP 0\r\n\
                    a=rtpmap:0 PCMU/8000\r\n";
        
        let message = format!("INVITE sip:bob@example.com SIP/2.0\r\n\
                              Via: SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds\r\n\
                              Max-Forwards: 70\r\n\
                              To: Bob <sip:bob@example.com>\r\n\
                              From: Alice <sip:alice@example.com>;tag=1928301774\r\n\
                              Call-ID: a84b4c76e66710@pc33.example.com\r\n\
                              CSeq: 314159 INVITE\r\n\
                              Content-Type: application/sdp\r\n\
                              Content-Length: {}\r\n\
                              \r\n\
                              {}", body.len(), body);
        
        // Split into chunks to simulate incremental receipt
        let header_end = message.find("\r\n\r\n").unwrap() + 4;
        let chunks = [
            &message[0..30],
            &message[30..80],
            &message[80..header_end],
            &message[header_end..],
        ];
        
        let mut parser = IncrementalParser::new();
        
        // Feed each chunk
        for (i, chunk) in chunks.iter().enumerate() {
            let state = parser.feed(chunk.as_bytes()).unwrap();
            
            match i {
                0 => assert!(matches!(state, ParseState::Initial)),
                1 => assert!(matches!(state, ParseState::Headers { .. })),
                2 => assert!(matches!(state, ParseState::Body { .. })),
                3 => assert!(matches!(state, ParseState::Complete(_))),
                _ => unreachable!(),
            }
        }
        
        // Get the completed message
        let message = parser.take_message().unwrap();
        assert!(message.is_request());
        
        if let Message::Request(req) = message {
            assert_eq!(req.method, Method::Invite);
            let body_str = std::str::from_utf8(&req.body).unwrap();
            assert_eq!(body_str, body);
        } else {
            panic!("Expected request, got response");
        }
    }
} 
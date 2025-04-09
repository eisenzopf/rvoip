use std::str::FromStr;

use bytes::Bytes;
use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_till, take_until, take_while, take_while1},
    character::complete::{char, digit1, line_ending, space0, space1},
    combinator::{map, map_res, opt, recognize, value, verify},
    multi::{many0, many1, many_till, separated_list0, separated_list1},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    Err, IResult, Needed,
};

use crate::error::{Error, Result};
use crate::header::{Header, HeaderName, HeaderValue};
use crate::message::{Message, Request, Response, StatusCode};
use crate::method::Method;
use crate::uri::Uri;
use crate::version::Version;

use super::headers::parse_headers;
use super::uri_parser::parse_uri;
use super::utils::crlf;

/// Maximum length of a single line in a SIP message
pub const MAX_LINE_LENGTH: usize = 8192;
/// Maximum number of headers in a SIP message
pub const MAX_HEADER_COUNT: usize = 100;
/// Maximum size of a SIP message body
pub const MAX_BODY_SIZE: usize = 16 * 1024 * 1024; // 16 MB

/// The state of incremental parsing
#[derive(Debug, Clone, PartialEq)]
pub enum ParseState {
    /// Waiting for the start line
    WaitingForStartLine,
    /// Parsing headers
    ParsingHeaders,
    /// Parsing the message body
    ParsingBody { 
        /// Content-Length value
        content_length: usize, 
        /// Bytes parsed so far
        bytes_parsed: usize 
    },
    /// Finished parsing the message
    Complete(Message),
    /// Failed to parse the message
    Failed(Error),
}

/// Incremental parser for SIP messages
#[derive(Debug)]
pub struct IncrementalParser {
    /// Current state of parsing
    state: ParseState,
    /// Buffer containing the message being parsed
    buffer: String,
    /// Request-Line or Status-Line
    start_line: Option<String>,
    /// Headers that have been parsed
    headers: Vec<Header>,
    /// Message body
    body: Option<String>,
}

impl IncrementalParser {
    /// Create a new incremental parser
    pub fn new() -> Self {
        Self {
            state: ParseState::WaitingForStartLine,
            buffer: String::new(),
            start_line: None,
            headers: Vec::new(),
            body: None,
        }
    }
    
    /// Get the current state of parsing
    pub fn state(&self) -> &ParseState {
        &self.state
    }
    
    /// Reset the parser to parse a new message
    pub fn reset(&mut self) {
        self.state = ParseState::WaitingForStartLine;
        self.buffer.clear();
        self.start_line = None;
        self.headers.clear();
        self.body = None;
    }
    
    /// Parse a chunk of data
    pub fn parse(&mut self, data: &str) -> &ParseState {
        // Add data to the buffer
        self.buffer.push_str(data);
        
        match self.state {
            ParseState::WaitingForStartLine => {
                // Try to find the start line
                if let Some(idx) = self.buffer.find("\r\n") {
                    if idx > MAX_LINE_LENGTH {
                        self.state = ParseState::Failed(Error::LineTooLong(idx));
                        return &self.state;
                    }
                    
                    let start_line = self.buffer[..idx].to_string();
                    self.start_line = Some(start_line);
                    self.buffer.drain(..idx + 2);
                    self.state = ParseState::ParsingHeaders;
                    
                    // Continue parsing headers
                    return self.parse("");
                }
            }
            ParseState::ParsingHeaders => {
                // Try to find the end of the headers
                if let Some(idx) = self.buffer.find("\r\n\r\n") {
                    let headers_str = self.buffer[..idx + 2].to_string();
                    self.buffer.drain(..idx + 4);
                    
                    // Parse headers
                    match parse_headers(&headers_str) {
                        Ok(headers) => {
                            if headers.len() > MAX_HEADER_COUNT {
                                self.state = ParseState::Failed(Error::TooManyHeaders(headers.len()));
                                return &self.state;
                            }
                            
                            self.headers = headers;
                            
                            // Check for Content-Length header
                            let content_length = self.get_content_length().unwrap_or(0);
                            
                            if content_length > MAX_BODY_SIZE {
                                self.state = ParseState::Failed(Error::BodyTooLarge(content_length));
                                return &self.state;
                            }
                            
                            if content_length == 0 {
                                // No body, message is complete
                                self.complete_message();
                            } else {
                                // Has body, need to read more data
                                self.state = ParseState::ParsingBody {
                                    content_length,
                                    bytes_parsed: self.buffer.len(),
                                };
                                
                                // Continue parsing the body if there's data already in the buffer
                                if !self.buffer.is_empty() {
                                    return self.parse("");
                                }
                            }
                        }
                        Err(e) => {
                            self.state = ParseState::Failed(e);
                        }
                    }
                }
            }
            ParseState::ParsingBody { content_length, bytes_parsed } => {
                let new_bytes_parsed = bytes_parsed + self.buffer.len();
                
                if new_bytes_parsed >= content_length {
                    // We have the entire body
                    let needed = content_length.saturating_sub(bytes_parsed);
                    let body = self.buffer.drain(..needed).collect::<String>();
                    
                    self.body = Some(body);
                    self.complete_message();
                } else {
                    // Still need more data
                    self.state = ParseState::ParsingBody {
                        content_length,
                        bytes_parsed: new_bytes_parsed,
                    };
                }
            }
            ParseState::Complete(_) | ParseState::Failed(_) => {
                // Nothing to do, already complete or failed
            }
        }
        
        &self.state
    }
    
    /// Get the content length from the headers
    fn get_content_length(&self) -> Option<usize> {
        for header in &self.headers {
            if header.name == HeaderName::ContentLength {
                if let Some(text) = header.value.as_text() {
                    return text.trim().parse::<usize>().ok();
                }
            }
        }
        
        None
    }
    
    /// Complete the parsed message
    fn complete_message(&mut self) {
        if let Some(start_line) = &self.start_line {
            // Parse request or response
            if start_line.starts_with("SIP/") {
                // Response
                match parse_response_line(start_line) {
                    Ok((_, (version, status, reason))) => {
                        // Create response using the correct constructor
                        let mut response = Response::new(status);
                        response = response.with_reason(reason);
                        
                        // Add headers
                        for header in &self.headers {
                            response = response.with_header(header.clone());
                        }
                        
                        if let Some(body) = &self.body {
                            let message = Message::Response(response.with_body(body.clone()));
                            self.state = ParseState::Complete(message);
                        } else {
                            let message = Message::Response(response);
                            self.state = ParseState::Complete(message);
                        }
                    }
                    Err(e) => {
                        self.state = ParseState::Failed(Error::Parser(format!("Invalid response line: {:?}", e)));
                    }
                }
            } else {
                // Request
                match parse_request_line(start_line) {
                    Ok((_, (method, uri, version))) => {
                        // Create request using the correct constructor
                        let mut request = Request::new(method, uri);
                        
                        // Add headers
                        for header in &self.headers {
                            request = request.with_header(header.clone());
                        }
                        
                        if let Some(body) = &self.body {
                            let message = Message::Request(request.with_body(body.clone()));
                            self.state = ParseState::Complete(message);
                        } else {
                            let message = Message::Request(request);
                            self.state = ParseState::Complete(message);
                        }
                    }
                    Err(e) => {
                        self.state = ParseState::Failed(Error::Parser(format!("Invalid request line: {:?}", e)));
                    }
                }
            }
        } else {
            self.state = ParseState::Failed(Error::Parser("No start line".to_string()));
        }
    }
    
    /// Take the completed message, resetting the parser
    pub fn take_message(&mut self) -> Option<Message> {
        if let ParseState::Complete(ref message) = self.state {
            let message = message.clone();
            self.reset();
            Some(message)
        } else {
            None
        }
    }
}

/// Parse a SIP message from a string
pub fn parse_message(input: &str) -> Result<Message> {
    // Try to parse as a request first
    if let Ok((_, message)) = request_parser(input) {
        return Ok(message);
    }
    
    // Try to parse as a response
    if let Ok((_, message)) = response_parser(input) {
        return Ok(message);
    }
    
    Err(Error::Parser("Failed to parse as request or response".to_string()))
}

/// Parse a SIP message from bytes
pub fn parse_message_bytes(input: &[u8]) -> Result<Message> {
    // Convert bytes to string for parsing
    match std::str::from_utf8(input) {
        Ok(text) => parse_message(text),
        Err(e) => Err(Error::Parser(format!("Invalid UTF-8 data: {}", e))),
    }
}

/// Parser for a SIP request line
fn parse_request_line(input: &str) -> IResult<&str, (Method, Uri, Version)> {
    let (input, method) = map_res(
        take_while1(|c: char| c.is_alphabetic() || c == '_'),
        |s: &str| Method::from_str(s)
    )(input)?;
    
    let (input, _) = space1(input)?;
    
    let (input, uri_str) = take_till(|c| c == ' ')(input)?;
    let uri = match parse_uri(uri_str) {
        Ok(uri) => uri,
        Err(e) => return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag))),
    };
    
    let (input, _) = space1(input)?;
    
    let (input, version) = map_res(
        take_till(|c| c == '\r' || c == '\n'),
        |s: &str| Version::from_str(s)
    )(input)?;
    
    Ok((input, (method, uri, version)))
}

/// Parser for a SIP response line
fn parse_response_line(input: &str) -> IResult<&str, (Version, StatusCode, String)> {
    let (input, version) = map_res(
        take_till(|c| c == ' '),
        |s: &str| Version::from_str(s)
    )(input)?;
    
    let (input, _) = space1(input)?;
    
    // First map to u16, then handle potential errors when creating StatusCode
    let (input, status_code) = map_res(
        digit1,
        |s: &str| s.parse::<u16>()
    )(input)?;
    
    // Convert u16 to StatusCode
    let status = match StatusCode::from_u16(status_code) {
        Ok(status) => status,
        Err(_) => return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag))),
    };
    
    let (input, _) = space1(input)?;
    
    let (input, reason) = map(
        take_till(|c| c == '\r' || c == '\n'),
        |s: &str| s.to_string()
    )(input)?;
    
    Ok((input, (version, status, reason)))
}

/// Parser for a complete SIP request
fn request_parser(input: &str) -> IResult<&str, Message> {
    // Parse the request line
    let (input, (method, uri, version)) = terminated(
        parse_request_line,
        crlf
    )(input)?;
    
    // Parse headers
    let (input, headers) = terminated(
        many0(super::headers::header_parser),
        crlf
    )(input)?;
    
    // Create the request
    let mut request = Request::new(method, uri);
    
    // Add headers
    for header in headers {
        request = request.with_header(header);
    }
    
    // Parse the body if present
    let body = input.trim_end_matches(|c| c == '\r' || c == '\n');
    if !body.is_empty() {
        request = request.with_body(body.to_string());
    }
    
    Ok(("", Message::Request(request)))
}

/// Parser for a complete SIP response
fn response_parser(input: &str) -> IResult<&str, Message> {
    // Parse the response line
    let (input, (version, status, reason)) = terminated(
        parse_response_line,
        crlf
    )(input)?;
    
    // Parse headers
    let (input, headers) = terminated(
        many0(super::headers::header_parser),
        crlf
    )(input)?;
    
    // Create the response
    let mut response = Response::new(status)
        .with_reason(reason);
    
    // Add headers
    for header in headers {
        response = response.with_header(header);
    }
    
    // Parse the body if present
    let body = input.trim_end_matches(|c| c == '\r' || c == '\n');
    if !body.is_empty() {
        response = response.with_body(body.to_string());
    }
    
    Ok(("", Message::Response(response)))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_request() {
        let input = "INVITE sip:bob@example.com SIP/2.0\r\n\
                     Via: SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds\r\n\
                     Max-Forwards: 70\r\n\
                     To: Bob <sip:bob@example.com>\r\n\
                     From: Alice <sip:alice@example.com>;tag=1928301774\r\n\
                     Call-ID: a84b4c76e66710@pc33.example.com\r\n\
                     CSeq: 314159 INVITE\r\n\
                     Contact: <sip:alice@pc33.example.com>\r\n\
                     Content-Type: application/sdp\r\n\
                     Content-Length: 142\r\n\
                     \r\n\
                     v=0\r\n\
                     o=- 1234 1234 IN IP4 127.0.0.1\r\n\
                     s=-\r\n\
                     c=IN IP4 127.0.0.1\r\n\
                     t=0 0\r\n\
                     m=audio 49170 RTP/AVP 0\r\n\
                     a=rtpmap:0 PCMU/8000\r\n";
        
        let message = parse_message(input).unwrap();
        
        assert!(matches!(message, Message::Request(_)));
        if let Message::Request(request) = message {
            assert_eq!(request.method, Method::Invite);
            assert_eq!(request.uri.scheme.as_str(), "sip");
            assert_eq!(request.uri.host.as_str(), "example.com");
            assert_eq!(request.uri.user.as_ref().unwrap(), "bob");
            
            assert_eq!(request.headers.len(), 9);
            
            assert!(!request.body.is_empty());
            assert!(std::str::from_utf8(&request.body).unwrap().starts_with("v=0"));
        }
    }
    
    #[test]
    fn test_parse_response() {
        let input = "SIP/2.0 200 OK\r\n\
                     Via: SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds;received=10.0.0.1\r\n\
                     To: Bob <sip:bob@example.com>;tag=a6c85cf\r\n\
                     From: Alice <sip:alice@example.com>;tag=1928301774\r\n\
                     Call-ID: a84b4c76e66710@pc33.example.com\r\n\
                     CSeq: 314159 INVITE\r\n\
                     Contact: <sip:bob@192.168.0.2>\r\n\
                     Content-Type: application/sdp\r\n\
                     Content-Length: 131\r\n\
                     \r\n\
                     v=0\r\n\
                     o=- 1234 1234 IN IP4 192.168.0.2\r\n\
                     s=-\r\n\
                     c=IN IP4 192.168.0.2\r\n\
                     t=0 0\r\n\
                     m=audio 49170 RTP/AVP 0\r\n\
                     a=rtpmap:0 PCMU/8000\r\n";
        
        let message = parse_message(input).unwrap();
        
        assert!(matches!(message, Message::Response(_)));
        if let Message::Response(response) = message {
            assert_eq!(response.status, StatusCode::Ok);
            assert_eq!(response.reason.as_deref().unwrap(), "OK");
            
            assert_eq!(response.headers.len(), 8);
            
            assert!(!response.body.is_empty());
            assert!(std::str::from_utf8(&response.body).unwrap().starts_with("v=0"));
        }
    }
    
    #[test]
    fn test_incremental_parser() {
        let mut parser = IncrementalParser::new();
        
        // Simulate receiving data in chunks
        let chunks = [
            "INVITE sip:bob@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds\r\n",
            "To: Bob <sip:bob@example.com>\r\n",
            "From: Alice <sip:alice@example.com>;tag=1928301774\r\n",
            "Call-ID: a84b4c76e66710@pc33.example.com\r\n",
            "CSeq: 314159 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n", // Empty line marks end of headers
        ];
        
        for (i, chunk) in chunks.iter().enumerate() {
            let state = parser.parse(chunk);
            
            if i < chunks.len() - 1 {
                assert!(matches!(state, ParseState::WaitingForStartLine) || 
                       matches!(state, ParseState::ParsingHeaders));
            } else {
                assert!(matches!(state, ParseState::Complete(_)));
            }
        }
        
        let message = parser.take_message().unwrap();
        assert!(matches!(message, Message::Request(_)));
        if let Message::Request(request) = message {
            assert_eq!(request.method, Method::Invite);
            assert_eq!(request.headers.len(), 6);
        }
    }
    
    #[test]
    fn test_incremental_parser_with_body() {
        let mut parser = IncrementalParser::new();
        
        // Simulate receiving data in chunks
        let chunks = [
            "SIP/2.0 200 OK\r\n",
            "Via: SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds\r\n",
            "To: Bob <sip:bob@example.com>;tag=a6c85cf\r\n",
            "Content-Type: text/plain\r\n",
            "Content-Length: 13\r\n",
            "\r\n", // Empty line marks end of headers
            "Hello, world!", // Body
        ];
        
        for (i, chunk) in chunks.iter().enumerate() {
            let state = parser.parse(chunk);
            
            if i < chunks.len() - 1 {
                assert!(!matches!(state, ParseState::Complete(_)));
            } else {
                assert!(matches!(state, ParseState::Complete(_)));
            }
        }
        
        let message = parser.take_message().unwrap();
        assert!(matches!(message, Message::Response(_)));
        if let Message::Response(response) = message {
            assert_eq!(response.status, StatusCode::Ok);
            assert_eq!(response.body, Bytes::from("Hello, world!"));
        }
    }
} 
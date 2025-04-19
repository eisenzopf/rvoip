use std::str::FromStr;

use bytes::Bytes;
use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_till, take_until, take_while, take_while1},
    character::complete::{char, digit1, line_ending, space0, space1},
    combinator::{map, map_res, opt, recognize, verify},
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

// Now use the new parser modules
use super::headers::parse_header;
use super::request::{request_parser, parse_request_line};
use super::response::{response_parser, parse_response_line};
use super::uri::parse_uri;
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
    /// Debug mode for increased logging
    debug_mode: bool,
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
            debug_mode: false,
        }
    }
    
    /// Create a new incremental parser with debug mode enabled
    pub fn new_with_debug() -> Self {
        Self {
            state: ParseState::WaitingForStartLine,
            buffer: String::new(),
            start_line: None,
            headers: Vec::new(),
            body: None,
            debug_mode: true,
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
        
        if self.debug_mode {
            println!("IncrementalParser: Received chunk: {:?}", data);
            println!("IncrementalParser: Current state: {:?}", self.state);
            println!("IncrementalParser: Buffer now contains {} bytes", self.buffer.len());
        }
        
        match self.state {
            ParseState::WaitingForStartLine => {
                // Try to find the start line, accepting either CRLF or just LF as line ending
                let idx_crlf = self.buffer.find("\r\n");
                let idx_lf = self.buffer.find("\n");
                
                // Determine which line ending was found first
                let (idx, end_len) = match (idx_crlf, idx_lf) {
                    (Some(crlf), Some(lf)) => {
                        // Handle case where \n is part of \r\n
                        if crlf + 1 == lf { (crlf, 2) } else if crlf < lf { (crlf, 2) } else { (lf, 1) }
                    },
                    (Some(crlf), None) => (crlf, 2),
                    (None, Some(lf)) => (lf, 1),
                    (None, None) => return &self.state,
                };
                
                if idx > MAX_LINE_LENGTH {
                    self.state = ParseState::Failed(Error::LineTooLong(idx));
                    return &self.state;
                }
                
                // Get the start line and remove it from the buffer
                let start_line = self.buffer[..idx].trim().to_string();
                self.start_line = Some(start_line);
                self.buffer.drain(..idx + end_len);
                
                if self.debug_mode {
                    println!("IncrementalParser: Found start line: {:?}", self.start_line);
                    println!("IncrementalParser: Buffer size after removing start line: {}", self.buffer.len());
                }
                
                self.state = ParseState::ParsingHeaders;
                
                // Continue parsing headers
                return self.parse("");
            }
            ParseState::ParsingHeaders => {
                // Look for the end of headers (empty line, accepting either \r\n\r\n or \n\n or mixed formats)
                // We need to handle multiple cases: \r\n\r\n, \n\n, \r\n\n, \n\r\n
                let possible_endings = ["\r\n\r\n", "\n\n", "\r\n\n", "\n\r\n"];
                
                let mut min_idx = None;
                let mut end_len = 0;
                
                for &ending in &possible_endings {
                    if let Some(idx) = self.buffer.find(ending) {
                        if min_idx.is_none() || idx < min_idx.unwrap() {
                            min_idx = Some(idx);
                            end_len = ending.len();
                        }
                    }
                }
                
                if min_idx.is_none() {
                    // Need more data - no complete header section found yet
                    return &self.state;
                }
                
                let idx = min_idx.unwrap();
                
                // Found end of headers
                let header_section = self.buffer[..idx].to_string();
                
                // Save the buffer after headers for body processing
                let body_start = idx + end_len;
                let body_data = if body_start < self.buffer.len() {
                    self.buffer[body_start..].to_string()
                } else {
                    String::new()
                };
                
                // Clear the buffer
                self.buffer.clear();
                
                if self.debug_mode {
                    println!("IncrementalParser: Found end of headers at position {}", idx);
                    println!("IncrementalParser: Header section:\n{}", header_section);
                    println!("IncrementalParser: Body data initially available: {} bytes", body_data.len());
                }
                
                // Parse the headers
                let mut headers = Vec::new();
                
                // First normalize all line endings to LF
                let normalized_lf = header_section.replace("\r\n", "\n");
                
                // Split into lines
                let lines: Vec<&str> = normalized_lf.split('\n').collect();
                
                if self.debug_mode {
                    println!("IncrementalParser: Processing {} header lines", lines.len());
                }
                
                // Process headers with folding
                let mut i = 0;
                while i < lines.len() {
                    let line = lines[i];
                    
                    if line.is_empty() {
                        i += 1;
                        continue;
                    }
                    
                    // Check if this is a header line (not a continuation)
                    if !line.starts_with(' ') && !line.starts_with('\t') && line.contains(':') {
                        let mut header_value = line.to_string();
                        
                        // Look ahead for folded lines (continuations)
                        let mut j = i + 1;
                        while j < lines.len() {
                            if lines[j].starts_with(' ') || lines[j].starts_with('\t') {
                                // This is a continuation - append it with proper spacing
                                if !header_value.ends_with(' ') {
                                    header_value.push(' ');
                                }
                                header_value.push_str(lines[j].trim());
                                j += 1;
                            } else {
                                // Not a continuation
                                break;
                            }
                        }
                        
                        // Add CRLF for header parsing
                        let header_str = format!("{}\r\n", header_value);
                        
                        // Parse the header
                        match parse_header(&header_str) {
                            Ok(header) => {
                                if self.debug_mode {
                                    println!("IncrementalParser: Parsed header: {}", header_value);
                                }
                                headers.push(header);
                            },
                            Err(e) => {
                                // Decide how to handle header parsing errors (ignore, return error?)
                                // For now, let's collect it as an Other header
                                let parts: Vec<&str> = header_str.splitn(2, ':').collect();
                                if parts.len() == 2 {
                                    headers.push(Header {
                                        name: HeaderName::Other(parts[0].trim().to_string()),
                                        value: HeaderValue::Raw(parts[1].trim().to_string()), // Store raw value on error
                                    });
                                } else {
                                     // Malformed header line, couldn't even split name/value
                                     println!("Malformed header line skipped: {}", header_str); // Log or return error?
                                }
                            }
                        }
                        
                        // Move to the next line after processing any continuations
                        i = j;
                    } else {
                        // Not a valid header, skip it
                        i += 1;
                    }
                }
                
                if self.debug_mode {
                    println!("IncrementalParser: Parsed {} headers", headers.len());
                    for header in &headers {
                        println!("  {} = {}", header.name, header.value);
                    }
                }
                
                // Store the headers
                self.headers = headers;
                
                // Check Content-Length to determine if we have a body
                let content_length = self.get_content_length().unwrap_or(0);
                
                if content_length > MAX_BODY_SIZE {
                    self.state = ParseState::Failed(Error::BodyTooLarge(content_length));
                    return &self.state;
                }
                
                if content_length == 0 {
                    // No body, message is complete
                    self.complete_message();
                    return &self.state;
                } else {
                    // Has body - add any existing body data to the buffer
                    self.buffer = body_data;
                    
                    // Set state to parsing body
                    self.state = ParseState::ParsingBody {
                        content_length,
                        bytes_parsed: 0,
                    };
                    
                    // Continue parsing the body if there's data already in the buffer
                    if !self.buffer.is_empty() {
                        return self.parse("");
                    }
                }
            }
            ParseState::ParsingBody { content_length, bytes_parsed } => {
                let buffer_len = self.buffer.len();
                let total_bytes = bytes_parsed + buffer_len;
                
                if self.debug_mode {
                    println!("IncrementalParser: Parsing body - have {} bytes, need {}", total_bytes, content_length);
                }
                
                if total_bytes >= content_length {
                    // We have the entire body
                    let needed = content_length - bytes_parsed;
                    
                    if self.debug_mode {
                        println!("IncrementalParser: Got complete body, needed {} bytes", needed);
                    }
                    
                    if needed <= buffer_len {
                        // Preserve the exact body content without modifying line endings
                        let body = self.buffer.drain(..needed).collect::<String>();
                        self.body = Some(body);
                        
                        if self.debug_mode {
                            if let Some(body) = &self.body {
                                println!("IncrementalParser: Body extracted, length: {}", body.len());
                                println!("IncrementalParser: Body content: {}", body);
                            }
                        }
                        
                        self.complete_message();
                    } else {
                        self.state = ParseState::Failed(Error::Parser(format!("Content-Length mismatch")));
                    }
                } else {
                    // Still need more data
                    self.state = ParseState::ParsingBody {
                        content_length,
                        bytes_parsed: total_bytes,
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
                        if !reason.is_empty() {
                        response = response.with_reason(reason);
                        }
                        
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
                        request.version = version;
                        
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

/// Parse a SIP message from a string or bytes
pub fn parse_message(input: impl AsRef<[u8]>) -> Result<Message> {
    // Convert input to string if needed
    let input_str = match std::str::from_utf8(input.as_ref()) {
        Ok(s) => s,
        Err(e) => return Err(Error::Parser(format!("Invalid UTF-8 data: {}", e))),
    };
    
    // Check if input is empty
    if input_str.trim().is_empty() {
        return Err(Error::Parser("Empty message".to_string()));
    }
    
    // Try to parse with incremental parser first for more accurate body handling
    let mut parser = IncrementalParser::new();
    let state = parser.parse(input_str);
    
    match state {
        ParseState::Complete(_) => {
            if let Some(message) = parser.take_message() {
                return Ok(message);
            }
        },
        ParseState::Failed(error) => {
            // If the incremental parser explicitly failed, report that error
            return Err(error.clone());
        },
        _ => {
            // If not complete yet, try the regular parsers
        }
    }
    
    // Otherwise try the regular parsers (calling the new modules)
    if let Ok((_, message)) = request_parser(input_str) {
        return Ok(message);
    }
    
    if let Ok((_, message)) = response_parser(input_str) {
        return Ok(message);
    }
    
    // If both failed, try normalizing the line endings and try again
    let normalized_input = input_str.replace("\r\n", "\n").replace("\n", "\r\n");
    
    // Ensure message ends with at least one CRLF
    let normalized_input = if !normalized_input.ends_with("\r\n") {
        format!("{}\r\n", normalized_input)
    } else {
        normalized_input
    };
    
    // Try again with normalized input
    if let Ok((_, message)) = request_parser(&normalized_input) {
        return Ok(message);
    }
    
    if let Ok((_, message)) = response_parser(&normalized_input) {
        return Ok(message);
    }
    
    // If we get here, all parsing attempts failed
    Err(Error::Parser("Failed to parse as request or response".to_string()))
}

/// Parse a SIP message from bytes (legacy API, kept for compatibility)
pub fn parse_message_bytes(input: &[u8]) -> Result<Message> {
    // Use the new implementation
    parse_message(input)
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

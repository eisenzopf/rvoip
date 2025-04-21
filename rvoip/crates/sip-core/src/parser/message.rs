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

use nom::error::{Error as NomError, ErrorKind};
use std::collections::HashMap;

use crate::error::{Error, Result};
use crate::header::{Header, HeaderName, HeaderValue};
use crate::types::{Message, Request, Response, StatusCode, Method};
use crate::uri::Uri;
use crate::version::Version;

// Now use the new parser modules
use crate::parser::headers::{parse_header, parse_headers, header_parser as single_nom_header_parser};
use crate::parser::request::parse_request_line;
use crate::parser::response::parse_response_line;
use crate::parser::utils::crlf;

/// Maximum length of a single line in a SIP message
pub const MAX_LINE_LENGTH: usize = 4096;
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
                
                // Process header section first
                let header_section = self.buffer[..idx].to_string(); 

                // Clone body slice to owned String *first*
                let body_start = idx + end_len; 
                let mut body_data = if body_start < self.buffer.len() { // Make body_data mutable
                    self.buffer[body_start..].to_string() 
                } else { String::new() };
                let body_data_len = body_data.len();

                // Clear buffer now that body_data is owned
                self.buffer.clear(); 
                
                if self.debug_mode {
                    println!("IncrementalParser: Found end of headers at position {}", idx);
                    println!("IncrementalParser: Header section:\n{}", header_section);
                    println!("IncrementalParser: Body slice initially available: {} bytes", body_data_len);
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
                
                // --- Validate Single-Value Headers --- 
                let mut found_cl = false;
                let mut found_cseq = false;
                // Add other single-value headers as needed (e.g., Call-ID, Max-Forwards)
                for header in &self.headers {
                    match header.name {
                        HeaderName::ContentLength => {
                            if found_cl { 
                                self.state = ParseState::Failed(Error::InvalidHeader("Multiple Content-Length headers".to_string()));
                            }
                            found_cl = true;
                        }
                        HeaderName::CSeq => {
                             if found_cseq { 
                                self.state = ParseState::Failed(Error::InvalidHeader("Multiple CSeq headers".to_string()));
                            }
                            found_cseq = true;
                        }
                        // Add checks for other single-value headers here
                        _ => {}
                    }
                }
                // --------------------------------------
                
                // Check Content-Length
                let content_length = self.get_content_length().unwrap_or(0);
                
                if content_length > MAX_BODY_SIZE {
                    self.state = ParseState::Failed(Error::BodyTooLarge(content_length));
                    return &self.state;
                }

                if content_length == 0 {
                    self.complete_message();
                    return &self.state;
                } else {
                    // Has body - need to transition
                    self.buffer = body_data;
                    self.state = ParseState::ParsingBody {
                        content_length,
                        bytes_parsed: 0,
                    };
                    if !self.buffer.is_empty() {
                        return self.parse(""); 
                    }
                    return &self.state;
                }
            }
            ParseState::ParsingBody { content_length, bytes_parsed } => {
                let buffer_len = self.buffer.len();
                let total_bytes = bytes_parsed + buffer_len;
                
                if self.debug_mode {
                    println!("IncrementalParser: Parsing body - have {} bytes, need {}", total_bytes, content_length);
                }
                
                if total_bytes >= content_length {
                    let needed = content_length - bytes_parsed;
                    if needed <= buffer_len {
                        let body = self.buffer.drain(..needed).collect::<String>();
                        self.body = Some(body);
                        self.complete_message();
                    } else {
                        self.state = ParseState::Failed(Error::Parser(format!("Content-Length mismatch")));
                    }
                } else {
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
                return match &header.value {
                    HeaderValue::Integer(int) => {
                        if *int >= 0 {
                            Some(*int as usize)
                        } else {
                            None
                        }
                    },
                    HeaderValue::Text(text) => {
                        text.trim().parse::<usize>().ok()
                    },
                     HeaderValue::Raw(raw) => {
                        raw.trim().parse::<usize>().ok()
                    },
                    _ => None
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

/// Top-level nom parser for a full SIP message (Request or Response)
fn full_message_parser(input: &str) -> IResult<&str, Message> {
    // 1. Parse Start Line
    let (rest, start_line_data) = alt((
        map(parse_request_line, |(m, u, v)| (true, Some(m), Some(u), Some(v), None, None)),
        map(parse_response_line, |(v, s, r)| (false, None, None, Some(v), Some(s), Some(r)))
    ))(input)?;
    
    let (is_request, method, uri, version, status, reason) = start_line_data;
    
    // 2. Parse Headers line by line until empty line marker
    let mut header_input = rest;
    let mut headers = Vec::new();
    loop {
        // Check for end of headers
        if header_input.starts_with("\r\n") { // Found empty line
            header_input = &header_input[2..]; // Consume CRLF
            break;
        } else if header_input.is_empty() {
            // Reached end unexpectedly before empty line
            break; 
        }
        
        // Attempt to parse a header line using the basic nom parser
        match single_nom_header_parser(header_input) {
            Ok((remaining, header)) => {
                 // Check for required CRLF termination after header value
                 if !remaining.starts_with("\r\n") {
                      // Header value didn't end with CRLF - parse error
                      return Err(nom::Err::Error(NomError::new(header_input, ErrorKind::CrLf)));
                 }
                 headers.push(header);
                 header_input = &remaining[2..]; // Consume CRLF
            }
            Err(nom::Err::Error(_)) | Err(nom::Err::Failure(_))=> {
                // Failed to parse as header, maybe malformed? Stop parsing headers.
                // We might lose body if this happens before the actual empty line.
                 break; 
            }
            Err(nom::Err::Incomplete(_)) => {
                 // Need more data
                 return Err(nom::Err::Incomplete(Needed::Unknown)); 
            }
        }
    }
    
    // header_input now contains the body + any trailing data
    let body_and_rest = header_input; 

    // 3. Extract Content-Length
    let content_length = headers.iter().find_map(|h| {
        if h.name == HeaderName::ContentLength {
            h.value.to_string_value().parse::<usize>().ok()
        } else {
            None
        }
    }).unwrap_or(0);

    // 4. Parse Body 
    if body_and_rest.len() < content_length {
        return Err(nom::Err::Incomplete(Needed::new(content_length - body_and_rest.len())));
    }
    let (final_rest, body_bytes) = nom::bytes::complete::take(content_length)(body_and_rest)?;
    let body_str = String::from_utf8_lossy(body_bytes.as_bytes()).to_string(); // Assuming UTF-8 body for now
    
    // 5. Construct Message
    if is_request {
        let mut req = Request::new(method.unwrap(), uri.unwrap());
        req.version = version.unwrap();
        req.headers = headers;
        if content_length > 0 { req.body = Bytes::copy_from_slice(body_bytes.as_bytes()); }
        Ok((final_rest, Message::Request(req)))
    } else {
        let mut resp = Response::new(status.unwrap());
        resp.version = version.unwrap();
        if let Some(r) = reason { if !r.is_empty() { resp = resp.with_reason(r); } }
        resp.headers = headers;
        if content_length > 0 { resp.body = Bytes::copy_from_slice(body_bytes.as_bytes()); }
        Ok((final_rest, Message::Response(resp)))
    }
}

/// Parse a SIP message from a string or bytes
pub fn parse_message(input: impl AsRef<[u8]>) -> Result<Message> {
    // Convert input to string if needed
    let input_str = match std::str::from_utf8(input.as_ref()) {
        Ok(s) => s,
        Err(e) => return Err(Error::Parser(format!("Invalid UTF-8 data: {}", e))),
    };
    
    if input_str.trim().is_empty() {
        return Err(Error::Parser("Empty message".to_string()));
    }
    
    // Use direct nom parser for full input
    match full_message_parser(input_str) {
        Ok((rest, message)) => {
            // Check if the entire input was consumed
            if !rest.is_empty() {
                 Err(Error::Parser(format!("Trailing data after message: {:?}", rest)))
            } else {
                 Ok(message)
            }
        },
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => {
             Err(Error::Parser(format!("Failed to parse message near '{}': {:?}", &input_str[..input_str.len() - e.input.len()], e.code)))
        },
        Err(nom::Err::Incomplete(needed)) => {
             Err(Error::IncompleteParse(format!("Incomplete message: Needed {:?}", needed)))
        }
    }
}

/// Parse a SIP message from bytes (legacy API, kept for compatibility)
pub fn parse_message_bytes(input: &[u8]) -> Result<Message> {
    // Use the new implementation
    parse_message(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use crate::uri::Host; // Import Host for assertions
    use crate::types::Request; // Import Request
    use crate::types::Response; // Import Response
    
    #[test]
    fn test_parse_request_full() { // Renamed test
        let input = "INVITE sip:bob@example.com SIP/2.0\r\n\
                     Via: SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds\r\n\
                     Max-Forwards: 70\r\n\
                     To: Bob <sip:bob@example.com>\r\n\
                     From: Alice <sip:alice@example.com>;tag=1928301774\r\n\
                     Call-ID: a84b4c76e66710@pc33.example.com\r\n\
                     CSeq: 314159 INVITE\r\n\
                     Contact: <sip:alice@pc33.example.com>\r\n\
                     Content-Type: application/sdp\r\n\
                     Content-Length: 128\r\n\
                     \r\n\
                     v=0\r\n\
                     o=- 1234 1234 IN IP4 127.0.0.1\r\n\
                     s=-\r\n\
                     c=IN IP4 127.0.0.1\r\n\
                     t=0 0\r\n\
                     m=audio 49170 RTP/AVP 0\r\n\
                     a=rtpmap:0 PCMU/8000\r\n";
        
        // Test feeding the full input at once using the new parse_message
        let result = parse_message(input.as_bytes());
        assert!(result.is_ok(), "parse_message failed: {:?}", result.err());
        
        let message = result.unwrap();
        
        // Check message content
        assert!(matches!(message, Message::Request(_)));
        if let Message::Request(request) = message { 
            assert_eq!(request.method, Method::Invite);
            assert_eq!(request.uri.scheme.as_str(), "sip");
            // Check Host enum directly
            assert_eq!(request.uri.host, Host::Domain("example.com".to_string()));
            assert_eq!(request.uri.user.as_ref().unwrap(), "bob");
            
            assert_eq!(request.headers.len(), 9);
            
            assert!(!request.body.is_empty());
            let body_str = std::str::from_utf8(&request.body).unwrap();
            assert!(body_str.starts_with("v=0"));
            assert_eq!(request.body.len(), 128); 
        } else {
            panic!("Parsed message was not a Request");
        }
    }
    
    #[test]
    fn test_parse_response_full() { // Renamed test
        let input = "SIP/2.0 200 OK\r\n\
                     Via: SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds;received=10.0.0.1\r\n\
                     To: Bob <sip:bob@example.com>;tag=a6c85cf\r\n\
                     From: Alice <sip:alice@example.com>;tag=1928301774\r\n\
                     Call-ID: a84b4c76e66710@pc33.example.com\r\n\
                     CSeq: 314159 INVITE\r\n\
                     Contact: <sip:bob@192.168.0.2>\r\n\
                     Content-Type: application/sdp\r\n\
                     Content-Length: 130\r\n\
                     \r\n\
                     v=0\r\n\
                     o=- 1234 1234 IN IP4 192.168.0.2\r\n\
                     s=-\r\n\
                     c=IN IP4 192.168.0.2\r\n\
                     t=0 0\r\n\
                     m=audio 49170 RTP/AVP 0\r\n\
                     a=rtpmap:0 PCMU/8000\r\n";
        
        // Test feeding the full input at once using the new parse_message
        let result = parse_message(input.as_bytes());
        assert!(result.is_ok(), "parse_message failed: {:?}", result.err());

        let message = result.unwrap();
        
        // Check message content
        assert!(matches!(message, Message::Response(_)));
        if let Message::Response(response) = message { 
            assert_eq!(response.status, StatusCode::Ok);
            assert_eq!(response.reason.as_deref().unwrap(), "OK");
            
            assert_eq!(response.headers.len(), 8);
            
            assert!(!response.body.is_empty());
            let body_str = std::str::from_utf8(&response.body).unwrap();
            assert!(body_str.starts_with("v=0"));
            assert_eq!(response.body.len(), 130); 
        } else {
            panic!("Parsed message was not a Response");
        }
    }
    
    // Keep incremental tests - they should still pass if parser logic is sound
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

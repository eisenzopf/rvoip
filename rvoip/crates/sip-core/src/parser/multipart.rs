use std::collections::HashMap;
use std::str::FromStr;

use nom::{
    branch::alt,
    bytes::complete::{tag, take_till, take_until, take_while, take_while1},
    character::complete::{char, line_ending, multispace0, space0, space1},
    combinator::{map, map_res, opt, recognize, value, verify},
    multi::{many0, many1, many_till, separated_list0, separated_list1},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    IResult,
};

use crate::error::{Error, Result};
use super::utils::{crlf, parse_param_name, parse_param_value, parse_semicolon_params};
use super::headers::header_parser;

/// A single part in a multipart MIME body
#[derive(Debug, Clone, PartialEq)]
pub struct MimePart {
    /// Headers for this part
    pub headers: HashMap<String, String>,
    /// Content of this part
    pub content: String,
}

impl MimePart {
    /// Create a new MIME part
    pub fn new() -> Self {
        Self {
            headers: HashMap::new(),
            content: String::new(),
        }
    }
    
    /// Get a header value, case-insensitive
    pub fn get_header(&self, name: &str) -> Option<&str> {
        let name_lower = name.to_lowercase();
        for (key, value) in &self.headers {
            if key.to_lowercase() == name_lower {
                return Some(value);
            }
        }
        None
    }
    
    /// Get the content-type of this part
    pub fn content_type(&self) -> Option<&str> {
        self.get_header("content-type")
    }
    
    /// Get the content-disposition of this part
    pub fn content_disposition(&self) -> Option<&str> {
        self.get_header("content-disposition")
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
    
    /// Find a part by its content-type (substring match)
    pub fn find_by_content_type(&self, content_type: &str) -> Option<&MimePart> {
        self.parts.iter().find(|part| {
            part.content_type()
                .map(|ct| ct.contains(content_type))
                .unwrap_or(false)
        })
    }
    
    /// Get the SDP part if present
    pub fn sdp_part(&self) -> Option<&MimePart> {
        self.find_by_content_type("application/sdp")
    }
    
    /// Get the SDP content if present
    pub fn sdp_content(&self) -> Option<&str> {
        self.sdp_part().map(|part| part.content.as_str())
    }
}

/// Parse a multipart body with the given boundary
pub fn parse_multipart(content: &str, boundary: &str) -> Result<MultipartBody> {
    let full_boundary = format!("--{}", boundary);
    
    match multipart_parser(content, &full_boundary) {
        Ok((_, mut body)) => {
            body.boundary = boundary.to_string();
            Ok(body)
        },
        Err(e) => Err(Error::Parser(format!("Failed to parse multipart body: {:?}", e))),
    }
}

/// Parser for a multipart body
fn multipart_parser<'a>(input: &'a str, boundary: &'a str) -> IResult<&'a str, MultipartBody> {
    let end_boundary = format!("{}--", boundary);
    
    // Skip any preamble before the first boundary
    let (mut input, _) = match take_until::<_, _, nom::error::Error<&str>>(boundary)(input) {
        Ok(result) => result,
        Err(_) => (input, ""), // No boundary found
    };
    
    // Create the multipart body
    let mut body = MultipartBody::new(boundary.trim_start_matches("--"));
    
    // Parse parts until we reach the end boundary or run out of input
    while let Ok((new_input, _)) = tag::<_, _, nom::error::Error<&str>>(boundary)(input) {
        input = new_input;
        
        // Skip CRLF after boundary
        if let Ok((new_input, _)) = super::utils::crlf(input) {
            input = new_input;
            
            // Parse headers
            let mut headers = HashMap::new();
            let mut content_start = input;
            
            // Keep parsing headers until we find an empty line
            while let Ok((new_input, header)) = super::headers::header_parser(content_start) {
                if let Some(value) = header.value.as_text() {
                    headers.insert(header.name.to_string(), value.to_string());
                }
                
                content_start = new_input;
                
                // Check if the next line is empty (end of headers)
                if let Ok((after_empty, _)) = super::utils::crlf(content_start) {
                    content_start = after_empty;
                    break;
                }
            }
            
            // Now find the next boundary
            if let Ok((new_input, content)) = take_until::<_, _, nom::error::Error<&str>>(boundary)(content_start) {
                // Create and add the part
                let mut part = MimePart::new();
                part.headers = headers;
                part.content = content.trim().to_string();
                body.add_part(part);
                
                input = new_input;
                continue;
            } else if let Ok((new_input, content)) = take_until::<_, _, nom::error::Error<&str>>(&end_boundary)(content_start) {
                // Create and add the final part
                let mut part = MimePart::new();
                part.headers = headers;
                part.content = content.trim().to_string();
                body.add_part(part);
                
                input = new_input;
                break;
            }
        }
        
        // If we reach here, something went wrong with parsing
        break;
    }
    
    Ok((input, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_simple_multipart() {
        let boundary = "boundary1";
        let content = "\r\n--boundary1\r\n\
                       Content-Type: text/plain\r\n\
                       \r\n\
                       This is the first part.\r\n\
                       --boundary1\r\n\
                       Content-Type: application/sdp\r\n\
                       \r\n\
                       v=0\r\n\
                       o=- 1234 1234 IN IP4 127.0.0.1\r\n\
                       s=-\r\n\
                       --boundary1--\r\n";
        
        let body = parse_multipart(content, boundary).unwrap();
        
        assert_eq!(body.boundary, boundary);
        assert_eq!(body.parts.len(), 2);
        
        assert_eq!(body.parts[0].content_type().unwrap(), "text/plain");
        assert_eq!(body.parts[0].content, "This is the first part.");
        
        assert_eq!(body.parts[1].content_type().unwrap(), "application/sdp");
        assert!(body.parts[1].content.contains("v=0"));
        
        let sdp = body.sdp_content().unwrap();
        assert!(sdp.contains("v=0"));
    }
    
    #[test]
    fn test_parse_complex_multipart() {
        let boundary = "boundary1";
        let content = "Preamble text\r\n\
                      --boundary1\r\n\
                      Content-Type: text/plain\r\n\
                      Content-Disposition: inline\r\n\
                      \r\n\
                      This is the first part.\r\n\
                      --boundary1\r\n\
                      Content-Type: application/sdp\r\n\
                      \r\n\
                      v=0\r\n\
                      o=- 1234 1234 IN IP4 127.0.0.1\r\n\
                      s=-\r\n\
                      --boundary1\r\n\
                      Content-Type: image/png\r\n\
                      Content-Disposition: attachment; filename=image.png\r\n\
                      \r\n\
                      [Binary data would be here]\r\n\
                      --boundary1--\r\n\
                      Epilogue text";
        
        let body = parse_multipart(content, boundary).unwrap();
        
        assert_eq!(body.parts.len(), 3);
        
        assert_eq!(body.parts[0].content_type().unwrap(), "text/plain");
        assert_eq!(body.parts[0].content_disposition().unwrap(), "inline");
        
        assert_eq!(body.parts[1].content_type().unwrap(), "application/sdp");
        
        assert_eq!(body.parts[2].content_type().unwrap(), "image/png");
        assert!(body.parts[2].content_disposition().unwrap().contains("attachment"));
    }
} 
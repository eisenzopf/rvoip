// Error-Info header type for SIP messages
// Format defined in RFC 3261 Section 20.11

use std::fmt;
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use crate::types::uri::Uri;
use crate::parser::headers::error_info::{ErrorInfoValue, parse_error_info, full_parse_error_info};
use crate::error::{Result, Error};
use std::str::FromStr;
use nom::combinator::all_consuming;
use crate::types::param::Param;

/// ErrorInfo represents an Error-Info header value
/// Used to provide additional information about errors in responses
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorInfo {
    /// URI pointing to additional information about the error
    pub uri: String,
    
    /// Optional comment explaining the error information
    pub comment: Option<String>,
    
    /// Optional parameters
    pub parameters: HashMap<String, String>,
}

impl ErrorInfo {
    /// Create a new ErrorInfo with just a URI
    pub fn new(uri: &str) -> Self {
        ErrorInfo {
            uri: uri.to_string(),
            comment: None,
            parameters: HashMap::new(),
        }
    }
    
    /// Add a comment to the ErrorInfo
    pub fn with_comment(mut self, comment: &str) -> Self {
        self.comment = Some(comment.to_string());
        self
    }
    
    /// Add a parameter to the ErrorInfo
    pub fn with_param(mut self, name: &str, value: &str) -> Self {
        self.parameters.insert(name.to_lowercase(), value.to_string());
        self
    }
}

impl fmt::Display for ErrorInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Start with the URI, potentially with angle brackets if it has spaces
        if self.uri.contains(' ') {
            write!(f, "<{}>", self.uri)?;
        } else {
            write!(f, "{}", self.uri)?;
        }
        
        // Optional comment
        if let Some(comment) = &self.comment {
            write!(f, " ({})", comment)?;
        }
        
        // Parameters if any
        for (name, value) in &self.parameters {
            write!(f, ";{}={}", name, value)?;
        }
        
        Ok(())
    }
}

/// A list of Error-Info URIs (since this header can appear multiple times)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ErrorInfoList {
    pub items: Vec<ErrorInfo>,
}

impl ErrorInfoList {
    /// Create a new empty ErrorInfoList
    pub fn new() -> Self {
        ErrorInfoList {
            items: Vec::new(),
        }
    }
    
    /// Add an ErrorInfo to the list
    pub fn add(&mut self, error_info: ErrorInfo) {
        self.items.push(error_info);
    }
    
    /// Create a builder method for adding ErrorInfo
    pub fn with(mut self, error_info: ErrorInfo) -> Self {
        self.items.push(error_info);
        self
    }
    
    /// Check if the list is empty
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    
    /// Get the number of items in the list
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

impl fmt::Display for ErrorInfoList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        
        for item in &self.items {
            if !first {
                write!(f, ", ")?;
            }
            write!(f, "{}", item)?;
            first = false;
        }
        
        Ok(())
    }
}

/// Represents a structured error-info header that can be used with the parser system 
/// Provides conversion between the structured ErrorInfoList and the parser's internal types.
pub struct ErrorInfoHeader {
    pub error_info_list: ErrorInfoList,
}

impl ErrorInfoHeader {
    /// Create a new empty ErrorInfoHeader
    pub fn new() -> Self {
        ErrorInfoHeader {
            error_info_list: ErrorInfoList::new(),
        }
    }
    
    /// Convert from parser's ErrorInfoValue to the structured ErrorInfo type
    pub fn from_error_info_value(value: &ErrorInfoValue) -> ErrorInfo {
        let mut info = ErrorInfo::new(&value.uri_str);
        
        // Convert params to parameters HashMap
        for param in &value.params {
            if let Param::Other(name, value_opt) = param {
                if let Some(value) = value_opt {
                    match value {
                        crate::types::param::GenericValue::Token(val) => {
                            info = info.with_param(name, val);
                        },
                        crate::types::param::GenericValue::Quoted(val) => {
                            info = info.with_param(name, val);
                        },
                        crate::types::param::GenericValue::Host(host) => {
                            // Convert host to string
                            info = info.with_param(name, &host.to_string());
                        },
                    }
                }
            }
        }
        
        info
    }
}

impl FromStr for ErrorInfoHeader {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let trimmed_s = s.trim();
        
        // Try parsing as a full header first (with "Error-Info:" prefix)
        let full_result = all_consuming(full_parse_error_info)(trimmed_s.as_bytes());
        if let Ok((_, values)) = full_result {
            let mut header = ErrorInfoHeader::new();
            for value in values {
                header.error_info_list.add(ErrorInfoHeader::from_error_info_value(&value));
            }
            return Ok(header);
        }
        
        // If that fails, try parsing just the value part
        let result = all_consuming(parse_error_info)(trimmed_s.as_bytes());
        match result {
            Ok((_, values)) => {
                let mut header = ErrorInfoHeader::new();
                for value in values {
                    header.error_info_list.add(ErrorInfoHeader::from_error_info_value(&value));
                }
                Ok(header)
            },
            Err(err) => Err(Error::from(err)),
        }
    }
}

impl fmt::Display for ErrorInfoHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.error_info_list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::uri::Uri;
    
    #[test]
    fn test_from_str_basic() {
        // Test parsing with just value
        let s = "<sip:busy@example.com>;reason=busy";
        let header: ErrorInfoHeader = s.parse().unwrap();
        
        assert_eq!(header.error_info_list.len(), 1);
        assert_eq!(header.error_info_list.items[0].uri, "sip:busy@example.com");
        assert_eq!(header.error_info_list.items[0].parameters.get("reason").unwrap(), "busy");
        assert!(header.error_info_list.items[0].comment.is_none());
    }
    
    #[test]
    fn test_from_str_with_header() {
        // Test parsing with header name
        let s = "Error-Info: <http://example.com/error.html>";
        let header: ErrorInfoHeader = s.parse().unwrap();
        
        assert_eq!(header.error_info_list.len(), 1);
        assert_eq!(header.error_info_list.items[0].uri, "http://example.com/error.html");
    }
    
    #[test]
    fn test_from_str_multiple() {
        // Test parsing multiple URIs
        let s = "<sip:busy@example.com>;reason=busy, <https://example.com/error.html>";
        let header: ErrorInfoHeader = s.parse().unwrap();
        
        assert_eq!(header.error_info_list.len(), 2);
        assert_eq!(header.error_info_list.items[0].uri, "sip:busy@example.com");
        assert_eq!(header.error_info_list.items[1].uri, "https://example.com/error.html");
    }
    
    #[test]
    fn test_display() {
        // Test formatting a single entry
        let mut list = ErrorInfoList::new();
        list.add(ErrorInfo::new("sip:busy@example.com").with_param("reason", "busy"));
        
        let header = ErrorInfoHeader { error_info_list: list };
        assert_eq!(header.to_string(), "sip:busy@example.com;reason=busy");
        
        // Test formatting multiple entries
        let mut list = ErrorInfoList::new();
        list.add(ErrorInfo::new("sip:busy@example.com").with_param("reason", "busy"));
        list.add(ErrorInfo::new("https://example.com/error.html"));
        
        let header = ErrorInfoHeader { error_info_list: list };
        assert_eq!(header.to_string(), "sip:busy@example.com;reason=busy, https://example.com/error.html");
    }
    
    #[test]
    fn test_empty() {
        // Test empty list
        let header = ErrorInfoHeader::new();
        assert_eq!(header.to_string(), "");
        assert!(header.error_info_list.is_empty());
    }
    
    #[test]
    fn test_add_methods() {
        // Test adding ErrorInfo objects
        let mut list = ErrorInfoList::new();
        list.add(ErrorInfo::new("sip:busy@example.com"));
        list.add(ErrorInfo::new("https://example.com/error.html"));
        
        assert_eq!(list.len(), 2);
        
        // Test using builder pattern
        let list = ErrorInfoList::new()
            .with(ErrorInfo::new("sip:busy@example.com"))
            .with(ErrorInfo::new("https://example.com/error.html"));
            
        assert_eq!(list.len(), 2);
    }
    
    #[test]
    fn test_comment_handling() {
        // Test comment handling
        let info = ErrorInfo::new("sip:busy@example.com").with_comment("User is busy");
        assert_eq!(info.to_string(), "sip:busy@example.com (User is busy)");
    }
    
    #[test]
    fn test_uri_with_spaces() {
        // Test URI with spaces (should be enclosed in angle brackets)
        let info = ErrorInfo::new("http://example.com/error message.html");
        assert_eq!(info.to_string(), "<http://example.com/error message.html>");
    }
} 
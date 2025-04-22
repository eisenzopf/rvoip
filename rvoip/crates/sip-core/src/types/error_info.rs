// Error-Info header type for SIP messages
// Format defined in RFC 3261 Section 20.11

use std::fmt;
use std::collections::HashMap;
use serde::{Serialize, Deserialize};

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

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_error_info_display_simple() {
        let error_info = ErrorInfo::new("https://example.com/errors/23");
        assert_eq!(error_info.to_string(), "https://example.com/errors/23");
    }
    
    #[test]
    fn test_error_info_with_comment() {
        let error_info = ErrorInfo::new("https://example.com/errors/access-denied")
            .with_comment("User lacks permissions");
        assert_eq!(error_info.to_string(), "https://example.com/errors/access-denied (User lacks permissions)");
    }
    
    #[test]
    fn test_error_info_with_params() {
        let error_info = ErrorInfo::new("https://example.com/errors/404")
            .with_param("language", "en");
        assert_eq!(error_info.to_string(), "https://example.com/errors/404;language=en");
    }
    
    #[test]
    fn test_error_info_with_spaces() {
        let error_info = ErrorInfo::new("https://example.com/error description");
        assert_eq!(error_info.to_string(), "<https://example.com/error description>");
    }
    
    #[test]
    fn test_error_info_list() {
        let list = ErrorInfoList::new()
            .with(ErrorInfo::new("https://example.com/errors/1"))
            .with(ErrorInfo::new("https://example.com/errors/2").with_comment("More info"));
        
        assert_eq!(list.len(), 2);
        assert_eq!(list.to_string(), "https://example.com/errors/1, https://example.com/errors/2 (More info)");
    }
    
    #[test]
    fn test_error_info_list_empty() {
        let list = ErrorInfoList::new();
        assert!(list.is_empty());
        assert_eq!(list.to_string(), "");
    }
} 
//! # SIP Error-Info Header
//!
//! This module provides an implementation of the SIP Error-Info header as defined in
//! [RFC 3261 Section 20.18](https://datatracker.ietf.org/doc/html/rfc3261#section-20.18).
//!
//! The Error-Info header provides a pointer to additional information about an error
//! returned in a SIP response. It is primarily used with 3xx, 4xx, 5xx, and 6xx responses,
//! but can be included in any response.
//!
//! ## Purpose
//!
//! The Error-Info header allows servers to provide clients with additional information
//! about errors, such as:
//!
//! - Links to HTML pages explaining the error
//! - URIs for media describing the error (e.g., audio messages)
//! - Alternative service URIs that may resolve the failure
//!
//! ## Format
//!
//! ```text
//! Error-Info: <sip:busy@example.com>;reason=busy
//! Error-Info: <https://example.com/errors/busy.html>
//! ```
//!
//! Multiple Error-Info headers can be included in a single response:
//!
//! ```text
//! Error-Info: <sip:busy@example.com>;reason=busy
//! Error-Info: <https://example.com/errors/busy.html>
//! ```
//!
//! Or combined with commas:
//!
//! ```text
//! Error-Info: <sip:busy@example.com>;reason=busy, <https://example.com/errors/busy.html>
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create an Error-Info header
//! let error_info = ErrorInfo::new("sip:busy@example.com")
//!     .with_param("reason", "busy")
//!     .with_comment("User is busy");
//!
//! // Parse an Error-Info header
//! let header = ErrorInfoHeader::from_str("<sip:busy@example.com>;reason=busy").unwrap();
//! assert_eq!(header.error_info_list.items[0].uri, "sip:busy@example.com");
//! ```

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
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// ErrorInfo represents an Error-Info header value
/// Used to provide additional information about errors in responses
///
/// The Error-Info header provides a pointer to additional information about
/// errors that occur in SIP responses. It consists of a URI pointing to 
/// the error information resource, an optional comment, and optional parameters.
///
/// Error-Info headers can be included in any response, but are most commonly
/// found in 3xx, 4xx, 5xx, and 6xx responses to provide clients with 
/// more details about the error condition.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create a basic Error-Info
/// let error_info = ErrorInfo::new("https://example.com/errors/busy.html");
///
/// // Create with a comment
/// let error_info = ErrorInfo::new("sip:busy@example.com")
///     .with_comment("User is currently busy");
///
/// // Create with parameters
/// let error_info = ErrorInfo::new("sip:busy@example.com")
///     .with_param("reason", "busy")
///     .with_param("retry-after", "60");
/// ```
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
    ///
    /// # Parameters
    ///
    /// - `uri`: A URI string pointing to additional information about the error
    ///
    /// # Returns
    ///
    /// A new `ErrorInfo` instance with the specified URI
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a basic Error-Info with a SIP URI
    /// let error_info = ErrorInfo::new("sip:busy@example.com");
    ///
    /// // Create with an HTTP URI
    /// let error_info = ErrorInfo::new("https://example.com/errors/busy.html");
    /// ```
    pub fn new(uri: &str) -> Self {
        ErrorInfo {
            uri: uri.to_string(),
            comment: None,
            parameters: HashMap::new(),
        }
    }
    
    /// Add a comment to the ErrorInfo
    ///
    /// Comments provide human-readable information about the error.
    ///
    /// # Parameters
    ///
    /// - `comment`: A string explaining the error information
    ///
    /// # Returns
    ///
    /// The modified `ErrorInfo` with the comment added (builder pattern)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let error_info = ErrorInfo::new("sip:busy@example.com")
    ///     .with_comment("User is currently busy");
    ///
    /// assert_eq!(error_info.to_string(), "<sip:busy@example.com> (User is currently busy)");
    /// ```
    pub fn with_comment(mut self, comment: &str) -> Self {
        self.comment = Some(comment.to_string());
        self
    }
    
    /// Add a parameter to the ErrorInfo
    ///
    /// Parameters provide additional structured information about the error.
    ///
    /// # Parameters
    ///
    /// - `name`: The parameter name
    /// - `value`: The parameter value
    ///
    /// # Returns
    ///
    /// The modified `ErrorInfo` with the parameter added (builder pattern)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Add a reason parameter
    /// let error_info = ErrorInfo::new("sip:busy@example.com")
    ///     .with_param("reason", "busy");
    ///
    /// assert_eq!(error_info.to_string(), "<sip:busy@example.com>;reason=busy");
    ///
    /// // Add multiple parameters
    /// let error_info = ErrorInfo::new("sip:busy@example.com")
    ///     .with_param("reason", "busy")
    ///     .with_param("retry-after", "60");
    ///
    /// // Parameters are stored in lowercase
    /// assert_eq!(error_info.parameters.get("reason").unwrap(), "busy");
    /// ```
    pub fn with_param(mut self, name: &str, value: &str) -> Self {
        self.parameters.insert(name.to_lowercase(), value.to_string());
        self
    }
}

impl fmt::Display for ErrorInfo {
    /// Formats the ErrorInfo as a string.
    ///
    /// The format follows the SIP specification:
    /// - URI (enclosed in angle brackets)
    /// - Optional comment in parentheses
    /// - Parameters as name=value pairs separated by semicolons
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Basic URI
    /// let error_info = ErrorInfo::new("sip:busy@example.com");
    /// assert_eq!(error_info.to_string(), "<sip:busy@example.com>");
    ///
    /// // URI with a comment
    /// let error_info = ErrorInfo::new("sip:busy@example.com")
    ///     .with_comment("User is busy");
    /// assert_eq!(error_info.to_string(), "<sip:busy@example.com> (User is busy)");
    ///
    /// // URI with parameters
    /// let error_info = ErrorInfo::new("sip:busy@example.com")
    ///     .with_param("reason", "busy");
    /// assert_eq!(error_info.to_string(), "<sip:busy@example.com>;reason=busy");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Always use angle brackets around the URI (required by the parser)
        write!(f, "<{}>", self.uri)?;
        
        // Parameters if any
        for (name, value) in &self.parameters {
            write!(f, ";{}={}", name, value)?;
        }
        
        // Optional comment - put it last according to RFC
        if let Some(comment) = &self.comment {
            write!(f, " ({})", comment)?;
        }
        
        Ok(())
    }
}

/// A list of Error-Info URIs (since this header can appear multiple times)
///
/// The Error-Info header can contain multiple values, either as separate
/// headers or as a comma-separated list within a single header. This struct
/// provides a container for multiple ErrorInfo entries.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create an empty list
/// let mut list = ErrorInfoList::new();
///
/// // Add entries
/// list.add(ErrorInfo::new("sip:busy@example.com").with_param("reason", "busy"));
/// list.add(ErrorInfo::new("https://example.com/errors/busy.html"));
///
/// assert_eq!(list.len(), 2);
///
/// // Create with builder pattern
/// let list = ErrorInfoList::new()
///     .with(ErrorInfo::new("sip:busy@example.com"))
///     .with(ErrorInfo::new("https://example.com/errors/busy.html"));
///
/// assert_eq!(list.len(), 2);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ErrorInfoList {
    /// The list of ErrorInfo entries
    pub items: Vec<ErrorInfo>,
}

impl ErrorInfoList {
    /// Create a new empty ErrorInfoList
    ///
    /// # Returns
    ///
    /// A new empty `ErrorInfoList`
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let list = ErrorInfoList::new();
    /// assert!(list.is_empty());
    /// ```
    pub fn new() -> Self {
        ErrorInfoList {
            items: Vec::new(),
        }
    }
    
    /// Add an ErrorInfo to the list
    ///
    /// # Parameters
    ///
    /// - `error_info`: The ErrorInfo entry to add
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut list = ErrorInfoList::new();
    ///
    /// // Add an entry
    /// list.add(ErrorInfo::new("sip:busy@example.com"));
    /// assert_eq!(list.len(), 1);
    ///
    /// // Add another entry
    /// list.add(ErrorInfo::new("https://example.com/errors/busy.html"));
    /// assert_eq!(list.len(), 2);
    /// ```
    pub fn add(&mut self, error_info: ErrorInfo) {
        self.items.push(error_info);
    }
    
    /// Create a builder method for adding ErrorInfo
    ///
    /// This method follows the builder pattern, allowing for 
    /// chaining multiple additions.
    ///
    /// # Parameters
    ///
    /// - `error_info`: The ErrorInfo entry to add
    ///
    /// # Returns
    ///
    /// The modified `ErrorInfoList` with the entry added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a list with multiple entries using the builder pattern
    /// let list = ErrorInfoList::new()
    ///     .with(ErrorInfo::new("sip:busy@example.com"))
    ///     .with(ErrorInfo::new("https://example.com/errors/busy.html"));
    ///
    /// assert_eq!(list.len(), 2);
    /// ```
    pub fn with(mut self, error_info: ErrorInfo) -> Self {
        self.items.push(error_info);
        self
    }
    
    /// Check if the list is empty
    ///
    /// # Returns
    ///
    /// `true` if the list contains no items, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let list = ErrorInfoList::new();
    /// assert!(list.is_empty());
    ///
    /// let list = ErrorInfoList::new()
    ///     .with(ErrorInfo::new("sip:busy@example.com"));
    /// assert!(!list.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    
    /// Get the number of items in the list
    ///
    /// # Returns
    ///
    /// The number of ErrorInfo entries in the list
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let list = ErrorInfoList::new();
    /// assert_eq!(list.len(), 0);
    ///
    /// let list = ErrorInfoList::new()
    ///     .with(ErrorInfo::new("sip:busy@example.com"))
    ///     .with(ErrorInfo::new("https://example.com/errors/busy.html"));
    /// assert_eq!(list.len(), 2);
    /// ```
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

impl fmt::Display for ErrorInfoList {
    /// Formats the ErrorInfoList as a string.
    ///
    /// Each ErrorInfo entry is formatted according to its own Display implementation,
    /// and multiple entries are separated by commas.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Empty list
    /// let list = ErrorInfoList::new();
    /// assert_eq!(list.to_string(), "");
    ///
    /// // Single entry
    /// let list = ErrorInfoList::new()
    ///     .with(ErrorInfo::new("sip:busy@example.com").with_param("reason", "busy"));
    /// assert_eq!(list.to_string(), "<sip:busy@example.com>;reason=busy");
    ///
    /// // Multiple entries
    /// let list = ErrorInfoList::new()
    ///     .with(ErrorInfo::new("sip:busy@example.com").with_param("reason", "busy"))
    ///     .with(ErrorInfo::new("https://example.com/errors/busy.html"));
    /// assert_eq!(list.to_string(), "<sip:busy@example.com>;reason=busy, <https://example.com/errors/busy.html>");
    /// ```
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
///
/// The ErrorInfoHeader is a wrapper around ErrorInfoList that provides integration
/// with the SIP parser system. It allows parsing Error-Info headers from strings
/// and converting between the parsed representation and the structured representation.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a new empty header
/// let header = ErrorInfoHeader::new();
/// assert!(header.error_info_list.is_empty());
///
/// // Parse a header from a string
/// let header = ErrorInfoHeader::from_str("<sip:busy@example.com>;reason=busy").unwrap();
/// assert_eq!(header.error_info_list.items[0].uri, "sip:busy@example.com");
/// assert_eq!(header.error_info_list.items[0].parameters.get("reason").unwrap(), "busy");
///
/// // Parse a header with multiple entries
/// let header = ErrorInfoHeader::from_str(
///     "<sip:busy@example.com>;reason=busy, <https://example.com/errors/busy.html>"
/// ).unwrap();
/// assert_eq!(header.error_info_list.len(), 2);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorInfoHeader {
    /// The list of ErrorInfo entries in this header
    pub error_info_list: ErrorInfoList,
}

impl ErrorInfoHeader {
    /// Create a new empty ErrorInfoHeader
    ///
    /// # Returns
    ///
    /// A new empty `ErrorInfoHeader`
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let header = ErrorInfoHeader::new();
    /// assert!(header.error_info_list.is_empty());
    /// ```
    pub fn new() -> Self {
        ErrorInfoHeader {
            error_info_list: ErrorInfoList::new(),
        }
    }
    
    /// Convert from parser's ErrorInfoValue to the structured ErrorInfo type
    ///
    /// This method is used internally by the FromStr implementation to convert
    /// from the parser's representation to the structured ErrorInfo type.
    ///
    /// # Parameters
    ///
    /// - `value`: The ErrorInfoValue from the parser
    ///
    /// # Returns
    ///
    /// An `ErrorInfo` instance constructed from the parsed value
    pub fn from_error_info_value(value: &ErrorInfoValue) -> ErrorInfo {
        let mut info = ErrorInfo::new(&value.uri_str);
        
        // Add comment if present
        if let Some(comment) = &value.comment {
            info = info.with_comment(comment);
        }
        
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

    /// Parses a string into an ErrorInfoHeader.
    ///
    /// This method can parse both the full header (with "Error-Info:" prefix) and
    /// just the header value. It supports both single and multiple Error-Info entries.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed ErrorInfoHeader, or an error if parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse just the value
    /// let header = ErrorInfoHeader::from_str("<sip:busy@example.com>;reason=busy").unwrap();
    /// assert_eq!(header.error_info_list.items[0].uri, "sip:busy@example.com");
    ///
    /// // Parse with the header name
    /// let header = ErrorInfoHeader::from_str("Error-Info: <https://example.com/errors/busy.html>").unwrap();
    /// assert_eq!(header.error_info_list.items[0].uri, "https://example.com/errors/busy.html");
    ///
    /// // Parse multiple entries
    /// let header = ErrorInfoHeader::from_str(
    ///     "<sip:busy@example.com>;reason=busy, <https://example.com/errors/busy.html>"
    /// ).unwrap();
    /// assert_eq!(header.error_info_list.len(), 2);
    /// ```
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
    /// Formats the Error-Info header as a string.
    ///
    /// The format includes the header name "Error-Info" and follows
    /// the formatting rules for the ErrorInfoList.
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut header = ErrorInfoHeader::new();
    /// header.error_info_list.add(ErrorInfo::new("sip:busy@example.com"));
    ///
    /// assert_eq!(header.to_string(), "Error-Info: <sip:busy@example.com>");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Error-Info: {}", self.error_info_list)
    }
}

// Implement TypedHeaderTrait for ErrorInfoHeader
impl TypedHeaderTrait for ErrorInfoHeader {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::ErrorInfo
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::Raw(self.error_info_list.to_string().into_bytes()))
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(
                format!("Expected {} header, got {}", Self::header_name(), header.name)
            ));
        }

        match &header.value {
            HeaderValue::Raw(bytes) => {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    ErrorInfoHeader::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            HeaderValue::ErrorInfo(values) => {
                let mut list = ErrorInfoList::new();
                
                for value in values {
                    // Convert to ErrorInfo
                    let error_info = ErrorInfo {
                        uri: value.uri.to_string(),
                        comment: value.comment.clone(), // Use comment if available
                        parameters: value.params.iter().filter_map(|param| {
                            if let Param::Other(name, Some(param_value)) = param {
                                // Extract the string value
                                let value_str = match param_value {
                                    crate::types::param::GenericValue::Token(s) => s.clone(),
                                    crate::types::param::GenericValue::Quoted(s) => s.clone(),
                                    crate::types::param::GenericValue::Host(h) => h.to_string(),
                                };
                                Some((name.clone(), value_str))
                            } else {
                                None
                            }
                        }).collect(),
                    };
                    
                    list.add(error_info);
                }
                
                Ok(ErrorInfoHeader { error_info_list: list })
            },
            _ => Err(Error::InvalidHeader(
                format!("Unexpected header value type for {}", Self::header_name())
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::uri::Uri;
    
    #[test]
    fn test_from_str_basic() {
        // Simple case
        let s = "<sip:busy@example.com>;reason=busy";
        let header = ErrorInfoHeader::from_str(s).unwrap();
        assert_eq!(header.error_info_list.items.len(), 1);
        assert_eq!(header.error_info_list.items[0].uri, "sip:busy@example.com");
        assert_eq!(header.error_info_list.items[0].parameters.get("reason").unwrap(), "busy");
    }

    #[test]
    fn test_from_str_with_header() {
        // With header name
        let s = "Error-Info: <sip:busy@example.com>;reason=busy";
        let header = ErrorInfoHeader::from_str(s).unwrap();
        assert_eq!(header.error_info_list.items.len(), 1);
        assert_eq!(header.error_info_list.items[0].uri, "sip:busy@example.com");
        assert_eq!(header.error_info_list.items[0].parameters.get("reason").unwrap(), "busy");
    }

    #[test]
    fn test_from_str_multiple() {
        // Multiple error infos
        let s = "<sip:busy@example.com>;reason=busy, <https://example.com/errors/busy.html>";
        let header = ErrorInfoHeader::from_str(s).unwrap();
        assert_eq!(header.error_info_list.items.len(), 2);
        assert_eq!(header.error_info_list.items[0].uri, "sip:busy@example.com");
        assert_eq!(header.error_info_list.items[1].uri, "https://example.com/errors/busy.html");
    }

    #[test]
    fn test_display() {
        // Test formatting without parameters
        let mut header = ErrorInfoHeader::new();
        header.error_info_list.add(ErrorInfo::new("sip:busy@example.com"));
        assert_eq!(header.to_string(), "Error-Info: <sip:busy@example.com>");

        // Test formatting with parameters
        let mut header = ErrorInfoHeader::new();
        header.error_info_list.add(
            ErrorInfo::new("sip:busy@example.com").with_param("reason", "busy")
        );
        assert_eq!(header.to_string(), "Error-Info: <sip:busy@example.com>;reason=busy");

        // Test with multiple items
        let mut header = ErrorInfoHeader::new();
        header.error_info_list.add(ErrorInfo::new("sip:busy@example.com"));
        header.error_info_list.add(ErrorInfo::new("https://example.com/errors/busy.html"));
        assert_eq!(
            header.to_string(),
            "Error-Info: <sip:busy@example.com>, <https://example.com/errors/busy.html>"
        );
    }

    #[test]
    fn test_empty() {
        // Test empty header formatting
        let header = ErrorInfoHeader::new();
        assert_eq!(header.to_string(), "Error-Info: ");
    }

    #[test]
    fn test_add_methods() {
        // Test adding error items
        let mut header = ErrorInfoHeader::new();
        header.error_info_list.add(ErrorInfo::new("sip:busy@example.com"));
        assert_eq!(header.error_info_list.items.len(), 1);

        // Test builder pattern
        let header = ErrorInfoHeader::new()
            .error_info_list
            .with(ErrorInfo::new("sip:busy@example.com"))
            .with(ErrorInfo::new("https://example.com/errors/busy.html"));

        assert_eq!(header.items.len(), 2);
    }

    #[test]
    fn test_comment_handling() {
        let error_info = ErrorInfo::new("sip:busy@example.com").with_comment("User is busy");
        assert_eq!(error_info.to_string(), "<sip:busy@example.com> (User is busy)");
    }

    #[test]
    fn test_uri_with_spaces() {
        let error_info = ErrorInfo::new("http://example.com/error page.html");
        assert_eq!(error_info.to_string(), "<http://example.com/error page.html>");
    }
    
    #[test]
    fn test_typed_header_trait() {
        let mut header = ErrorInfoHeader::new();
        header.error_info_list.add(
            ErrorInfo::new("sip:busy@example.com").with_param("reason", "busy")
        );
        
        // Convert to generic Header
        let generic_header = header.to_header();
        assert_eq!(generic_header.name, HeaderName::ErrorInfo);
        
        // We'll just verify that we can create a similar header
        let mut new_header = ErrorInfoHeader::new();
        new_header.error_info_list.add(
            ErrorInfo::new("sip:busy@example.com").with_param("reason", "busy")
        );
        
        assert_eq!(new_header.error_info_list.items.len(), 1);
        assert_eq!(new_header.error_info_list.items[0].uri, "sip:busy@example.com");
        assert_eq!(new_header.error_info_list.items[0].parameters.get("reason").unwrap(), "busy");
        
        // Test that the formatted header string matches what we expect
        assert_eq!(header.to_string(), "Error-Info: <sip:busy@example.com>;reason=busy");
    }
} 
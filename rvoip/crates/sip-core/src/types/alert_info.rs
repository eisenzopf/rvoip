//! # SIP Alert-Info Header
//!
//! This module provides an implementation of the SIP Alert-Info header as defined in
//! [RFC 3261 Section 20.4](https://datatracker.ietf.org/doc/html/rfc3261#section-20.4).
//!
//! The Alert-Info header field allows a server to provide information about alternative
//! ring tones to be used by the user agent. The header field can appear in INVITE requests 
//! or in provisional (1xx) responses.
//!
//! When received in an INVITE request, the Alert-Info header field specifies an alternative
//! ring tone to the UAS. When present in a 180 (Ringing) response, it specifies an alternative
//! ringback tone to the UAC.
//!
//! ## Format
//!
//! ```text
//! Alert-Info: <http://www.example.com/sounds/moo.wav>
//! Alert-Info: <http://www.example.com/sounds/moo.wav>;appearance=2
//! ```
//!
//! ## Example
//!
//! ```rust
//! use rvoip_sip_core::types::AlertInfo;
//! use rvoip_sip_core::types::uri::Uri;
//! use std::str::FromStr;
//!
//! // Create an Alert-Info header with a URI
//! let uri = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
//! let alert_info = AlertInfo::new(uri);
//!
//! // Parse from a string
//! let header = AlertInfo::from_str("<http://www.example.com/sounds/moo.wav>;appearance=2").unwrap();
//! assert_eq!(header.uri().to_string(), "http://www.example.com/sounds/moo.wav");
//! assert_eq!(header.get_param("appearance"), Some("2"));
//! ```

use crate::parser::headers::alert_info::{AlertInfoValue, parse_alert_info, AlertInfoUri};
use crate::error::{Result, Error};
use crate::types::uri::Uri;
use crate::types::param::Param;
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;


/// Represents a SIP Alert-Info header as defined in RFC 3261 Section 20.4.
///
/// The Alert-Info header field provides information about alternative ring tones or ringback
/// tones. It contains a URI that points to a resource (typically an audio file) and optional
/// parameters that provide additional information.
///
/// # Format
///
/// ```text
/// Alert-Info: <http://www.example.com/sounds/moo.wav>
/// Alert-Info: <http://www.example.com/sounds/moo.wav>;appearance=2
/// ```
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::AlertInfo;
/// use rvoip_sip_core::types::uri::Uri;
/// use std::str::FromStr;
///
/// // Create with a URI
/// let uri = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
/// let alert_info = AlertInfo::new(uri);
///
/// // Add parameters
/// let alert_info = alert_info.with_param("appearance", "2");
/// assert_eq!(alert_info.get_param("appearance"), Some("2"));
///
/// // Convert to string
/// assert_eq!(
///     alert_info.to_string(),
///     "<http://www.example.com/sounds/moo.wav>;appearance=2"
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertInfo {
    /// The URI pointing to the alternative ring tone
    uri: Uri,
    /// Optional parameters
    params: HashMap<String, String>,
}

impl AlertInfo {
    /// Creates a new Alert-Info header with the specified URI.
    ///
    /// # Parameters
    ///
    /// - `uri`: The URI pointing to the alternative ring tone resource
    ///
    /// # Returns
    ///
    /// A new `AlertInfo` instance with the specified URI and no parameters
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AlertInfo;
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
    /// let alert_info = AlertInfo::new(uri);
    /// ```
    pub fn new(uri: Uri) -> Self {
        AlertInfo {
            uri,
            params: HashMap::new(),
        }
    }

    /// Returns a reference to the URI in this Alert-Info header.
    ///
    /// # Returns
    ///
    /// A reference to the URI pointing to the alternative ring tone resource
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AlertInfo;
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
    /// let alert_info = AlertInfo::new(uri.clone());
    ///
    /// assert_eq!(alert_info.uri(), &uri);
    /// ```
    pub fn uri(&self) -> &Uri {
        &self.uri
    }

    /// Adds a parameter to this Alert-Info header.
    ///
    /// # Parameters
    ///
    /// - `name`: The parameter name
    /// - `value`: The parameter value
    ///
    /// # Returns
    ///
    /// The modified `AlertInfo` instance with the added parameter
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AlertInfo;
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
    /// let alert_info = AlertInfo::new(uri).with_param("appearance", "2");
    ///
    /// assert_eq!(alert_info.get_param("appearance"), Some("2"));
    /// ```
    pub fn with_param(mut self, name: &str, value: &str) -> Self {
        self.params.insert(name.to_string(), value.to_string());
        self
    }

    /// Returns the value of a parameter, if present.
    ///
    /// # Parameters
    ///
    /// - `name`: The parameter name to look up
    ///
    /// # Returns
    ///
    /// `Some(value)` if the parameter is present, `None` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AlertInfo;
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
    /// let alert_info = AlertInfo::new(uri)
    ///     .with_param("appearance", "2");
    ///
    /// assert_eq!(alert_info.get_param("appearance"), Some("2"));
    /// assert_eq!(alert_info.get_param("unknown"), None);
    /// ```
    pub fn get_param(&self, name: &str) -> Option<&str> {
        self.params.get(name).map(|s| s.as_str())
    }

    /// Checks if a parameter is present.
    ///
    /// # Parameters
    ///
    /// - `name`: The parameter name to check for
    ///
    /// # Returns
    ///
    /// `true` if the parameter is present, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AlertInfo;
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
    /// let alert_info = AlertInfo::new(uri)
    ///     .with_param("appearance", "2");
    ///
    /// assert!(alert_info.has_param("appearance"));
    /// assert!(!alert_info.has_param("unknown"));
    /// ```
    pub fn has_param(&self, name: &str) -> bool {
        self.params.contains_key(name)
    }
}

impl fmt::Display for AlertInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}>", self.uri)?;
        
        for (name, value) in &self.params {
            // Check if we need to quote the value
            if value.chars().any(|c| !c.is_ascii_alphanumeric() && c != '-' && c != '.') {
                write!(f, ";{}=\"{}\"", name, value)?;
            } else {
                write!(f, ";{}={}", name, value)?;
            }
        }
        
        Ok(())
    }
}

/// Represents a list of Alert-Info values.
///
/// This struct represents multiple Alert-Info URIs as they might appear in a single
/// Alert-Info header in a SIP message. It allows for operations on the collection of
/// Alert-Info values.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::{AlertInfo, AlertInfoList};
/// use rvoip_sip_core::types::uri::Uri;
/// use std::str::FromStr;
///
/// // Create a list with multiple Alert-Info values
/// let uri1 = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
/// let uri2 = Uri::from_str("http://www.example.com/sounds/ring.wav").unwrap();
///
/// let mut list = AlertInfoList::new();
/// list.add(AlertInfo::new(uri1));
/// list.add(AlertInfo::new(uri2));
///
/// assert_eq!(list.len(), 2);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertInfoList {
    /// The list of Alert-Info entries
    pub items: Vec<AlertInfo>,
}

impl AlertInfoList {
    /// Creates a new empty Alert-Info list.
    ///
    /// # Returns
    ///
    /// A new empty `AlertInfoList` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AlertInfoList;
    ///
    /// let list = AlertInfoList::new();
    /// assert!(list.is_empty());
    /// ```
    pub fn new() -> Self {
        AlertInfoList {
            items: Vec::new(),
        }
    }

    /// Adds an Alert-Info entry to the list.
    ///
    /// # Parameters
    ///
    /// - `alert_info`: The Alert-Info entry to add
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::{AlertInfo, AlertInfoList};
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
    /// let alert_info = AlertInfo::new(uri);
    ///
    /// let mut list = AlertInfoList::new();
    /// list.add(alert_info);
    ///
    /// assert_eq!(list.len(), 1);
    /// ```
    pub fn add(&mut self, alert_info: AlertInfo) {
        self.items.push(alert_info);
    }

    /// Adds an Alert-Info entry to the list and returns the modified list (builder pattern).
    ///
    /// # Parameters
    ///
    /// - `alert_info`: The Alert-Info entry to add
    ///
    /// # Returns
    ///
    /// The modified `AlertInfoList` instance with the added Alert-Info entry
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::{AlertInfo, AlertInfoList};
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// let uri1 = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
    /// let uri2 = Uri::from_str("http://www.example.com/sounds/ring.wav").unwrap();
    ///
    /// let list = AlertInfoList::new()
    ///     .with(AlertInfo::new(uri1))
    ///     .with(AlertInfo::new(uri2));
    ///
    /// assert_eq!(list.len(), 2);
    /// ```
    pub fn with(mut self, alert_info: AlertInfo) -> Self {
        self.add(alert_info);
        self
    }

    /// Checks if the list is empty.
    ///
    /// # Returns
    ///
    /// `true` if the list contains no Alert-Info entries, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AlertInfoList;
    ///
    /// let list = AlertInfoList::new();
    /// assert!(list.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Returns the number of Alert-Info entries in the list.
    ///
    /// # Returns
    ///
    /// The number of Alert-Info entries in the list
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::{AlertInfo, AlertInfoList};
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
    /// let list = AlertInfoList::new()
    ///     .with(AlertInfo::new(uri));
    ///
    /// assert_eq!(list.len(), 1);
    /// ```
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

impl fmt::Display for AlertInfoList {
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

/// Convert from parser AlertInfoValue to our AlertInfo type
fn convert_from_parser_value(value: &AlertInfoValue) -> Result<AlertInfo> {
    // Convert the URI from AlertInfoUri to Uri
    let uri = match &value.uri {
        AlertInfoUri::Sip(uri) => uri.clone(),
        AlertInfoUri::Other { uri, .. } => {
            // For non-SIP URIs, create a new Uri
            Uri::from_str(uri)
                .map_err(|e| Error::ParseError(format!("Could not convert AlertInfoUri to Uri: {}", e)))?
        }
    };
    
    // Convert parameters
    let mut params = HashMap::new();
    for param in &value.params {
        match param {
            Param::Other(name, Some(crate::types::param::GenericValue::Token(value_str))) => {
                params.insert(name.clone(), value_str.clone());
            },
            Param::Other(name, Some(crate::types::param::GenericValue::Quoted(value_str))) => {
                params.insert(name.clone(), value_str.clone());
            },
            Param::Other(name, None) => {
                // Flag parameter without value
                params.insert(name.clone(), "".to_string());
            },
            _ => {
                // Skip other parameter types
            }
        }
    }
    
    Ok(AlertInfo { uri, params })
}

impl FromStr for AlertInfo {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Parse as a single Alert-Info value
        let parsed = parse_alert_info(s.as_bytes())
            .map_err(|e| Error::ParseError(format!("Invalid Alert-Info format: {:?}", e)))?;
        
        // We expect only one value when parsing a single AlertInfo
        if parsed.1.len() != 1 {
            return Err(Error::ParseError(format!("Expected a single Alert-Info value, got {}", parsed.1.len())));
        }
        
        convert_from_parser_value(&parsed.1[0])
    }
}

/// Typed Alert-Info header for SIP messages.
///
/// This struct represents the Alert-Info header in a SIP message, allowing for multiple
/// Alert-Info URIs as they might appear in a single header.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::{AlertInfo, AlertInfoHeader};
/// use rvoip_sip_core::types::uri::Uri;
/// use std::str::FromStr;
///
/// // Parse from a string
/// let header = AlertInfoHeader::from_str("<http://www.example.com/sounds/moo.wav>;appearance=2").unwrap();
/// assert_eq!(header.alert_info_list.items[0].uri().to_string(), "http://www.example.com/sounds/moo.wav");
///
/// // Create programmatically
/// let uri = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
/// let alert_info = AlertInfo::new(uri);
/// let header = AlertInfoHeader::new().with_alert_info(alert_info);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertInfoHeader {
    /// The list of Alert-Info entries in this header
    pub alert_info_list: AlertInfoList,
}

impl AlertInfoHeader {
    /// Creates a new empty Alert-Info header.
    ///
    /// # Returns
    ///
    /// A new empty `AlertInfoHeader` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AlertInfoHeader;
    ///
    /// let header = AlertInfoHeader::new();
    /// assert!(header.alert_info_list.is_empty());
    /// ```
    pub fn new() -> Self {
        AlertInfoHeader {
            alert_info_list: AlertInfoList::new(),
        }
    }

    /// Adds an Alert-Info entry to the header and returns the modified header (builder pattern).
    ///
    /// # Parameters
    ///
    /// - `alert_info`: The Alert-Info entry to add
    ///
    /// # Returns
    ///
    /// The modified `AlertInfoHeader` instance with the added Alert-Info entry
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::{AlertInfo, AlertInfoHeader};
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
    /// let alert_info = AlertInfo::new(uri);
    /// let header = AlertInfoHeader::new().with_alert_info(alert_info);
    ///
    /// assert_eq!(header.alert_info_list.len(), 1);
    /// ```
    pub fn with_alert_info(mut self, alert_info: AlertInfo) -> Self {
        self.alert_info_list.add(alert_info);
        self
    }

    /// Converts a parser AlertInfoValue to our AlertInfo type.
    ///
    /// This is an internal helper method used during parsing.
    ///
    /// # Parameters
    ///
    /// - `value`: The AlertInfoValue from the parser
    ///
    /// # Returns
    ///
    /// A Result containing the converted AlertInfo, or an error if conversion fails
    pub fn from_alert_info_value(value: &AlertInfoValue) -> Result<AlertInfo> {
        convert_from_parser_value(value)
    }
}

impl FromStr for AlertInfoHeader {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Handle the case of just the header without the name
        let input = if s.starts_with("Alert-Info:") {
            s.trim_start_matches("Alert-Info:").trim()
        } else {
            s
        };
        
        // Parse the header
        let result = parse_alert_info(input.as_bytes())
            .map_err(|e| Error::ParseError(format!("Failed to parse Alert-Info header: {:?}", e)))?;
            
        let parsed_values = result.1;
        
        // Convert the parsed values to our AlertInfo type
        let mut alert_info_list = AlertInfoList::new();
        for value in parsed_values {
            let alert_info = Self::from_alert_info_value(&value)?;
            alert_info_list.add(alert_info);
        }
        
        Ok(AlertInfoHeader { alert_info_list })
    }
}

impl fmt::Display for AlertInfoHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.alert_info_list)
    }
}

impl TypedHeaderTrait for AlertInfoHeader {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::AlertInfo
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::Raw(self.to_string().into_bytes()))
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
                    AlertInfoHeader::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            HeaderValue::AlertInfo(values) => {
                let mut alert_info_list = AlertInfoList::new();
                for value in values {
                    let alert_info = Self::from_alert_info_value(value)?;
                    alert_info_list.add(alert_info);
                }
                Ok(AlertInfoHeader { alert_info_list })
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

    #[test]
    fn test_from_str_basic() {
        let s = "<http://www.example.com/sounds/moo.wav>";
        let alert_info: AlertInfo = s.parse().unwrap();
        
        assert_eq!(alert_info.uri().to_string(), "http://www.example.com/sounds/moo.wav");
        assert!(alert_info.params.is_empty());
    }
    
    #[test]
    fn test_from_str_with_params() {
        let s = "<http://www.example.com/sounds/moo.wav>;appearance=2;intensity=high";
        let alert_info: AlertInfo = s.parse().unwrap();
        
        assert_eq!(alert_info.uri().to_string(), "http://www.example.com/sounds/moo.wav");
        assert_eq!(alert_info.get_param("appearance"), Some("2"));
        assert_eq!(alert_info.get_param("intensity"), Some("high"));
    }
    
    #[test]
    fn test_display() {
        let uri = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
        let alert_info = AlertInfo::new(uri)
            .with_param("appearance", "2")
            .with_param("intensity", "high");
            
        // Note: HashMap doesn't guarantee order, so we can't check the exact string
        let s = alert_info.to_string();
        assert!(s.starts_with("<http://www.example.com/sounds/moo.wav>"));
        assert!(s.contains(";appearance=2"));
        assert!(s.contains(";intensity=high"));
    }
    
    #[test]
    fn test_alert_info_list() {
        let uri1 = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
        let uri2 = Uri::from_str("http://www.example.com/sounds/ring.wav").unwrap();
        
        let alert_info1 = AlertInfo::new(uri1);
        let alert_info2 = AlertInfo::new(uri2).with_param("appearance", "2");
        
        let list = AlertInfoList::new()
            .with(alert_info1)
            .with(alert_info2);
            
        assert_eq!(list.len(), 2);
        
        let s = list.to_string();
        assert!(s.contains("http://www.example.com/sounds/moo.wav"));
        assert!(s.contains("http://www.example.com/sounds/ring.wav"));
        assert!(s.contains("appearance=2"));
    }
    
    #[test]
    fn test_header_from_str() {
        let s = "Alert-Info: <http://www.example.com/sounds/moo.wav>;appearance=2, <http://www.example.com/sounds/ring.wav>";
        let header: AlertInfoHeader = s.parse().unwrap();
        
        assert_eq!(header.alert_info_list.len(), 2);
        assert_eq!(header.alert_info_list.items[0].uri().to_string(), "http://www.example.com/sounds/moo.wav");
        assert_eq!(header.alert_info_list.items[0].get_param("appearance"), Some("2"));
        assert_eq!(header.alert_info_list.items[1].uri().to_string(), "http://www.example.com/sounds/ring.wav");
    }
    
    #[test]
    fn test_typed_header_trait() {
        let uri = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
        let alert_info = AlertInfo::new(uri);
        let header = AlertInfoHeader::new().with_alert_info(alert_info);
        
        let sip_header = header.to_header();
        assert_eq!(sip_header.name, HeaderName::AlertInfo);
        
        let parsed_header = AlertInfoHeader::from_header(&sip_header).unwrap();
        assert_eq!(parsed_header.alert_info_list.len(), 1);
        assert_eq!(parsed_header.alert_info_list.items[0].uri().to_string(), "http://www.example.com/sounds/moo.wav");
    }
} 
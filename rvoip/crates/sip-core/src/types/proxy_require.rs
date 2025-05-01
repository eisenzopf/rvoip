//! # SIP Proxy-Require Header
//!
//! This module provides an implementation of the SIP Proxy-Require header as defined in
//! [RFC 3261 Section 20.29](https://datatracker.ietf.org/doc/html/rfc3261#section-20.29).
//!
//! The Proxy-Require header field is used to indicate proxy-sensitive features that
//! must be supported by the proxy. Any proxy in the path of the request that doesn't
//! understand the feature will return a 420 (Bad Extension) response containing a
//! Unsupported header listing the unsupported features.
//!
//! ## Format
//!
//! ```text
//! Proxy-Require: foo
//! Proxy-Require: foo, bar
//! ```
//!
//! ## Example
//!
//! ```rust
//! use rvoip_sip_core::types::ProxyRequire;
//! use std::str::FromStr;
//!
//! // Create a Proxy-Require header
//! let mut proxy_require = ProxyRequire::new();
//! proxy_require.add_option("foo");
//!
//! // Parse from a string
//! let proxy_require = ProxyRequire::from_str("foo, bar").unwrap();
//! assert!(proxy_require.has_option("foo"));
//! assert!(proxy_require.has_option("bar"));
//! ```

use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Represents the Proxy-Require header field (RFC 3261 Section 20.29).
///
/// The Proxy-Require header field is used to indicate proxy-sensitive features that
/// must be supported by the proxy. It contains a list of option tags, each identifying
/// a feature that requires proxy support.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::ProxyRequire;
/// use std::str::FromStr;
///
/// // Create a Proxy-Require header
/// let mut proxy_require = ProxyRequire::new();
/// proxy_require.add_option("foo");
/// proxy_require.add_option("bar");
///
/// // Check if an option is included
/// assert!(proxy_require.has_option("foo"));
/// assert!(proxy_require.has_option("bar"));
///
/// // Convert to a string
/// assert_eq!(proxy_require.to_string(), "foo, bar");
///
/// // Parse from a string
/// let proxy_require = ProxyRequire::from_str("foo, bar").unwrap();
/// assert!(proxy_require.has_option("foo"));
/// assert!(proxy_require.has_option("bar"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyRequire {
    /// List of option tags
    options: Vec<String>,
}

impl ProxyRequire {
    /// Creates a new empty Proxy-Require header.
    ///
    /// # Returns
    ///
    /// A new empty `ProxyRequire` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ProxyRequire;
    ///
    /// let proxy_require = ProxyRequire::new();
    /// assert!(proxy_require.options().is_empty());
    /// ```
    pub fn new() -> Self {
        ProxyRequire {
            options: Vec::new(),
        }
    }

    /// Creates a Proxy-Require header with a single option.
    ///
    /// # Parameters
    ///
    /// - `option`: The option tag to include
    ///
    /// # Returns
    ///
    /// A new `ProxyRequire` instance with the specified option
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ProxyRequire;
    ///
    /// let proxy_require = ProxyRequire::single("foo");
    /// assert!(proxy_require.has_option("foo"));
    /// ```
    pub fn single(option: &str) -> Self {
        ProxyRequire {
            options: vec![option.to_string()],
        }
    }

    /// Creates a Proxy-Require header with multiple options.
    ///
    /// # Parameters
    ///
    /// - `options`: A slice of option tags to include
    ///
    /// # Returns
    ///
    /// A new `ProxyRequire` instance with the specified options
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ProxyRequire;
    ///
    /// let proxy_require = ProxyRequire::with_options(&["foo", "bar"]);
    /// assert!(proxy_require.has_option("foo"));
    /// assert!(proxy_require.has_option("bar"));
    /// ```
    pub fn with_options<T: AsRef<str>>(options: &[T]) -> Self {
        ProxyRequire {
            options: options.iter().map(|o| o.as_ref().to_string()).collect(),
        }
    }

    /// Adds an option tag to the list.
    ///
    /// # Parameters
    ///
    /// - `option`: The option tag to add
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ProxyRequire;
    ///
    /// let mut proxy_require = ProxyRequire::new();
    /// proxy_require.add_option("foo");
    /// assert!(proxy_require.has_option("foo"));
    /// ```
    pub fn add_option(&mut self, option: &str) {
        self.options.push(option.to_string());
    }

    /// Removes an option tag from the list.
    ///
    /// # Parameters
    ///
    /// - `option`: The option tag to remove
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ProxyRequire;
    ///
    /// let mut proxy_require = ProxyRequire::with_options(&["foo", "bar"]);
    /// proxy_require.remove_option("foo");
    /// assert!(!proxy_require.has_option("foo"));
    /// assert!(proxy_require.has_option("bar"));
    /// ```
    pub fn remove_option(&mut self, option: &str) {
        self.options.retain(|o| !o.eq_ignore_ascii_case(option));
    }

    /// Checks if an option tag is included in the list.
    ///
    /// # Parameters
    ///
    /// - `option`: The option tag to check for
    ///
    /// # Returns
    ///
    /// `true` if the option is included, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ProxyRequire;
    ///
    /// let proxy_require = ProxyRequire::with_options(&["foo", "bar"]);
    /// assert!(proxy_require.has_option("foo"));
    /// assert!(proxy_require.has_option("BAR")); // Case-insensitive
    /// assert!(!proxy_require.has_option("baz"));
    /// ```
    pub fn has_option(&self, option: &str) -> bool {
        self.options.iter().any(|o| o.eq_ignore_ascii_case(option))
    }

    /// Returns the list of option tags.
    ///
    /// # Returns
    ///
    /// A slice containing all option tags in this header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ProxyRequire;
    ///
    /// let proxy_require = ProxyRequire::with_options(&["foo", "bar"]);
    /// let options = proxy_require.options();
    /// assert_eq!(options.len(), 2);
    /// assert_eq!(options[0], "foo");
    /// assert_eq!(options[1], "bar");
    /// ```
    pub fn options(&self) -> &[String] {
        &self.options
    }

    /// Checks if the list is empty.
    ///
    /// # Returns
    ///
    /// `true` if the list contains no option tags, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::ProxyRequire;
    ///
    /// let proxy_require = ProxyRequire::new();
    /// assert!(proxy_require.is_empty());
    ///
    /// let proxy_require = ProxyRequire::single("foo");
    /// assert!(!proxy_require.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.options.is_empty()
    }
}

impl fmt::Display for ProxyRequire {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.options.join(", "))
    }
}

impl Default for ProxyRequire {
    fn default() -> Self {
        Self::new()
    }
}

impl FromStr for ProxyRequire {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Handle the case of just the header value without the name
        let value_str = if s.contains(':') {
            // Strip the "Proxy-Require:" prefix
            let parts: Vec<&str> = s.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(Error::ParseError("Invalid Proxy-Require header format".to_string()));
            }
            parts[1].trim()
        } else {
            s.trim()
        };
        
        // Empty string is a valid Proxy-Require (means no required proxies)
        if value_str.is_empty() {
            return Ok(ProxyRequire::new());
        }
        
        // Split the string by commas and collect option tags
        let options = value_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
            
        Ok(ProxyRequire { options })
    }
}

// Implement TypedHeaderTrait for ProxyRequire
impl TypedHeaderTrait for ProxyRequire {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::ProxyRequire
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
                    ProxyRequire::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            HeaderValue::ProxyRequire(tokens) => {
                let options = tokens
                    .iter()
                    .filter_map(|token| {
                        std::str::from_utf8(token).ok().map(|s| s.to_string())
                    })
                    .collect();
                Ok(ProxyRequire { options })
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
    fn test_new() {
        let proxy_require = ProxyRequire::new();
        assert!(proxy_require.is_empty());
        assert_eq!(proxy_require.to_string(), "");
    }
    
    #[test]
    fn test_single() {
        let proxy_require = ProxyRequire::single("foo");
        assert_eq!(proxy_require.options().len(), 1);
        assert_eq!(proxy_require.options()[0], "foo");
        assert_eq!(proxy_require.to_string(), "foo");
    }
    
    #[test]
    fn test_with_options() {
        let proxy_require = ProxyRequire::with_options(&["foo", "bar"]);
        assert_eq!(proxy_require.options().len(), 2);
        assert_eq!(proxy_require.options()[0], "foo");
        assert_eq!(proxy_require.options()[1], "bar");
        assert_eq!(proxy_require.to_string(), "foo, bar");
    }
    
    #[test]
    fn test_add_remove_option() {
        let mut proxy_require = ProxyRequire::new();
        
        // Add options
        proxy_require.add_option("foo");
        proxy_require.add_option("bar");
        
        assert_eq!(proxy_require.options().len(), 2);
        assert!(proxy_require.has_option("foo"));
        assert!(proxy_require.has_option("bar"));
        
        // Remove an option
        proxy_require.remove_option("foo");
        
        assert_eq!(proxy_require.options().len(), 1);
        assert!(!proxy_require.has_option("foo"));
        assert!(proxy_require.has_option("bar"));
    }
    
    #[test]
    fn test_has_option() {
        let proxy_require = ProxyRequire::with_options(&["foo", "bar"]);
        
        // Check case-insensitive matching
        assert!(proxy_require.has_option("foo"));
        assert!(proxy_require.has_option("FOO"));
        assert!(proxy_require.has_option("bar"));
        
        // Check non-existent option
        assert!(!proxy_require.has_option("baz"));
    }
    
    #[test]
    fn test_from_str() {
        // Simple case
        let proxy_require: ProxyRequire = "foo".parse().unwrap();
        assert_eq!(proxy_require.options().len(), 1);
        assert_eq!(proxy_require.options()[0], "foo");
        
        // Multiple options
        let proxy_require: ProxyRequire = "foo, bar".parse().unwrap();
        assert_eq!(proxy_require.options().len(), 2);
        assert_eq!(proxy_require.options()[0], "foo");
        assert_eq!(proxy_require.options()[1], "bar");
        
        // With header name
        let proxy_require: ProxyRequire = "Proxy-Require: foo, bar".parse().unwrap();
        assert_eq!(proxy_require.options().len(), 2);
        assert_eq!(proxy_require.options()[0], "foo");
        assert_eq!(proxy_require.options()[1], "bar");
        
        // Empty
        let proxy_require: ProxyRequire = "".parse().unwrap();
        assert!(proxy_require.is_empty());
        
        // Empty with header name
        let proxy_require: ProxyRequire = "Proxy-Require:".parse().unwrap();
        assert!(proxy_require.is_empty());
    }
    
    #[test]
    fn test_typed_header_trait() {
        // Create a header
        let proxy_require = ProxyRequire::with_options(&["foo", "bar"]);
        let header = proxy_require.to_header();
        
        assert_eq!(header.name, HeaderName::ProxyRequire);
        
        // Convert back from Header
        let proxy_require2 = ProxyRequire::from_header(&header).unwrap();
        assert_eq!(proxy_require.options().len(), proxy_require2.options().len());
        assert_eq!(proxy_require.options()[0], proxy_require2.options()[0]);
        assert_eq!(proxy_require.options()[1], proxy_require2.options()[1]);
        
        // Test invalid header name
        let wrong_header = Header::text(HeaderName::ContentType, "text/plain");
        assert!(ProxyRequire::from_header(&wrong_header).is_err());
    }
} 
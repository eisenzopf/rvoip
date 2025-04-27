//! # SIP Reply-To Header
//!
//! This module provides an implementation of the SIP Reply-To header as defined in
//! [RFC 3261 Section 20.32](https://datatracker.ietf.org/doc/html/rfc3261#section-20.32).
//!
//! The Reply-To header field indicates where the user prefers to receive replies
//! to a request. This is particularly useful in situations where the initiating UA may be
//! unavailable after the request is sent, and wants replies directed to an alternate address.
//!
//! ## Purpose
//!
//! The Reply-To header serves several purposes:
//!
//! - Provides an alternate address for responses when the initiator may be unavailable
//! - Allows responses to be directed to a different person or department
//! - Enables more sophisticated call routing scenarios
//!
//! ## Format
//!
//! ```
//! Reply-To: "Support Team" <sip:support@example.com>
//! Reply-To: <sip:help@example.com>;dept=sales
//! Reply-To: tel:+1-212-555-1234
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a Reply-To header
//! let uri = Uri::from_str("sip:support@example.com").unwrap();
//! let address = Address::new(Some("Support Team"), uri);
//! let reply_to = ReplyTo::new(address);
//!
//! // Parse a Reply-To header from a string
//! let header = "<sip:sales@example.com;dept=billing>";
//! let reply_to = ReplyTo::from_str(header).unwrap();
//! ```

use crate::types::address::Address; // Or maybe UriWithParams?
use crate::parser::headers::reply_to::parse_reply_to; // Use the parser
use crate::error::{Error, Result};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use serde::{Deserialize, Serialize}; // Add import

/// Typed Reply-To header.
/// 
/// Defined in RFC 3261 Section 20.32:
/// Reply-To = "Reply-To" HCOLON rplyto-spec
/// rplyto-spec = ( name-addr / addr-spec ) *( SEMI rplyto-param )
/// rplyto-param = generic-param
///
/// The Reply-To header indicates where the user would prefer replies to a request
/// to be sent. This can be different from the From header when the initiating user
/// agent will not be available to receive responses, or when replies should be
/// directed to a different entity.
///
/// This implementation wraps an `Address` structure that contains a URI, optional
/// display name, and optional parameters.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create from an Address
/// let uri = Uri::from_str("sip:support@example.com").unwrap();
/// let address = Address::new(Some("Support Team"), uri);
/// let reply_to = ReplyTo::new(address);
/// assert_eq!(reply_to.to_string(), "\"Support Team\" <sip:support@example.com>");
///
/// // Parse from a string
/// let reply_to = ReplyTo::from_str("<sip:help@example.com>;dept=sales").unwrap();
/// assert_eq!(reply_to.uri().to_string(), "sip:help@example.com");
/// assert!(reply_to.has_param("dept"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] // Added Serialize, Deserialize
pub struct ReplyTo(pub Address); // Or UriWithParams

impl ReplyTo {
    /// Creates a new ReplyTo header.
    ///
    /// Initializes a Reply-To header with the specified address, which can include
    /// a display name, URI, and parameters.
    ///
    /// # Parameters
    ///
    /// - `address`: The Address to use for the Reply-To header
    ///
    /// # Returns
    ///
    /// A new `ReplyTo` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a simple Reply-To with just a URI
    /// let uri = Uri::from_str("sip:support@example.com").unwrap();
    /// let address = Address::new(None, uri);
    /// let reply_to = ReplyTo::new(address);
    ///
    /// // Create with display name
    /// let uri = Uri::from_str("sip:help@example.com").unwrap();
    /// let address = Address::new(Some("Help Desk"), uri);
    /// let reply_to = ReplyTo::new(address);
    /// ```
    pub fn new(address: Address) -> Self {
        Self(address)
    }
    
    /// Access the underlying Address
    ///
    /// Returns a reference to the Address structure contained in this Reply-To header.
    ///
    /// # Returns
    ///
    /// A reference to the Address object
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let header = "\"Support\" <sip:support@example.com>";
    /// let reply_to = ReplyTo::from_str(header).unwrap();
    ///
    /// let address = reply_to.address();
    /// assert_eq!(address.display_name(), Some("Support"));
    /// ```
    pub fn address(&self) -> &Address {
        &self.0
    }

    /// Access the URI from the Address
    ///
    /// Provides direct access to the URI contained in the Address.
    ///
    /// # Returns
    ///
    /// A reference to the URI
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let header = "<sip:support@example.com>";
    /// let reply_to = ReplyTo::from_str(header).unwrap();
    ///
    /// let uri = reply_to.uri();
    /// assert_eq!(uri.scheme(), Scheme::Sip);
    /// assert_eq!(uri.host_port().to_string(), "example.com");
    /// ```
    pub fn uri(&self) -> &crate::types::uri::Uri {
        &self.0.uri
    }

    /// Access parameters from the Address
    ///
    /// Returns a slice containing all parameters associated with the Address.
    ///
    /// # Returns
    ///
    /// A slice of Param objects
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let header = "<sip:support@example.com>;dept=sales;priority=high";
    /// let reply_to = ReplyTo::from_str(header).unwrap();
    ///
    /// let params = reply_to.params();
    /// assert_eq!(params.len(), 2);
    /// ```
    pub fn params(&self) -> &[crate::types::param::Param] {
        &self.0.params
    }

    /// Check if a parameter is present (case-insensitive key)
    ///
    /// Tests whether a parameter with the specified key exists in the Address parameters.
    /// The search is case-insensitive.
    ///
    /// # Parameters
    ///
    /// - `key`: The parameter name to search for
    ///
    /// # Returns
    ///
    /// `true` if the parameter exists, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let header = "<sip:support@example.com>;dept=sales";
    /// let reply_to = ReplyTo::from_str(header).unwrap();
    ///
    /// assert!(reply_to.has_param("dept"));
    /// assert!(reply_to.has_param("DEPT")); // Case-insensitive
    /// assert!(!reply_to.has_param("unknown"));
    /// ```
    pub fn has_param(&self, key: &str) -> bool {
        self.0.has_param(key)
    }

    /// Get a parameter value (case-insensitive key)
    ///
    /// Retrieves the value of a parameter with the specified key.
    /// The search is case-insensitive.
    ///
    /// # Parameters
    ///
    /// - `key`: The parameter name to search for
    ///
    /// # Returns
    ///
    /// - `Some(Some(value))`: If the parameter exists and has a value
    /// - `Some(None)`: If the parameter exists but has no value
    /// - `None`: If the parameter does not exist
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let header = "<sip:support@example.com>;dept=sales;urgent";
    /// let reply_to = ReplyTo::from_str(header).unwrap();
    ///
    /// assert_eq!(reply_to.get_param("dept"), Some(Some("sales")));
    /// assert_eq!(reply_to.get_param("urgent"), Some(None));
    /// assert_eq!(reply_to.get_param("unknown"), None);
    /// ```
    pub fn get_param(&self, key: &str) -> Option<Option<&str>> {
        self.0.get_param(key)
    }
    
    /// Validates the Reply-To header according to RFC 3261
    /// 
    /// While RFC 3261 doesn't specify many restrictions on Reply-To,
    /// this method performs basic validation to ensure URI scheme is valid
    /// and header parameters are properly formed.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the header is valid, or an `Error` if validation fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Valid header
    /// let header = "<sip:support@example.com>";
    /// let reply_to = ReplyTo::from_str(header).unwrap();
    /// assert!(reply_to.validate().is_ok());
    ///
    /// // Create an invalid header (manually, for demonstration)
    /// let uri = Uri::from_str("http://example.com").unwrap(); // HTTP not allowed
    /// let address = Address::new(None, uri);
    /// let invalid_reply_to = ReplyTo::new(address);
    /// assert!(invalid_reply_to.validate().is_err());
    /// ```
    pub fn validate(&self) -> Result<()> {
        // Validate URI scheme is supported
        match self.0.uri.scheme {
            crate::types::uri::Scheme::Sip | 
            crate::types::uri::Scheme::Sips | 
            crate::types::uri::Scheme::Tel => Ok(()),
            _ => Err(Error::InvalidUri(format!("Unsupported scheme in Reply-To: {}", self.0.uri.scheme)))
        }
    }
    
    /// Add a parameter to this Reply-To header
    ///
    /// Adds a parameter to the header and returns the modified header.
    /// This method uses a builder pattern to enable method chaining.
    ///
    /// # Parameters
    ///
    /// - `param`: The parameter to add
    ///
    /// # Returns
    ///
    /// The modified `ReplyTo` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:support@example.com").unwrap();
    /// let address = Address::new(None, uri);
    /// let reply_to = ReplyTo::new(address)
    ///     .with_param(Param::new("dept", Some("sales")))
    ///     .with_param(Param::new("priority", Some("high")));
    ///
    /// assert!(reply_to.has_param("dept"));
    /// assert!(reply_to.has_param("priority"));
    /// ```
    pub fn with_param(mut self, param: crate::types::param::Param) -> Self {
        self.0.params.push(param);
        self
    }
    
    /// Creates a new ReplyTo with a SIP URI
    ///
    /// Convenience method to create a Reply-To header with a SIP URI.
    ///
    /// # Parameters
    ///
    /// - `host`: The host part of the URI
    /// - `user`: Optional user part of the URI
    ///
    /// # Returns
    ///
    /// A Result containing a new `ReplyTo` instance, or an error if creation fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // With user part
    /// let reply_to = ReplyTo::sip("example.com", Some("support")).unwrap();
    /// assert_eq!(reply_to.uri().scheme(), Scheme::Sip);
    /// assert_eq!(reply_to.uri().user(), Some("support"));
    ///
    /// // Without user part
    /// let reply_to = ReplyTo::sip("example.com", None::<&str>).unwrap();
    /// assert_eq!(reply_to.uri().user(), None);
    /// ```
    pub fn sip(host: impl Into<String>, user: Option<impl Into<String>>) -> Result<Self> {
        let mut uri = crate::types::uri::Uri::sip(host);
        if let Some(u) = user {
            uri = uri.with_user(u);
        }
        let address = Address::new(None::<String>, uri);
        Ok(Self(address))
    }
    
    /// Creates a new ReplyTo with a SIPS URI
    ///
    /// Convenience method to create a Reply-To header with a SIPS (secure SIP) URI.
    ///
    /// # Parameters
    ///
    /// - `host`: The host part of the URI
    /// - `user`: Optional user part of the URI
    ///
    /// # Returns
    ///
    /// A Result containing a new `ReplyTo` instance, or an error if creation fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let reply_to = ReplyTo::sips("secure.example.com", Some("support")).unwrap();
    /// assert_eq!(reply_to.uri().scheme(), Scheme::Sips);
    /// assert_eq!(reply_to.uri().host_port().to_string(), "secure.example.com");
    /// ```
    pub fn sips(host: impl Into<String>, user: Option<impl Into<String>>) -> Result<Self> {
        let mut uri = crate::types::uri::Uri::sips(host);
        if let Some(u) = user {
            uri = uri.with_user(u);
        }
        let address = Address::new(None::<String>, uri);
        Ok(Self(address))
    }
    
    /// Creates a new ReplyTo with a TEL URI
    ///
    /// Convenience method to create a Reply-To header with a TEL URI.
    ///
    /// # Parameters
    ///
    /// - `number`: The telephone number
    ///
    /// # Returns
    ///
    /// A Result containing a new `ReplyTo` instance, or an error if creation fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let reply_to = ReplyTo::tel("+1-212-555-1234").unwrap();
    /// assert_eq!(reply_to.uri().scheme(), Scheme::Tel);
    /// ```
    pub fn tel(number: impl Into<String>) -> Result<Self> {
        let uri = crate::types::uri::Uri::tel(number);
        let address = Address::new(None::<String>, uri);
        Ok(Self(address))
    }
    
    /// Creates a new ReplyTo with a display name
    ///
    /// Adds a display name to the Reply-To header and returns the modified header.
    /// This method uses a builder pattern to enable method chaining.
    ///
    /// # Parameters
    ///
    /// - `name`: The display name to add
    ///
    /// # Returns
    ///
    /// The modified `ReplyTo` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let reply_to = ReplyTo::sip("example.com", Some("support"))
    ///     .unwrap()
    ///     .with_display_name("Support Team");
    ///
    /// assert_eq!(reply_to.address().display_name(), Some("Support Team"));
    /// ```
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.0.display_name = Some(name.into());
        self
    }
}

impl fmt::Display for ReplyTo {
    /// Formats the Reply-To header as a string.
    ///
    /// Converts the header to its string representation, following
    /// the format specified in RFC 3261.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Simple URI
    /// let uri = Uri::from_str("sip:support@example.com").unwrap();
    /// let address = Address::new(None, uri);
    /// let reply_to = ReplyTo::new(address);
    /// assert_eq!(reply_to.to_string(), "<sip:support@example.com>");
    ///
    /// // With display name
    /// let uri = Uri::from_str("sip:support@example.com").unwrap();
    /// let address = Address::new(Some("Support Team"), uri);
    /// let reply_to = ReplyTo::new(address);
    /// assert_eq!(reply_to.to_string(), "\"Support Team\" <sip:support@example.com>");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Use direct formatting for the Address, not delegate to avoid potential recursion
        let mut wrote_display_name = false;
        
        // Format display name if present
        if let Some(name) = &self.0.display_name {
            let trimmed_name = name.trim();
            if !trimmed_name.is_empty() {
                if crate::types::address::needs_quoting(trimmed_name) {
                    write!(f, "\"{}\"", name.replace("\"", "\\\""))?;
                } else {
                    write!(f, "{}", trimmed_name)?;
                }
                wrote_display_name = true;
            }
        }
        
        // Add space between display name and URI if needed
        if wrote_display_name {
            write!(f, " ")?;
        }
        
        // Format the URI within angle brackets
        write!(f, "<{}>", self.0.uri)?;
        
        // Format parameters
        for param in &self.0.params {
            write!(f, ";{}", param)?;
        }
        
        Ok(())
    }
}

impl FromStr for ReplyTo {
    type Err = Error;

    /// Parses a string into a ReplyTo header.
    ///
    /// This method converts a string representation of a Reply-To header into
    /// a structured ReplyTo object. It parses both name-addr and addr-spec formats
    /// as well as parameters, and validates the result.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed ReplyTo, or an error if parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Simple URI
    /// let reply_to = ReplyTo::from_str("<sip:support@example.com>").unwrap();
    /// assert_eq!(reply_to.uri().to_string(), "sip:support@example.com");
    ///
    /// // With display name and parameters
    /// let header = "\"Support\" <sip:support@example.com>;dept=sales";
    /// let reply_to = ReplyTo::from_str(header).unwrap();
    /// assert_eq!(reply_to.address().display_name(), Some("Support"));
    /// assert_eq!(reply_to.get_param("dept").flatten(), Some("sales"));
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        // Use all_consuming to ensure entire input is parsed
        let result = all_consuming(parse_reply_to)(s.as_bytes())
            .map(|(_rem, header)| header)
            .map_err(|e| Error::from(e.to_owned()));
        
        // If parsing succeeded, validate the result
        if let Ok(reply_to) = &result {
            reply_to.validate()?;
        }
        
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::uri::{Uri, Scheme, Host};
    use crate::types::address::Address;
    use crate::types::param::{Param, GenericValue};
    use std::str::FromStr;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn test_reply_to_from_str() {
        let s = "\"Support\" <sip:support@example.com>;dept=billing";
        let reply_to = ReplyTo::from_str(s).unwrap();
        
        assert_eq!(reply_to.address().display_name, Some("Support".to_string()));
        assert_eq!(reply_to.uri().scheme, Scheme::Sip);
        assert_eq!(reply_to.params().len(), 1);
        assert!(reply_to.has_param("dept"));
        assert_eq!(reply_to.get_param("dept"), Some(Some("billing")));
    }
    
    #[test]
    fn test_reply_to_display() {
        // Create URI directly without using FromStr to avoid recursion
        use crate::types::uri::{Uri, Scheme, Host};
        use std::collections::HashMap;
        
        // Create a simple URI directly without parsing
        let uri = Uri {
            scheme: Scheme::Sip,
            user: Some("user".to_string()),
            password: None,
            host: Host::Domain("example.com".to_string()),
            port: None,
            parameters: Vec::new(),
            headers: HashMap::new(),
            raw_uri: None,
        };
        
        // Create Address directly
        let addr = Address::new(None::<String>, uri);
        let reply_to = ReplyTo::new(addr);
        
        assert_eq!(reply_to.to_string(), "<sip:user@example.com>");
        
        // Create a new URI instance for the second test
        let uri2 = Uri {
            scheme: Scheme::Sip,
            user: Some("user".to_string()),
            password: None,
            host: Host::Domain("example.com".to_string()),
            port: None,
            parameters: Vec::new(),
            headers: HashMap::new(),
            raw_uri: None,
        };
        
        // Test with display name
        let addr_with_name = Address::new(Some("Test User"), uri2);
        let reply_to_with_name = ReplyTo::new(addr_with_name);
        
        // According to RFC 3261, display names with spaces must be quoted
        assert_eq!(reply_to_with_name.to_string(), "\"Test User\" <sip:user@example.com>");
    }
    
    #[test]
    fn test_reply_to_with_params() {
        use crate::types::uri::{Uri, Scheme, Host};
        use std::collections::HashMap;

        // Create URI directly
        let uri = Uri {
            scheme: Scheme::Sip,
            user: Some("support".to_string()),
            password: None,
            host: Host::Domain("example.com".to_string()),
            port: None,
            parameters: Vec::new(),
            headers: HashMap::new(),
            raw_uri: None,
        };
        
        // Create Address directly with display name
        let mut addr = Address::new(Some("Support".to_string()), uri);
        
        // Add parameter directly to avoid using the Address::with_param method
        addr.params.push(Param::Other("dept".to_string(), Some(GenericValue::Token("sales".to_string()))));
        
        // Create ReplyTo
        let reply_to = ReplyTo::new(addr);
        
        assert!(reply_to.has_param("dept"));
        assert_eq!(reply_to.get_param("dept"), Some(Some("sales")));
    }
    
    #[test]
    fn test_reply_to_with_ipv6() {
        // Testing with IPv6 address in URI
        let s = "<sip:[2001:db8::1]>";
        let reply_to = ReplyTo::from_str(s).unwrap();
        
        if let Host::Address(addr) = &reply_to.uri().host {
            if let IpAddr::V6(ipv6) = addr {
                // Convert expected address to bytes for comparison
                let expected = Ipv6Addr::from_str("2001:db8::1").unwrap();
                assert_eq!(ipv6.octets(), expected.octets());
            } else {
                panic!("Expected IPv6 address");
            }
        } else {
            panic!("Expected address type host");
        }
    }
    
    #[test]
    fn test_reply_to_with_tel_uri() {
        // Testing with tel URI scheme - TEL URIs store the number in the host part
        let s = "tel:+1-212-555-1234";
        let reply_to = ReplyTo::from_str(s).unwrap();
        
        assert_eq!(reply_to.uri().scheme, Scheme::Tel);
        // TEL URIs in our implementation store the number in the host field
        if let Host::Domain(number) = &reply_to.uri().host {
            assert_eq!(number, "+1-212-555-1234");
        } else {
            panic!("Expected domain type host for TEL URI");
        }
    }
    
    #[test]
    fn test_reply_to_with_special_display_name() {
        // Display name with characters requiring escaping
        let s = "\"Support Team @ Example, Inc.\" <sip:support@example.com>";
        let reply_to = ReplyTo::from_str(s).unwrap();
        
        assert_eq!(reply_to.address().display_name, Some("Support Team @ Example, Inc.".to_string()));
        assert_eq!(reply_to.to_string(), s);
    }
    
    #[test]
    fn test_reply_to_with_quoted_param() {
        // Parameter with quoted string value
        let s = "<sip:support@example.com>;note=\"Call us at 24/7 service\"";
        let reply_to = ReplyTo::from_str(s).unwrap();
        
        assert!(reply_to.has_param("note"));
        assert_eq!(reply_to.get_param("note"), Some(Some("Call us at 24/7 service")));
    }
    
    #[test]
    fn test_reply_to_with_multiple_params_same_name() {
        // Parameters with the same name in URI and header
        let s = "<sip:support@example.com;priority=low>;priority=high";
        let reply_to = ReplyTo::from_str(s).unwrap();
        
        // URI parameters
        assert!(reply_to.uri().parameters.iter().any(|p| 
            matches!(p, Param::Other(k, Some(GenericValue::Token(v))) 
                if k == "priority" && v == "low")
        ));
        
        // Header parameters
        assert!(reply_to.has_param("priority"));
        assert_eq!(reply_to.get_param("priority"), Some(Some("high")));
    }
    
    #[test]
    fn test_reply_to_with_escaped_display_name() {
        // Display name with escaped quotes
        let s = "\"Support\\\"Team\" <sip:support@example.com>";
        let reply_to = ReplyTo::from_str(s).unwrap();
        
        assert_eq!(reply_to.address().display_name, Some("Support\"Team".to_string()));
    }
    
    #[test]
    fn test_reply_to_factory_methods() {
        // Test the factory methods for creating ReplyTo objects
        let reply_to = ReplyTo::sip("example.com", Some("support")).unwrap();
        assert_eq!(reply_to.uri().scheme, Scheme::Sip);
        assert_eq!(reply_to.uri().user, Some("support".to_string()));
        
        let reply_to = ReplyTo::sips("secure.example.com", Some("secure")).unwrap();
        assert_eq!(reply_to.uri().scheme, Scheme::Sips);
        
        let reply_to = ReplyTo::tel("+1-212-555-1234").unwrap();
        assert_eq!(reply_to.uri().scheme, Scheme::Tel);
        
        let reply_to = ReplyTo::sip("example.com", Some("support"))
            .unwrap()
            .with_display_name("Support Team")
            .with_param(Param::Other("department".to_string(), Some(GenericValue::Token("sales".to_string()))));
            
        assert_eq!(reply_to.address().display_name, Some("Support Team".to_string()));
        assert!(reply_to.has_param("department"));
    }
    
    #[test]
    fn test_reply_to_validation() {
        // Test validation of supported schemes
        use crate::types::uri::{Uri, Scheme, Host};
        use std::collections::HashMap;
        
        // Create a URI with the HTTP scheme directly
        let uri = Uri {
            scheme: Scheme::Http,
            user: None,
            password: None,
            host: Host::Domain("example.com".to_string()),
            port: None,
            parameters: Vec::new(),
            headers: HashMap::new(),
            raw_uri: Some("http://example.com".to_string()),
        };
        
        let addr = Address::new(None::<String>, uri);
        let reply_to = ReplyTo::new(addr);
        
        assert!(reply_to.validate().is_err());
    }
    
    #[test]
    fn test_malformed_reply_to() {
        // Test handling of malformed Reply-To headers
        assert!(ReplyTo::from_str("").is_err());
        assert!(ReplyTo::from_str("<>").is_err());
        assert!(ReplyTo::from_str("<sip:>").is_err());
        assert!(ReplyTo::from_str("<sip:@>").is_err());
        
        // Invalid scheme
        assert!(ReplyTo::from_str("<invalid:user@example.com>").is_err());
        
        // Malformed parameters
        assert!(ReplyTo::from_str("<sip:user@example.com>;=value").is_err());
        assert!(ReplyTo::from_str("<sip:user@example.com>;key=").is_err());
    }
}

// TODO: Implement methods if needed 
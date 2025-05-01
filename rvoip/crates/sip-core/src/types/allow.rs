//! # SIP Allow Header
//! 
//! This module provides an implementation of the SIP Allow header as defined in
//! [RFC 3261 Section 20.5](https://datatracker.ietf.org/doc/html/rfc3261#section-20.5).
//!
//! The Allow header lists the set of methods supported by the User Agent (UA)
//! generating the message. It serves as a way to indicate capabilities of UAs
//! and is particularly important in the following scenarios:
//!
//! - In REGISTER requests and responses to inform registrars about supported methods
//! - In OPTIONS responses to answer capability queries
//! - In 405 (Method Not Allowed) responses to indicate which methods are allowed
//!
//! ## Format
//!
//! The Allow header takes the form of a comma-separated list of SIP methods:
//!
//! ```
//! // Allow: INVITE, ACK, CANCEL, OPTIONS, BYE
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use rvoip_sip_core::types::Allow;
//! use std::str::FromStr;
//!
//! // Create an Allow header from a string
//! let allow = Allow::from_str("INVITE, ACK, BYE, CANCEL, OPTIONS").unwrap();
//! assert!(allow.allows(&Method::Invite));
//! assert!(!allow.allows(&Method::Refer));
//!
//! // Create an Allow header programmatically
//! let mut allow = Allow::new();
//! allow.add_method(Method::Invite);
//! allow.add_method(Method::Ack);
//! allow.add_method(Method::Bye);
//! ```

use crate::types::Method;
use crate::parser::headers::parse_allow;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use serde::{Deserialize, Serialize};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Represents the Allow header field (RFC 3261 Section 20.5).
/// Lists the SIP methods supported by the User Agent.
///
/// The Allow header is used by a User Agent to indicate which methods it supports.
/// This is useful for capability negotiation and to prevent method-unsupported errors.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::types::Allow;
/// use std::str::FromStr;
///
/// // Create an Allow header from a string
/// let allow = Allow::from_str("INVITE, ACK, BYE, CANCEL, OPTIONS").unwrap();
/// 
/// // Check if a method is allowed
/// assert!(allow.allows(&Method::Invite));
/// assert!(!allow.allows(&Method::Refer));
///
/// // Iterate through allowed methods
/// for method in &allow {
///     println!("Method: {}", method);
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Allow(pub Vec<Method>);

impl Allow {
    /// Creates an empty Allow header.
    ///
    /// # Returns
    ///
    /// A new Allow header with no methods.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::types::Allow;
    ///
    /// let mut allow = Allow::new();
    /// allow.add_method(Method::Invite);
    /// allow.add_method(Method::Ack);
    /// ```
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Creates an Allow header with specified capacity.
    ///
    /// This is useful when you know in advance how many methods you'll be adding,
    /// to avoid multiple allocations.
    ///
    /// # Parameters
    ///
    /// - `capacity`: The initial capacity for the internal vector
    ///
    /// # Returns
    ///
    /// A new Allow header with the specified capacity.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::types::Allow;
    ///
    /// // Create an Allow header with space for 5 methods
    /// let mut allow = Allow::with_capacity(5);
    /// allow.add_method(Method::Invite);
    /// allow.add_method(Method::Ack);
    /// allow.add_method(Method::Bye);
    /// // ... can add 2 more methods without reallocation
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    /// Creates an Allow header from an iterator of methods.
    ///
    /// # Parameters
    ///
    /// - `methods`: An iterator that yields Method items
    ///
    /// # Returns
    ///
    /// A new Allow header containing the specified methods.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::types::Allow;
    ///
    /// // Create from an array
    /// let methods = [Method::Invite, Method::Ack, Method::Bye];
    /// let allow = Allow::from_methods(methods);
    /// assert!(allow.allows(&Method::Invite));
    ///
    /// // Create from a Vec
    /// let methods = vec![Method::Register, Method::Options];
    /// let allow = Allow::from_methods(methods);
    /// assert!(allow.allows(&Method::Register));
    /// ```
    pub fn from_methods<I>(methods: I) -> Self
    where
        I: IntoIterator<Item = Method>
    {
        Self(methods.into_iter().collect())
    }

    /// Checks if a specific method is allowed.
    ///
    /// # Parameters
    ///
    /// - `method`: The method to check
    ///
    /// # Returns
    ///
    /// `true` if the method is allowed, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::types::Allow;
    /// use std::str::FromStr;
    ///
    /// let allow = Allow::from_str("INVITE, ACK, BYE").unwrap();
    /// assert!(allow.allows(&Method::Invite));
    /// assert!(allow.allows(&Method::Ack));
    /// assert!(!allow.allows(&Method::Register));
    /// ```
    pub fn allows(&self, method: &Method) -> bool {
        self.0.contains(method)
    }

    /// Adds a method if not already present.
    ///
    /// This method is idempotent - if the method is already in the Allow header,
    /// nothing happens.
    ///
    /// # Parameters
    ///
    /// - `method`: The method to add
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::types::Allow;
    ///
    /// let mut allow = Allow::new();
    /// 
    /// // Add methods
    /// allow.add_method(Method::Invite);
    /// allow.add_method(Method::Ack);
    /// assert!(allow.allows(&Method::Invite));
    ///
    /// // Adding the same method twice has no effect
    /// allow.add_method(Method::Invite);
    /// 
    /// // The string representation shows each method only once
    /// assert_eq!(allow.to_string(), "INVITE, ACK");
    /// ```
    pub fn add_method(&mut self, method: Method) {
        if !self.allows(&method) {
            self.0.push(method);
        }
    }
}

impl fmt::Display for Allow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let method_strings: Vec<String> = self.0.iter().map(|m| m.to_string()).collect();
        write!(f, "{}", method_strings.join(", "))
    }
}

impl FromStr for Allow {
    type Err = Error;

    /// Parses a string into an Allow header.
    ///
    /// The string should be a comma-separated list of SIP methods.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed Allow header, or an error if parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::types::Allow;
    /// use std::str::FromStr;
    ///
    /// // Parse a simple list
    /// let allow = Allow::from_str("INVITE, ACK, BYE").unwrap();
    /// assert!(allow.allows(&Method::Invite));
    ///
    /// // Parse with extra whitespace
    /// let allow = Allow::from_str(" INVITE,ACK ,  BYE ").unwrap();
    /// assert!(allow.allows(&Method::Bye));
    ///
    /// // Parse with extended methods (e.g., methods not defined in the standard)
    /// let allow = Allow::from_str("INVITE, MEETING").unwrap();
    /// assert!(allow.allows(&Method::Invite));
    /// assert!(allow.allows(&Method::Extension("MEETING".into())));
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        let (_, methods_bytes) = all_consuming(parse_allow)(s.as_bytes()).map_err(Error::from)?;
        Ok(methods_bytes)
    }
}

// TODO: Implement methods (e.g., allows(Method)) 

// Implement IntoIterator for Allow
impl IntoIterator for Allow {
    type Item = Method;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

// Implement IntoIterator for &Allow
impl<'a> IntoIterator for &'a Allow {
    type Item = &'a Method;
    type IntoIter = std::slice::Iter<'a, Method>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

// Implement IntoIterator for &mut Allow
impl<'a> IntoIterator for &'a mut Allow {
    type Item = &'a mut Method;
    type IntoIter = std::slice::IterMut<'a, Method>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}

impl TypedHeaderTrait for Allow {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::Allow
    }

    fn to_header(&self) -> Header {
        // Convert the Vec<Method> to Vec<Vec<u8>> as expected by HeaderValue::Allow
        let methods_bytes: Vec<Vec<u8>> = self.0.iter()
            .map(|method| method.to_string().into_bytes())
            .collect();
        
        Header::new(Self::header_name(), HeaderValue::Allow(methods_bytes))
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
                    Allow::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            HeaderValue::Allow(methods_bytes) => {
                // Convert Vec<Vec<u8>> to Vec<Method>
                let mut methods = Vec::with_capacity(methods_bytes.len());
                for method_bytes in methods_bytes {
                    if let Ok(method_str) = std::str::from_utf8(method_bytes) {
                        if let Ok(method) = Method::from_str(method_str) {
                            methods.push(method);
                        } else {
                            return Err(Error::InvalidHeader(
                                format!("Invalid method name: {}", method_str)
                            ));
                        }
                    } else {
                        return Err(Error::InvalidHeader(
                            format!("Invalid UTF-8 in method name")
                        ));
                    }
                }
                Ok(Allow(methods))
            },
            _ => Err(Error::InvalidHeader(
                format!("Unexpected header value type for {}", Self::header_name())
            )),
        }
    }
} 
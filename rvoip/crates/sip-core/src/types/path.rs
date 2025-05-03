//! # SIP Path Header
//!
//! This module provides an implementation of the SIP Path header as defined in
//! [RFC 3327](https://datatracker.ietf.org/doc/html/rfc3327).
//!
//! The Path header field is used in SIP registrations to indicate a path that future
//! requests to the user agent should traverse. This is particularly useful in 
//! environments using edge proxies, NAT traversal, and complex routing topologies.
//!
//! ## Purpose
//!
//! The Path header serves several purposes in SIP:
//!
//! - Provides a means for proxies to specify a route set for future requests to a UA
//! - Facilitates NAT traversal by ensuring requests follow a specific path
//! - Supports complex routing scenarios like multi-homed devices
//! - Enables edge proxy traversal for incoming requests
//!
//! ## Format
//!
//! ```text
//! Path: <sip:p1.example.com;lr>
//! Path: <sip:p1.example.com;lr>, <sip:p2.example.com;lr>
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create an empty Path header
//! let mut path = Path::empty();
//!
//! // Add path entries
//! let uri1 = Uri::from_str("sip:p1.example.com;lr").unwrap();
//! let uri2 = Uri::from_str("sip:p2.example.com;lr").unwrap();
//! path.add_uri(uri1);
//! path.add_uri(uri2);
//!
//! // Create a Path header with a single entry
//! let uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
//! let path = Path::with_uri(uri);
//!
//! // Parse a Path header from a string
//! let path = Path::from_str("<sip:p1.example.com;lr>, <sip:p2.example.com;lr>").unwrap();
//! ```

use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use std::ops::Deref;
use nom::combinator::all_consuming;
use crate::types::Address;
use crate::parser::headers::route::RouteEntry as PathEntry;  // Reuse RouteEntry for Path
use serde::{Deserialize, Serialize};
use crate::types::uri::Uri;
use crate::types::header::Header;
use crate::types::{HeaderName, HeaderValue, TypedHeader, TypedHeaderTrait};

/// Represents the Path header field (RFC 3327).
/// 
/// The Path header field is used in SIP REGISTER requests and responses to enable
/// requests to traverse intermediary proxies between the registrar and the user agent.
/// Each proxy that needs to stay on the path adds a URI identifying itself to the Path header.
///
/// The Path header contains an ordered list of URIs. When a request is sent to a registered
/// user, the registrar will use the Path URIs to construct a Route header with the URIs in
/// the same order as they appeared in the Path header.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a path with two entries
/// let proxy1 = Uri::from_str("sip:p1.example.com;lr").unwrap();
/// let proxy2 = Uri::from_str("sip:p2.example.com;lr").unwrap();
///
/// let mut path = Path::empty();
/// path.add_uri(proxy1);
/// path.add_uri(proxy2);
///
/// assert_eq!(path.len(), 2);
/// assert_eq!(path.to_string(), "<sip:p1.example.com;lr>, <sip:p2.example.com;lr>");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Path(pub Vec<PathEntry>);

impl Path {
    /// Creates a new Path header with a list of path entries.
    ///
    /// This method initializes a Path header containing multiple path entries.
    ///
    /// # Parameters
    ///
    /// - `list`: A vector of path entries (PathEntry)
    ///
    /// # Returns
    ///
    /// A new `Path` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create path entries
    /// let uri1 = Uri::from_str("sip:p1.example.com;lr").unwrap();
    /// let uri2 = Uri::from_str("sip:p2.example.com;lr").unwrap();
    /// let addr1 = Address::new(uri1);
    /// let addr2 = Address::new(uri2);
    /// 
    /// // Use path entry type (which is the same as RouteEntry)
    /// let entries = vec![
    ///     PathEntry(addr1), 
    ///     PathEntry(addr2)
    /// ];
    ///
    /// // Create the Path header
    /// let path = Path::new(entries);
    /// assert_eq!(path.len(), 2);
    /// ```
    pub fn new(list: Vec<PathEntry>) -> Self {
        Self(list)
    }
    
    /// Creates a new empty Path header.
    ///
    /// Initializes a Path header with no path entries.
    ///
    /// # Returns
    ///
    /// A new empty `Path` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let path = Path::empty();
    /// assert!(path.is_empty());
    /// assert_eq!(path.len(), 0);
    /// assert_eq!(path.to_string(), "");
    /// ```
    pub fn empty() -> Self {
        Self(Vec::new())
    }
    
    /// Creates a new Path header with a single address.
    ///
    /// Initializes a Path header with a single entry representing the given address.
    /// This is a convenience method for creating a Path header with one named URI.
    ///
    /// # Parameters
    ///
    /// - `address`: The Address to use for the path entry
    ///
    /// # Returns
    ///
    /// A new `Path` instance with one entry
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create an address with a display name
    /// let uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
    /// let address = Address::new_with_display_name("Edge Proxy", uri);
    ///
    /// // Create a Path header with this address
    /// let path = Path::with_address(address);
    /// assert_eq!(path.len(), 1);
    /// assert_eq!(path.to_string(), "\"Edge Proxy\" <sip:proxy.example.com;lr>");
    /// ```
    pub fn with_address(address: Address) -> Self {
        Self(vec![PathEntry(address)])
    }
    
    /// Creates a new Path header with a single URI.
    ///
    /// Initializes a Path header with a single entry representing the given URI.
    /// This is a convenience method for creating a Path header with one URI and no display name.
    ///
    /// # Parameters
    ///
    /// - `uri`: The URI to use for the path entry
    ///
    /// # Returns
    ///
    /// A new `Path` instance with one entry
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a URI
    /// let uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
    ///
    /// // Create a Path header with this URI
    /// let path = Path::with_uri(uri);
    /// assert_eq!(path.len(), 1);
    /// assert_eq!(path.to_string(), "<sip:proxy.example.com;lr>");
    /// ```
    pub fn with_uri(uri: Uri) -> Self {
        let address = Address::new(uri);
        Self::with_address(address)
    }
    
    /// Checks if the Path header has no entries.
    ///
    /// # Returns
    ///
    /// `true` if there are no path entries, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let path = Path::empty();
    /// assert!(path.is_empty());
    ///
    /// let uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
    /// let path = Path::with_uri(uri);
    /// assert!(!path.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    
    /// Returns the number of path entries.
    ///
    /// # Returns
    ///
    /// The number of path entries in this header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let path = Path::empty();
    /// assert_eq!(path.len(), 0);
    ///
    /// let uri1 = Uri::from_str("sip:p1.example.com;lr").unwrap();
    /// let uri2 = Uri::from_str("sip:p2.example.com;lr").unwrap();
    /// let mut path = Path::with_uri(uri1);
    /// path.add_uri(uri2);
    /// assert_eq!(path.len(), 2);
    /// ```
    pub fn len(&self) -> usize {
        self.0.len()
    }
    
    /// Returns the first path entry, if any.
    ///
    /// The first entry in the list is the closest proxy to the registrar.
    ///
    /// # Returns
    ///
    /// An optional reference to the first path entry
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let path = Path::empty();
    /// assert!(path.first().is_none());
    ///
    /// let uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
    /// let path = Path::with_uri(uri);
    /// assert!(path.first().is_some());
    /// ```
    pub fn first(&self) -> Option<&PathEntry> {
        self.0.first()
    }
    
    /// Returns the last path entry, if any.
    ///
    /// The last entry in the list is the closest proxy to the user agent.
    ///
    /// # Returns
    ///
    /// An optional reference to the last path entry
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let path = Path::empty();
    /// assert!(path.last().is_none());
    ///
    /// let uri1 = Uri::from_str("sip:p1.example.com;lr").unwrap();
    /// let uri2 = Uri::from_str("sip:p2.example.com;lr").unwrap();
    /// let mut path = Path::with_uri(uri1);
    /// path.add_uri(uri2);
    /// assert_eq!(path.last().unwrap().0.uri.to_string(), "sip:p2.example.com;lr");
    /// ```
    pub fn last(&self) -> Option<&PathEntry> {
        self.0.last()
    }
    
    /// Adds a path entry to the end of the list.
    ///
    /// Adds a raw path entry (which is the same type as a RouteEntry).
    ///
    /// # Parameters
    ///
    /// - `entry`: The path entry to add
    ///
    /// # Returns
    ///
    /// A mutable reference to this Path header for chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
    /// let addr = Address::new(uri);
    /// let entry = PathEntry(addr);
    ///
    /// let mut path = Path::empty();
    /// path.add(entry);
    /// assert_eq!(path.len(), 1);
    /// ```
    pub fn add(&mut self, entry: PathEntry) -> &mut Self {
        self.0.push(entry);
        self
    }
    
    /// Adds an address as a path entry.
    ///
    /// This is a convenience method that creates a path entry from an address.
    ///
    /// # Parameters
    ///
    /// - `address`: The address to add as a path entry
    ///
    /// # Returns
    ///
    /// A mutable reference to this Path header for chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
    /// let addr = Address::new_with_display_name("Edge Proxy", uri);
    ///
    /// let mut path = Path::empty();
    /// path.add_address(addr);
    /// assert_eq!(path.len(), 1);
    /// assert!(path.to_string().contains("Edge Proxy"));
    /// ```
    pub fn add_address(&mut self, address: Address) -> &mut Self {
        self.0.push(PathEntry(address));
        self
    }
    
    /// Adds a URI as a path entry.
    ///
    /// This is a convenience method that creates a path entry from a URI without a display name.
    ///
    /// # Parameters
    ///
    /// - `uri`: The URI to add as a path entry
    ///
    /// # Returns
    ///
    /// A mutable reference to this Path header for chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
    ///
    /// let mut path = Path::empty();
    /// path.add_uri(uri);
    /// assert_eq!(path.len(), 1);
    /// assert!(path.to_string().contains("sip:proxy.example.com;lr"));
    /// ```
    pub fn add_uri(&mut self, uri: Uri) -> &mut Self {
        let address = Address::new(uri);
        self.add_address(address)
    }
    
    /// Returns an iterator over the path entries.
    ///
    /// # Returns
    ///
    /// An iterator over references to the path entries
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri1 = Uri::from_str("sip:p1.example.com;lr").unwrap();
    /// let uri2 = Uri::from_str("sip:p2.example.com;lr").unwrap();
    ///
    /// let mut path = Path::empty();
    /// path.add_uri(uri1);
    /// path.add_uri(uri2);
    ///
    /// let mut count = 0;
    /// for entry in path.iter() {
    ///     count += 1;
    /// }
    /// assert_eq!(count, 2);
    /// ```
    pub fn iter(&self) -> impl Iterator<Item = &PathEntry> {
        self.0.iter()
    }
}

impl fmt::Display for Path {
    /// Formats the Path header as a string.
    ///
    /// Converts the header to its string representation according to RFC 3327.
    ///
    /// # Returns
    ///
    /// A string representation of the Path header
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        
        for entry in &self.0 {
            if !first {
                write!(f, ", ")?;
            }
            write!(f, "{}", entry.0)?;
            first = false;
        }
        
        Ok(())
    }
}

impl FromStr for Path {
    type Err = Error;

    /// Parses a string into a Path header.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse, which should contain a comma-separated list of SIP URIs
    ///
    /// # Returns
    ///
    /// A Result containing the parsed Path header if successful, or an error otherwise
    fn from_str(s: &str) -> Result<Self> {
        if s.is_empty() {
            return Ok(Path::empty());
        }
        
        // Use the Route parser for Path since they have the same format
        let input_bytes = s.as_bytes();
        let parse_result = all_consuming(crate::parser::headers::parse_route)(input_bytes);
        
        match parse_result {
            Ok((_, route)) => Ok(Path(route.0)),
            Err(_) => Err(Error::ParseError(format!("Failed to parse Path header: {}", s)))
        }
    }
}

impl Deref for Path {
    type Target = Vec<PathEntry>;

    /// Implements Deref to allow direct access to the underlying vector of path entries.
    ///
    /// This allows operations like indexing (path[0]) to work directly on a Path instance.
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> IntoIterator for &'a Path {
    type Item = &'a PathEntry;
    type IntoIter = std::slice::Iter<'a, PathEntry>;

    /// Implements IntoIterator for Path references.
    ///
    /// This allows for..in loops to work on Path instances.
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl From<Vec<PathEntry>> for Path {
    /// Creates a Path header from a vector of PathEntry objects.
    ///
    /// # Parameters
    ///
    /// - `list`: A vector of PathEntry objects
    ///
    /// # Returns
    ///
    /// A new Path instance
    fn from(list: Vec<PathEntry>) -> Self {
        Path(list)
    }
}

impl From<PathEntry> for Path {
    /// Creates a Path header from a single PathEntry.
    ///
    /// # Parameters
    ///
    /// - `entry`: A single PathEntry
    ///
    /// # Returns
    ///
    /// A new Path instance with a single entry
    fn from(entry: PathEntry) -> Self {
        Path(vec![entry])
    }
}

impl From<Address> for Path {
    /// Creates a Path header from a single Address.
    ///
    /// # Parameters
    ///
    /// - `address`: A single Address
    ///
    /// # Returns
    ///
    /// A new Path instance with a single entry
    fn from(address: Address) -> Self {
        Path::with_address(address)
    }
}

impl TypedHeaderTrait for Path {
    type Name = HeaderName;

    /// Returns the header name for this header type.
    ///
    /// # Returns
    ///
    /// The `HeaderName::Path` enum variant
    fn header_name() -> Self::Name {
        HeaderName::Path
    }

    /// Converts this Path header into a generic Header.
    ///
    /// Creates a Header instance from this Path header, which can be used
    /// when constructing SIP messages.
    ///
    /// # Returns
    ///
    /// A Header instance representing this Path header
    fn to_header(&self) -> Header {
        let value_string = self.to_string();
        let value = crate::types::headers::HeaderValue::Raw(value_string.into_bytes());
        Header::new(Self::header_name(), value)
    }

    /// Creates a Path header from a generic Header.
    ///
    /// Attempts to parse and convert a generic Header into a Path header.
    /// This will succeed if the header is a valid Path header.
    ///
    /// # Parameters
    ///
    /// - `header`: The generic Header to convert
    ///
    /// # Returns
    ///
    /// A Result containing the parsed Path header if successful, or an error otherwise
    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(
                format!("Expected {} header, got {}", Self::header_name(), header.name)
            ));
        }

        match &header.value {
            HeaderValue::Raw(bytes) => {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    Path::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            // Reuse Route header processing since they have the same format
            HeaderValue::Route(entries) => {
                Ok(Path(entries.clone()))
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
    fn test_path_empty() {
        let path = Path::empty();
        assert!(path.is_empty());
        assert_eq!(path.len(), 0);
        assert_eq!(path.to_string(), "");
    }

    #[test]
    fn test_path_with_uri() {
        let uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
        let path = Path::with_uri(uri);
        
        assert_eq!(path.len(), 1);
        assert_eq!(path.to_string(), "<sip:proxy.example.com;lr>");
    }

    #[test]
    fn test_path_with_address() {
        let uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
        let address = Address::new_with_display_name("Edge Proxy", uri);
        let path = Path::with_address(address);
        
        assert_eq!(path.len(), 1);
        assert_eq!(path.to_string(), "\"Edge Proxy\" <sip:proxy.example.com;lr>");
    }

    #[test]
    fn test_path_add_methods() {
        let mut path = Path::empty();
        
        // Test add_uri
        let uri1 = Uri::from_str("sip:proxy1.example.com;lr").unwrap();
        path.add_uri(uri1);
        assert_eq!(path.len(), 1);
        
        // Test add_address
        let uri2 = Uri::from_str("sip:proxy2.example.com;lr").unwrap();
        let address = Address::new_with_display_name("Second Proxy", uri2);
        path.add_address(address);
        assert_eq!(path.len(), 2);
        
        // Test add (with raw PathEntry)
        let uri3 = Uri::from_str("sip:proxy3.example.com;lr").unwrap();
        let address3 = Address::new(uri3);
        let entry = PathEntry(address3);
        path.add(entry);
        assert_eq!(path.len(), 3);
        
        // Check string representation
        let path_str = path.to_string();
        assert!(path_str.contains("proxy1.example.com"));
        assert!(path_str.contains("Second Proxy"));
        assert!(path_str.contains("proxy3.example.com"));
    }

    #[test]
    fn test_path_from_impls() {
        // Test From<Vec<PathEntry>>
        let uri1 = Uri::from_str("sip:proxy1.example.com;lr").unwrap();
        let uri2 = Uri::from_str("sip:proxy2.example.com;lr").unwrap();
        let addr1 = Address::new(uri1);
        let addr2 = Address::new(uri2);
        let entries = vec![PathEntry(addr1), PathEntry(addr2)];
        
        let path = Path::from(entries);
        assert_eq!(path.len(), 2);
        
        // Test From<PathEntry>
        let uri3 = Uri::from_str("sip:proxy3.example.com;lr").unwrap();
        let addr3 = Address::new(uri3);
        let entry = PathEntry(addr3);
        
        let path = Path::from(entry);
        assert_eq!(path.len(), 1);
        
        // Test From<Address>
        let uri4 = Uri::from_str("sip:proxy4.example.com;lr").unwrap();
        let addr4 = Address::new(uri4);
        
        let path = Path::from(addr4);
        assert_eq!(path.len(), 1);
    }

    #[test]
    fn test_path_fromstr_and_display() {
        // Test parsing a simple path header
        let path_str = "<sip:proxy1.example.com;lr>, <sip:proxy2.example.com;lr>";
        let path = Path::from_str(path_str).unwrap();
        
        assert_eq!(path.len(), 2);
        assert_eq!(path[0].0.uri.to_string(), "sip:proxy1.example.com;lr");
        assert_eq!(path[1].0.uri.to_string(), "sip:proxy2.example.com;lr");
        
        // Test round-trip (format back to string)
        assert_eq!(path.to_string(), path_str);
    }

    #[test]
    fn test_path_typed_header_trait() {
        // Test header_name
        assert_eq!(Path::header_name(), HeaderName::Path);
        
        // Test to_header and from_header
        let uri1 = Uri::from_str("sip:proxy1.example.com;lr").unwrap();
        let uri2 = Uri::from_str("sip:proxy2.example.com;lr").unwrap();
        let mut path = Path::empty();
        path.add_uri(uri1);
        path.add_uri(uri2);
        
        let header = path.to_header();
        assert_eq!(header.name, HeaderName::Path);
        
        let path2 = Path::from_header(&header).unwrap();
        assert_eq!(path2.len(), 2);
        assert_eq!(path2[0].0.uri.to_string(), "sip:proxy1.example.com;lr");
        assert_eq!(path2[1].0.uri.to_string(), "sip:proxy2.example.com;lr");
    }
} 
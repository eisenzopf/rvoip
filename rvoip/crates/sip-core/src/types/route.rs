//! # SIP Route Header
//!
//! This module provides an implementation of the SIP Route header as defined in
//! [RFC 3261 Section 20.34](https://datatracker.ietf.org/doc/html/rfc3261#section-20.34).
//!
//! The Route header field is used to force routing for a request through a list
//! of proxies. Each proxy in the route set is represented with a SIP/SIPS URI.
//!
//! ## Purpose
//!
//! The Route header serves several purposes in SIP:
//!
//! - Implements loose routing and strict routing mechanisms
//! - Forces a request to visit a set of proxies in a specified order
//! - Used by proxies and UACs to route requests to their destinations via specific paths
//! - Provides a way for proxies to record the route a request has taken
//!
//! ## Format
//!
//! ```text
//! Route: <sip:proxy1.example.com;lr>, <sip:proxy2.example.com;lr>
//! Route: "Proxy 1" <sip:proxy1.example.com;lr>, "Proxy 2" <sip:proxy2.example.com;lr>
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create an empty Route header
//! let mut route = Route::empty();
//!
//! // Add route entries
//! let uri1 = Uri::from_str("sip:proxy1.example.com;lr").unwrap();
//! let uri2 = Uri::from_str("sip:proxy2.example.com;lr").unwrap();
//! route.add_uri(uri1);
//! route.add_uri(uri2);
//!
//! // Create a Route header with a single entry
//! let uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
//! let route = Route::with_uri(uri);
//!
//! // Parse a Route header from a string
//! let route = Route::from_str("<sip:proxy1.example.com;lr>, <sip:proxy2.example.com;lr>").unwrap();
//! ```

use crate::parser::headers::parse_route;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use std::ops::Deref;
use nom::combinator::all_consuming;
use crate::types::Address;
use crate::parser::headers::route::RouteEntry as ParserRouteValue;
use serde::{Deserialize, Serialize};
use crate::parser::ParseResult;
use crate::types::param::Param;
use crate::types::uri::Uri;

/// Represents the Route header field (RFC 3261 Section 20.34).
/// Contains a list of route entries (typically Addresses).
/// 
/// The Route header field is used to force routing for a request through a list
/// of proxies. Each proxy in the route set is represented with a URI.
///
/// The Route header contains an ordered list of URIs. Each URI in the list represents 
/// a proxy server that the request must visit on its way to the final destination.
/// The leftmost URI in the list represents the next hop server.
///
/// A Route header typically has the loose routing parameter (`;lr`) attached to each URI.
/// When a proxy receives a request with its address in the first Route header field value,
/// it removes that value from the Route header field and forwards the request to the URI
/// in the next Route header field value.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a route with two entries
/// let proxy1 = Uri::from_str("sip:proxy1.example.com;lr").unwrap();
/// let proxy2 = Uri::from_str("sip:proxy2.example.com;lr").unwrap();
///
/// let mut route = Route::empty();
/// route.add_uri(proxy1);
/// route.add_uri(proxy2);
///
/// assert_eq!(route.len(), 2);
/// assert_eq!(route.to_string(), "<sip:proxy1.example.com;lr>, <sip:proxy2.example.com;lr>");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route(pub Vec<ParserRouteValue>);

impl Route {
    /// Creates a new Route header with a list of route entries.
    ///
    /// This method initializes a Route header containing multiple route entries.
    ///
    /// # Parameters
    ///
    /// - `list`: A vector of route entries (ParserRouteValue)
    ///
    /// # Returns
    ///
    /// A new `Route` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create route entries
    /// let uri1 = Uri::from_str("sip:proxy1.example.com;lr").unwrap();
    /// let uri2 = Uri::from_str("sip:proxy2.example.com;lr").unwrap();
    /// let addr1 = Address::new(None::<&str>, uri1);
    /// let addr2 = Address::new(None::<&str>, uri2);
    /// 
    /// // Use parser route value type
    /// let entries = vec![
    ///     ParserRouteValue(addr1), 
    ///     ParserRouteValue(addr2)
    /// ];
    ///
    /// // Create the Route header
    /// let route = Route::new(entries);
    /// assert_eq!(route.len(), 2);
    /// ```
    pub fn new(list: Vec<ParserRouteValue>) -> Self {
        Self(list)
    }
    
    /// Creates a new empty Route header.
    ///
    /// Initializes a Route header with no route entries.
    ///
    /// # Returns
    ///
    /// A new empty `Route` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let route = Route::empty();
    /// assert!(route.is_empty());
    /// assert_eq!(route.len(), 0);
    /// assert_eq!(route.to_string(), "");
    /// ```
    pub fn empty() -> Self {
        Self(Vec::new())
    }
    
    /// Creates a new Route header with a single address.
    ///
    /// Initializes a Route header with a single entry representing the given address.
    /// This is a convenience method for creating a Route header with one named URI.
    ///
    /// # Parameters
    ///
    /// - `address`: The Address to use for the route entry
    ///
    /// # Returns
    ///
    /// A new `Route` instance with one entry
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create an address with a display name
    /// let uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
    /// let address = Address::new(Some("Main Proxy"), uri);
    ///
    /// // Create a Route header with this address
    /// let route = Route::with_address(address);
    /// assert_eq!(route.len(), 1);
    /// assert_eq!(route.to_string(), "\"Main Proxy\" <sip:proxy.example.com;lr>");
    /// ```
    pub fn with_address(address: Address) -> Self {
        Self(vec![ParserRouteValue(address)])
    }
    
    /// Creates a new Route header with a single URI.
    ///
    /// Initializes a Route header with a single entry representing the given URI.
    /// This is a convenience method for creating a Route header with one URI and no display name.
    ///
    /// # Parameters
    ///
    /// - `uri`: The URI to use for the route entry
    ///
    /// # Returns
    ///
    /// A new `Route` instance with one entry
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
    /// // Create a Route header with this URI
    /// let route = Route::with_uri(uri);
    /// assert_eq!(route.len(), 1);
    /// assert_eq!(route.to_string(), "<sip:proxy.example.com;lr>");
    /// ```
    pub fn with_uri(uri: Uri) -> Self {
        Self(vec![ParserRouteValue(Address::new(None::<String>, uri))])
    }
    
    /// Checks if the route list is empty.
    ///
    /// # Returns
    ///
    /// `true` if the Route header contains no entries, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let route = Route::empty();
    /// assert!(route.is_empty());
    ///
    /// let uri = Uri::sip("proxy.example.com");
    /// let route = Route::with_uri(uri);
    /// assert!(!route.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    
    /// Returns the number of route entries.
    ///
    /// # Returns
    ///
    /// The number of route entries in this Route header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let route = Route::empty();
    /// assert_eq!(route.len(), 0);
    ///
    /// let route_str = "<sip:proxy1.example.com;lr>, <sip:proxy2.example.com;lr>";
    /// let route = Route::from_str(route_str).unwrap();
    /// assert_eq!(route.len(), 2);
    /// ```
    pub fn len(&self) -> usize {
        self.0.len()
    }
    
    /// Returns a reference to the first route entry, if any.
    ///
    /// # Returns
    ///
    /// A reference to the first route entry, or `None` if the route is empty
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let empty_route = Route::empty();
    /// assert!(empty_route.first().is_none());
    ///
    /// let uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
    /// let route = Route::with_uri(uri.clone());
    /// assert!(route.first().is_some());
    /// assert_eq!(route.first().unwrap().0.uri, uri);
    /// ```
    pub fn first(&self) -> Option<&ParserRouteValue> {
        self.0.first()
    }
    
    /// Returns a reference to the last route entry, if any.
    ///
    /// # Returns
    ///
    /// A reference to the last route entry, or `None` if the route is empty
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let empty_route = Route::empty();
    /// assert!(empty_route.last().is_none());
    ///
    /// let mut route = Route::empty();
    /// let uri1 = Uri::from_str("sip:proxy1.example.com;lr").unwrap();
    /// let uri2 = Uri::from_str("sip:proxy2.example.com;lr").unwrap(); 
    /// route.add_uri(uri1);
    /// route.add_uri(uri2.clone());
    ///
    /// assert!(route.last().is_some());
    /// assert_eq!(route.last().unwrap().0.uri, uri2);
    /// ```
    pub fn last(&self) -> Option<&ParserRouteValue> {
        self.0.last()
    }
    
    /// Adds a route entry to the end of the list.
    ///
    /// # Parameters
    ///
    /// - `entry`: The route entry to add
    ///
    /// # Returns
    ///
    /// A mutable reference to this Route instance for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let mut route = Route::empty();
    ///
    /// // Create and add a route entry
    /// let uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
    /// let address = Address::new(None::<&str>, uri);
    /// let entry = ParserRouteValue(address);
    ///
    /// route.add(entry);
    /// assert_eq!(route.len(), 1);
    /// ```
    pub fn add(&mut self, entry: ParserRouteValue) -> &mut Self {
        self.0.push(entry);
        self
    }
    
    /// Adds an address as a route entry to the end of the list.
    ///
    /// # Parameters
    ///
    /// - `address`: The address to add as a route entry
    ///
    /// # Returns
    ///
    /// A mutable reference to this Route instance for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let mut route = Route::empty();
    ///
    /// // Create and add an address
    /// let uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
    /// let address = Address::new(Some("Main Proxy"), uri);
    ///
    /// route.add_address(address);
    /// assert_eq!(route.len(), 1);
    /// assert_eq!(route.to_string(), "\"Main Proxy\" <sip:proxy.example.com;lr>");
    /// ```
    pub fn add_address(&mut self, address: Address) -> &mut Self {
        self.0.push(ParserRouteValue(address));
        self
    }
    
    /// Adds a URI as a route entry to the end of the list.
    ///
    /// Creates a route entry from the URI without a display name.
    ///
    /// # Parameters
    ///
    /// - `uri`: The URI to add as a route entry
    ///
    /// # Returns
    ///
    /// A mutable reference to this Route instance for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let mut route = Route::empty();
    ///
    /// // Add URIs to the route
    /// let uri1 = Uri::from_str("sip:proxy1.example.com;lr").unwrap();
    /// let uri2 = Uri::from_str("sip:proxy2.example.com;lr").unwrap();
    ///
    /// route.add_uri(uri1);
    /// route.add_uri(uri2);
    ///
    /// assert_eq!(route.len(), 2);
    /// assert_eq!(route.to_string(), "<sip:proxy1.example.com;lr>, <sip:proxy2.example.com;lr>");
    /// ```
    pub fn add_uri(&mut self, uri: Uri) -> &mut Self {
        self.0.push(ParserRouteValue(Address::new(None::<String>, uri)));
        self
    }
    
    /// Returns an iterator over the route entries.
    ///
    /// # Returns
    ///
    /// An iterator over the route entries in this Route header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let mut route = Route::empty();
    ///
    /// // Add URIs to the route
    /// let uri1 = Uri::from_str("sip:proxy1.example.com;lr").unwrap();
    /// let uri2 = Uri::from_str("sip:proxy2.example.com;lr").unwrap();
    ///
    /// route.add_uri(uri1.clone());
    /// route.add_uri(uri2.clone());
    ///
    /// // Iterate over the route entries
    /// let collected_uris: Vec<_> = route.iter().map(|entry| &entry.0.uri).collect();
    /// assert_eq!(collected_uris.len(), 2);
    /// assert_eq!(collected_uris[0], &uri1);
    /// assert_eq!(collected_uris[1], &uri2);
    /// ```
    pub fn iter(&self) -> impl Iterator<Item = &ParserRouteValue> {
        self.0.iter()
    }
}

impl fmt::Display for Route {
    /// Formats the Route header as a string.
    ///
    /// Converts the Route header to its string representation, with each
    /// route entry separated by a comma and a space, as specified in RFC 3261.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a route with two entries
    /// let mut route = Route::empty();
    /// route.add_uri(Uri::from_str("sip:proxy1.example.com;lr").unwrap());
    /// route.add_uri(Uri::from_str("sip:proxy2.example.com;lr").unwrap());
    ///
    /// // Format as a string
    /// let route_str = route.to_string();
    /// assert_eq!(route_str, "<sip:proxy1.example.com;lr>, <sip:proxy2.example.com;lr>");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.iter().map(|r| r.to_string()).collect::<Vec<String>>().join(", "))
    }
}

impl FromStr for Route {
    type Err = Error;

    /// Parses a string into a Route header.
    ///
    /// This method converts a string representation of a Route header into a
    /// structured Route object. It parses the comma-separated list of route entries.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed Route header, or an error if parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a simple Route header
    /// let route = Route::from_str("<sip:proxy.example.com;lr>").unwrap();
    /// assert_eq!(route.len(), 1);
    ///
    /// // Parse a Route header with multiple entries
    /// let route = Route::from_str("<sip:proxy1.example.com;lr>, <sip:proxy2.example.com;lr>").unwrap();
    /// assert_eq!(route.len(), 2);
    ///
    /// // Parse a Route header with display names
    /// let route = Route::from_str("\"Proxy 1\" <sip:proxy1.example.com;lr>, \"Proxy 2\" <sip:proxy2.example.com;lr>").unwrap();
    /// assert_eq!(route.len(), 2);
    /// assert_eq!(route.first().unwrap().0.display_name, Some("Proxy 1".to_string()));
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        use crate::parser::headers::route::parse_route;

        match all_consuming(parse_route)(s.as_bytes()) {
            Ok((_, route_header)) => Ok(route_header),
            Err(e) => Err(Error::ParseError( 
                format!("Failed to parse Route header: {:?}", e)
            ))
        }
    }
}

impl Deref for Route {
    type Target = Vec<ParserRouteValue>;

    /// Implements the Deref trait for Route.
    ///
    /// This allows a Route instance to be treated as a reference to a Vec<ParserRouteValue>,
    /// which enables using Vec methods directly on a Route instance.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let route = Route::from_str("<sip:proxy1.example.com;lr>, <sip:proxy2.example.com;lr>").unwrap();
    ///
    /// // Use Vec methods directly on Route
    /// assert_eq!(route.len(), 2);
    /// 
    /// // Use the iter() method instead of direct iteration
    /// for entry in route.iter() {
    ///     // Access each entry
    ///     let uri = &entry.0.uri;
    /// }
    /// ```
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Vec<ParserRouteValue>> for Route {
    /// Creates a Route from a Vec of ParserRouteValue.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create route entries
    /// let uri1 = Uri::from_str("sip:proxy1.example.com;lr").unwrap();
    /// let uri2 = Uri::from_str("sip:proxy2.example.com;lr").unwrap();
    /// let addr1 = Address::new(None::<&str>, uri1);
    /// let addr2 = Address::new(None::<&str>, uri2);
    ///
    /// let entries = vec![
    ///     ParserRouteValue(addr1),
    ///     ParserRouteValue(addr2)
    /// ];
    ///
    /// // Create Route using From trait
    /// let route = Route::from(entries);
    /// assert_eq!(route.len(), 2);
    /// ```
    fn from(list: Vec<ParserRouteValue>) -> Self {
        Self(list)
    }
}

impl From<ParserRouteValue> for Route {
    /// Creates a Route from a single ParserRouteValue.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a route entry
    /// let uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
    /// let addr = Address::new(None::<&str>, uri);
    /// let entry = ParserRouteValue(addr);
    ///
    /// // Create Route using From trait
    /// let route = Route::from(entry);
    /// assert_eq!(route.len(), 1);
    /// ```
    fn from(entry: ParserRouteValue) -> Self {
        Self(vec![entry])
    }
}

impl From<Address> for Route {
    /// Creates a Route from a single Address.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create an address
    /// let uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
    /// let addr = Address::new(Some("Main Proxy"), uri);
    ///
    /// // Create Route using From trait
    /// let route = Route::from(addr);
    /// assert_eq!(route.len(), 1);
    /// assert_eq!(route.first().unwrap().0.display_name, Some("Main Proxy".to_string()));
    /// ```
    fn from(address: Address) -> Self {
        Self(vec![ParserRouteValue(address)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::uri::{Uri, Scheme, Host};
    use crate::types::param::Param;
    
    #[test]
    fn test_route_empty() {
        let route = Route::empty();
        assert!(route.is_empty());
        assert_eq!(route.len(), 0);
        assert!(route.first().is_none());
        assert!(route.last().is_none());
    }
    
    #[test]
    fn test_route_with_uri() {
        let uri = Uri::sip("example.com");
        let route = Route::with_uri(uri.clone());
        assert!(!route.is_empty());
        assert_eq!(route.len(), 1);
        assert_eq!(route.first().unwrap().0.uri, uri);
    }
    
    #[test]
    fn test_route_with_address() {
        let uri = Uri::sip("example.com");
        let address = Address::new(Some("Test Proxy"), uri.clone());
        let route = Route::with_address(address.clone());
        assert_eq!(route.len(), 1);
        assert_eq!(route.first().unwrap().0, address);
        assert_eq!(route.first().unwrap().0.display_name, Some("Test Proxy".to_string()));
    }
    
    #[test]
    fn test_route_add_methods() {
        let mut route = Route::empty();
        
        // Add a URI
        let uri1 = Uri::sip("proxy1.example.com");
        route.add_uri(uri1.clone());
        assert_eq!(route.len(), 1);
        
        // Add an address
        let uri2 = Uri::sip("proxy2.example.com");
        let address = Address::new(Some("Proxy 2"), uri2.clone());
        route.add_address(address.clone());
        assert_eq!(route.len(), 2);
        
        // Add a route entry
        let uri3 = Uri::sips("secure.example.com");
        let address3 = Address::new(None::<String>, uri3.clone());
        let entry = ParserRouteValue(address3.clone());
        route.add(entry);
        assert_eq!(route.len(), 3);
        
        // Check first and last
        assert_eq!(route.first().unwrap().0.uri, uri1);
        assert_eq!(route.last().unwrap().0.uri, uri3);
        
        // Check iteration
        let uris: Vec<_> = route.iter().map(|e| &e.0.uri).collect();
        assert_eq!(uris.len(), 3);
        assert_eq!(uris[0], &uri1);
        assert_eq!(uris[1], &uri2);
        assert_eq!(uris[2], &uri3);
    }
    
    #[test]
    fn test_route_from_impls() {
        // From Vec<ParserRouteValue>
        let uri1 = Uri::sip("proxy1.example.com");
        let uri2 = Uri::sip("proxy2.example.com");
        let addr1 = Address::new(None::<&str>, uri1);
        let addr2 = Address::new(None::<&str>, uri2);
        let entries = vec![ParserRouteValue(addr1), ParserRouteValue(addr2)];
        let route = Route::from(entries);
        assert_eq!(route.len(), 2);
        
        // From ParserRouteValue
        let uri = Uri::sip("proxy.example.com");
        let addr = Address::new(None::<&str>, uri);
        let entry = ParserRouteValue(addr);
        let route = Route::from(entry);
        assert_eq!(route.len(), 1);
        
        // From Address
        let uri = Uri::sip("proxy.example.com");
        let addr = Address::new(None::<&str>, uri);
        let route = Route::from(addr);
        assert_eq!(route.len(), 1);
    }
    
    #[test]
    fn test_route_fromstr_and_display() {
        let route_str = "<sip:proxy1.example.com;lr>, <sip:proxy2.example.com;transport=tcp>";
        let route = Route::from_str(route_str).unwrap();
        
        assert_eq!(route.len(), 2);
        assert_eq!(route.first().unwrap().0.uri.scheme, Scheme::Sip);
        
        // Test Display implementation
        let route_displayed = route.to_string();
        assert_eq!(route_displayed, route_str);
        
        // Test round-trip
        let route2 = Route::from_str(&route_displayed).unwrap();
        assert_eq!(route, route2);
    }
} 
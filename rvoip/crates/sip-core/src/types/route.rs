use crate::types::uri_with_params_list::UriWithParamsList;
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route(pub Vec<ParserRouteValue>);

impl Route {
    /// Creates a new Route header with a list of route entries.
    pub fn new(list: Vec<ParserRouteValue>) -> Self {
        Self(list)
    }
    
    /// Creates a new empty Route header.
    pub fn empty() -> Self {
        Self(Vec::new())
    }
    
    /// Creates a new Route header with a single address.
    pub fn with_address(address: Address) -> Self {
        Self(vec![ParserRouteValue(address)])
    }
    
    /// Creates a new Route header with a single URI.
    pub fn with_uri(uri: Uri) -> Self {
        Self(vec![ParserRouteValue(Address::new(None::<String>, uri))])
    }
    
    /// Checks if the route list is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    
    /// Returns the number of route entries.
    pub fn len(&self) -> usize {
        self.0.len()
    }
    
    /// Returns a reference to the first route entry, if any.
    pub fn first(&self) -> Option<&ParserRouteValue> {
        self.0.first()
    }
    
    /// Returns a reference to the last route entry, if any.
    pub fn last(&self) -> Option<&ParserRouteValue> {
        self.0.last()
    }
    
    /// Adds a route entry to the end of the list.
    pub fn add(&mut self, entry: ParserRouteValue) -> &mut Self {
        self.0.push(entry);
        self
    }
    
    /// Adds an address as a route entry to the end of the list.
    pub fn add_address(&mut self, address: Address) -> &mut Self {
        self.0.push(ParserRouteValue(address));
        self
    }
    
    /// Adds a URI as a route entry to the end of the list.
    pub fn add_uri(&mut self, uri: Uri) -> &mut Self {
        self.0.push(ParserRouteValue(Address::new(None::<String>, uri)));
        self
    }
    
    /// Returns an iterator over the route entries.
    pub fn iter(&self) -> impl Iterator<Item = &ParserRouteValue> {
        self.0.iter()
    }
}

impl fmt::Display for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.iter().map(|r| r.to_string()).collect::<Vec<String>>().join(", "))
    }
}

impl FromStr for Route {
    type Err = Error;

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
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Vec<ParserRouteValue>> for Route {
    fn from(list: Vec<ParserRouteValue>) -> Self {
        Self(list)
    }
}

impl From<ParserRouteValue> for Route {
    fn from(entry: ParserRouteValue) -> Self {
        Self(vec![entry])
    }
}

impl From<Address> for Route {
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
        let addr1 = Address::new(None::<String>, uri1);
        let addr2 = Address::new(None::<String>, uri2);
        let entries = vec![ParserRouteValue(addr1), ParserRouteValue(addr2)];
        let route = Route::from(entries);
        assert_eq!(route.len(), 2);
        
        // From ParserRouteValue
        let uri = Uri::sip("proxy.example.com");
        let addr = Address::new(None::<String>, uri);
        let entry = ParserRouteValue(addr);
        let route = Route::from(entry);
        assert_eq!(route.len(), 1);
        
        // From Address
        let uri = Uri::sip("proxy.example.com");
        let addr = Address::new(None::<String>, uri);
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
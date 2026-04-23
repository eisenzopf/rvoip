//! # SIP Service-Route Header
//!
//! This module provides an implementation of the SIP Service-Route header as defined in
//! [RFC 3608](https://datatracker.ietf.org/doc/html/rfc3608).
//!
//! The Service-Route header field is returned by a registrar in a 2xx response to a
//! REGISTER request. It carries a route set that the registered UA MUST use as a
//! pre-loaded Route for subsequent out-of-dialog requests issued within that
//! registration binding. This is the inverse companion to the Path header (RFC 3327):
//!
//! - **Path** (RFC 3327): proxies on the *inbound* path toward the UA.
//! - **Service-Route** (RFC 3608): proxies on the *outbound* path *from* the UA.
//! - **Record-Route** (RFC 3261): proxies on the in-dialog path.
//!
//! ## Format
//!
//! ```text
//! Service-Route: <sip:orig-proxy.example.com;lr>
//! Service-Route: <sip:orig-proxy.example.com;lr>, <sip:core.example.com;lr>
//! ```

use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use std::ops::Deref;
use nom::combinator::all_consuming;
use crate::types::Address;
use crate::parser::headers::route::RouteEntry as ServiceRouteEntry;
use serde::{Deserialize, Serialize};
use crate::types::uri::Uri;
use crate::types::header::Header;
use crate::types::{HeaderName, HeaderValue, TypedHeaderTrait};

/// Represents the Service-Route header field (RFC 3608).
///
/// Contains an ordered list of URIs supplied by the registrar. When the UA sends
/// an out-of-dialog request within the registration binding, it MUST pre-load
/// these URIs as Route headers in the order received.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceRoute(pub Vec<ServiceRouteEntry>);

impl ServiceRoute {
    /// Creates a new Service-Route header from a list of entries.
    pub fn new(list: Vec<ServiceRouteEntry>) -> Self {
        Self(list)
    }

    /// Creates an empty Service-Route header.
    pub fn empty() -> Self {
        Self(Vec::new())
    }

    /// Creates a Service-Route header from a single Address.
    pub fn with_address(address: Address) -> Self {
        Self(vec![ServiceRouteEntry(address)])
    }

    /// Creates a Service-Route header from a single URI.
    pub fn with_uri(uri: Uri) -> Self {
        let address = Address::new(uri);
        Self::with_address(address)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn first(&self) -> Option<&ServiceRouteEntry> {
        self.0.first()
    }

    pub fn last(&self) -> Option<&ServiceRouteEntry> {
        self.0.last()
    }

    pub fn add(&mut self, entry: ServiceRouteEntry) -> &mut Self {
        self.0.push(entry);
        self
    }

    pub fn add_address(&mut self, address: Address) -> &mut Self {
        self.0.push(ServiceRouteEntry(address));
        self
    }

    pub fn add_uri(&mut self, uri: Uri) -> &mut Self {
        self.add_address(Address::new(uri))
    }

    pub fn iter(&self) -> impl Iterator<Item = &ServiceRouteEntry> {
        self.0.iter()
    }

    /// Returns the ordered list of URIs. Callers that need the raw route set
    /// for pre-loading outbound requests will use this.
    pub fn uris(&self) -> Vec<Uri> {
        self.0.iter().map(|e| e.0.uri.clone()).collect()
    }
}

impl fmt::Display for ServiceRoute {
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

impl FromStr for ServiceRoute {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        if s.is_empty() {
            return Ok(ServiceRoute::empty());
        }
        // Service-Route uses the same grammar as Route / Path (route-param list).
        let input_bytes = s.as_bytes();
        let parse_result = all_consuming(crate::parser::headers::parse_route)(input_bytes);

        match parse_result {
            Ok((_, route)) => Ok(ServiceRoute(route.0)),
            Err(_) => Err(Error::ParseError(format!("Failed to parse Service-Route header: {}", s)))
        }
    }
}

impl Deref for ServiceRoute {
    type Target = Vec<ServiceRouteEntry>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> IntoIterator for &'a ServiceRoute {
    type Item = &'a ServiceRouteEntry;
    type IntoIter = std::slice::Iter<'a, ServiceRouteEntry>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl From<Vec<ServiceRouteEntry>> for ServiceRoute {
    fn from(list: Vec<ServiceRouteEntry>) -> Self {
        ServiceRoute(list)
    }
}

impl From<ServiceRouteEntry> for ServiceRoute {
    fn from(entry: ServiceRouteEntry) -> Self {
        ServiceRoute(vec![entry])
    }
}

impl From<Address> for ServiceRoute {
    fn from(address: Address) -> Self {
        ServiceRoute::with_address(address)
    }
}

impl TypedHeaderTrait for ServiceRoute {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::ServiceRoute
    }

    fn to_header(&self) -> Header {
        let value_string = self.to_string();
        let value = crate::types::headers::HeaderValue::Raw(value_string.into_bytes());
        Header::new(Self::header_name(), value)
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
                    ServiceRoute::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            // Service-Route shares the Route grammar, so HeaderValue::Route is acceptable.
            HeaderValue::Route(entries) => {
                Ok(ServiceRoute(entries.clone()))
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
    fn test_service_route_empty() {
        let sr = ServiceRoute::empty();
        assert!(sr.is_empty());
        assert_eq!(sr.len(), 0);
        assert_eq!(sr.to_string(), "");
    }

    #[test]
    fn test_service_route_with_uri() {
        let uri = Uri::from_str("sip:orig-proxy.example.com;lr").unwrap();
        let sr = ServiceRoute::with_uri(uri);
        assert_eq!(sr.len(), 1);
        assert_eq!(sr.to_string(), "<sip:orig-proxy.example.com;lr>");
    }

    #[test]
    fn test_service_route_with_address() {
        let uri = Uri::from_str("sip:orig-proxy.example.com;lr").unwrap();
        let address = Address::new_with_display_name("Originating Proxy", uri);
        let sr = ServiceRoute::with_address(address);
        assert_eq!(sr.len(), 1);
        assert_eq!(
            sr.to_string(),
            "\"Originating Proxy\" <sip:orig-proxy.example.com;lr>"
        );
    }

    #[test]
    fn test_service_route_add_methods() {
        let mut sr = ServiceRoute::empty();

        let uri1 = Uri::from_str("sip:p1.example.com;lr").unwrap();
        sr.add_uri(uri1);
        assert_eq!(sr.len(), 1);

        let uri2 = Uri::from_str("sip:p2.example.com;lr").unwrap();
        let addr = Address::new_with_display_name("Second", uri2);
        sr.add_address(addr);
        assert_eq!(sr.len(), 2);

        let uri3 = Uri::from_str("sip:p3.example.com;lr").unwrap();
        sr.add(ServiceRouteEntry(Address::new(uri3)));
        assert_eq!(sr.len(), 3);
    }

    #[test]
    fn test_service_route_from_impls() {
        let uri = Uri::from_str("sip:p1.example.com;lr").unwrap();
        let entries = vec![ServiceRouteEntry(Address::new(uri))];
        let sr = ServiceRoute::from(entries);
        assert_eq!(sr.len(), 1);

        let uri2 = Uri::from_str("sip:p2.example.com;lr").unwrap();
        let entry = ServiceRouteEntry(Address::new(uri2));
        let sr = ServiceRoute::from(entry);
        assert_eq!(sr.len(), 1);

        let uri3 = Uri::from_str("sip:p3.example.com;lr").unwrap();
        let sr = ServiceRoute::from(Address::new(uri3));
        assert_eq!(sr.len(), 1);
    }

    #[test]
    fn test_service_route_fromstr_and_display() {
        let input = "<sip:orig1.example.com;lr>, <sip:orig2.example.com;lr>";
        let sr = ServiceRoute::from_str(input).unwrap();

        assert_eq!(sr.len(), 2);
        assert_eq!(sr[0].0.uri.to_string(), "sip:orig1.example.com;lr");
        assert_eq!(sr[1].0.uri.to_string(), "sip:orig2.example.com;lr");
        assert_eq!(sr.to_string(), input);
    }

    #[test]
    fn test_service_route_uris() {
        let mut sr = ServiceRoute::empty();
        sr.add_uri(Uri::from_str("sip:a.example.com;lr").unwrap());
        sr.add_uri(Uri::from_str("sip:b.example.com;lr").unwrap());

        let uris = sr.uris();
        assert_eq!(uris.len(), 2);
        assert_eq!(uris[0].to_string(), "sip:a.example.com;lr");
        assert_eq!(uris[1].to_string(), "sip:b.example.com;lr");
    }

    #[test]
    fn test_service_route_typed_header_trait() {
        assert_eq!(ServiceRoute::header_name(), HeaderName::ServiceRoute);

        let mut sr = ServiceRoute::empty();
        sr.add_uri(Uri::from_str("sip:a.example.com;lr").unwrap());
        sr.add_uri(Uri::from_str("sip:b.example.com;lr").unwrap());

        let header = sr.to_header();
        assert_eq!(header.name, HeaderName::ServiceRoute);

        let sr2 = ServiceRoute::from_header(&header).unwrap();
        assert_eq!(sr2.len(), 2);
        assert_eq!(sr2[0].0.uri.to_string(), "sip:a.example.com;lr");
        assert_eq!(sr2[1].0.uri.to_string(), "sip:b.example.com;lr");
    }

    #[test]
    fn test_service_route_empty_fromstr() {
        let sr = ServiceRoute::from_str("").unwrap();
        assert!(sr.is_empty());
    }

    #[test]
    fn test_service_route_header_name_roundtrip() {
        assert_eq!(HeaderName::ServiceRoute.as_str(), "Service-Route");
        assert_eq!(
            HeaderName::from_str("Service-Route").unwrap(),
            HeaderName::ServiceRoute
        );
        assert_eq!(
            HeaderName::from_str("service-route").unwrap(),
            HeaderName::ServiceRoute
        );
    }
}

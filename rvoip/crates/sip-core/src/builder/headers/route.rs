//! Route header builder
//!
//! This module provides builder methods for the Route header.

use crate::error::{Error, Result};
use crate::types::{
    address::Address,
    route::Route,
    uri::Uri,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use crate::parser::headers::route::RouteEntry;
use crate::{RequestBuilder, ResponseBuilder};
use super::HeaderSetter;

/// Extension trait that adds Route header building capabilities to request and response builders
pub trait RouteBuilderExt {
    /// Add a Route header with a URI
    fn route_uri(self, uri: Uri) -> Self;

    /// Add a Route header with an Address
    fn route_address(self, address: Address) -> Self;

    /// Add a Route header with a raw entry
    fn route_entry(self, entry: RouteEntry) -> Self;

    /// Add a Route header with multiple entries
    fn route_entries(self, entries: Vec<RouteEntry>) -> Self;
}

impl RouteBuilderExt for RequestBuilder {
    fn route_uri(self, uri: Uri) -> Self {
        let address = Address::new(uri);
        let entry = RouteEntry(address);
        self.route_entry(entry)
    }

    fn route_address(self, address: Address) -> Self {
        let entry = RouteEntry(address);
        self.route_entry(entry)
    }

    fn route_entry(self, entry: RouteEntry) -> Self {
        let route = Route::new(vec![entry]);
        self.header(TypedHeader::Route(route))
    }

    fn route_entries(self, entries: Vec<RouteEntry>) -> Self {
        let route = Route::new(entries);
        self.header(TypedHeader::Route(route))
    }
}

impl RouteBuilderExt for ResponseBuilder {
    fn route_uri(self, uri: Uri) -> Self {
        let address = Address::new(uri);
        let entry = RouteEntry(address);
        self.route_entry(entry)
    }

    fn route_address(self, address: Address) -> Self {
        let entry = RouteEntry(address);
        self.route_entry(entry)
    }

    fn route_entry(self, entry: RouteEntry) -> Self {
        let route = Route::new(vec![entry]);
        self.header(TypedHeader::Route(route))
    }

    fn route_entries(self, entries: Vec<RouteEntry>) -> Self {
        let route = Route::new(entries);
        self.header(TypedHeader::Route(route))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_request_route_uri() {
        let uri = Uri::from_str("sip:proxy.example.com").unwrap();
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .route_uri(uri.clone())
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Route(route)) = request.header(&HeaderName::Route) {
            assert_eq!(route.0.len(), 1);
            assert_eq!(route.0[0].0.uri, uri);
        } else {
            panic!("Route header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_route_address() {
        let uri = Uri::from_str("sip:proxy.example.com").unwrap();
        let address = Address::new(uri.clone());
        
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .route_address(address.clone())
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Route(route)) = response.header(&HeaderName::Route) {
            assert_eq!(route.0.len(), 1);
            assert_eq!(route.0[0].0.uri, uri);
        } else {
            panic!("Route header not found or has wrong type");
        }
    }

    #[test]
    fn test_route_entries() {
        let uri1 = Uri::from_str("sip:proxy1.example.com").unwrap();
        let uri2 = Uri::from_str("sip:proxy2.example.com").unwrap();
        
        let entry1 = RouteEntry(Address::new(uri1.clone()));
        let entry2 = RouteEntry(Address::new(uri2.clone()));
        
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .route_entries(vec![entry1, entry2])
            .build();
            
        if let Some(TypedHeader::Route(route)) = request.header(&HeaderName::Route) {
            assert_eq!(route.0.len(), 2);
            assert_eq!(route.0[0].0.uri, uri1);
            assert_eq!(route.0[1].0.uri, uri2);
        } else {
            panic!("Route header not found or has wrong type");
        }
    }
} 
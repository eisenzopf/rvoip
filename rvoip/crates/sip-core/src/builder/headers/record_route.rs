//! RecordRoute header builder
//!
//! This module provides builder methods for the Record-Route header.

use crate::error::{Error, Result};
use crate::types::{
    address::Address,
    record_route::{RecordRoute, RecordRouteEntry},
    uri::Uri,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;

/// Extension trait that adds Record-Route header building capabilities to request builders
pub trait RecordRouteBuilderExt {
    /// Add a Record-Route header with URI
    fn record_route_uri(self, uri: Uri) -> Self;

    /// Add a Record-Route header with Address
    fn record_route_address(self, address: Address) -> Self;

    /// Add a Record-Route header with a raw entry
    fn record_route_entry(self, entry: RecordRouteEntry) -> Self;

    /// Add a Record-Route header with multiple entries
    fn record_route_entries(self, entries: Vec<RecordRouteEntry>) -> Self;
}

impl<T> RecordRouteBuilderExt for T
where
    T: HeaderSetter,
{
    fn record_route_uri(self, uri: Uri) -> Self {
        let entry = RecordRouteEntry::new(Address::new(uri));
        self.record_route_entry(entry)
    }

    fn record_route_address(self, address: Address) -> Self {
        let entry = RecordRouteEntry::new(address);
        self.record_route_entry(entry)
    }

    fn record_route_entry(self, entry: RecordRouteEntry) -> Self {
        let record_route = RecordRoute::new(vec![entry]);
        self.set_header(record_route)
    }

    fn record_route_entries(self, entries: Vec<RecordRouteEntry>) -> Self {
        let record_route = RecordRoute::new(entries);
        self.set_header(record_route)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_request_record_route_uri() {
        let uri = Uri::from_str("sip:proxy.example.com").unwrap();
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .record_route_uri(uri.clone())
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        // Find the RecordRoute header
        let record_route = headers.iter()
            .filter_map(|h| match h {
                TypedHeader::RecordRoute(r) => Some(r),
                _ => None
            })
            .next();
        
        assert!(record_route.is_some(), "Record-Route header not found or has wrong type");
        let record_route = record_route.unwrap();
        assert_eq!(record_route.0.len(), 1);
        let entry_uri = record_route.0[0].uri();
        assert_eq!(entry_uri.to_string(), uri.to_string());
    }

    #[test]
    fn test_response_record_route_address() {
        let uri = Uri::from_str("sip:proxy.example.com").unwrap();
        let address = Address::new(uri.clone());
        
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .record_route_address(address.clone())
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        // Find the RecordRoute header
        let record_route = headers.iter()
            .filter_map(|h| match h {
                TypedHeader::RecordRoute(r) => Some(r),
                _ => None
            })
            .next();
        
        assert!(record_route.is_some(), "Record-Route header not found or has wrong type");
        let record_route = record_route.unwrap();
        assert_eq!(record_route.0.len(), 1);
        let entry_uri = record_route.0[0].uri();
        assert_eq!(entry_uri.to_string(), uri.to_string());
    }

    #[test]
    fn test_record_route_entries() {
        let uri1 = Uri::from_str("sip:proxy1.example.com").unwrap();
        let uri2 = Uri::from_str("sip:proxy2.example.com").unwrap();
        
        let entry1 = RecordRouteEntry::new(Address::new(uri1.clone()));
        let entry2 = RecordRouteEntry::new(Address::new(uri2.clone()));
        
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .record_route_entries(vec![entry1, entry2])
            .build();
        
        let headers = &request.headers;
        
        // Find the RecordRoute header
        let record_route = headers.iter()
            .filter_map(|h| match h {
                TypedHeader::RecordRoute(r) => Some(r),
                _ => None
            })
            .next();
        
        assert!(record_route.is_some(), "Record-Route header not found or has wrong type");
        let record_route = record_route.unwrap();
        assert_eq!(record_route.0.len(), 2);
        let entry1_uri = record_route.0[0].uri();
        let entry2_uri = record_route.0[1].uri();
        assert_eq!(entry1_uri.to_string(), uri1.to_string());
        assert_eq!(entry2_uri.to_string(), uri2.to_string());
    }
} 
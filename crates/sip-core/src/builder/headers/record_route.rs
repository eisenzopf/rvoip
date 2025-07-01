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

/// RecordRoute header builder
///
/// This module provides builder methods for the Record-Route header in SIP messages.
///
/// ## SIP Dialog Routing and Record-Route
///
/// As defined in [RFC 3261 Section 20.30](https://datatracker.ietf.org/doc/html/rfc3261#section-20.30),
/// the Record-Route header is inserted by proxies to ensure they remain in the signaling path
/// for all future requests within a dialog.
///
/// When a proxy inserts a Record-Route header, it's saying: "Please route all future requests
/// in this dialog through me."
///
/// ## How Record-Route Works
///
/// The Record-Route mechanism works as follows:
///
/// 1. Proxy inserts its URI in a Record-Route header in a request it forwards
/// 2. Additional proxies may insert their own Record-Route headers (each prepended to the list)
/// 3. The destination server copies all Record-Route headers into responses
/// 4. When the client receives the response, it stores the Record-Route headers as a route set
/// 5. For all future requests in the dialog, the client uses this route set as Route headers
///
/// ## Record-Route vs. Route
///
/// - **Record-Route**: Used by proxies to establish a route path for future requests
/// - **Route**: Used by endpoints to force requests through specific proxies, typically
///   constructed from Record-Route headers seen in previous responses
///
/// ## Common Use Cases
///
/// - **NAT traversal**: Keeping signaling flowing through edge proxies that maintain NAT bindings
/// - **Dialog state tracking**: Allowing proxies to maintain dialog state for services
/// - **Load balancing**: Ensuring subsequent requests in a dialog reach the same processing node
/// - **Security enforcement**: Keeping call control services in the signaling path
/// - **Forking control**: Maintaining control over forked requests
///
/// # Examples
///
/// ## Basic Proxy Record-Route Example
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::RecordRouteBuilderExt};
/// use std::str::FromStr;
///
/// // A proxy server receiving a request would add its address to Record-Route
/// // before forwarding, to stay in the path for future requests
/// 
/// // 1. Create proxy address with loose routing parameter
/// let mut proxy_uri = Uri::from_str("sip:proxy1.example.com").unwrap();
/// proxy_uri = proxy_uri.with_parameter(Param::Lr);
///
/// // 2. Add Record-Route to responses to establish a route set
/// // (This would normally be done when forwarding a request, but shown here on a response)
/// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
///     .to("Bob", "sip:bob@example.com", Some("a73kszlfl"))
///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
///     .cseq(1, Method::Invite)
///     .record_route_uri(proxy_uri)
///     .build();
///
/// // 3. Now the UAs will use this as a Route header in subsequent requests in this dialog
/// ```
///
/// ## Multi-Proxy Record-Route Chain
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::RecordRouteBuilderExt};
/// use std::str::FromStr;
///
/// // Scenario: An initial INVITE traverses multiple proxies, each adding a Record-Route.
/// // This example shows the final 200 OK response with all Record-Routes included.
///
/// // Create URIs for the proxy chain (each would be added by respective proxy)
/// let mut edge_proxy = Uri::from_str("sip:edge.example.com").unwrap()
///     .with_parameter(Param::Lr)
///     .with_parameter(Param::Transport("tcp".to_string()));
///
/// let mut app_proxy = Uri::from_str("sip:app.example.com").unwrap()
///     .with_parameter(Param::Lr);
///
/// let mut lb_proxy = Uri::from_str("sip:lb42.example.com").unwrap()
///     .with_parameter(Param::Lr);
///
/// // The entry order in Record-Route is important:
/// // - First proxy to handle request is placed last in the list
/// // - Most recent proxy is placed first
/// 
/// // Create response with Record-Route headers
/// // The order reflects the reverse of the request path: lb_proxy → app_proxy → edge_proxy
/// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
///     .to("Bob", "sip:bob@example.com", Some("a73kszlfl"))
///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
///     .cseq(1, Method::Invite)
///     // Add Record-Route entries in the order they were added in the request
///     // (lb_proxy was the last to process, so it comes first)
///     .record_route_uri(lb_proxy)      // Added last to request, first in response
///     .record_route_uri(app_proxy)     // Added second to request 
///     .record_route_uri(edge_proxy)    // Added first to request, last in response
///     .build();
///
/// // When client (Alice) receives this response, it saves the route set as:
/// // Route: <sip:lb42.example.com;lr>, <sip:app.example.com;lr>, <sip:edge.example.com;transport=tcp;lr>
/// // Future requests in this dialog will be sent to lb42.example.com first
///
/// // When server (Bob) sends a request in this dialog, its route set will be the reverse:
/// // Route: <sip:edge.example.com;transport=tcp;lr>, <sip:app.example.com;lr>, <sip:lb42.example.com;lr>
/// // Bob's requests will be sent to edge.example.com first
/// ```
pub trait RecordRouteBuilderExt {
    /// Add a Record-Route header with URI
    ///
    /// This method adds a Record-Route header with a single URI. The URI typically 
    /// identifies a proxy server that should remain in the signaling path for all future
    /// requests in the dialog.
    ///
    /// # Parameters
    ///
    /// * `uri` - The URI to add as a Record-Route entry
    ///
    /// # Returns
    ///
    /// The builder for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::RecordRouteBuilderExt};
    /// use std::str::FromStr;
    ///
    /// // Create a URI with the loose routing parameter
    /// let mut proxy_uri = Uri::from_str("sip:proxy.example.com").unwrap();
    /// proxy_uri = proxy_uri.with_parameter(Param::Lr);
    ///
    /// // Add the URI as a Record-Route
    /// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .record_route_uri(proxy_uri)
    ///     .build();
    /// ```
    fn record_route_uri(self, uri: Uri) -> Self;

    /// Add a Record-Route header with Address
    ///
    /// This method adds a Record-Route header with a single Address, which includes a URI
    /// and potentially a display name. This is useful when adding routing information that
    /// should preserve display names.
    ///
    /// # Parameters
    ///
    /// * `address` - The Address containing the URI and optional display name
    ///
    /// # Returns
    ///
    /// The builder for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::RecordRouteBuilderExt};
    /// use std::str::FromStr;
    ///
    /// // Create an Address with display name and URI
    /// let mut uri = Uri::from_str("sip:edge.example.com").unwrap();
    /// uri = uri.with_parameter(Param::Lr);
    /// 
    /// let mut address = Address::new(uri);
    /// address.display_name = Some("Edge Proxy".to_string());
    ///
    /// // Add the record route with the address
    /// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .record_route_address(address)
    ///     .build();
    /// ```
    fn record_route_address(self, address: Address) -> Self;

    /// Add a Record-Route header with a raw entry
    ///
    /// This method adds a Record-Route header with a single RecordRouteEntry, which is the
    /// internal representation of a Record-Route element. This is typically used for more 
    /// advanced scenarios when you have a pre-constructed RecordRouteEntry.
    ///
    /// # Parameters
    ///
    /// * `entry` - The RecordRouteEntry to add
    ///
    /// # Returns
    ///
    /// The builder for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::RecordRouteBuilderExt};
    /// use std::str::FromStr;
    ///
    /// // Create a RecordRouteEntry manually
    /// let uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
    /// let address = Address::new(uri);
    /// let entry = RecordRouteEntry::new(address);
    ///
    /// // Add the record route with the entry
    /// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .record_route_entry(entry)
    ///     .build();
    /// ```
    fn record_route_entry(self, entry: RecordRouteEntry) -> Self;

    /// Add a Record-Route header with multiple entries
    ///
    /// This method adds a Record-Route header with multiple entries. This is typically used
    /// when a response reflects multiple Record-Route headers that were added by different
    /// proxies in the path of the original request.
    ///
    /// # Parameters
    ///
    /// * `entries` - A vector of RecordRouteEntry objects
    ///
    /// # Returns
    ///
    /// The builder for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::RecordRouteBuilderExt};
    /// use std::str::FromStr;
    ///
    /// // Create multiple record route entries
    /// let uri1 = Uri::from_str("sip:proxy1.example.com;lr").unwrap();
    /// let uri2 = Uri::from_str("sip:proxy2.example.com;lr").unwrap();
    ///
    /// let entry1 = RecordRouteEntry::new(Address::new(uri1));
    /// let entry2 = RecordRouteEntry::new(Address::new(uri2));
    ///
    /// // Add all record routes at once
    /// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .record_route_entries(vec![entry1, entry2])
    ///     .build();
    /// ```
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
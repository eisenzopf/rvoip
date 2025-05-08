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

/// Route header builder
///
/// This module provides builder methods for the Route header in SIP messages.
/// 
/// ## SIP Routing Overview
/// 
/// The Route header is a crucial part of SIP message routing as defined in [RFC 3261 Section 20.30](https://datatracker.ietf.org/doc/html/rfc3261#section-20.30).
/// It contains a list of URIs that represent proxies the request must traverse to reach its destination.
/// 
/// When a client has a route set (a set of Route headers), it will:
/// 1. Place those Route headers into the request
/// 2. Set the Request-URI to the target URI (typically the final destination)
/// 3. Send the request to the first URI in the Route set
/// 
/// ## Route Header vs. Record-Route
/// 
/// The Route and Record-Route headers work together to enable SIP dialog routing:
/// 
/// - **Record-Route**: Added by proxies to responses to establish the route path for future requests
/// - **Route**: Used in requests to force them through specific proxies, often constructed from
///   Record-Route headers seen in previous responses
/// 
/// ## Loose Routing vs. Strict Routing
/// 
/// RFC 3261 mandates support for "loose routing" which expects Route headers with the `lr` parameter:
/// 
/// - **Loose routing**: The proxy forwards to the next hop in the Route set without changing the Request-URI
/// - **Strict routing**: (Legacy) The proxy places the Request-URI in a new Route header and replaces the Request-URI with the next route
/// 
/// Modern SIP implementations should use loose routing (indicated by the `lr` parameter in Route URIs).
/// 
/// ## Common Use Cases
/// 
/// - Directing requests through specific proxies
/// - Maintaining dialog routing paths across multiple requests
/// - Implementing service chaining (traversing multiple services in sequence)
/// - NAT traversal by routing through edge proxies
/// - Load balancing across multiple servers
///
/// # Examples
///
/// ## Basic Dialog Routing Flow
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::RouteBuilderExt};
/// use std::str::FromStr;
///
/// // Step 1: In a dialog, the client constructs a Request using Route headers
/// // derived from Record-Route headers received in previous responses
/// let proxy1 = Uri::from_str("sip:proxy1.example.com;lr").unwrap();
/// let proxy2 = Uri::from_str("sip:proxy2.example.com;lr").unwrap();
///
/// // Client creates a request with the Route set to route the request
/// // through the two proxies before reaching the final destination
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
///     // Add routes in order, first Route is where we send the message first
///     .route_uri(proxy1)    // First hop will be proxy1
///     .route_uri(proxy2)    // Second hop will be proxy2
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Bob", "sip:bob@example.com", None)
///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@alice")
///     .build();
///
/// // What happens next:
/// // 1. Client sends to proxy1.example.com (first Route URI)
/// // 2. Proxy1 removes itself from Route, forwards to proxy2.example.com
/// // 3. Proxy2 removes itself from Route, forwards to bob@example.com (Request-URI)
/// ```
///
/// ## Service Chaining
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::RouteBuilderExt};
/// use std::str::FromStr;
///
/// // Create a route through a chain of specialized services
/// let account_service = Uri::from_str("sip:accounting.example.com;lr").unwrap();
/// let transcoder = Uri::from_str("sip:transcode.example.com;lr").unwrap();
/// let firewall = Uri::from_str("sip:firewall.example.com;lr").unwrap();
///
/// // Create a request that must traverse specialized services in a specific order
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:conference@example.com").unwrap()
///     // Services will be visited in this order
///     .route_uri(account_service)  // First service tracks call accounting
///     .route_uri(transcoder)       // Second service handles media transcoding
///     .route_uri(firewall)         // Third service enforces security policy
///     .from("Alice", "sip:alice@example.com", Some("tag=1234"))
///     .to("Conference", "sip:conference@example.com", None)
///     .build();
/// ```
///
/// ## Routing with Named Parameters
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::RouteBuilderExt};
/// use std::str::FromStr;
///
/// // Create a route URI with parameters
/// let mut edge_proxy = Uri::from_str("sip:edge.example.com").unwrap();
/// 
/// // Add parameters using with_parameter method
/// edge_proxy = edge_proxy.with_parameter(Param::Lr); // Loose routing parameter
/// edge_proxy = edge_proxy.with_parameter(Param::Transport("tcp".to_string())); // TCP transport
/// edge_proxy = edge_proxy.with_parameter(Param::Other("x-session-id".to_string(), Some("abcdef123".into()))); // Custom param
///
/// // Build the request with the parameterized Route
/// let request = SimpleRequestBuilder::new(Method::Subscribe, "sip:presence@example.com").unwrap()
///     .route_uri(edge_proxy)
///     .from("Alice", "sip:alice@example.com", Some("tag=abc123"))
///     .to("Presence", "sip:presence@example.com", None)
///     .build();
/// ```
pub trait RouteBuilderExt {
    /// Add a Route header with a URI
    ///
    /// This method adds a Route header with a single URI. The URI typically identifies a SIP proxy
    /// that should be included in the path of the request.
    ///
    /// # Parameters
    ///
    /// * `uri` - The URI of the proxy to route through
    ///
    /// # Returns
    ///
    /// The builder for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::RouteBuilderExt};
    /// use std::str::FromStr;
    ///
    /// // Create a request that routes through a specific proxy
    /// let proxy = Uri::from_str("sip:proxy.example.com;lr").unwrap();
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .route_uri(proxy)
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .build();
    /// ```
    fn route_uri(self, uri: Uri) -> Self;

    /// Add a Route header with an Address
    ///
    /// This method adds a Route header with a single Address, which includes a URI and potentially
    /// a display name. This is useful when adding routing information that should preserve 
    /// display names.
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
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::RouteBuilderExt};
    /// use std::str::FromStr;
    ///
    /// // Create an Address with display name and URI
    /// let uri = Uri::from_str("sip:edge-proxy.example.com;lr").unwrap();
    /// let mut address = Address::new(uri);
    /// address.display_name = Some("East Coast Edge Proxy".to_string());
    ///
    /// // Add the route with the address
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .route_address(address)
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .build();
    /// ```
    fn route_address(self, address: Address) -> Self;

    /// Add a Route header with a raw RouteEntry
    ///
    /// This method adds a Route header with a single RouteEntry, which is the internal
    /// representation of a Route element. This is typically used for more advanced scenarios
    /// when you have a pre-constructed RouteEntry.
    ///
    /// # Parameters
    ///
    /// * `entry` - The RouteEntry to add
    ///
    /// # Returns
    ///
    /// The builder for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::RouteBuilderExt};
    /// use rvoip_sip_core::parser::headers::route::RouteEntry;
    /// use std::str::FromStr;
    ///
    /// // Create a RouteEntry manually
    /// let uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
    /// let address = Address::new(uri);
    /// let entry = RouteEntry(address);
    ///
    /// // Add the route with the entry
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .route_entry(entry)
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .build();
    /// ```
    fn route_entry(self, entry: RouteEntry) -> Self;

    /// Add a Route header with multiple entries
    ///
    /// This method adds a Route header with multiple entries, representing a complete route set.
    /// The entries will be visited in the order provided (first entry is the first hop).
    ///
    /// # Parameters
    ///
    /// * `entries` - A vector of RouteEntry objects representing the complete route set
    ///
    /// # Returns
    ///
    /// The builder for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::RouteBuilderExt};
    /// use rvoip_sip_core::parser::headers::route::RouteEntry;
    /// use std::str::FromStr;
    ///
    /// // Create multiple route entries
    /// let uri1 = Uri::from_str("sip:proxy1.example.com;lr").unwrap();
    /// let uri2 = Uri::from_str("sip:proxy2.example.com;lr").unwrap();
    ///
    /// let entry1 = RouteEntry(Address::new(uri1));
    /// let entry2 = RouteEntry(Address::new(uri2));
    ///
    /// // Add all routes at once
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .route_entries(vec![entry1, entry2])
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .build();
    ///
    /// // The request will be sent to proxy1, then proxy2, then finally to bob@example.com
    /// ```
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
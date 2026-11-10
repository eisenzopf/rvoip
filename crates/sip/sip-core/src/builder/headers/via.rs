use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::str::FromStr;

use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::error::{Error, Result};
use crate::types::uri::Host;
use crate::types::via::{SentProtocol, Via, ViaHeader};
use crate::types::{Param, TypedHeader};

/// Extension trait for adding Via headers to SIP message builders.
///
/// This trait provides a standard way to add Via headers to both request and response builders
/// as specified in [RFC 3261 Section 20.42](https://datatracker.ietf.org/doc/html/rfc3261#section-20.42).
///
/// ## Purpose and Importance
///
/// The Via header serves several critical functions in SIP:
///
/// - **Response Routing**: Enables responses to follow the same path as requests in reverse
/// - **Loop Detection**: Prevents messages from being processed more than once
/// - **Branch Identification**: Uniquely identifies transactions through the branch parameter
/// - **Protocol Information**: Specifies the transport protocol used at each hop (UDP, TCP, TLS, etc.)
/// - **NAT Traversal**: Assists with routing through NATs using the received and rport parameters
///
/// Each SIP element (UAC, UAS, proxy) involved in processing a request adds its own Via header
/// to the top of the list. When generating a response, all Via headers must be preserved in
/// the same order, and the response is routed by removing the topmost Via at each hop.
///
/// ## Branch Parameter
///
/// The branch parameter must be globally unique for each transaction and must start with the
/// magic cookie "z9hG4bK" for requests conforming to RFC 3261. This ensures transaction identification
/// across distributed systems.
///
/// ## Via Structure
///
/// A typical Via header has the format:
/// ```text
/// Via: SIP/2.0/UDP proxy.example.com:5060;branch=z9hG4bK776asdhds;received=192.0.2.1;rport=5060
/// ```
///
/// Which includes:
/// - Protocol version (SIP/2.0)
/// - Transport protocol (UDP)
/// - Host (proxy.example.com)
/// - Port (5060)
/// - Branch parameter (z9hG4bK776asdhds)
/// - Optional parameters (received, rport, etc.)
///
/// # Examples
///
/// ## Basic Via Header
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ViaBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
///     .via("192.168.1.1:5060", "UDP", Some("z9hG4bK776asdhds"))
///     .build();
/// ```
///
/// ## Multiple Via Headers (Request Through Proxies)
///
/// When a SIP message traverses multiple proxies, each one adds a Via header.
/// The topmost (first) Via header represents the most recent hop.
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ViaBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Simulate a request that went through two proxies
/// let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
///     // First Via (most recent, added by last proxy)
///     .via("proxy2.example.com:5060", "UDP", Some("z9hG4bK33792"))
///     // Second Via (added by first proxy)
///     .via("proxy1.example.com:5060", "UDP", Some("z9hG4bK123e5"))
///     // Third Via (original client)
///     .via("192.168.1.1:5060", "UDP", Some("z9hG4bK776asdhds"))
///     .build();
/// ```
///
/// ## Complete SIP Dialog with Via Headers
///
/// This example shows how Via headers are used throughout a complete SIP dialog,
/// including how they're preserved in responses:
///
/// ```rust
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
/// use rvoip_sip_core::builder::headers::{ViaBuilderExt, FromBuilderExt, ToBuilderExt, CallIdBuilderExt};
/// use rvoip_sip_core::builder::headers::cseq::CSeqBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // 1. Initial INVITE from UAC through a proxy to UAS
/// let invite = SimpleRequestBuilder::invite("sip:bob@biloxi.example.com").unwrap()
///     .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
///     .to("Bob", "sip:bob@biloxi.example.com", None)
///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@atlanta.example.com")
///     .cseq(1)
///     // Proxy adds its Via first (top)
///     .via("proxy.example.com:5060", "UDP", Some("z9hG4bK123a4b"))
///     // UAC's original Via comes next
///     .via("client.atlanta.example.com:5060", "UDP", Some("z9hG4bK74bf9"))
///     .build();
///
/// // 2. Response from UAS preserves both Via headers in same order
/// // The response will naturally be routed first to the proxy, then to the UAC
/// let response = SimpleResponseBuilder::ok()
///     .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
///     .to("Bob", "sip:bob@biloxi.example.com", Some("b84b23"))  // Response adds a To tag
///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@atlanta.example.com")
///     .cseq_with_method(1, Method::Invite)
///     // Same Via headers in same order as received in the request
///     .via("proxy.example.com:5060", "UDP", Some("z9hG4bK123a4b"))
///     .via("client.atlanta.example.com:5060", "UDP", Some("z9hG4bK74bf9"))
///     .build();
/// ```
///
/// ## Via Headers with NAT Traversal Support
///
/// This example demonstrates how received and rport parameters help with NAT traversal:
///
/// ```rust
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
/// use rvoip_sip_core::types::{Method, TypedHeader, Param};
/// use rvoip_sip_core::types::via::Via;
///
/// // 1. Client sends request indicating support for rport
/// let initial_request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap();
///
/// // Client's Via with empty rport parameter (requesting NAT handling)
/// let mut params = Vec::new();
/// params.push(Param::branch("z9hG4bK776asdhds"));
/// params.push(Param::new("rport", None::<String>)); // Empty rport parameter
///
/// let client_via = Via::new("SIP", "2.0", "UDP", "client.example.com", Some(5060), params).unwrap();
/// let request = initial_request.header(TypedHeader::Via(client_via)).build();
///
/// // 2. When server/proxy receives this request, it would see the client's IP and port
/// // and add them to the Via header before forwarding or responding
///
/// // 3. Server/proxy would generate a response like this:
/// let response = SimpleResponseBuilder::ok();
///
/// // Server adds received and rport parameters to help NAT traversal
/// let mut params = Vec::new();
/// params.push(Param::branch("z9hG4bK776asdhds"));
/// params.push(Param::new("rport", Some("12345"))); // The actual source port
/// params.push(Param::new("received", Some("203.0.113.1"))); // The actual source IP
///
/// let server_via = Via::new("SIP", "2.0", "UDP", "client.example.com", Some(5060), params).unwrap();
/// let response_with_via = response.header(TypedHeader::Via(server_via));
///
/// // Now the response will be routed to 203.0.113.1:12345 instead of client.example.com:5060
/// ```
///
/// ## Via Headers with Additional Parameters
///
/// Via headers can contain additional parameters like 'ttl' for multicast, 'maddr', etc.:
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::types::{Method, TypedHeader, Param};
/// use rvoip_sip_core::types::via::Via;
///
/// // Create a request with Via header containing special parameters
/// let mut request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap();
///
/// // Create Via header with multiple parameters
/// let mut params = Vec::new();
/// params.push(Param::branch("z9hG4bK776asdhds"));
/// params.push(Param::new("ttl", Some("16")));          // Time-to-live for multicast
/// params.push(Param::new("maddr", Some("224.2.0.1"))); // Multicast address
/// params.push(Param::new("hidden", None::<String>));   // Flag parameter (no value)
///
/// let via = Via::new("SIP", "2.0", "UDP", "proxy.example.com", Some(5060), params).unwrap();
/// let final_request = request.header(TypedHeader::Via(via)).build();
/// ```
///
/// ## Stateful Proxy Via Handling
///
/// This example shows how a stateful proxy handles Via headers:
///
/// ```rust
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
/// use rvoip_sip_core::builder::headers::ViaBuilderExt;
/// use rvoip_sip_core::types::{Method, TypedHeader, Param};
/// use rvoip_sip_core::types::via::Via;
///
/// // 1. Incoming request to the proxy with the UAC's Via header
/// let incoming_request = SimpleRequestBuilder::invite("sip:bob@biloxi.example.com").unwrap()
///     .via("client.atlanta.example.com:5060", "UDP", Some("z9hG4bK74bf9"))
///     .build();
///
/// // 2. Proxy adds its own Via header and forwards request
/// let outgoing_request = SimpleRequestBuilder::invite("sip:bob@biloxi.example.com").unwrap()
///     // Proxy adds its Via first (will be top/first Via header)
///     .via("proxy.example.com:5060", "UDP", Some("z9hG4bK123a4b"))
///     // Original UAC's Via follows
///     .via("client.atlanta.example.com:5060", "UDP", Some("z9hG4bK74bf9"))
///     .build();
///
/// // 3. Proxy receives response, processes top Via header (its own),
/// // then forwards response with that Via removed
/// // (In a real implementation, the proxy would extract Via headers from the response)
/// let incoming_response = SimpleResponseBuilder::ok()
///     // Response with both Via headers
///     .via("proxy.example.com:5060", "UDP", Some("z9hG4bK123a4b"))
///     .via("client.atlanta.example.com:5060", "UDP", Some("z9hG4bK74bf9"))
///     .build();
///
/// // 4. Proxy forwards response with its Via header removed
/// let outgoing_response = SimpleResponseBuilder::ok()
///     // Only the original UAC's Via remains
///     .via("client.atlanta.example.com:5060", "UDP", Some("z9hG4bK74bf9"))
///     .build();
/// ```
pub trait ViaBuilderExt {
    /// Add a Via header with optional branch parameter
    ///
    /// Creates and adds a Via header as specified in [RFC 3261 Section 20.42](https://datatracker.ietf.org/doc/html/rfc3261#section-20.42).
    /// The Via header indicates the path taken by the request so far and helps route responses back.
    ///
    /// # Parameters
    /// - `host`: The sent-by, `host [":" port]` (e.g., "192.168.1.1", "example.com:5060",
    ///   `"[2001:db8::1]:5060"` or a bare "2001:db8::1"). An IPv6 host followed by a port must
    ///   be bracketed (RFC 3261 §25.1 `IPv6reference`, RFC 5118 §4.1), which is the form
    ///   `SocketAddr` displays.
    /// - `transport`: The transport protocol (UDP, TCP, TLS, etc.)
    /// - `branch`: Optional branch parameter (should be prefixed with z9hG4bK per RFC 3261)
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ## Basic Via Header
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ViaBuilderExt;
    ///
    /// let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    ///     .via("192.168.1.1:5060", "UDP", Some("z9hG4bK776asdhds"))
    ///     .build();
    /// ```
    ///
    /// ## For a Response
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::builder::headers::ViaBuilderExt;
    /// use rvoip_sip_core::types::{Method, StatusCode};
    /// use rvoip_sip_core::builder::headers::cseq::CSeqBuilderExt;
    ///
    /// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .via("proxy.example.com:5060", "UDP", Some("z9hG4bK123a4b"))
    ///     .cseq_with_method(1, Method::Invite)
    ///     .build();
    /// ```
    ///
    /// # Note
    ///
    /// The builder API is infallible, so a `host` that is not a valid sent-by is logged and
    /// no Via is added. Use [`via_socket_addr`](Self::via_socket_addr) when the sent-by is a
    /// socket address; it cannot fail.
    ///
    /// For more complex Via headers with additional parameters (received, rport, ttl, etc.),
    /// you should use the TypedHeader approach with Via constructor directly, as shown in
    /// the trait documentation examples.
    fn via(self, host: &str, transport: &str, branch: Option<&str>) -> Self;

    /// Add a Via header whose sent-by is a socket address.
    ///
    /// This is the infallible form of [`via`](Self::via) for the common case
    /// of advertising a bound or configured transport address. The address is
    /// written as `sent-by = host [ COLON port ]` (RFC 3261 §20.42, §25.1):
    /// an IPv6 address becomes an `IPv6reference` (`[2001:db8::1]:5060`,
    /// RFC 5118 §4.1). The SIP grammar has no zone-identifier syntax, so the
    /// scope id of a link-local `SocketAddrV6` is not representable and is
    /// not written; configurations that need one are rejected before this
    /// point.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ViaBuilderExt;
    ///
    /// let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    ///     .via_socket_addr("[2001:db8::1]:5060".parse().unwrap(), "UDP", Some("z9hG4bK776asdhds"))
    ///     .build();
    /// assert!(request.to_string().contains("Via: SIP/2.0/UDP [2001:db8::1]:5060;branch=z9hG4bK776asdhds"));
    /// ```
    fn via_socket_addr(self, addr: SocketAddr, transport: &str, branch: Option<&str>) -> Self;
}

/// Parse the `sent-by = host [ COLON port ]` text accepted by
/// [`ViaBuilderExt::via`] (RFC 3261 §20.42, §25.1).
///
/// An IPv6 host is accepted both as an `IPv6reference` (`[2001:db8::1]`,
/// optionally followed by `:port`, the form `SocketAddr` displays) and as a
/// bare `IPv6address` without a port. A bare IPv6 address cannot carry a
/// port: its colons make the split ambiguous, which is why RFC 3261 requires
/// the bracketed form whenever a port follows (RFC 5118 §4.1).
pub(crate) fn parse_sent_by(text: &str) -> Result<(Host, Option<u16>)> {
    let invalid = || Error::InvalidUri(format!("Invalid Via sent-by: {text}"));
    let parse_port = |port: &str| port.parse::<u16>().map_err(|_| invalid());

    if let Some(reference) = text.strip_prefix('[') {
        let (address, rest) = reference.split_once(']').ok_or_else(invalid)?;
        let address = address.parse::<Ipv6Addr>().map_err(|_| invalid())?;
        let port = match rest {
            "" => None,
            _ => Some(parse_port(rest.strip_prefix(':').ok_or_else(invalid)?)?),
        };
        return Ok((Host::Address(IpAddr::V6(address)), port));
    }

    if let Ok(address) = text.parse::<Ipv6Addr>() {
        return Ok((Host::Address(IpAddr::V6(address)), None));
    }

    match text.split_once(':') {
        Some((host, port)) => Ok((Host::from_str(host)?, Some(parse_port(port)?))),
        None => Ok((Host::from_str(text)?, None)),
    }
}

fn via_header(host: Host, port: Option<u16>, transport: &str, branch: Option<&str>) -> TypedHeader {
    let params = branch.map(Param::branch).into_iter().collect();
    TypedHeader::Via(Via(vec![ViaHeader {
        sent_protocol: SentProtocol {
            name: "SIP".to_string(),
            version: "2.0".to_string(),
            transport: transport.to_string(),
        },
        sent_by_host: host,
        sent_by_port: port,
        params,
    }]))
}

/// Build the Via for [`ViaBuilderExt::via`], or `None` (with a warning) when
/// `host` is not a valid sent-by. The builder API is infallible, so an
/// invalid sent-by cannot be returned as an error; it is logged rather than
/// dropped silently. Callers holding a socket address should use
/// [`ViaBuilderExt::via_socket_addr`], which cannot fail.
fn via_from_text(host: &str, transport: &str, branch: Option<&str>) -> Option<TypedHeader> {
    match parse_sent_by(host) {
        Ok((host, port)) => Some(via_header(host, port, transport, branch)),
        Err(error) => {
            tracing::warn!(%error, "Via header not added: invalid sent-by");
            None
        }
    }
}

impl ViaBuilderExt for SimpleRequestBuilder {
    fn via(self, host: &str, transport: &str, branch: Option<&str>) -> Self {
        match via_from_text(host, transport, branch) {
            Some(via) => self.header(via),
            None => self,
        }
    }

    fn via_socket_addr(self, addr: SocketAddr, transport: &str, branch: Option<&str>) -> Self {
        self.header(via_header(
            Host::Address(addr.ip()),
            Some(addr.port()),
            transport,
            branch,
        ))
    }
}

impl ViaBuilderExt for SimpleResponseBuilder {
    fn via(self, host: &str, transport: &str, branch: Option<&str>) -> Self {
        match via_from_text(host, transport, branch) {
            Some(via) => self.header(via),
            None => self,
        }
    }

    fn via_socket_addr(self, addr: SocketAddr, transport: &str, branch: Option<&str>) -> Self {
        self.header(via_header(
            Host::Address(addr.ip()),
            Some(addr.port()),
            transport,
            branch,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Method;

    #[test]
    fn test_request_via_header() {
        // Test with hostname and port
        let request = SimpleRequestBuilder::invite("sip:bob@example.com")
            .unwrap()
            .via("example.com:5060", "UDP", Some("z9hG4bK776asdhds"))
            .build();

        let via_headers = request
            .all_headers()
            .iter()
            .filter_map(|h| {
                if let TypedHeader::Via(v) = h {
                    Some(v)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        assert_eq!(via_headers.len(), 1);
        let header = &via_headers[0].headers()[0]; // Get first ViaHeader
        assert_eq!(header.host().to_string(), "example.com");
        assert_eq!(header.port(), Some(5060));
        assert_eq!(header.transport(), "UDP");
        assert_eq!(header.branch(), Some("z9hG4bK776asdhds"));

        // Test with hostname only
        let request = SimpleRequestBuilder::invite("sip:bob@example.com")
            .unwrap()
            .via("example.com", "TCP", None)
            .build();

        let via_headers = request
            .all_headers()
            .iter()
            .filter_map(|h| {
                if let TypedHeader::Via(v) = h {
                    Some(v)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        assert_eq!(via_headers.len(), 1);
        let header = &via_headers[0].headers()[0]; // Get first ViaHeader
        assert_eq!(header.host().to_string(), "example.com");
        assert_eq!(header.port(), None);
        assert_eq!(header.transport(), "TCP");
        assert_eq!(header.branch(), None);

        // An invalid port is not a sent-by (RFC 3261 §25.1: `port = 1*DIGIT`,
        // and `example.com:invalid` is not a hostname), so no Via is added
        // rather than one whose host is the whole text.
        let request = SimpleRequestBuilder::invite("sip:bob@example.com")
            .unwrap()
            .via("example.com:invalid", "UDP", Some("z9hG4bK776asdhds"))
            .build();

        assert!(!request
            .all_headers()
            .iter()
            .any(|h| matches!(h, TypedHeader::Via(_))));
    }

    #[test]
    fn test_response_via_header() {
        // Test with hostname and port
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .call_id("test-call-id")
            .cseq(1, Method::Invite)
            .via("example.com:5060", "UDP", Some("z9hG4bK776asdhds"))
            .build();

        let via_headers = response
            .all_headers()
            .iter()
            .filter_map(|h| {
                if let TypedHeader::Via(v) = h {
                    Some(v)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        assert_eq!(via_headers.len(), 1);
        let header = &via_headers[0].headers()[0]; // Get first ViaHeader
        assert_eq!(header.host().to_string(), "example.com");
        assert_eq!(header.port(), Some(5060));
        assert_eq!(header.transport(), "UDP");
        assert_eq!(header.branch(), Some("z9hG4bK776asdhds"));

        // Test with IP address
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .call_id("test-call-id")
            .cseq(1, Method::Invite)
            .via("192.168.1.1", "TCP", None)
            .build();

        let via_headers = response
            .all_headers()
            .iter()
            .filter_map(|h| {
                if let TypedHeader::Via(v) = h {
                    Some(v)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        assert_eq!(via_headers.len(), 1);
        let header = &via_headers[0].headers()[0]; // Get first ViaHeader
        assert_eq!(header.host().to_string(), "192.168.1.1");
        assert_eq!(header.port(), None);
        assert_eq!(header.transport(), "TCP");
        assert_eq!(header.branch(), None);
    }

    fn only_via(request: &crate::types::sip_request::Request) -> ViaHeader {
        let vias: Vec<_> = request
            .all_headers()
            .iter()
            .filter_map(|h| match h {
                TypedHeader::Via(v) => Some(v.headers().to_vec()),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(vias.len(), 1, "exactly one Via expected");
        vias[0].clone()
    }

    fn invite_with_via(sent_by: &str) -> crate::types::sip_request::Request {
        SimpleRequestBuilder::invite("sip:bob@example.com")
            .unwrap()
            .via(sent_by, "UDP", Some("z9hG4bK776asdhds"))
            .build()
    }

    #[test]
    fn via_accepts_bracketed_ipv6_with_port() {
        // `SocketAddr` displays an IPv6 address as an IPv6reference with
        // a port; the builder must not drop that Via (RFC 5118 §4.1).
        let via = only_via(&invite_with_via("[2001:db8::1]:5060"));
        assert_eq!(
            via.sent_by_host,
            Host::Address("2001:db8::1".parse().unwrap())
        );
        assert_eq!(via.sent_by_port, Some(5060));
        assert_eq!(
            via.to_string(),
            "SIP/2.0/UDP [2001:db8::1]:5060;branch=z9hG4bK776asdhds"
        );
    }

    #[test]
    fn via_accepts_ipv6_without_port() {
        for sent_by in ["[2001:db8::1]", "2001:db8::1"] {
            let via = only_via(&invite_with_via(sent_by));
            assert_eq!(
                via.sent_by_host,
                Host::Address("2001:db8::1".parse().unwrap())
            );
            assert_eq!(via.sent_by_port, None, "{sent_by}");
            assert_eq!(
                via.to_string(),
                "SIP/2.0/UDP [2001:db8::1];branch=z9hG4bK776asdhds"
            );
        }
    }

    #[test]
    fn via_accepts_ipv4_and_domain_sent_by() {
        let via = only_via(&invite_with_via("192.0.2.1:5070"));
        assert_eq!(
            via.sent_by_host,
            Host::Address("192.0.2.1".parse().unwrap())
        );
        assert_eq!(via.sent_by_port, Some(5070));

        let via = only_via(&invite_with_via("proxy.example.com"));
        assert_eq!(via.sent_by_host, Host::domain("proxy.example.com"));
        assert_eq!(via.sent_by_port, None);
    }

    #[test]
    fn via_rejects_malformed_sent_by() {
        for sent_by in [
            "[2001:db8::1",
            "[2001:db8::1]5060",
            "[2001:db8::1]:port",
            "[192.0.2.1]:5060",
            "[fe80::1%eth0]:5060",
            "example.com:99999",
        ] {
            assert!(parse_sent_by(sent_by).is_err(), "{sent_by}");
        }
    }

    #[test]
    fn via_socket_addr_writes_sent_by_for_both_families() {
        for (addr, expected) in [
            (
                "192.0.2.1:5060",
                "SIP/2.0/TCP 192.0.2.1:5060;branch=z9hG4bK1",
            ),
            ("[::1]:5062", "SIP/2.0/TCP [::1]:5062;branch=z9hG4bK1"),
        ] {
            let request = SimpleRequestBuilder::invite("sip:bob@example.com")
                .unwrap()
                .via_socket_addr(addr.parse().unwrap(), "TCP", Some("z9hG4bK1"))
                .build();
            assert_eq!(only_via(&request).to_string(), expected);
        }

        let response = SimpleResponseBuilder::ok()
            .via_socket_addr("[2001:db8::7]:5060".parse().unwrap(), "UDP", None)
            .build();
        assert!(response
            .to_string()
            .contains("Via: SIP/2.0/UDP [2001:db8::7]:5060\r\n"));
    }
}

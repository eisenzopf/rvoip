//! Asynchronous authorization seam at the SIP transaction ingress boundary.
//!
//! The transaction layer deliberately does not implement credential parsing.
//! An upper layer may install a [`SipRequestIngressAuthorizer`] that evaluates
//! a new request after its server transaction exists but before the request is
//! published to the transaction user. This ordering lets a rejected
//! transaction cache and retransmit its challenge without exposing the request
//! to dialog or application code.

use async_trait::async_trait;
use rvoip_core_traits::identity::AuthenticatedPrincipal;
use rvoip_sip_core::{Request, StatusCode, TypedHeader};
use rvoip_sip_transport::transport::{
    TransportConnectionMetadata, TransportFlowId, TransportRoute, TransportType,
};
use std::fmt;
use std::net::{IpAddr, SocketAddr};

/// Transport-truth input supplied to an ingress authorizer.
#[derive(Clone)]
pub struct SipRequestIngressContext {
    /// Remote socket address that sent the request.
    pub source: SocketAddr,
    /// Local socket address that received the request.
    pub destination: SocketAddr,
    /// Concrete receiving transport.
    pub transport_type: TransportType,
    /// Exact connection-oriented flow that carried the request.
    pub flow_id: Option<TransportFlowId>,
    /// Identity produced by the transport after client-certificate
    /// verification.
    ///
    /// This must only be populated by the transport boundary after successful
    /// client-certificate verification. A SIP header, URI, or source address
    /// is never sufficient to populate this field.
    pub connection_metadata: Option<TransportConnectionMetadata>,
}

impl fmt::Debug for SipRequestIngressContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SipRequestIngressContext")
            .field("source_address_family", &address_family(self.source))
            .field("source_port", &self.source.port())
            .field(
                "destination_address_family",
                &address_family(self.destination),
            )
            .field("destination_port", &self.destination.port())
            .field("transport_type", &self.transport_type)
            .field("flow_id", &self.flow_id)
            .field(
                "connection_metadata_present",
                &self.connection_metadata.is_some(),
            )
            .finish()
    }
}

const fn address_family(address: SocketAddr) -> &'static str {
    if address.is_ipv4() {
        "ipv4"
    } else {
        "ipv6"
    }
}

impl SipRequestIngressContext {
    /// Build ingress context without a transport-authenticated peer.
    pub fn new(source: SocketAddr, destination: SocketAddr, transport_type: TransportType) -> Self {
        Self {
            source,
            destination,
            transport_type,
            flow_id: None,
            connection_metadata: None,
        }
    }

    /// Attach transport-verified peer identity metadata.
    ///
    /// Callers must derive this value from the completed TLS/WSS handshake,
    /// never from SIP message contents.
    pub fn with_connection_metadata(mut self, metadata: TransportConnectionMetadata) -> Self {
        self.connection_metadata = Some(metadata);
        self
    }

    /// Attach the exact connection-oriented ingress flow.
    pub fn with_flow_id(mut self, flow_id: TransportFlowId) -> Self {
        self.flow_id = Some(flow_id);
        self
    }

    /// Build the response route back to the concrete ingress flow.
    pub fn response_route(&self) -> TransportRoute {
        let mut route = TransportRoute::new(self.source).with_transport_type(self.transport_type);
        route.flow_id = self.flow_id;
        route
    }

    /// Derive the RFC 3261 §18.2.2 / RFC 3581 response destination for a
    /// concrete inbound request.
    ///
    /// [`Self::response_route`] deliberately remains the exact transport peer
    /// and flow identity used by authorization, replay matching, and
    /// connection-oriented responses. UDP response routing is different:
    ///
    /// - `maddr` selects the destination address and the Via sent-by port;
    /// - `rport` opts into the packet source address and source port;
    /// - without `rport`, the packet source address is paired with the Via
    ///   sent-by port (or the SIP UDP default, 5060).
    ///
    /// A domain-valued `maddr` needs asynchronous RFC 3263 resolution, which
    /// is outside this synchronous transaction-ingress helper. In that case
    /// the transport-truth source address is retained instead of inventing a
    /// resolved destination.
    pub fn response_route_for_request(&self, request: &Request) -> TransportRoute {
        let mut route = self.response_route();
        if self.transport_type != TransportType::Udp || self.flow_id.is_some() {
            return route;
        }

        let vias = request.via_headers();
        let Some(via) = vias.first() else {
            return route;
        };
        let Some(top_via) = via.headers().first() else {
            return route;
        };
        let sent_by_port = top_via.port().unwrap_or(5060);

        if let Some(maddr) = top_via.maddr().and_then(parse_via_ip_literal) {
            route.destination = SocketAddr::new(maddr, sent_by_port);
            return route;
        }

        if via.rport().is_some() {
            return route;
        }

        route.destination = SocketAddr::new(self.source.ip(), sent_by_port);
        route
    }
}

fn parse_via_ip_literal(value: &str) -> Option<IpAddr> {
    value
        .parse()
        .ok()
        .or_else(|| value.strip_prefix('[')?.strip_suffix(']')?.parse().ok())
}

/// A denial response sent by the transaction layer without TU dispatch.
#[derive(Clone)]
pub struct SipRequestRejection {
    /// Final SIP status returned to the peer.
    pub status: StatusCode,
    /// Additional response headers, such as `WWW-Authenticate`.
    pub headers: Vec<TypedHeader>,
    /// Credential-free diagnostic detail. This is never sent on the wire.
    pub reason: Option<String>,
}

impl fmt::Debug for SipRequestRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SipRequestRejection")
            .field("status", &self.status)
            .field("header_count", &self.headers.len())
            .field("has_reason", &self.reason.is_some())
            .finish()
    }
}

impl SipRequestRejection {
    /// Build a rejection with no additional headers.
    pub fn new(status: StatusCode) -> Self {
        Self {
            status,
            headers: Vec::new(),
            reason: None,
        }
    }

    /// Append a response header.
    pub fn with_header(mut self, header: TypedHeader) -> Self {
        self.headers.push(header);
        self
    }

    /// Add credential-free local diagnostic detail.
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

/// Result returned by [`SipRequestIngressAuthorizer`].
#[derive(Clone)]
pub enum SipRequestAuthorization {
    /// The request may proceed to the transaction user under this principal.
    Authorized {
        /// Canonical identity that owns the accepted request.
        principal: AuthenticatedPrincipal,
    },
    /// The request must be answered locally and not dispatched upward.
    Rejected(SipRequestRejection),
}

impl fmt::Debug for SipRequestAuthorization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Authorized { .. } => f.write_str("Authorized { principal: <redacted> }"),
            Self::Rejected(rejection) => f.debug_tuple("Rejected").field(rejection).finish(),
        }
    }
}

/// Policy hook for new inbound SIP transactions.
#[async_trait]
pub trait SipRequestIngressAuthorizer: Send + Sync + fmt::Debug {
    /// Authorize one newly created inbound request transaction.
    async fn authorize(
        &self,
        request: &Request,
        context: &SipRequestIngressContext,
    ) -> SipRequestAuthorization;
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    use rvoip_sip_core::types::param::Param;
    use rvoip_sip_core::{Method, Via};
    use rvoip_sip_transport::transport::TlsPeerIdentity;

    fn request_with_via(port: Option<u16>, params: Vec<Param>) -> Request {
        Request::new(
            Method::Options,
            "sip:service@example.test".parse().expect("valid SIP URI"),
        )
        .with_header(TypedHeader::Via(
            Via::new("SIP", "2.0", "UDP", "192.0.2.10", port, params).expect("valid Via"),
        ))
    }

    #[test]
    fn ingress_context_debug_redacts_addresses_and_tls_fingerprint() {
        const FINGERPRINT_CANARY: &str = "ingress-tls-fingerprint-secret-canary";
        let context = SipRequestIngressContext::new(
            "192.0.2.44:5061".parse().unwrap(),
            "198.51.100.22:5061".parse().unwrap(),
            TransportType::Tls,
        )
        .with_connection_metadata(TransportConnectionMetadata {
            tls_peer_identity: TlsPeerIdentity {
                leaf_certificate_sha256: FINGERPRINT_CANARY.into(),
                presented_chain_len: 1,
            },
        });
        let rendered = format!("{context:?}");
        assert!(!rendered.contains("192.0.2.44"));
        assert!(!rendered.contains("198.51.100.22"));
        assert!(!rendered.contains(FINGERPRINT_CANARY));
        assert!(rendered.contains("source_address_family: \"ipv4\""));
        assert!(rendered.contains("connection_metadata_present: true"));
    }

    #[test]
    fn udp_without_rport_uses_source_ip_and_via_sent_by_port() {
        let context = SipRequestIngressContext::new(
            "203.0.113.9:62000".parse().unwrap(),
            "198.51.100.22:5060".parse().unwrap(),
            TransportType::Udp,
        );
        let request = request_with_via(Some(5070), vec![Param::branch("z9hG4bK-no-rport")]);

        assert_eq!(
            context.response_route_for_request(&request).destination,
            "203.0.113.9:5070".parse().unwrap()
        );
        assert_eq!(
            context.response_route().destination,
            "203.0.113.9:62000".parse().unwrap(),
            "peer identity must remain the exact ingress tuple"
        );
    }

    #[test]
    fn udp_without_rport_defaults_to_sip_port() {
        let context = SipRequestIngressContext::new(
            "203.0.113.9:62000".parse().unwrap(),
            "198.51.100.22:5060".parse().unwrap(),
            TransportType::Udp,
        );
        let request = request_with_via(None, vec![Param::branch("z9hG4bK-default-port")]);

        assert_eq!(
            context.response_route_for_request(&request).destination,
            "203.0.113.9:5060".parse().unwrap()
        );
    }

    #[test]
    fn udp_rport_uses_exact_packet_source() {
        let context = SipRequestIngressContext::new(
            "203.0.113.9:62000".parse().unwrap(),
            "198.51.100.22:5060".parse().unwrap(),
            TransportType::Udp,
        );
        let request = request_with_via(
            Some(5070),
            vec![Param::branch("z9hG4bK-rport"), Param::Rport(None)],
        );

        assert_eq!(
            context.response_route_for_request(&request).destination,
            "203.0.113.9:62000".parse().unwrap()
        );
    }

    #[test]
    fn udp_ip_maddr_takes_response_precedence() {
        let context = SipRequestIngressContext::new(
            "203.0.113.9:62000".parse().unwrap(),
            "198.51.100.22:5060".parse().unwrap(),
            TransportType::Udp,
        );
        let request = request_with_via(
            Some(5070),
            vec![
                Param::branch("z9hG4bK-maddr"),
                Param::Maddr("192.0.2.77".into()),
                Param::Rport(None),
            ],
        );

        assert_eq!(
            context.response_route_for_request(&request).destination,
            "192.0.2.77:5070".parse().unwrap()
        );
    }

    #[test]
    fn reliable_transport_response_keeps_exact_ingress_route() {
        let context = SipRequestIngressContext::new(
            "203.0.113.9:62000".parse().unwrap(),
            "198.51.100.22:5060".parse().unwrap(),
            TransportType::Tcp,
        );
        let request = request_with_via(Some(5070), vec![Param::branch("z9hG4bK-reliable")]);

        assert_eq!(
            context.response_route_for_request(&request),
            context.response_route()
        );
    }
}

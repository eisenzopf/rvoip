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
use rvoip_sip_transport::transport::{TransportConnectionMetadata, TransportType};
use std::fmt;
use std::net::SocketAddr;

/// Transport-truth input supplied to an ingress authorizer.
#[derive(Clone, Debug)]
pub struct SipRequestIngressContext {
    /// Remote socket address that sent the request.
    pub source: SocketAddr,
    /// Local socket address that received the request.
    pub destination: SocketAddr,
    /// Concrete receiving transport.
    pub transport_type: TransportType,
    /// Identity produced by the transport after client-certificate
    /// verification.
    ///
    /// This must only be populated by the transport boundary after successful
    /// client-certificate verification. A SIP header, URI, or source address
    /// is never sufficient to populate this field.
    pub connection_metadata: Option<TransportConnectionMetadata>,
}

impl SipRequestIngressContext {
    /// Build ingress context without a transport-authenticated peer.
    pub fn new(source: SocketAddr, destination: SocketAddr, transport_type: TransportType) -> Self {
        Self {
            source,
            destination,
            transport_type,
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

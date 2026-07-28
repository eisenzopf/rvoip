//! RFC 3263 SIP URI → transport+SocketAddr resolution.
//!
//! Two surfaces live here:
//!
//! 1. The pluggable [`Resolver`] trait — applications inject an
//!    `Arc<dyn Resolver>` into the dialog layer to control how URIs are
//!    walked into next-hop candidates.
//! 2. Pure algorithm helpers ([`srv`]) that any resolver impl can lean
//!    on, plus the URI-flavour classifier ([`select_transport_for_uri`]).
//!
//! The reference implementation [`hickory::HickoryResolver`] (behind the
//! `dns` cargo feature) walks the full RFC 3263 §4 ladder
//! NAPTR → SRV → A/AAAA, with the §4.2 short-circuits for IP literals
//! and explicit ports.
//!
//! Callers that already have a pre-resolved `SocketAddr` (the
//! transport-manager and proxy paths) **do not** need a `Resolver` — the
//! transport trait still takes `SocketAddr` directly. The resolver is
//! consulted upstream, at the URI→candidate boundary.

use std::net::SocketAddr;
use std::time::Instant;

use async_trait::async_trait;
use rvoip_sip_core::Uri;
use thiserror::Error;

use crate::transport::{TransportAuthority, TransportType};

pub mod srv;

#[cfg(feature = "dns")]
pub mod hickory;

#[cfg(feature = "dns")]
pub use hickory::HickoryResolver;

/// One resolved next-hop candidate produced by walking RFC 3263 §4.
///
/// Callers iterate candidates in the returned order, trying the next one
/// on transport-level failure per RFC 3263 §4.3. `expires` carries the
/// hickory-reported TTL deadline for caches that want to refresh
/// proactively; IP literals (no DNS involved) leave it `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTarget {
    pub addr: SocketAddr,
    pub transport: TransportType,
    /// Authority authenticated for this candidate. DNS expansion preserves
    /// the input SIP authority instead of reconstructing it from the IP/SRV
    /// result.
    pub authority: Option<TransportAuthority>,
    pub expires: Option<Instant>,
}

impl ResolvedTarget {
    /// Convenience: candidate from a hard-coded address (e.g. IP literal
    /// short-circuit). `expires` is `None`.
    pub fn immediate(addr: SocketAddr, transport: TransportType) -> Self {
        Self {
            addr,
            transport,
            // A resolved socket does not by itself identify the logical DNS
            // authority that TLS/WSS must authenticate. Callers that know the
            // authority attach it explicitly with `with_authority`; otherwise
            // the request's top Route/Request-URI authority is retained.
            authority: None,
            expires: None,
        }
    }

    pub fn with_authority(mut self, authority: TransportAuthority) -> Self {
        self.authority = Some(authority);
        self
    }
}

/// Errors surfaced from a [`Resolver::resolve`] call.
///
/// `Dns` covers transient lookup failures. `Forbidden` is raised for
/// URI shapes RFC 3263 explicitly rejects (e.g. `sips:` with
/// `;transport=udp`). `NoCandidates` is a hard failure of the ladder —
/// all NAPTR/SRV/A paths produced nothing usable.
#[derive(Debug, Error)]
pub enum ResolverError {
    #[error("DNS lookup failed: {0}")]
    Dns(String),
    #[error("URI forbidden by RFC 3263: {0}")]
    Forbidden(&'static str),
    #[error("No candidates after NAPTR/SRV/A ladder")]
    NoCandidates,
    /// The request URI has no routable host (e.g. `sip:600` — a bare
    /// extension addressed without a domain). Surfaced *before* DNS so the
    /// caller sees a routing hint instead of a cryptic NAPTR/SRV/A failure.
    #[error("{0}")]
    InvalidHost(String),
}

impl From<ResolverError> for crate::error::Error {
    fn from(value: ResolverError) -> Self {
        match value {
            ResolverError::Dns(msg) => crate::error::Error::DnsResolutionFailed(msg),
            ResolverError::Forbidden(reason) => {
                crate::error::Error::UnsupportedTransport(reason.to_string())
            }
            ResolverError::NoCandidates => {
                crate::error::Error::DnsResolutionFailed("no candidates".to_string())
            }
            ResolverError::InvalidHost(msg) => crate::error::Error::InvalidUri(msg),
        }
    }
}

/// Pluggable URI resolver — applications inject implementations
/// at `DialogManager::set_resolver` to override the default
/// [`HickoryResolver`].
///
/// Implementations MUST be cancel-safe under tokio: a resolver returned
/// to the caller while the caller's future is dropped should leave no
/// background work running indefinitely. Hickory's `TokioAsyncResolver`
/// honours this.
#[async_trait]
pub trait Resolver: Send + Sync {
    async fn resolve(&self, uri: &Uri) -> Result<Vec<ResolvedTarget>, ResolverError>;
}

/// Select transport flavour from a SIP URI per RFC 3261 §8.1.2 / §18.1.1
/// (`;transport=` URI parameter, RFC 3261 §19.1.5) and §26.2 (`sips:`
/// requires TLS-capable transport).
///
/// The `sips:` security requirement always dominates transport hints:
/// `tcp` means TLS-over-TCP, `tls` remains TLS, and `wss` remains WSS.
/// Insecure `udp`/`ws` hints are classified to TLS here so syntax-only
/// callers never downgrade; resolvers and send paths reject those invalid
/// combinations explicitly.
///
/// This is a pure-syntax classifier — no DNS lookups, no I/O. Lives in
/// this crate so resolvers and the dialog-layer multiplexer can share a
/// single source of truth.
pub fn select_transport_for_uri(uri: &Uri) -> TransportType {
    use rvoip_sip_core::types::uri::Scheme;

    if matches!(uri.scheme(), Scheme::Sips) {
        return match uri
            .transport()
            .map(|transport| transport.to_ascii_lowercase())
            .as_deref()
        {
            Some("wss") => TransportType::Wss,
            _ => TransportType::Tls,
        };
    }

    if let Some(transport_param) = uri.transport() {
        match transport_param.to_ascii_lowercase().as_str() {
            "udp" => return TransportType::Udp,
            "tcp" => return TransportType::Tcp,
            "tls" => return TransportType::Tls,
            "ws" => return TransportType::Ws,
            "wss" => return TransportType::Wss,
            _ => {
                // Unknown transport tag — fall through to scheme.
            }
        }
    }

    match uri.scheme() {
        Scheme::Sips => TransportType::Tls,
        _ => TransportType::Udp,
    }
}

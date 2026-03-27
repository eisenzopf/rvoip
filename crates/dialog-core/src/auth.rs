//! Server-side SIP digest authentication.
//!
//! Provides an [`AuthProvider`] trait that dialog-core calls to verify
//! credentials on REGISTER and INVITE requests.

use std::net::SocketAddr;
use rvoip_sip_core::Request;

/// Result of credential verification.
#[derive(Debug, Clone)]
pub enum AuthResult {
    /// Credentials valid — proceed with request.
    Authenticated {
        /// The authenticated username.
        username: String,
    },
    /// No credentials or invalid — challenge the client.
    Challenge,
    /// Skip authentication for this request.
    Skip,
}

/// Pluggable authentication provider.
///
/// Implement this trait and attach it to `DialogServer` via
/// `set_auth_provider()`. Dialog-core will call [`AuthProvider::check_request`]
/// for every REGISTER and INVITE received.
#[async_trait::async_trait]
pub trait AuthProvider: Send + Sync + 'static {
    /// Verify credentials in the request.
    ///
    /// * Return [`AuthResult::Authenticated`] if the Authorization header is present and valid.
    /// * Return [`AuthResult::Challenge`] to send a 401/407 response with a fresh nonce.
    /// * Return [`AuthResult::Skip`] to bypass authentication entirely.
    async fn check_request(&self, request: &Request, source: SocketAddr) -> AuthResult;

    /// The SIP realm used in WWW-Authenticate challenges.
    fn realm(&self) -> &str;
}

/// Generate a cryptographically random nonce for digest challenges.
pub fn generate_nonce() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let rand_val: u64 = rand::random();
    format!("{ts:x}{rand_val:x}")
}

/// No-op auth provider that skips all authentication (for testing/demo).
pub struct NoopAuthProvider;

#[async_trait::async_trait]
impl AuthProvider for NoopAuthProvider {
    async fn check_request(&self, _request: &Request, _source: SocketAddr) -> AuthResult {
        AuthResult::Skip
    }

    fn realm(&self) -> &str {
        "rvoip"
    }
}

/// Routing decision returned by a [`ProxyRouter`].
#[derive(Debug, Clone)]
pub enum ProxyAction {
    /// Forward the request to a specific destination address.
    Forward {
        /// The socket address to forward the request to.
        destination: SocketAddr,
    },
    /// Let the local B2BUA handle it (default behaviour).
    LocalB2BUA,
    /// Reject the request with the given SIP status code and reason phrase.
    Reject {
        /// SIP status code (e.g. 403, 480).
        status: u16,
        /// Human-readable reason phrase.
        reason: String,
    },
}

/// Pluggable proxy routing policy.
///
/// Implement this trait and attach it to `DialogManager` via
/// `set_proxy_router()`.  Dialog-core will call [`ProxyRouter::route_request`]
/// for every initial INVITE received, before falling through to session-core.
#[async_trait::async_trait]
pub trait ProxyRouter: Send + Sync + 'static {
    /// Decide how to handle an incoming SIP request.
    async fn route_request(&self, request: &Request, source: SocketAddr) -> ProxyAction;
}

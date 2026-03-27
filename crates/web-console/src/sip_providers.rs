//! Database-backed SIP AuthProvider and ProxyRouter implementations.
//!
//! These providers query PostgreSQL on every request so configuration changes
//! (add/remove credentials, trunks, routes) take effect immediately without
//! restarting the server.
//!
//! # Tables
//!
//! `sip_credentials` — SIP Digest usernames and passwords:
//! ```sql
//! CREATE TABLE IF NOT EXISTS sip_credentials (
//!     id TEXT PRIMARY KEY,
//!     username TEXT NOT NULL UNIQUE,
//!     password TEXT NOT NULL,
//!     realm TEXT NOT NULL DEFAULT 'rvoip',
//!     enabled BOOLEAN NOT NULL DEFAULT true,
//!     created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
//! );
//! ```
//!
//! `sip_trunks` (managed by `api/trunks.rs`) has `host`, `port`, and optional
//! `routing_prefix`.  Calls whose Request-URI user part starts with that prefix
//! are forwarded to the trunk.

use std::net::SocketAddr;
use std::sync::Arc;

use rvoip_call_engine::database::sqlx::{self, Row};
use rvoip_call_engine::database::DatabaseManager;
use rvoip_dialog_core::auth::{AuthProvider, AuthResult, ProxyAction, ProxyRouter};
use rvoip_sip_core::{
    Request,
    types::auth::{Algorithm, Credentials, DigestParam, Qop},
    types::headers::HeaderName,
};

// ── DbAuthProvider ────────────────────────────────────────────────────────────

/// Verifies SIP Digest credentials against the `sip_credentials` PostgreSQL table.
pub struct DbAuthProvider {
    db: Arc<DatabaseManager>,
    realm: String,
}

impl DbAuthProvider {
    pub fn new(db: Arc<DatabaseManager>, realm: impl Into<String>) -> Arc<Self> {
        Arc::new(Self { db, realm: realm.into() })
    }

    /// Ensure the `sip_credentials` table exists.
    pub async fn init_table(db: &DatabaseManager) -> Result<(), sqlx::Error> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS sip_credentials (\
                id TEXT PRIMARY KEY, \
                username TEXT NOT NULL UNIQUE, \
                password TEXT NOT NULL, \
                realm TEXT NOT NULL DEFAULT 'rvoip', \
                enabled BOOLEAN NOT NULL DEFAULT true, \
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
            )",
        )
        .execute(db.pool())
        .await?;
        Ok(())
    }

    /// Look up plain-text password for `username` (enabled only).
    async fn fetch_password(&self, username: &str) -> Option<String> {
        let row = sqlx::query(
            "SELECT password FROM sip_credentials WHERE username = $1 AND enabled = true",
        )
        .bind(username)
        .fetch_optional(self.db.pool())
        .await
        .ok()??;

        row.try_get::<String, _>("password").ok()
    }
}

#[async_trait::async_trait]
impl AuthProvider for DbAuthProvider {
    async fn check_request(&self, request: &Request, _source: SocketAddr) -> AuthResult {
        // Prefer Proxy-Authorization for INVITE (RFC 3261 §22.2), fall back to Authorization.
        let credentials = request
            .typed_header::<rvoip_sip_core::types::auth::ProxyAuthorization>()
            .map(|h| &h.0)
            .or_else(|| {
                request
                    .typed_header::<rvoip_sip_core::types::auth::Authorization>()
                    .map(|h| &h.0)
            });

        let params = match credentials {
            Some(Credentials::Digest { params }) => params,
            _ => return AuthResult::Challenge,
        };

        // Extract digest fields.
        let mut username = None::<&str>;
        let mut realm = None::<&str>;
        let mut nonce = None::<&str>;
        let mut uri_str = None::<String>;
        let mut response = None::<&str>;
        let mut algorithm = Algorithm::Md5;
        let mut msg_qop = None::<&Qop>;
        let mut nc: u32 = 0;
        let mut cnonce = "";

        for param in params {
            match param {
                DigestParam::Username(v) => username = Some(v.as_str()),
                DigestParam::Realm(v) => realm = Some(v.as_str()),
                DigestParam::Nonce(v) => nonce = Some(v.as_str()),
                DigestParam::Uri(v) => uri_str = Some(v.to_string()),
                DigestParam::Response(v) => response = Some(v.as_str()),
                DigestParam::Algorithm(a) => algorithm = a.clone(),
                DigestParam::MsgQop(q) => msg_qop = Some(q),
                DigestParam::NonceCount(n) => nc = *n,
                DigestParam::Cnonce(c) => cnonce = c.as_str(),
                _ => {}
            }
        }

        let (Some(username), Some(_realm), Some(nonce), Some(uri_str), Some(response)) =
            (username, realm, nonce, uri_str, response)
        else {
            return AuthResult::Challenge;
        };

        // Fetch stored password.
        let password = match self.fetch_password(username).await {
            Some(p) => p,
            None => return AuthResult::Challenge,
        };

        // Compute expected digest response.
        let method = request.method().to_string();
        let expected = match rvoip_sip_core::auth::compute_digest_response(
            username,
            &password,
            &self.realm,
            nonce,
            &method,
            &uri_str,
            msg_qop,
            nc,
            cnonce,
            &algorithm,
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("digest compute error: {e}");
                return AuthResult::Challenge;
            }
        };

        if expected == response {
            AuthResult::Authenticated { username: username.to_string() }
        } else {
            tracing::debug!("digest mismatch for user '{username}'");
            AuthResult::Challenge
        }
    }

    fn realm(&self) -> &str {
        &self.realm
    }
}

// ── DbProxyRouter ─────────────────────────────────────────────────────────────

/// Routes INVITEs to SIP trunks by matching the Request-URI user part against
/// `sip_trunks.routing_prefix`.  Falls back to `LocalB2BUA` when no trunk matches.
pub struct DbProxyRouter {
    db: Arc<DatabaseManager>,
}

impl DbProxyRouter {
    pub fn new(db: Arc<DatabaseManager>) -> Arc<Self> {
        Arc::new(Self { db })
    }

    /// Add `routing_prefix` column to `sip_trunks` if it doesn't already exist.
    pub async fn init_schema(db: &DatabaseManager) -> Result<(), sqlx::Error> {
        sqlx::query(
            "ALTER TABLE sip_trunks ADD COLUMN IF NOT EXISTS routing_prefix TEXT",
        )
        .execute(db.pool())
        .await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl ProxyRouter for DbProxyRouter {
    async fn route_request(&self, request: &Request, _source: SocketAddr) -> ProxyAction {
        // Extract called number from Request-URI user part.
        let called = request
            .uri()
            .username()
            .map(|u| u.to_string())
            .unwrap_or_default();

        if called.is_empty() {
            return ProxyAction::LocalB2BUA;
        }

        // Find the longest matching routing_prefix in sip_trunks.
        let rows = sqlx::query(
            "SELECT host, port, routing_prefix \
             FROM sip_trunks \
             WHERE routing_prefix IS NOT NULL AND status = 'active' \
             ORDER BY LENGTH(routing_prefix) DESC",
        )
        .fetch_all(self.db.pool())
        .await;

        let rows = match rows {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("sip_trunks query failed: {e}");
                return ProxyAction::LocalB2BUA;
            }
        };

        for row in &rows {
            let prefix: String = match row.try_get("routing_prefix") {
                Ok(p) => p,
                Err(_) => continue,
            };
            if called.starts_with(&prefix) {
                let host: String = match row.try_get("host") {
                    Ok(h) => h,
                    Err(_) => continue,
                };
                let port: i32 = row.try_get("port").unwrap_or(5060);
                let addr_str = format!("{host}:{port}");
                match addr_str.parse::<SocketAddr>() {
                    Ok(dest) => {
                        tracing::debug!("routing {} via trunk {}", called, dest);
                        return ProxyAction::Forward { destination: dest };
                    }
                    Err(e) => {
                        tracing::warn!("invalid trunk addr '{addr_str}': {e}");
                        continue;
                    }
                }
            }
        }

        ProxyAction::LocalB2BUA
    }
}

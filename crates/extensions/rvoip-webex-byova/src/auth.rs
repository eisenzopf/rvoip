//! Webex BYODS JWS authentication.
//!
//! Webex sends a signed token with every BYOVA connection: in the WebSocket
//! upgrade's `Authorization: Bearer <token>` header and in the gRPC
//! `authorization` metadata of every call. [`WebexJwsValidator`] verifies it
//! against signing keys the host supplies and binds it to one configured
//! organization and data source registration. It never fetches a key from a
//! URL found in a token: the host fetches the JWKS from its configured, trusted
//! Webex identity broker (for example
//! `https://idbroker.webex.com/idb/oauth2/v2/keys/verificationjwk`) and hands
//! it over with [`WebexJwsValidator::new`] or
//! [`WebexJwsValidator::replace_keys`].
//!
//! # What is verified
//!
//! - the signature, with a key selected by the header `kid`, using only the
//!   configured algorithms (RS256 by default);
//! - `iss` is one of the allowlisted issuers, `aud` matches, and `sub` matches
//!   when a subject is configured;
//! - `exp` (required), `nbf` when present, and `iat` (required, not in the
//!   future) within the configured leeway, and a total lifetime of at most
//!   [`WebexJwsConfig::max_token_lifetime`] (24 hours by default);
//! - `jti` equals one of the registration's current nonces (compared in
//!   constant time). The nonce is a registration value shared by concurrent
//!   calls, not a single-use token, so it is not replay-tracked;
//! - the organization, data source URL and schema ID claims equal the
//!   configured registration for the transport that received the token;
//! - the key set is younger than [`WebexJwsConfig::keys_max_age`] (one hour by
//!   default). Stale keys fail closed until the host replaces them.
//!
//! # Assumptions
//!
//! Cisco's public BYOVA pages state that the token is a JWS verified with the
//! identity broker's `verificationjwk` key set and that the data source URL
//! and schema claims must match the registration. They do not publish the
//! claim names for the organization, URL and schema. The defaults in
//! [`WebexClaimNames`] (`com.cisco.user.orgid`, `com.cisco.datasource.url`,
//! `com.cisco.datasource.schema.uuid`) are assumptions to confirm against a
//! live token; override them if a real token differs. RS256 is likewise
//! assumed, and the allowed algorithms are configurable.

use crate::{Result, WebexError};
use chrono::{DateTime, TimeZone, Utc};
use jsonwebtoken::{jwk::JwkSet, Algorithm, DecodingKey, Validation};
use parking_lot::RwLock;
use serde_json::{Map, Value};
use std::{fmt, time::Duration};
use subtle::ConstantTimeEq;
use tokio::time::Instant;

/// Cisco's Voice Virtual Agent WebSocket schema ID.
pub const WEBSOCKET_SCHEMA_ID: &str = "a38a10b7-43e4-4676-a076-a7d6dce9387d";
/// Cisco's Voice Virtual Agent gRPC schema ID.
pub const GRPC_SCHEMA_ID: &str = "5397013b-7920-4ffc-807c-e8a3e0a18f43";

const MAX_TOKEN_BYTES: usize = 16 * 1024;

/// The wire variant that received a token. Each variant has its own data
/// source registration (schema ID, URL and nonce).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebexTransportKind {
    WebSocket,
    Grpc,
}

/// One BYODS data source registration.
#[derive(Clone)]
pub struct WebexRegistration {
    pub schema_id: String,
    pub datasource_url: String,
    /// Current registration nonces. Webex places the nonce in `jti`. Keep the
    /// previous nonce here during a bounded rotation overlap.
    pub nonces: Vec<String>,
}

impl WebexRegistration {
    pub fn websocket(datasource_url: impl Into<String>, nonce: impl Into<String>) -> Self {
        Self {
            schema_id: WEBSOCKET_SCHEMA_ID.into(),
            datasource_url: datasource_url.into(),
            nonces: vec![nonce.into()],
        }
    }

    pub fn grpc(datasource_url: impl Into<String>, nonce: impl Into<String>) -> Self {
        Self {
            schema_id: GRPC_SCHEMA_ID.into(),
            datasource_url: datasource_url.into(),
            nonces: vec![nonce.into()],
        }
    }

    fn validate(&self) -> Result<()> {
        if self.schema_id.is_empty()
            || self.datasource_url.is_empty()
            || self.nonces.is_empty()
            || self.nonces.iter().any(|nonce| nonce.len() < 8)
        {
            return Err(WebexError::Configuration);
        }
        Ok(())
    }
}

impl fmt::Debug for WebexRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebexRegistration")
            .field("schema_id", &self.schema_id)
            .field("nonce_count", &self.nonces.len())
            .finish_non_exhaustive()
    }
}

/// Claim names for values Cisco has not published. See the module
/// documentation: the defaults are assumptions.
#[derive(Clone, Debug)]
pub struct WebexClaimNames {
    pub organization_id: String,
    pub datasource_url: String,
    pub schema_id: String,
}

impl Default for WebexClaimNames {
    fn default() -> Self {
        Self {
            organization_id: "com.cisco.user.orgid".into(),
            datasource_url: "com.cisco.datasource.url".into(),
            schema_id: "com.cisco.datasource.schema.uuid".into(),
        }
    }
}

/// Validation policy for [`WebexJwsValidator`].
#[derive(Clone)]
pub struct WebexJwsConfig {
    /// Allowlisted issuers, for example `https://idbroker-b-us.webex.com/idb`.
    pub issuers: Vec<String>,
    pub audience: String,
    /// Required `sub`, when the registration fixes one.
    pub subject: Option<String>,
    /// The one Webex organization this adapter serves.
    pub organization_id: String,
    /// Registration for WebSocket tokens; `None` rejects every WebSocket token.
    pub websocket: Option<WebexRegistration>,
    /// Registration for gRPC tokens; `None` rejects every gRPC token.
    pub grpc: Option<WebexRegistration>,
    pub algorithms: Vec<Algorithm>,
    pub leeway: Duration,
    pub max_token_lifetime: Duration,
    pub keys_max_age: Duration,
    pub claim_names: WebexClaimNames,
}

impl WebexJwsConfig {
    pub fn new(
        issuers: Vec<String>,
        audience: impl Into<String>,
        organization_id: impl Into<String>,
    ) -> Self {
        Self {
            issuers,
            audience: audience.into(),
            subject: None,
            organization_id: organization_id.into(),
            websocket: None,
            grpc: None,
            algorithms: vec![Algorithm::RS256],
            leeway: Duration::from_secs(60),
            max_token_lifetime: Duration::from_secs(24 * 60 * 60),
            keys_max_age: Duration::from_secs(60 * 60),
            claim_names: WebexClaimNames::default(),
        }
    }

    pub fn with_websocket(mut self, registration: WebexRegistration) -> Self {
        self.websocket = Some(registration);
        self
    }

    pub fn with_grpc(mut self, registration: WebexRegistration) -> Self {
        self.grpc = Some(registration);
        self
    }

    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    fn validate(&self) -> Result<()> {
        let symmetric = self.algorithms.iter().any(|algorithm| {
            matches!(
                algorithm,
                Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512
            )
        });
        if self.issuers.is_empty()
            || self.issuers.iter().any(String::is_empty)
            || self.audience.is_empty()
            || self.organization_id.is_empty()
            || (self.websocket.is_none() && self.grpc.is_none())
            || self.algorithms.is_empty()
            || symmetric
            || self.max_token_lifetime.is_zero()
            || self.keys_max_age.is_zero()
            || self.leeway > Duration::from_secs(300)
        {
            return Err(WebexError::Configuration);
        }
        for registration in [&self.websocket, &self.grpc].into_iter().flatten() {
            registration.validate()?;
        }
        Ok(())
    }
}

impl fmt::Debug for WebexJwsConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebexJwsConfig")
            .field("issuer_count", &self.issuers.len())
            .field("websocket", &self.websocket)
            .field("grpc", &self.grpc)
            .field("algorithms", &self.algorithms)
            .finish_non_exhaustive()
    }
}

/// A verified Webex service principal. It identifies the Webex organization
/// and data source registration, not the human caller.
#[derive(Clone)]
pub struct WebexPrincipal {
    pub organization_id: String,
    pub subject: String,
    pub issuer: String,
    pub expires_at: DateTime<Utc>,
}

impl fmt::Debug for WebexPrincipal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WebexPrincipal([redacted])")
    }
}

/// Authenticates the bearer token of one Webex connection or gRPC call.
///
/// The adapter never admits input without a principal from this trait.
/// [`WebexJwsValidator`] is the provided implementation.
pub trait WebexAuthenticator: Send + Sync {
    fn authenticate(&self, token: &str, transport: WebexTransportKind) -> Result<WebexPrincipal>;
}

/// Cisco JWS validator over host-supplied keys. See the module documentation.
pub struct WebexJwsValidator {
    config: WebexJwsConfig,
    keys: RwLock<(JwkSet, Instant)>,
}

impl WebexJwsValidator {
    pub fn new(config: WebexJwsConfig, keys: JwkSet) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            keys: RwLock::new((keys, Instant::now())),
        })
    }

    /// Install a freshly fetched key set and restart its age.
    pub fn replace_keys(&self, keys: JwkSet) {
        *self.keys.write() = (keys, Instant::now());
    }

    pub fn config(&self) -> &WebexJwsConfig {
        &self.config
    }

    fn registration(&self, transport: WebexTransportKind) -> Option<&WebexRegistration> {
        match transport {
            WebexTransportKind::WebSocket => self.config.websocket.as_ref(),
            WebexTransportKind::Grpc => self.config.grpc.as_ref(),
        }
    }
}

impl fmt::Debug for WebexJwsValidator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebexJwsValidator")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    bool::from(left.as_bytes().ct_eq(right.as_bytes()))
}

fn string_claim<'a>(claims: &'a Map<String, Value>, name: &str) -> Result<&'a str> {
    claims
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(WebexError::Unauthorized)
}

fn timestamp_claim(claims: &Map<String, Value>, name: &str) -> Result<i64> {
    claims
        .get(name)
        .and_then(Value::as_i64)
        .ok_or(WebexError::Unauthorized)
}

impl WebexAuthenticator for WebexJwsValidator {
    fn authenticate(&self, token: &str, transport: WebexTransportKind) -> Result<WebexPrincipal> {
        let registration = self
            .registration(transport)
            .ok_or(WebexError::Unauthorized)?;
        if token.is_empty() || token.len() > MAX_TOKEN_BYTES {
            return Err(WebexError::Unauthorized);
        }
        let header = jsonwebtoken::decode_header(token).map_err(|_| WebexError::Unauthorized)?;
        if !self.config.algorithms.contains(&header.alg) {
            return Err(WebexError::Unauthorized);
        }
        let kid = header.kid.as_deref().ok_or(WebexError::Unauthorized)?;
        let key = {
            let keys = self.keys.read();
            if keys.1.elapsed() > self.config.keys_max_age {
                return Err(WebexError::KeysUnavailable);
            }
            let jwk = keys.0.find(kid).ok_or(WebexError::Unauthorized)?;
            DecodingKey::from_jwk(jwk).map_err(|_| WebexError::Unauthorized)?
        };
        let mut validation = Validation::new(header.alg);
        validation.algorithms = self.config.algorithms.clone();
        validation.leeway = self.config.leeway.as_secs();
        validation.validate_nbf = true;
        validation.set_issuer(&self.config.issuers);
        validation.set_audience(&[&self.config.audience]);
        validation.set_required_spec_claims(&["exp", "iat", "iss", "aud", "sub", "jti"]);
        if let Some(subject) = &self.config.subject {
            validation.sub = Some(subject.clone());
        }
        let claims = jsonwebtoken::decode::<Map<String, Value>>(token, &key, &validation)
            .map_err(|_| WebexError::Unauthorized)?
            .claims;

        let now = Utc::now().timestamp();
        let leeway = self.config.leeway.as_secs() as i64;
        let issued_at = timestamp_claim(&claims, "iat")?;
        let expires_at = timestamp_claim(&claims, "exp")?;
        if issued_at > now.saturating_add(leeway)
            || expires_at <= issued_at
            || (expires_at - issued_at) as u64 > self.config.max_token_lifetime.as_secs()
        {
            return Err(WebexError::Unauthorized);
        }
        let nonce = string_claim(&claims, "jti")?;
        if !registration
            .nonces
            .iter()
            .any(|current| constant_time_eq(current, nonce))
        {
            return Err(WebexError::Unauthorized);
        }
        let names = &self.config.claim_names;
        let organization = string_claim(&claims, &names.organization_id)?;
        if organization != self.config.organization_id
            || string_claim(&claims, &names.datasource_url)? != registration.datasource_url
            || string_claim(&claims, &names.schema_id)? != registration.schema_id
        {
            return Err(WebexError::Unauthorized);
        }
        Ok(WebexPrincipal {
            organization_id: organization.to_owned(),
            subject: string_claim(&claims, "sub")?.to_owned(),
            issuer: string_claim(&claims, "iss")?.to_owned(),
            expires_at: Utc
                .timestamp_opt(expires_at, 0)
                .single()
                .ok_or(WebexError::Unauthorized)?,
        })
    }
}

/// Bearer token from an `Authorization` value. WebSocket requires the
/// `Bearer ` scheme; Cisco's gRPC samples send the bare token, so gRPC
/// accepts either form.
pub(crate) fn bearer(value: &str, transport: WebexTransportKind) -> Option<&str> {
    let token = match value.strip_prefix("Bearer ") {
        Some(token) => token,
        None if transport == WebexTransportKind::Grpc => value,
        None => return None,
    };
    let token = token.trim();
    (!token.is_empty()).then_some(token)
}

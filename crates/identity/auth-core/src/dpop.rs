//! DPoP-Proof JWT validation per RFC 9449.
//!
//! DPoP ("Demonstrating Proof-of-Possession") binds a bearer token to a
//! per-client key pair: every request carries both the access token AND
//! a freshly-signed DPoP-Proof JWT whose signature proves the client
//! still holds the private key. The access token's `cnf.jkt` claim
//! locks it to a specific public-key thumbprint (RFC 7638) so a
//! stolen token can't be replayed without the matching private key.
//!
//! This module ships the foundational building blocks of RFC 9449:
//!
//! - [`DpopProof`] — parsed DPoP-Proof JWT.
//! - [`DpopValidator`] — verifies the proof JWT's signature against
//!   the embedded `jwk` header, checks `iat` freshness, tracks `jti`
//!   for replay protection, and returns the JWK thumbprint per
//!   RFC 7638.
//!
//! The full access-token binding (matching `cnf.jkt` against the
//! returned thumbprint, plus `htm`/`htu` UCTP-equivalent semantics)
//! lives one layer up and is left to the caller because the
//! UCTP-spec equivalents of `htm` ("HTTP method") and `htu` ("HTTP
//! URI") need explicit definition in CONVERSATION_PROTOCOL.md before
//! they can be enforced — out of scope for this crate.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use chrono::Utc;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use moka::future::Cache;
use ring::digest::{digest, SHA256};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Default `iat` window: a proof JWT issued more than this far in the
/// past (or future) is rejected. RFC 9449 recommends "small" (single-
/// digit seconds in strict configurations); 60s is generous enough
/// for normal clock skew without opening a large replay window.
pub const DEFAULT_IAT_LEEWAY: Duration = Duration::from_secs(60);

/// Default jti cache size. Tokens are tracked by jti for `iat_leeway *
/// 2` so the same proof can't be replayed within its acceptance
/// window. 100k entries × ~64 bytes each ~= 6 MB; bounded by moka's
/// LRU eviction.
pub const DEFAULT_JTI_CACHE_CAPACITY: u64 = 100_000;

pub enum DpopError {
    Header(String),
    Signature(String),
    Claims(String),
    IatOutOfWindow,
    Replayed,
    UnsupportedKty(String),
    MissingJwkField(&'static str),
}

impl DpopError {
    fn diagnostic_class(&self) -> &'static str {
        match self {
            Self::Header(_) => "header",
            Self::Signature(_) => "signature",
            Self::Claims(_) => "claims",
            Self::IatOutOfWindow => "iat-window",
            Self::Replayed => "replay",
            Self::UnsupportedKty(_) => "unsupported-key-type",
            Self::MissingJwkField(_) => "missing-key-field",
        }
    }
}

impl fmt::Display for DpopError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "DPoP validation failed (class={})",
            self.diagnostic_class()
        )
    }
}

impl fmt::Debug for DpopError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DpopError")
            .field("class", &self.diagnostic_class())
            .finish()
    }
}

impl std::error::Error for DpopError {}

/// Standard DPoP-Proof JWT claims (RFC 9449 §4.2).
///
/// `htm` / `htu` are echoed back so callers can apply method-binding
/// at their layer (this crate doesn't enforce them itself because
/// UCTP doesn't have an HTTP method/URI by default — see module docs).
#[derive(Clone, Deserialize, Serialize)]
pub struct DpopProof {
    /// Replay-protection nonce. Required.
    pub jti: String,
    /// "HTTP method" — for UCTP, treat as an opaque method identifier.
    /// Required by RFC 9449; clients should set it consistently for
    /// the operation.
    pub htm: String,
    /// "HTTP URI" — for UCTP, treat as the UCTP endpoint URL.
    /// Required by RFC 9449.
    pub htu: String,
    /// Issued at, Unix seconds. Required for replay-window checks.
    pub iat: i64,
    /// Optional access-token hash binding (RFC 9449 §6.1). Present
    /// when the proof binds to a specific access token.
    #[serde(default)]
    pub ath: Option<String>,
}

impl fmt::Debug for DpopProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DpopProof")
            .field("jti_present", &!self.jti.is_empty())
            .field("method_present", &!self.htm.is_empty())
            .field("uri_present", &!self.htu.is_empty())
            .field("issued_at_present", &true)
            .field("access_token_hash_present", &self.ath.is_some())
            .finish()
    }
}

/// One-shot validation result. Carries the RFC 7638 SHA-256
/// thumbprint of the public key the client used to sign the proof —
/// the caller compares this against the access token's `cnf.jkt`
/// claim to complete the DPoP binding.
#[derive(Clone)]
pub struct ValidatedDpop {
    pub jkt: String,
    pub proof: DpopProof,
}

impl fmt::Debug for ValidatedDpop {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedDpop")
            .field("thumbprint_present", &!self.jkt.is_empty())
            .field("proof", &self.proof)
            .finish()
    }
}

/// Validate DPoP-Proof JWTs against the embedded `jwk` header. The
/// validator owns the jti replay cache; callers share one instance
/// across all requests they want to gate.
#[derive(Clone)]
pub struct DpopValidator {
    inner: Arc<DpopInner>,
}

struct DpopInner {
    iat_leeway: Duration,
    jti_cache: Cache<String, ()>,
    allowed_algorithms: Vec<Algorithm>,
}

impl DpopValidator {
    /// Default: 60s leeway, jti cache holding up to
    /// [`DEFAULT_JTI_CACHE_CAPACITY`] entries for `iat_leeway * 2`.
    pub fn new() -> Self {
        Self::with_leeway(DEFAULT_IAT_LEEWAY)
    }

    pub fn with_leeway(iat_leeway: Duration) -> Self {
        let ttl = iat_leeway * 2;
        Self {
            inner: Arc::new(DpopInner {
                iat_leeway,
                jti_cache: Cache::builder()
                    .max_capacity(DEFAULT_JTI_CACHE_CAPACITY)
                    .time_to_live(ttl)
                    .build(),
                // RFC 9449 §4.2 allows EC (ES256/ES384/ES512), RSA
                // (PS256+), and Ed25519. Restrict to the modern set
                // by default — explicit override via with_algorithms.
                allowed_algorithms: vec![
                    Algorithm::ES256,
                    Algorithm::ES384,
                    Algorithm::PS256,
                    Algorithm::PS384,
                    Algorithm::PS512,
                    Algorithm::EdDSA,
                ],
            }),
        }
    }

    pub fn with_algorithms(self, algorithms: Vec<Algorithm>) -> Self {
        // Reconstruct with the new algorithm set. The jti cache is
        // re-initialized here too; deployments setting algorithms
        // should do so at validator-construction time before tokens
        // flow.
        let ttl = self.inner.iat_leeway * 2;
        Self {
            inner: Arc::new(DpopInner {
                iat_leeway: self.inner.iat_leeway,
                jti_cache: Cache::builder()
                    .max_capacity(DEFAULT_JTI_CACHE_CAPACITY)
                    .time_to_live(ttl)
                    .build(),
                allowed_algorithms: algorithms,
            }),
        }
    }

    /// Validate a DPoP-Proof JWT.
    ///
    /// 1. Parse header → extract `typ` (must be `"dpop+jwt"`), `alg`
    ///    (must be in allowed list), and `jwk` (the client's
    ///    public key).
    /// 2. Verify the JWT signature against `jwk`.
    /// 3. Check `iat` is within [`Self::with_leeway`] of now.
    /// 4. Check `jti` hasn't been seen recently (replay protection).
    /// 5. Compute and return the JWK thumbprint per RFC 7638.
    pub async fn validate(&self, proof_jwt: &str) -> Result<ValidatedDpop, DpopError> {
        // 1. Header inspection without signature check (need to learn
        // the jwk and alg before we can verify).
        let header = decode_header(proof_jwt).map_err(|e| DpopError::Header(e.to_string()))?;
        if header.typ.as_deref() != Some("dpop+jwt") {
            return Err(DpopError::Header(format!(
                "typ must be \"dpop+jwt\", got {:?}",
                header.typ
            )));
        }
        if !self.inner.allowed_algorithms.contains(&header.alg) {
            return Err(DpopError::Header(format!(
                "alg {:?} not in allowed set",
                header.alg
            )));
        }
        let jwk_value = header
            .jwk
            .ok_or_else(|| DpopError::Header("missing jwk in header".into()))?;
        let jwk_json = serde_json::to_value(&jwk_value)
            .map_err(|e| DpopError::Header(format!("jwk re-serialize: {e}")))?;

        // 2. Verify signature against jwk. jsonwebtoken supports
        // `from_jwk` so we can hand the parsed shape straight in.
        let decoding_key = DecodingKey::from_jwk(&jwk_value)
            .map_err(|e| DpopError::Signature(format!("decoding key: {e}")))?;
        let mut validation = Validation::new(header.alg);
        // The proof JWT doesn't carry standard JWT claims (no exp/iss/aud);
        // strip the default required claims so jsonwebtoken doesn't
        // reject it before we get to look at iat/jti.
        validation.required_spec_claims.clear();
        validation.validate_exp = false;
        validation.validate_aud = false;
        let data = decode::<DpopProof>(proof_jwt, &decoding_key, &validation)
            .map_err(|e| DpopError::Signature(e.to_string()))?;
        let claims = data.claims;

        // 3. iat window check.
        let now = Utc::now().timestamp();
        let leeway = self.inner.iat_leeway.as_secs() as i64;
        if (now - claims.iat).abs() > leeway {
            return Err(DpopError::IatOutOfWindow);
        }

        // 4. jti replay check + insert. Cache TTL = 2 × leeway so a
        // proof can't be replayed within its acceptance window even
        // at the leeway boundary.
        if self.inner.jti_cache.get(&claims.jti).await.is_some() {
            return Err(DpopError::Replayed);
        }
        self.inner.jti_cache.insert(claims.jti.clone(), ()).await;

        // 5. RFC 7638 thumbprint.
        let jkt = jwk_thumbprint(&jwk_json)?;

        Ok(ValidatedDpop { jkt, proof: claims })
    }
}

impl Default for DpopValidator {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute the RFC 7638 SHA-256 thumbprint of a JWK.
///
/// The canonical JSON is constructed from a fixed subset of fields in
/// lexicographic order (RFC 7638 §3). EC keys use {crv, kty, x, y};
/// RSA keys use {e, kty, n}; OKP keys use {crv, kty, x}.
pub fn jwk_thumbprint(jwk: &serde_json::Value) -> Result<String, DpopError> {
    let kty = jwk
        .get("kty")
        .and_then(|v| v.as_str())
        .ok_or(DpopError::MissingJwkField("kty"))?;

    // Build the canonical fields as a fresh sorted Map.
    let mut canonical = serde_json::Map::new();
    match kty {
        "EC" => {
            for field in ["crv", "kty", "x", "y"] {
                let v = jwk
                    .get(field)
                    .ok_or(DpopError::MissingJwkField(match field {
                        "crv" => "crv",
                        "kty" => "kty",
                        "x" => "x",
                        "y" => "y",
                        _ => "unknown",
                    }))?;
                canonical.insert(field.into(), v.clone());
            }
        }
        "RSA" => {
            for field in ["e", "kty", "n"] {
                let v = jwk
                    .get(field)
                    .ok_or(DpopError::MissingJwkField(match field {
                        "e" => "e",
                        "kty" => "kty",
                        "n" => "n",
                        _ => "unknown",
                    }))?;
                canonical.insert(field.into(), v.clone());
            }
        }
        "OKP" => {
            for field in ["crv", "kty", "x"] {
                let v = jwk
                    .get(field)
                    .ok_or(DpopError::MissingJwkField(match field {
                        "crv" => "crv",
                        "kty" => "kty",
                        "x" => "x",
                        _ => "unknown",
                    }))?;
                canonical.insert(field.into(), v.clone());
            }
        }
        other => return Err(DpopError::UnsupportedKty(other.into())),
    }

    let canonical_json =
        serde_json::to_string(&canonical).map_err(|e| DpopError::Header(e.to_string()))?;
    let hash = digest(&SHA256, canonical_json.as_bytes());
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash.as_ref()))
}

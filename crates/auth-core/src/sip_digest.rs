//! SIP Digest Authentication per RFC 2617, RFC 7616, and RFC 8760.
//!
//! Supports `MD5`, `MD5-sess`, `SHA-256`, and `SHA-256-sess`. Other
//! algorithm tokens (notably RFC 8760 `SHA-512-256` / `SHA-512-256-sess`)
//! are recognised in the parser but degrade to `MD5` with a warning —
//! add explicit handling when a deployment requests them.
//!
//! ## Sprint 3 nonce-count hardening (RFC 7616 §3.4.5)
//!
//! Per-(realm, nonce) `nc` is owned by the *caller* (typically
//! `session-core::SessionState::digest_nc`). The legacy
//! [`DigestClient::compute_response`] entry point is preserved as a thin
//! wrapper that always passes `nc=1, body=None`; new code should call
//! [`DigestClient::compute_response_with_state`] and pass an
//! incrementing counter plus the request body (for `auth-int`).

use crate::error::{AuthError, Result};
use hex;
use rand::Rng;
use sha2::{Digest as Sha2Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

/// Digest authentication algorithm.
///
/// `*-sess` variants follow RFC 7616 §3.4.2: the basic `H(user:realm:pwd)`
/// HA1 is rehashed with `nonce:cnonce` to derive a session-bound key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestAlgorithm {
    MD5,
    MD5Sess,
    SHA256,
    SHA256Sess,
}

impl DigestAlgorithm {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MD5 => "MD5",
            Self::MD5Sess => "MD5-sess",
            Self::SHA256 => "SHA-256",
            Self::SHA256Sess => "SHA-256-sess",
        }
    }

    /// Whether the algorithm is a `-sess` variant requiring HA1 to fold
    /// in `nonce:cnonce` (RFC 7616 §3.4.2).
    pub fn is_sess(&self) -> bool {
        matches!(self, Self::MD5Sess | Self::SHA256Sess)
    }

    /// Hash an input slice with the algorithm's underlying hash function.
    /// MD5 / MD5-sess → MD5; SHA-256 / SHA-256-sess → SHA-256.
    fn hash(&self, input: &[u8]) -> String {
        match self {
            Self::MD5 | Self::MD5Sess => hex::encode(md5::compute(input).0),
            Self::SHA256 | Self::SHA256Sess => hex::encode(Sha256::digest(input)),
        }
    }
}

impl std::fmt::Display for DigestAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

fn parse_algorithm(value: &str) -> DigestAlgorithm {
    let value = value.trim();
    if value.eq_ignore_ascii_case("MD5") {
        DigestAlgorithm::MD5
    } else if value.eq_ignore_ascii_case("MD5-sess") {
        DigestAlgorithm::MD5Sess
    } else if value.eq_ignore_ascii_case("SHA-256") {
        DigestAlgorithm::SHA256
    } else if value.eq_ignore_ascii_case("SHA-256-sess") {
        DigestAlgorithm::SHA256Sess
    } else {
        tracing::warn!(
            "Unknown digest algorithm '{}' — falling back to MD5 (RFC 8760 SHA-512-256 not yet wired)",
            value
        );
        DigestAlgorithm::MD5
    }
}

fn split_auth_params(params: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut escaped = false;

    for (idx, ch) in params.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                parts.push(params[start..idx].trim());
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }

    parts.push(params[start..].trim());
    parts.into_iter().filter(|part| !part.is_empty()).collect()
}

fn unquote_auth_value(value: &str) -> String {
    let value = value.trim();
    let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
        return value.to_string();
    };

    let mut unescaped = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                unescaped.push(next);
            }
        } else {
            unescaped.push(ch);
        }
    }
    unescaped
}

fn parse_qop_options(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Digest challenge issued by server (401/407 response).
#[derive(Debug, Clone)]
pub struct DigestChallenge {
    pub realm: String,
    pub nonce: String,
    pub algorithm: DigestAlgorithm,
    pub qop: Option<Vec<String>>, // "auth", "auth-int"
    pub opaque: Option<String>,
}

/// Parsed Authorization header on the server side.
#[derive(Debug, Clone)]
pub struct DigestResponse {
    pub username: String,
    pub realm: String,
    pub nonce: String,
    pub uri: String,
    pub response: String,
    pub algorithm: DigestAlgorithm,
    pub cnonce: Option<String>,
    pub qop: Option<String>,
    pub nc: Option<String>,
    pub opaque: Option<String>,
}

/// Result of computing a digest response with explicit state.
///
/// Returned by [`DigestClient::compute_response_with_state`] so the
/// caller can fold the same `nc` and `qop` values into the
/// Authorization header without duplicating the qop-selection logic.
#[derive(Debug, Clone)]
pub struct DigestComputed {
    pub response: String,
    pub cnonce: Option<String>,
    /// Hex-formatted nonce-count (e.g. `"00000002"`). `None` when no
    /// qop was negotiated (legacy mode).
    pub nc: Option<String>,
    /// Negotiated qop value (`"auth"` or `"auth-int"`), or `None` when
    /// the server didn't offer qop.
    pub qop: Option<String>,
}

/// SIP Digest authenticator for generating challenges and validating responses.
pub struct DigestAuthenticator {
    realm: String,
    algorithm: DigestAlgorithm,
}

impl DigestAuthenticator {
    pub fn new(realm: impl Into<String>) -> Self {
        Self {
            realm: realm.into(),
            algorithm: DigestAlgorithm::MD5,
        }
    }

    pub fn generate_challenge(&self) -> DigestChallenge {
        DigestChallenge {
            realm: self.realm.clone(),
            nonce: Self::generate_nonce(),
            algorithm: self.algorithm,
            qop: Some(vec!["auth".to_string()]),
            opaque: Some(Self::generate_opaque()),
        }
    }

    pub fn format_www_authenticate(&self, challenge: &DigestChallenge) -> String {
        let mut parts = vec![
            format!(r#"realm="{}""#, challenge.realm),
            format!(r#"nonce="{}""#, challenge.nonce),
            format!(r#"algorithm={}"#, challenge.algorithm),
        ];

        if let Some(ref qop) = challenge.qop {
            parts.push(format!(r#"qop="{}""#, qop.join(",")));
        }

        if let Some(ref opaque) = challenge.opaque {
            parts.push(format!(r#"opaque="{}""#, opaque));
        }

        format!("Digest {}", parts.join(", "))
    }

    /// Validate a digest response against the stored password.
    ///
    /// Honors the algorithm carried on the response (`MD5` /
    /// `MD5-sess` / `SHA-256` / `SHA-256-sess`) — clients sending
    /// SHA-256 are validated with SHA-256, not silently downgraded.
    pub fn validate_response(
        &self,
        response: &DigestResponse,
        method: &str,
        password: &str,
    ) -> Result<bool> {
        self.validate_response_with_body(response, method, password, None)
    }

    /// Like [`Self::validate_response`] but also accepts the request
    /// body so `qop=auth-int` validation can include it in HA2.
    pub fn validate_response_with_body(
        &self,
        response: &DigestResponse,
        method: &str,
        password: &str,
        body: Option<&[u8]>,
    ) -> Result<bool> {
        let algorithm = response.algorithm;
        // HA1
        let basic_ha1 = algorithm
            .hash(format!("{}:{}:{}", response.username, response.realm, password).as_bytes());
        let ha1 = if algorithm.is_sess() {
            let cnonce = response.cnonce.as_deref().ok_or_else(|| {
                AuthError::InvalidResponse("Missing cnonce for -sess algorithm".into())
            })?;
            algorithm.hash(format!("{}:{}:{}", basic_ha1, response.nonce, cnonce).as_bytes())
        } else {
            basic_ha1
        };

        // HA2
        let ha2 = match response.qop.as_deref() {
            Some("auth-int") => {
                let body_bytes = body.unwrap_or(&[]);
                let body_hash = algorithm.hash(body_bytes);
                algorithm.hash(format!("{}:{}:{}", method, response.uri, body_hash).as_bytes())
            }
            _ => algorithm.hash(format!("{}:{}", method, response.uri).as_bytes()),
        };

        // Expected response
        let expected = if let (Some(qop), Some(nc), Some(cnonce)) = (
            response.qop.as_ref(),
            response.nc.as_ref(),
            response.cnonce.as_ref(),
        ) {
            algorithm.hash(
                format!(
                    "{}:{}:{}:{}:{}:{}",
                    ha1, response.nonce, nc, cnonce, qop, ha2
                )
                .as_bytes(),
            )
        } else {
            algorithm.hash(format!("{}:{}:{}", ha1, response.nonce, ha2).as_bytes())
        };

        Ok(expected == response.response)
    }

    /// Parse WWW-Authenticate header to extract challenge.
    pub fn parse_challenge(header: &str) -> Result<DigestChallenge> {
        let header = header.trim();

        let params_str = if header.starts_with("Digest ") || header.starts_with("digest ") {
            &header[7..]
        } else {
            return Err(AuthError::InvalidChallenge(
                "Missing 'Digest' prefix".into(),
            ));
        };

        let mut realm = None;
        let mut nonce = None;
        let mut algorithm = DigestAlgorithm::MD5;
        let mut qop = None;
        let mut opaque = None;

        for param in split_auth_params(params_str) {
            let param = param.trim();
            if let Some((key, value)) = param.split_once('=') {
                let key = key.trim().to_ascii_lowercase();
                let value = unquote_auth_value(value);

                match key.as_str() {
                    "realm" => realm = Some(value),
                    "nonce" => nonce = Some(value),
                    "algorithm" => algorithm = parse_algorithm(&value),
                    "qop" => {
                        qop = Some(parse_qop_options(&value));
                    }
                    "opaque" => opaque = Some(value),
                    _ => {}
                }
            }
        }

        Ok(DigestChallenge {
            realm: realm.ok_or_else(|| AuthError::InvalidChallenge("Missing realm".into()))?,
            nonce: nonce.ok_or_else(|| AuthError::InvalidChallenge("Missing nonce".into()))?,
            algorithm,
            qop,
            opaque,
        })
    }

    /// Parse Authorization header to extract response.
    pub fn parse_authorization(header: &str) -> Result<DigestResponse> {
        let header = header.trim();

        let params_str = if header.starts_with("Digest ") || header.starts_with("digest ") {
            &header[7..]
        } else {
            return Err(AuthError::InvalidResponse("Missing 'Digest' prefix".into()));
        };

        let mut username = None;
        let mut realm = None;
        let mut nonce = None;
        let mut uri = None;
        let mut response = None;
        let mut algorithm = DigestAlgorithm::MD5;
        let mut cnonce = None;
        let mut qop = None;
        let mut nc = None;
        let mut opaque = None;

        for param in split_auth_params(params_str) {
            let param = param.trim();
            if let Some((key, value)) = param.split_once('=') {
                let key = key.trim().to_ascii_lowercase();
                let value = unquote_auth_value(value);

                match key.as_str() {
                    "username" => username = Some(value),
                    "realm" => realm = Some(value),
                    "nonce" => nonce = Some(value),
                    "uri" => uri = Some(value),
                    "response" => response = Some(value),
                    "algorithm" => algorithm = parse_algorithm(&value),
                    "cnonce" => cnonce = Some(value),
                    "qop" => qop = Some(value.to_ascii_lowercase()),
                    "nc" => nc = Some(value),
                    "opaque" => opaque = Some(value),
                    _ => {}
                }
            }
        }

        Ok(DigestResponse {
            username: username
                .ok_or_else(|| AuthError::InvalidResponse("Missing username".into()))?,
            realm: realm.ok_or_else(|| AuthError::InvalidResponse("Missing realm".into()))?,
            nonce: nonce.ok_or_else(|| AuthError::InvalidResponse("Missing nonce".into()))?,
            uri: uri.ok_or_else(|| AuthError::InvalidResponse("Missing uri".into()))?,
            response: response
                .ok_or_else(|| AuthError::InvalidResponse("Missing response".into()))?,
            algorithm,
            cnonce,
            qop,
            nc,
            opaque,
        })
    }

    fn generate_nonce() -> String {
        let mut rng = rand::thread_rng();
        let random_bytes: [u8; 16] = rng.gen();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let data = format!("{}{}", timestamp, hex::encode(random_bytes));
        hex::encode(md5::compute(data.as_bytes()).0)
    }

    fn generate_opaque() -> String {
        let mut rng = rand::thread_rng();
        let random_bytes: [u8; 16] = rng.gen();
        hex::encode(random_bytes)
    }
}

/// Client-side digest authentication helper.
pub struct DigestClient;

impl DigestClient {
    /// Legacy entry point. Always uses `nc=1` and never `auth-int`.
    /// New callers should prefer
    /// [`DigestClient::compute_response_with_state`] so per-(realm,
    /// nonce) counter increments survive across requests.
    pub fn compute_response(
        username: &str,
        password: &str,
        challenge: &DigestChallenge,
        method: &str,
        uri: &str,
    ) -> Result<(String, Option<String>)> {
        let computed =
            Self::compute_response_with_state(username, password, challenge, method, uri, 1, None)?;
        Ok((computed.response, computed.cnonce))
    }

    /// Compute a digest response with caller-managed state.
    ///
    /// `nc` is the per-(realm, nonce) request counter (RFC 7616
    /// §3.4.5); the caller is responsible for incrementing it on
    /// each request that reuses the same nonce, and resetting to 1
    /// when a fresh challenge with a different nonce arrives.
    ///
    /// `body` is the request body (e.g. INVITE SDP). When `Some(_)`
    /// AND the challenge offers `qop=auth-int`, `auth-int` is
    /// selected and the body is folded into HA2 per RFC 7616 §3.4.3.
    /// Pass `None` (or an empty slice) for bodyless requests like
    /// REGISTER.
    pub fn compute_response_with_state(
        username: &str,
        password: &str,
        challenge: &DigestChallenge,
        method: &str,
        uri: &str,
        nc: u32,
        body: Option<&[u8]>,
    ) -> Result<DigestComputed> {
        let algorithm = challenge.algorithm;
        let cnonce_value = Self::generate_cnonce();
        let nc_str = format!("{:08x}", nc);

        // HA1 (RFC 7616 §3.4.2). `-sess` wraps the basic HA1 in a
        // session-bound construction with nonce + cnonce.
        let basic_ha1 =
            algorithm.hash(format!("{}:{}:{}", username, challenge.realm, password).as_bytes());
        let ha1 = if algorithm.is_sess() {
            algorithm.hash(format!("{}:{}:{}", basic_ha1, challenge.nonce, cnonce_value).as_bytes())
        } else {
            basic_ha1
        };

        // qop selection (RFC 7616 §3.4): prefer auth-int when the
        // server offers it AND the caller supplied a body; else auth;
        // else legacy (no qop).
        let chosen_qop = challenge.qop.as_ref().and_then(|opts| {
            if body.is_some() && opts.iter().any(|q| q == "auth-int") {
                Some("auth-int".to_string())
            } else if opts.iter().any(|q| q == "auth") {
                Some("auth".to_string())
            } else {
                None
            }
        });

        // HA2 (RFC 7616 §3.4.3).
        let ha2 = match chosen_qop.as_deref() {
            Some("auth-int") => {
                let body_bytes = body.unwrap_or(&[]);
                let body_hash = algorithm.hash(body_bytes);
                algorithm.hash(format!("{}:{}:{}", method, uri, body_hash).as_bytes())
            }
            _ => algorithm.hash(format!("{}:{}", method, uri).as_bytes()),
        };

        // Response (RFC 7616 §3.4.1).
        let response = if let Some(ref qop) = chosen_qop {
            algorithm.hash(
                format!(
                    "{}:{}:{}:{}:{}:{}",
                    ha1, challenge.nonce, nc_str, cnonce_value, qop, ha2
                )
                .as_bytes(),
            )
        } else {
            algorithm.hash(format!("{}:{}:{}", ha1, challenge.nonce, ha2).as_bytes())
        };

        let (cnonce_out, nc_out) = if chosen_qop.is_some() {
            (Some(cnonce_value), Some(nc_str))
        } else {
            (None, None)
        };

        Ok(DigestComputed {
            response,
            cnonce: cnonce_out,
            nc: nc_out,
            qop: chosen_qop,
        })
    }

    /// Legacy Authorization header formatter. Always emits
    /// `nc=00000001` when qop is in play. Preserved for callers that
    /// don't yet thread state; new callers should use
    /// [`DigestClient::format_authorization_with_state`].
    pub fn format_authorization(
        username: &str,
        challenge: &DigestChallenge,
        uri: &str,
        response: &str,
        cnonce: Option<&str>,
    ) -> String {
        let mut parts = vec![
            format!(r#"username="{}""#, username),
            format!(r#"realm="{}""#, challenge.realm),
            format!(r#"nonce="{}""#, challenge.nonce),
            format!(r#"uri="{}""#, uri),
            format!(r#"response="{}""#, response),
            format!(r#"algorithm={}"#, challenge.algorithm),
        ];

        if let Some(ref qop_options) = challenge.qop {
            if qop_options.iter().any(|q| q == "auth") {
                parts.push("qop=auth".to_string());
                parts.push("nc=00000001".to_string());
                let cn_owned;
                let cn = match cnonce {
                    Some(c) => c,
                    None => {
                        cn_owned = Self::generate_cnonce();
                        cn_owned.as_str()
                    }
                };
                parts.push(format!(r#"cnonce="{}""#, cn));
            }
        }

        if let Some(ref opaque) = challenge.opaque {
            parts.push(format!(r#"opaque="{}""#, opaque));
        }

        format!("Digest {}", parts.join(", "))
    }

    /// Format the Authorization header using a precomputed
    /// [`DigestComputed`]. The `nc`, `cnonce`, and `qop` fields are
    /// emitted exactly as they were used in the response computation
    /// — no recomputation, no risk of drift.
    pub fn format_authorization_with_state(
        username: &str,
        challenge: &DigestChallenge,
        uri: &str,
        computed: &DigestComputed,
    ) -> String {
        let mut parts = vec![
            format!(r#"username="{}""#, username),
            format!(r#"realm="{}""#, challenge.realm),
            format!(r#"nonce="{}""#, challenge.nonce),
            format!(r#"uri="{}""#, uri),
            format!(r#"response="{}""#, computed.response),
            format!(r#"algorithm={}"#, challenge.algorithm),
        ];

        if let (Some(qop), Some(nc), Some(cnonce)) = (
            computed.qop.as_ref(),
            computed.nc.as_ref(),
            computed.cnonce.as_ref(),
        ) {
            parts.push(format!("qop={}", qop));
            parts.push(format!("nc={}", nc));
            parts.push(format!(r#"cnonce="{}""#, cnonce));
        }

        if let Some(ref opaque) = challenge.opaque {
            parts.push(format!(r#"opaque="{}""#, opaque));
        }

        format!("Digest {}", parts.join(", "))
    }

    fn generate_cnonce() -> String {
        let mut rng = rand::thread_rng();
        let random_bytes: [u8; 8] = rng.gen();
        hex::encode(random_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn algorithm_parser_recognises_known_tokens() {
        assert_eq!(parse_algorithm("MD5"), DigestAlgorithm::MD5);
        assert_eq!(parse_algorithm("md5"), DigestAlgorithm::MD5);
        assert_eq!(parse_algorithm("MD5-sess"), DigestAlgorithm::MD5Sess);
        assert_eq!(parse_algorithm("md5-sess"), DigestAlgorithm::MD5Sess);
        assert_eq!(parse_algorithm("SHA-256"), DigestAlgorithm::SHA256);
        assert_eq!(parse_algorithm("SHA-256-sess"), DigestAlgorithm::SHA256Sess);
        // Unknown falls back to MD5 with a warning (RFC 8760 SHA-512-256 deferred).
        assert_eq!(parse_algorithm("SHA-512-256"), DigestAlgorithm::MD5);
        assert_eq!(parse_algorithm("garbage"), DigestAlgorithm::MD5);
    }

    #[test]
    fn challenge_parser_preserves_quoted_qop_list() {
        let challenge = DigestAuthenticator::parse_challenge(
            r#"Digest realm="example.com", nonce="fixed", algorithm=md5, qop="auth,auth-int""#,
        )
        .unwrap();

        assert_eq!(challenge.algorithm, DigestAlgorithm::MD5);
        assert_eq!(
            challenge.qop,
            Some(vec!["auth".to_string(), "auth-int".to_string()])
        );
    }

    #[test]
    fn authorization_parser_ignores_commas_inside_quotes() {
        let response = DigestAuthenticator::parse_authorization(
            r#"Digest username="alice,ua", realm="example.com", nonce="fixed", uri="sip:example.com", response="abcd", algorithm=MD5, qop=auth, nc=00000001, cnonce="cn,once", opaque="op,aque""#,
        )
        .unwrap();

        assert_eq!(response.username, "alice,ua");
        assert_eq!(response.cnonce.as_deref(), Some("cn,once"));
        assert_eq!(response.opaque.as_deref(), Some("op,aque"));
        assert_eq!(response.qop.as_deref(), Some("auth"));
    }

    #[test]
    fn algorithm_is_sess_only_for_sess_variants() {
        assert!(!DigestAlgorithm::MD5.is_sess());
        assert!(DigestAlgorithm::MD5Sess.is_sess());
        assert!(!DigestAlgorithm::SHA256.is_sess());
        assert!(DigestAlgorithm::SHA256Sess.is_sess());
    }

    #[test]
    fn nc_increments_across_calls_with_same_nonce() {
        let challenge = DigestChallenge {
            realm: "example.com".to_string(),
            nonce: "shared-nonce".to_string(),
            algorithm: DigestAlgorithm::MD5,
            qop: Some(vec!["auth".to_string()]),
            opaque: None,
        };

        let r1 = DigestClient::compute_response_with_state(
            "alice",
            "secret",
            &challenge,
            "INVITE",
            "sip:bob@example.com",
            1,
            None,
        )
        .unwrap();
        let r2 = DigestClient::compute_response_with_state(
            "alice",
            "secret",
            &challenge,
            "INVITE",
            "sip:bob@example.com",
            2,
            None,
        )
        .unwrap();

        assert_eq!(r1.nc.as_deref(), Some("00000001"));
        assert_eq!(r2.nc.as_deref(), Some("00000002"));
        // Different nc + different cnonce → different response hashes.
        assert_ne!(r1.response, r2.response);
    }

    #[test]
    fn nc_resets_implicitly_on_new_nonce() {
        // Two distinct nonces both starting at nc=1 — they're
        // independent counter spaces. The caller is responsible for
        // resetting; here we just verify the API permits it.
        let mk = |nonce: &str| DigestChallenge {
            realm: "example.com".to_string(),
            nonce: nonce.to_string(),
            algorithm: DigestAlgorithm::MD5,
            qop: Some(vec!["auth".to_string()]),
            opaque: None,
        };

        let r1 = DigestClient::compute_response_with_state(
            "alice",
            "secret",
            &mk("nonce-A"),
            "REGISTER",
            "sip:reg.example.com",
            1,
            None,
        )
        .unwrap();
        let r2 = DigestClient::compute_response_with_state(
            "alice",
            "secret",
            &mk("nonce-B"),
            "REGISTER",
            "sip:reg.example.com",
            1,
            None,
        )
        .unwrap();

        assert_eq!(r1.nc.as_deref(), Some("00000001"));
        assert_eq!(r2.nc.as_deref(), Some("00000001"));
        assert_ne!(
            r1.response, r2.response,
            "different nonces must produce different responses"
        );
    }

    #[test]
    fn sha256_round_trip_with_authenticator() {
        let auth = DigestAuthenticator::new("example.com");
        let challenge = DigestChallenge {
            realm: "example.com".to_string(),
            nonce: "fixed-nonce".to_string(),
            algorithm: DigestAlgorithm::SHA256,
            qop: Some(vec!["auth".to_string()]),
            opaque: None,
        };

        let computed = DigestClient::compute_response_with_state(
            "alice",
            "secret",
            &challenge,
            "INVITE",
            "sip:bob@example.com",
            1,
            None,
        )
        .unwrap();

        let response = DigestResponse {
            username: "alice".to_string(),
            realm: "example.com".to_string(),
            nonce: "fixed-nonce".to_string(),
            uri: "sip:bob@example.com".to_string(),
            response: computed.response.clone(),
            algorithm: DigestAlgorithm::SHA256,
            cnonce: computed.cnonce.clone(),
            qop: computed.qop.clone(),
            nc: computed.nc.clone(),
            opaque: None,
        };

        assert!(auth
            .validate_response(&response, "INVITE", "secret")
            .unwrap());
        // Wrong password rejected.
        assert!(!auth
            .validate_response(&response, "INVITE", "WRONG")
            .unwrap());
    }

    #[test]
    fn sha256_sess_uses_session_key_ha1() {
        // Same inputs except algorithm. -sess HA1 folds in nonce +
        // cnonce, so the response must differ.
        let mk = |alg| DigestChallenge {
            realm: "example.com".to_string(),
            nonce: "fixed-nonce".to_string(),
            algorithm: alg,
            qop: Some(vec!["auth".to_string()]),
            opaque: None,
        };

        // We can't compare hashes directly because cnonce is random.
        // Instead, exercise the validate path on each.
        let auth_plain = DigestAuthenticator::new("example.com");

        for alg in [
            DigestAlgorithm::SHA256,
            DigestAlgorithm::SHA256Sess,
            DigestAlgorithm::MD5,
            DigestAlgorithm::MD5Sess,
        ] {
            let ch = mk(alg);
            let computed = DigestClient::compute_response_with_state(
                "alice",
                "secret",
                &ch,
                "INVITE",
                "sip:bob@example.com",
                1,
                None,
            )
            .unwrap();
            let resp = DigestResponse {
                username: "alice".to_string(),
                realm: "example.com".to_string(),
                nonce: "fixed-nonce".to_string(),
                uri: "sip:bob@example.com".to_string(),
                response: computed.response,
                algorithm: alg,
                cnonce: computed.cnonce,
                qop: computed.qop,
                nc: computed.nc,
                opaque: None,
            };
            assert!(
                auth_plain
                    .validate_response(&resp, "INVITE", "secret")
                    .unwrap(),
                "algorithm {:?} failed self-validation",
                alg
            );
        }
    }

    #[test]
    fn auth_int_includes_body_in_ha2() {
        // Same inputs except body bytes. With auth-int negotiated,
        // the response must differ between bodies.
        let challenge = DigestChallenge {
            realm: "example.com".to_string(),
            nonce: "fixed-nonce".to_string(),
            algorithm: DigestAlgorithm::MD5,
            qop: Some(vec!["auth".to_string(), "auth-int".to_string()]),
            opaque: None,
        };

        let body_a = b"v=0\r\no=alice 1 1 IN IP4 1.2.3.4\r\n";
        let body_b = b"v=0\r\no=alice 2 2 IN IP4 5.6.7.8\r\n";

        let r_a = DigestClient::compute_response_with_state(
            "alice",
            "secret",
            &challenge,
            "INVITE",
            "sip:bob@example.com",
            1,
            Some(body_a),
        )
        .unwrap();
        let r_b = DigestClient::compute_response_with_state(
            "alice",
            "secret",
            &challenge,
            "INVITE",
            "sip:bob@example.com",
            1,
            Some(body_b),
        )
        .unwrap();

        assert_eq!(r_a.qop.as_deref(), Some("auth-int"));
        assert_eq!(r_b.qop.as_deref(), Some("auth-int"));
        assert_ne!(
            r_a.response, r_b.response,
            "auth-int must fold the body into HA2"
        );
    }

    #[test]
    fn qop_selector_prefers_auth_int_when_offered_with_body() {
        let challenge = DigestChallenge {
            realm: "example.com".to_string(),
            nonce: "fixed-nonce".to_string(),
            algorithm: DigestAlgorithm::MD5,
            qop: Some(vec!["auth".to_string(), "auth-int".to_string()]),
            opaque: None,
        };

        // With body present, auth-int wins.
        let r = DigestClient::compute_response_with_state(
            "alice",
            "secret",
            &challenge,
            "INVITE",
            "sip:bob@example.com",
            1,
            Some(b"sdp"),
        )
        .unwrap();
        assert_eq!(r.qop.as_deref(), Some("auth-int"));

        // Without body, auth wins (we don't pad an empty body for an
        // option the caller didn't ask for).
        let r2 = DigestClient::compute_response_with_state(
            "alice",
            "secret",
            &challenge,
            "INVITE",
            "sip:bob@example.com",
            1,
            None,
        )
        .unwrap();
        assert_eq!(r2.qop.as_deref(), Some("auth"));
    }

    #[test]
    fn format_authorization_with_state_emits_nc_from_computed() {
        let challenge = DigestChallenge {
            realm: "example.com".to_string(),
            nonce: "fixed-nonce".to_string(),
            algorithm: DigestAlgorithm::MD5,
            qop: Some(vec!["auth".to_string()]),
            opaque: None,
        };

        let computed = DigestClient::compute_response_with_state(
            "alice",
            "secret",
            &challenge,
            "REGISTER",
            "sip:reg.example.com",
            42,
            None,
        )
        .unwrap();

        let header = DigestClient::format_authorization_with_state(
            "alice",
            &challenge,
            "sip:reg.example.com",
            &computed,
        );
        assert!(header.contains("nc=0000002a"), "header was: {}", header);
        assert!(header.contains("qop=auth"));
        assert!(header.contains(r#"cnonce=""#));
    }

    #[test]
    fn legacy_compute_response_still_works() {
        let challenge = DigestChallenge {
            realm: "realm".to_string(),
            nonce: "nonce".to_string(),
            algorithm: DigestAlgorithm::MD5,
            qop: None,
            opaque: None,
        };

        let response = DigestClient::compute_response(
            "user",
            "password",
            &challenge,
            "REGISTER",
            "sip:registrar.example.com",
        )
        .unwrap();

        assert_eq!(response.0.len(), 32); // MD5 hex
        assert!(response.1.is_none()); // No qop, no cnonce.
    }

    #[test]
    fn parse_challenge_recognises_sha256_sess() {
        let header = r#"Digest realm="test", nonce="abc", algorithm=SHA-256-sess, qop="auth""#;
        let ch = DigestAuthenticator::parse_challenge(header).unwrap();
        assert_eq!(ch.algorithm, DigestAlgorithm::SHA256Sess);
    }

    #[test]
    fn test_generate_nonce() {
        let nonce1 = DigestAuthenticator::generate_nonce();
        let nonce2 = DigestAuthenticator::generate_nonce();
        assert_eq!(nonce1.len(), 32);
        assert_ne!(nonce1, nonce2);
    }

    #[test]
    fn test_format_www_authenticate() {
        let auth = DigestAuthenticator::new("testrealm");
        let challenge = DigestChallenge {
            realm: "testrealm".to_string(),
            nonce: "nonce123".to_string(),
            algorithm: DigestAlgorithm::MD5,
            qop: Some(vec!["auth".to_string()]),
            opaque: Some("opaque456".to_string()),
        };
        let header = auth.format_www_authenticate(&challenge);
        assert!(header.contains("Digest"));
        assert!(header.contains(r#"realm="testrealm""#));
        assert!(header.contains(r#"nonce="nonce123""#));
    }
}

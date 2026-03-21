//! # RFC 2617/7616 Digest Authentication Computation
//!
//! Implements the digest response calculation for SIP authentication,
//! supporting MD5, MD5-sess, SHA-256, and SHA-256-sess algorithms
//! with both `auth` and `auth-int` quality of protection modes.

use md5::Md5;
use sha2::Sha256;
use sha2::Digest as _;
use rand::Rng;

use crate::error::{Error, Result};
use crate::types::auth::{
    Algorithm, Authorization, Challenge, Credentials, DigestParam, Qop, WwwAuthenticate,
};
use crate::types::uri::Uri;

/// Compute the hex-encoded hash for the given algorithm.
fn hash_hex(algorithm: &Algorithm, data: &str) -> Result<String> {
    match algorithm {
        Algorithm::Md5 | Algorithm::Md5Sess => {
            let mut hasher = Md5::new();
            hasher.update(data.as_bytes());
            Ok(format!("{:x}", hasher.finalize()))
        }
        Algorithm::Sha256 | Algorithm::Sha256Sess => {
            let mut hasher = Sha256::new();
            hasher.update(data.as_bytes());
            Ok(format!("{:x}", hasher.finalize()))
        }
        other => Err(Error::InvalidInput(format!(
            "Unsupported digest algorithm: {}",
            other
        ))),
    }
}

/// Generate a random cnonce as a hex string.
fn generate_cnonce() -> String {
    let mut rng = rand::thread_rng();
    let bytes: [u8; 16] = rng.r#gen();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Compute the digest response hash per RFC 2617 / RFC 7616.
///
/// # Parameters
///
/// - `username` - The user's name in the specified realm
/// - `password` - The user's password
/// - `realm` - The authentication realm from the challenge
/// - `nonce` - The server nonce from the challenge
/// - `method` - The SIP method (e.g. "REGISTER", "INVITE")
/// - `uri` - The digest URI (usually the Request-URI)
/// - `qop` - Quality of protection, if any
/// - `nc` - Nonce count (required when qop is present)
/// - `cnonce` - Client nonce (required when qop is present)
/// - `algorithm` - The hash algorithm to use
/// - `body` - The message body (required for `auth-int` qop)
///
/// # Returns
///
/// The hex-encoded digest response string.
pub fn compute_digest_response(
    username: &str,
    password: &str,
    realm: &str,
    nonce: &str,
    method: &str,
    uri: &str,
    qop: Option<&Qop>,
    nc: u32,
    cnonce: &str,
    algorithm: &Algorithm,
    body: Option<&str>,
) -> Result<String> {
    // HA1 = H(username:realm:password)
    let ha1_base = hash_hex(algorithm, &format!("{}:{}:{}", username, realm, password))?;

    // For -sess variants: HA1 = H(H(username:realm:password):nonce:cnonce)
    let ha1 = match algorithm {
        Algorithm::Md5Sess | Algorithm::Sha256Sess => {
            hash_hex(algorithm, &format!("{}:{}:{}", ha1_base, nonce, cnonce))?
        }
        _ => ha1_base,
    };

    // HA2 depends on qop
    let ha2 = match qop {
        Some(Qop::AuthInt) => {
            let body_hash = hash_hex(algorithm, body.unwrap_or(""))?;
            hash_hex(algorithm, &format!("{}:{}:{}", method, uri, body_hash))?
        }
        _ => {
            // qop=auth or no qop: HA2 = H(method:uri)
            hash_hex(algorithm, &format!("{}:{}", method, uri))?
        }
    };

    // Final response depends on whether qop is present
    let response = match qop {
        Some(Qop::Auth) | Some(Qop::AuthInt) => {
            // response = H(HA1:nonce:nc:cnonce:qop:HA2)
            let qop_str = match qop {
                Some(q) => q.to_string(),
                None => String::new(),
            };
            hash_hex(
                algorithm,
                &format!(
                    "{}:{}:{:08x}:{}:{}:{}",
                    ha1, nonce, nc, cnonce, qop_str, ha2
                ),
            )?
        }
        _ => {
            // RFC 2069 compatibility (no qop): response = H(HA1:nonce:HA2)
            hash_hex(algorithm, &format!("{}:{}:{}", ha1, nonce, ha2))?
        }
    };

    Ok(response)
}

/// Extract digest parameters from a Challenge.
///
/// Returns (realm, nonce, opaque, algorithm, qop_options, stale).
fn extract_challenge_params(
    challenge: &Challenge,
) -> Result<(
    String,
    String,
    Option<String>,
    Algorithm,
    Vec<Qop>,
    bool,
)> {
    match challenge {
        Challenge::Digest { params } => {
            let mut realm = None;
            let mut nonce = None;
            let mut opaque = None;
            let mut algorithm = Algorithm::Md5; // default per RFC
            let mut qop_options = Vec::new();
            let mut stale = false;

            for param in params {
                match param {
                    DigestParam::Realm(r) => realm = Some(r.clone()),
                    DigestParam::Nonce(n) => nonce = Some(n.clone()),
                    DigestParam::Opaque(o) => opaque = Some(o.clone()),
                    DigestParam::Algorithm(a) => algorithm = a.clone(),
                    DigestParam::Qop(q) => qop_options = q.clone(),
                    DigestParam::Stale(s) => stale = *s,
                    _ => {}
                }
            }

            let realm = realm.ok_or_else(|| {
                Error::InvalidInput("Missing realm in digest challenge".to_string())
            })?;
            let nonce = nonce.ok_or_else(|| {
                Error::InvalidInput("Missing nonce in digest challenge".to_string())
            })?;

            Ok((realm, nonce, opaque, algorithm, qop_options, stale))
        }
        _ => Err(Error::InvalidInput(
            "Expected a Digest challenge".to_string(),
        )),
    }
}

/// Select the preferred Qop from the server-offered options.
///
/// Prefers `auth` over `auth-int` since body hashing is more expensive
/// and not always available. Returns `None` if no qop was offered.
fn select_qop(offered: &[Qop]) -> Option<Qop> {
    if offered.is_empty() {
        return None;
    }
    // Prefer auth, fall back to auth-int, then first offered
    if offered.contains(&Qop::Auth) {
        Some(Qop::Auth)
    } else if offered.contains(&Qop::AuthInt) {
        Some(Qop::AuthInt)
    } else {
        offered.first().cloned()
    }
}

/// Build an Authorization header in response to a WWW-Authenticate challenge.
///
/// This is a convenience function for one-shot authentication. For repeated
/// authentication with nonce-count tracking, use [`DigestAuthContext`] instead.
///
/// # Parameters
///
/// - `www_auth` - The WWW-Authenticate header from the 401/407 response
/// - `username` - The user's name
/// - `password` - The user's password
/// - `method` - The SIP method being authenticated (e.g. "REGISTER")
/// - `uri` - The Request-URI as a string
/// - `body` - Optional message body (needed for `auth-int`)
///
/// # Returns
///
/// An `Authorization` header ready to include in the authenticated request.
pub fn build_authorization_header(
    www_auth: &WwwAuthenticate,
    username: &str,
    password: &str,
    method: &str,
    uri: &str,
    body: Option<&str>,
) -> Result<Authorization> {
    let challenge = www_auth
        .first_digest()
        .ok_or_else(|| Error::InvalidInput("No Digest challenge found".to_string()))?;

    let (realm, nonce, opaque, algorithm, qop_options, _stale) =
        extract_challenge_params(challenge)?;

    let selected_qop = select_qop(&qop_options);
    let cnonce = generate_cnonce();
    let nc: u32 = 1;

    let response = compute_digest_response(
        username,
        password,
        &realm,
        &nonce,
        method,
        uri,
        selected_qop.as_ref(),
        nc,
        &cnonce,
        &algorithm,
        body,
    )?;

    let parsed_uri: Uri = uri
        .parse()
        .map_err(|_| Error::InvalidInput(format!("Invalid URI: {}", uri)))?;

    let mut auth = Authorization::new(
        crate::types::auth::AuthScheme::Digest,
        username,
        &realm,
        &nonce,
        parsed_uri,
        &response,
    )
    .with_algorithm(algorithm);

    if let Some(ref qop) = selected_qop {
        auth = auth
            .with_qop(qop.clone())
            .with_cnonce(&cnonce)
            .with_nonce_count(nc);
    }

    if let Some(opaque_val) = opaque {
        auth = auth.with_opaque(opaque_val);
    }

    Ok(auth)
}

/// Stateful context for SIP Digest Authentication.
///
/// Tracks the nonce count across multiple requests using the same nonce,
/// and handles `stale=true` challenges by recomputing with the new nonce
/// without requiring fresh credentials.
///
/// # Example
///
/// ```ignore
/// use rvoip_sip_core::auth::digest::DigestAuthContext;
///
/// let mut ctx = DigestAuthContext::new("alice", "secret123");
/// let auth = ctx.respond_to_challenge(&www_authenticate, "REGISTER", "sip:example.com", None)?;
/// ```
pub struct DigestAuthContext {
    username: String,
    password: String,
    /// Current nonce from server (updated on stale challenge)
    current_nonce: Option<String>,
    /// Nonce count, incremented per request with the same nonce
    nc: u32,
    /// Client nonce (regenerated when nonce changes)
    cnonce: String,
}

impl DigestAuthContext {
    /// Create a new digest authentication context.
    ///
    /// # Parameters
    ///
    /// - `username` - The SIP username
    /// - `password` - The SIP password
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
            current_nonce: None,
            nc: 0,
            cnonce: generate_cnonce(),
        }
    }

    /// Respond to a WWW-Authenticate or Proxy-Authenticate challenge.
    ///
    /// If the challenge contains `stale=true` and the nonce matches a previously
    /// seen nonce, the context recomputes using the new nonce without requiring
    /// the user to re-enter credentials.
    ///
    /// The nonce count is automatically incremented when reusing the same nonce,
    /// and reset to 1 when a new nonce is received.
    ///
    /// # Parameters
    ///
    /// - `www_auth` - The WWW-Authenticate header from the 401/407 response
    /// - `method` - The SIP method (e.g. "REGISTER", "INVITE")
    /// - `uri` - The Request-URI as a string
    /// - `body` - Optional message body (needed for `auth-int` qop)
    ///
    /// # Returns
    ///
    /// An `Authorization` header ready for the authenticated request.
    pub fn respond_to_challenge(
        &mut self,
        www_auth: &WwwAuthenticate,
        method: &str,
        uri: &str,
        body: Option<&str>,
    ) -> Result<Authorization> {
        let challenge = www_auth
            .first_digest()
            .ok_or_else(|| Error::InvalidInput("No Digest challenge found".to_string()))?;

        let (realm, nonce, opaque, algorithm, qop_options, stale) =
            extract_challenge_params(challenge)?;

        let selected_qop = select_qop(&qop_options);

        // Determine if we need a fresh nonce count
        let nonce_changed = self.current_nonce.as_deref() != Some(&nonce);
        if nonce_changed {
            // New nonce (or first challenge): reset counter and regenerate cnonce
            self.current_nonce = Some(nonce.clone());
            self.nc = 1;
            self.cnonce = generate_cnonce();
        } else if stale {
            // Stale nonce: server gave us a new nonce, reset counter
            self.current_nonce = Some(nonce.clone());
            self.nc = 1;
            self.cnonce = generate_cnonce();
        } else {
            // Same nonce, increment counter
            self.nc = self.nc.saturating_add(1);
        }

        let response = compute_digest_response(
            &self.username,
            &self.password,
            &realm,
            &nonce,
            method,
            uri,
            selected_qop.as_ref(),
            self.nc,
            &self.cnonce,
            &algorithm,
            body,
        )?;

        let parsed_uri: Uri = uri
            .parse()
            .map_err(|_| Error::InvalidInput(format!("Invalid URI: {}", uri)))?;

        let mut auth = Authorization::new(
            crate::types::auth::AuthScheme::Digest,
            &self.username,
            &realm,
            &nonce,
            parsed_uri,
            &response,
        )
        .with_algorithm(algorithm);

        if let Some(ref qop) = selected_qop {
            auth = auth
                .with_qop(qop.clone())
                .with_cnonce(&self.cnonce)
                .with_nonce_count(self.nc);
        }

        if let Some(opaque_val) = opaque {
            auth = auth.with_opaque(opaque_val);
        }

        Ok(auth)
    }

    /// Returns the current nonce count.
    pub fn nonce_count(&self) -> u32 {
        self.nc
    }

    /// Returns the username.
    pub fn username(&self) -> &str {
        &self.username
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 2617 Section 3.5 example values
    // username = "Mufasa", password = "Circle Of Life"
    // realm = "testrealm@host.com", nonce = "dcd98b7102dd2f0e8b11d0f600bfb0c093"
    // method = "GET", uri = "/dir/index.html"
    // Expected HA1 = 939e7578ed9e3c518a452acee763bce9
    // Expected HA2 = 39aff3a2bab6126f332b942af5e6afc3
    // Expected response (no qop) = 6629fae49393a05397450978507c4ef1

    #[test]
    fn test_md5_hash_hex() {
        let result = hash_hex(&Algorithm::Md5, "Mufasa:testrealm@host.com:Circle Of Life");
        assert!(result.is_ok());
        assert_eq!(result.unwrap_or_default(), "939e7578ed9e3c518a452acee763bce9");
    }

    #[test]
    fn test_sha256_hash_hex() {
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let result = hash_hex(&Algorithm::Sha256, "");
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap_or_default(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_unsupported_algorithm() {
        let result = hash_hex(&Algorithm::Sha512, "test");
        assert!(result.is_err());
    }

    #[test]
    fn test_compute_digest_no_qop_md5() {
        // Without qop: response = MD5(HA1:nonce:HA2)
        // HA1 = MD5("Mufasa:testrealm@host.com:Circle Of Life") = 939e7578ed9e3c518a452acee763bce9
        // HA2 = MD5("GET:/dir/index.html") = 39aff3a2bab6126f332b942af5e6afc3
        // response = MD5("939e7578ed9e3c518a452acee763bce9:dcd98b7102dd2f0e8b11d0f600bfb0c093:39aff3a2bab6126f332b942af5e6afc3")
        let response = compute_digest_response(
            "Mufasa",
            "Circle Of Life",
            "testrealm@host.com",
            "dcd98b7102dd2f0e8b11d0f600bfb0c093",
            "GET",
            "/dir/index.html",
            None, // no qop
            1,
            "",
            &Algorithm::Md5,
            None,
        );

        assert!(response.is_ok());
        assert_eq!(
            response.unwrap_or_default(),
            "670fd8c2df070c60b045671b8b24ff02"
        );
    }

    #[test]
    fn test_compute_digest_qop_auth_md5() {
        // With qop=auth: response = MD5(HA1:nonce:nc:cnonce:qop:HA2)
        // nc=00000001, cnonce="0a4f113b"
        let response = compute_digest_response(
            "Mufasa",
            "Circle Of Life",
            "testrealm@host.com",
            "dcd98b7102dd2f0e8b11d0f600bfb0c093",
            "GET",
            "/dir/index.html",
            Some(&Qop::Auth),
            1,
            "0a4f113b",
            &Algorithm::Md5,
            None,
        );

        assert!(response.is_ok());
        let resp = response.unwrap_or_default();
        assert_eq!(resp.len(), 32); // MD5 produces 32 hex chars
        assert_eq!(resp, "6629fae49393a05397450978507c4ef1");
    }

    #[test]
    fn test_compute_digest_qop_auth_sha256() {
        // Verify SHA-256 produces a different (longer) hash than MD5
        let response = compute_digest_response(
            "Mufasa",
            "Circle Of Life",
            "testrealm@host.com",
            "dcd98b7102dd2f0e8b11d0f600bfb0c093",
            "GET",
            "/dir/index.html",
            Some(&Qop::Auth),
            1,
            "0a4f113b",
            &Algorithm::Sha256,
            None,
        );

        assert!(response.is_ok());
        let resp = response.unwrap_or_default();
        // SHA-256 response is 64 hex chars
        assert_eq!(resp.len(), 64);
    }

    #[test]
    fn test_compute_digest_md5_sess() {
        // MD5-sess: HA1 = MD5(MD5(user:realm:pass):nonce:cnonce)
        let response = compute_digest_response(
            "Mufasa",
            "Circle Of Life",
            "testrealm@host.com",
            "dcd98b7102dd2f0e8b11d0f600bfb0c093",
            "GET",
            "/dir/index.html",
            Some(&Qop::Auth),
            1,
            "0a4f113b",
            &Algorithm::Md5Sess,
            None,
        );
        assert!(response.is_ok());
        let resp = response.unwrap_or_default();
        // MD5 response is 32 hex chars
        assert_eq!(resp.len(), 32);
        // Should differ from non-sess variant
        assert_ne!(resp, "6629fae49394a05397450978507c4ef1");
    }

    #[test]
    fn test_compute_digest_auth_int() {
        // auth-int: HA2 = MD5(method:uri:MD5(body))
        let response_with_body = compute_digest_response(
            "alice",
            "password",
            "biloxi.com",
            "abc123",
            "REGISTER",
            "sip:biloxi.com",
            Some(&Qop::AuthInt),
            1,
            "xyz789",
            &Algorithm::Md5,
            Some("some body content"),
        );
        assert!(response_with_body.is_ok());

        let response_empty_body = compute_digest_response(
            "alice",
            "password",
            "biloxi.com",
            "abc123",
            "REGISTER",
            "sip:biloxi.com",
            Some(&Qop::AuthInt),
            1,
            "xyz789",
            &Algorithm::Md5,
            None,
        );
        assert!(response_empty_body.is_ok());

        // Different body should produce different response
        assert_ne!(
            response_with_body.unwrap_or_default(),
            response_empty_body.unwrap_or_default()
        );
    }

    #[test]
    fn test_compute_digest_sip_register_md5() {
        // Typical SIP REGISTER scenario
        let response = compute_digest_response(
            "bob",
            "zanzibar",
            "biloxi.com",
            "ea9c8e88df84f1cec4341ae6cbe5a359",
            "REGISTER",
            "sip:biloxi.com",
            Some(&Qop::Auth),
            1,
            "ab29fcc3",
            &Algorithm::Md5,
            None,
        );
        assert!(response.is_ok());
        let resp = response.unwrap_or_default();
        assert_eq!(resp.len(), 32); // MD5 hex output is 32 chars
    }

    #[test]
    fn test_build_authorization_header_basic() {
        let www_auth = WwwAuthenticate::new("biloxi.com", "abc123nonce")
            .with_algorithm(Algorithm::Md5)
            .with_qop(Qop::Auth);

        let result = build_authorization_header(
            &www_auth,
            "bob",
            "zanzibar",
            "REGISTER",
            "sip:biloxi.com",
            None,
        );

        assert!(result.is_ok());
        let auth = match result {
            Ok(a) => a,
            Err(e) => panic!("Expected Ok, got Err: {}", e),
        };

        let display = auth.to_string();
        assert!(display.contains("Digest"));
        assert!(display.contains("username=\"bob\""));
        assert!(display.contains("realm=\"biloxi.com\""));
        assert!(display.contains("nonce=\"abc123nonce\""));
        assert!(display.contains("response="));
        assert!(display.contains("algorithm=MD5"));
        assert!(display.contains("qop=auth"));
        assert!(display.contains("nc=00000001"));
        assert!(display.contains("cnonce="));
    }

    #[test]
    fn test_build_authorization_header_no_qop() {
        let www_auth = WwwAuthenticate::new("atlanta.com", "serverNonce42");

        let result = build_authorization_header(
            &www_auth,
            "alice",
            "secret",
            "INVITE",
            "sip:bob@atlanta.com",
            None,
        );

        assert!(result.is_ok());
        let auth = match result {
            Ok(a) => a,
            Err(e) => panic!("Expected Ok, got Err: {}", e),
        };

        let display = auth.to_string();
        assert!(display.contains("Digest"));
        assert!(display.contains("username=\"alice\""));
        // Without qop, there should be no nc or cnonce
        assert!(!display.contains("nc="));
        assert!(!display.contains("cnonce="));
    }

    #[test]
    fn test_build_authorization_header_with_opaque() {
        let www_auth = WwwAuthenticate::new("example.com", "nonce123")
            .with_opaque("opaque456")
            .with_qop(Qop::Auth);

        let result = build_authorization_header(
            &www_auth,
            "user",
            "pass",
            "REGISTER",
            "sip:example.com",
            None,
        );

        assert!(result.is_ok());
        let display = result
            .map(|a| a.to_string())
            .unwrap_or_default();
        assert!(display.contains("opaque=\"opaque456\""));
    }

    #[test]
    fn test_build_authorization_header_no_digest_challenge() {
        let www_auth = WwwAuthenticate::new_basic("example.com");

        let result = build_authorization_header(
            &www_auth,
            "user",
            "pass",
            "REGISTER",
            "sip:example.com",
            None,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_digest_auth_context_first_challenge() {
        let www_auth = WwwAuthenticate::new("biloxi.com", "nonce1")
            .with_algorithm(Algorithm::Md5)
            .with_qop(Qop::Auth);

        let mut ctx = DigestAuthContext::new("bob", "zanzibar");
        let result = ctx.respond_to_challenge(&www_auth, "REGISTER", "sip:biloxi.com", None);

        assert!(result.is_ok());
        assert_eq!(ctx.nonce_count(), 1);
    }

    #[test]
    fn test_digest_auth_context_increments_nc() {
        let www_auth = WwwAuthenticate::new("biloxi.com", "nonce1")
            .with_algorithm(Algorithm::Md5)
            .with_qop(Qop::Auth);

        let mut ctx = DigestAuthContext::new("bob", "zanzibar");

        // First challenge
        let _ = ctx.respond_to_challenge(&www_auth, "REGISTER", "sip:biloxi.com", None);
        assert_eq!(ctx.nonce_count(), 1);

        // Same nonce -> nc should increment
        let _ = ctx.respond_to_challenge(&www_auth, "REGISTER", "sip:biloxi.com", None);
        assert_eq!(ctx.nonce_count(), 2);

        let _ = ctx.respond_to_challenge(&www_auth, "REGISTER", "sip:biloxi.com", None);
        assert_eq!(ctx.nonce_count(), 3);
    }

    #[test]
    fn test_digest_auth_context_new_nonce_resets_nc() {
        let www_auth1 = WwwAuthenticate::new("biloxi.com", "nonce1")
            .with_qop(Qop::Auth);
        let www_auth2 = WwwAuthenticate::new("biloxi.com", "nonce2")
            .with_qop(Qop::Auth);

        let mut ctx = DigestAuthContext::new("bob", "zanzibar");

        let _ = ctx.respond_to_challenge(&www_auth1, "REGISTER", "sip:biloxi.com", None);
        let _ = ctx.respond_to_challenge(&www_auth1, "REGISTER", "sip:biloxi.com", None);
        assert_eq!(ctx.nonce_count(), 2);

        // New nonce -> nc resets to 1
        let _ = ctx.respond_to_challenge(&www_auth2, "REGISTER", "sip:biloxi.com", None);
        assert_eq!(ctx.nonce_count(), 1);
    }

    #[test]
    fn test_digest_auth_context_stale_resets_nc() {
        let www_auth_stale = WwwAuthenticate::new("biloxi.com", "nonce_fresh")
            .with_qop(Qop::Auth)
            .with_stale(true);

        let mut ctx = DigestAuthContext::new("bob", "zanzibar");

        // Simulate a stale=true response: nc should reset
        let result = ctx.respond_to_challenge(&www_auth_stale, "REGISTER", "sip:biloxi.com", None);
        assert!(result.is_ok());
        assert_eq!(ctx.nonce_count(), 1);
    }

    #[test]
    fn test_digest_auth_context_username() {
        let ctx = DigestAuthContext::new("alice", "wonderland");
        assert_eq!(ctx.username(), "alice");
    }

    #[test]
    fn test_generate_cnonce_length() {
        let cnonce = generate_cnonce();
        // 16 bytes -> 32 hex chars
        assert_eq!(cnonce.len(), 32);
    }

    #[test]
    fn test_generate_cnonce_uniqueness() {
        let c1 = generate_cnonce();
        let c2 = generate_cnonce();
        // Extremely unlikely to be equal
        assert_ne!(c1, c2);
    }

    #[test]
    fn test_select_qop_prefers_auth() {
        let offered = vec![Qop::AuthInt, Qop::Auth];
        assert_eq!(select_qop(&offered), Some(Qop::Auth));
    }

    #[test]
    fn test_select_qop_auth_int_only() {
        let offered = vec![Qop::AuthInt];
        assert_eq!(select_qop(&offered), Some(Qop::AuthInt));
    }

    #[test]
    fn test_select_qop_empty() {
        let offered: Vec<Qop> = vec![];
        assert_eq!(select_qop(&offered), None);
    }

    #[test]
    fn test_extract_challenge_params_complete() {
        let challenge = Challenge::Digest {
            params: vec![
                DigestParam::Realm("example.com".to_string()),
                DigestParam::Nonce("abc".to_string()),
                DigestParam::Opaque("xyz".to_string()),
                DigestParam::Algorithm(Algorithm::Sha256),
                DigestParam::Qop(vec![Qop::Auth, Qop::AuthInt]),
                DigestParam::Stale(true),
            ],
        };

        let result = extract_challenge_params(&challenge);
        assert!(result.is_ok());
        let (realm, nonce, opaque, algorithm, qop_options, stale) = result.unwrap_or_else(|_| {
            (String::new(), String::new(), None, Algorithm::Md5, vec![], false)
        });
        assert_eq!(realm, "example.com");
        assert_eq!(nonce, "abc");
        assert_eq!(opaque, Some("xyz".to_string()));
        assert_eq!(algorithm, Algorithm::Sha256);
        assert_eq!(qop_options.len(), 2);
        assert!(stale);
    }

    #[test]
    fn test_extract_challenge_params_missing_realm() {
        let challenge = Challenge::Digest {
            params: vec![DigestParam::Nonce("abc".to_string())],
        };
        let result = extract_challenge_params(&challenge);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_challenge_params_missing_nonce() {
        let challenge = Challenge::Digest {
            params: vec![DigestParam::Realm("example.com".to_string())],
        };
        let result = extract_challenge_params(&challenge);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_challenge_params_non_digest() {
        let challenge = Challenge::Basic {
            params: vec![],
        };
        let result = extract_challenge_params(&challenge);
        assert!(result.is_err());
    }

    #[test]
    fn test_compute_digest_deterministic() {
        // Same inputs should always produce the same output
        let r1 = compute_digest_response(
            "user", "pass", "realm", "nonce", "GET", "/path",
            Some(&Qop::Auth), 1, "cnonce", &Algorithm::Md5, None,
        );
        let r2 = compute_digest_response(
            "user", "pass", "realm", "nonce", "GET", "/path",
            Some(&Qop::Auth), 1, "cnonce", &Algorithm::Md5, None,
        );
        assert_eq!(r1.unwrap_or_default(), r2.unwrap_or_default());
    }

    #[test]
    fn test_different_nc_produces_different_response() {
        let r1 = compute_digest_response(
            "user", "pass", "realm", "nonce", "GET", "/path",
            Some(&Qop::Auth), 1, "cnonce", &Algorithm::Md5, None,
        );
        let r2 = compute_digest_response(
            "user", "pass", "realm", "nonce", "GET", "/path",
            Some(&Qop::Auth), 2, "cnonce", &Algorithm::Md5, None,
        );
        assert_ne!(r1.unwrap_or_default(), r2.unwrap_or_default());
    }

    #[test]
    fn test_sha256_sess_variant() {
        let response = compute_digest_response(
            "alice",
            "password",
            "example.com",
            "nonce123",
            "REGISTER",
            "sip:example.com",
            Some(&Qop::Auth),
            1,
            "cnonce456",
            &Algorithm::Sha256Sess,
            None,
        );
        assert!(response.is_ok());
        let resp = response.unwrap_or_default();
        // SHA-256 hex output is 64 chars
        assert_eq!(resp.len(), 64);
    }
}

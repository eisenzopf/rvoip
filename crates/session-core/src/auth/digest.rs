//! SIP Digest Authentication (RFC 2617 / RFC 7616)
//!
//! Computes the digest response for SIP authentication challenges.
//! This module handles the client-side computation needed to respond
//! to WWW-Authenticate and Proxy-Authenticate challenges.

use md5::{Md5, Digest as Md5Digest};
use sha2::{Sha256, Digest as Sha256Digest};
use rvoip_sip_core::types::auth::{
    Challenge, DigestParam, Authorization, ProxyAuthorization,
    Credentials, AuthScheme,
};
use rvoip_sip_core::{Algorithm, Qop, Uri};
use crate::errors::{Result, SessionError};

/// Parameters extracted from a digest authentication challenge
#[derive(Debug, Clone)]
pub struct DigestChallenge {
    /// Authentication realm
    pub realm: String,
    /// Server nonce
    pub nonce: String,
    /// Opaque value (must be returned unchanged)
    pub opaque: Option<String>,
    /// Algorithm to use (defaults to MD5)
    pub algorithm: Algorithm,
    /// Quality of protection options
    pub qop_options: Vec<Qop>,
    /// Whether the nonce is stale
    pub stale: bool,
}

/// Credentials used for digest computation
#[derive(Debug, Clone)]
pub struct DigestCredentials {
    pub username: String,
    pub password: String,
}

/// Compute a hex-encoded hash using the specified algorithm
fn hex_hash(algorithm: &Algorithm, data: &str) -> String {
    match algorithm {
        Algorithm::Sha256 | Algorithm::Sha256Sess => {
            let mut hasher = Sha256::new();
            hasher.update(data.as_bytes());
            format!("{:x}", hasher.finalize())
        }
        // Default to MD5 for MD5, MD5-sess, and any other algorithm
        _ => {
            let mut hasher = Md5::new();
            hasher.update(data.as_bytes());
            format!("{:x}", hasher.finalize())
        }
    }
}

/// Extract digest parameters from a Challenge
pub fn extract_challenge(challenge: &Challenge) -> Result<DigestChallenge> {
    match challenge {
        Challenge::Digest { params } => {
            let mut realm = None;
            let mut nonce = None;
            let mut opaque = None;
            let mut algorithm = Algorithm::Md5;
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
                SessionError::ProtocolError {
                    message: "Digest challenge missing realm parameter".to_string(),
                }
            })?;
            let nonce = nonce.ok_or_else(|| {
                SessionError::ProtocolError {
                    message: "Digest challenge missing nonce parameter".to_string(),
                }
            })?;

            Ok(DigestChallenge {
                realm,
                nonce,
                opaque,
                algorithm,
                qop_options,
                stale,
            })
        }
        _ => Err(SessionError::ProtocolError {
            message: "Expected Digest challenge, got different scheme".to_string(),
        }),
    }
}

/// Compute digest authentication response per RFC 2617 / RFC 7616
///
/// # Arguments
/// * `credentials` - Username and password
/// * `challenge` - Extracted challenge parameters
/// * `method` - SIP method (e.g., "REGISTER")
/// * `digest_uri` - The request URI used in the digest computation
/// * `cnonce` - Client-generated nonce (required if qop is used)
/// * `nonce_count` - Nonce usage count (required if qop is used)
pub fn compute_digest_response(
    credentials: &DigestCredentials,
    challenge: &DigestChallenge,
    method: &str,
    digest_uri: &str,
    cnonce: Option<&str>,
    nonce_count: u32,
) -> String {
    let alg = &challenge.algorithm;

    // HA1 = H(username:realm:password)
    let ha1_input = format!("{}:{}:{}", credentials.username, challenge.realm, credentials.password);
    let ha1 = hex_hash(alg, &ha1_input);

    // For -sess variants, HA1 = H(H(username:realm:password):nonce:cnonce)
    let ha1 = match alg {
        Algorithm::Md5Sess | Algorithm::Sha256Sess | Algorithm::Sha512Sess => {
            let cnonce_val = cnonce.unwrap_or("");
            let sess_input = format!("{}:{}:{}", ha1, challenge.nonce, cnonce_val);
            hex_hash(alg, &sess_input)
        }
        _ => ha1,
    };

    // HA2 = H(method:digestURI)
    let ha2_input = format!("{}:{}", method, digest_uri);
    let ha2 = hex_hash(alg, &ha2_input);

    // Select qop to use (prefer "auth" if available)
    let selected_qop = challenge.qop_options.iter().find(|q| matches!(q, Qop::Auth));

    // Compute response
    if let Some(_qop) = selected_qop {
        // RFC 2617 with qop: response = H(HA1:nonce:nc:cnonce:qop:HA2)
        let cnonce_val = cnonce.unwrap_or("");
        let response_input = format!(
            "{}:{}:{:08x}:{}:auth:{}",
            ha1, challenge.nonce, nonce_count, cnonce_val, ha2
        );
        hex_hash(alg, &response_input)
    } else {
        // RFC 2069 (no qop): response = H(HA1:nonce:HA2)
        let response_input = format!("{}:{}:{}", ha1, challenge.nonce, ha2);
        hex_hash(alg, &response_input)
    }
}

/// Build an Authorization header from challenge and credentials
pub fn build_authorization(
    credentials: &DigestCredentials,
    challenge: &DigestChallenge,
    method: &str,
    request_uri: &Uri,
) -> Result<Authorization> {
    let cnonce = format!("{:016x}", rand::random::<u64>());
    let nonce_count: u32 = 1;
    let digest_uri = request_uri.to_string();

    let response = compute_digest_response(
        credentials,
        challenge,
        method,
        &digest_uri,
        Some(&cnonce),
        nonce_count,
    );

    let mut auth = Authorization::new(
        AuthScheme::Digest,
        &credentials.username,
        &challenge.realm,
        &challenge.nonce,
        request_uri.clone(),
        &response,
    )
    .with_algorithm(challenge.algorithm.clone());

    // Add qop-related parameters if qop was used
    let selected_qop = challenge.qop_options.iter().find(|q| matches!(q, Qop::Auth));
    if selected_qop.is_some() {
        auth = auth
            .with_qop(Qop::Auth)
            .with_cnonce(&cnonce)
            .with_nonce_count(nonce_count);
    }

    // Return opaque if the server sent it
    if let Some(ref opaque) = challenge.opaque {
        auth = auth.with_opaque(opaque);
    }

    Ok(auth)
}

/// Build a Proxy-Authorization header from challenge and credentials
pub fn build_proxy_authorization(
    credentials: &DigestCredentials,
    challenge: &DigestChallenge,
    method: &str,
    request_uri: &Uri,
) -> Result<ProxyAuthorization> {
    let auth = build_authorization(credentials, challenge, method, request_uri)?;
    // ProxyAuthorization wraps the same Credentials as Authorization
    Ok(ProxyAuthorization::new(auth.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_digest_response_md5_no_qop() {
        let creds = DigestCredentials {
            username: "Mufasa".to_string(),
            password: "Circle Of Life".to_string(),
        };
        let challenge = DigestChallenge {
            realm: "testrealm@host.com".to_string(),
            nonce: "dcd98b7102dd2f0e8b11d0f600bfb0c093".to_string(),
            opaque: None,
            algorithm: Algorithm::Md5,
            qop_options: vec![],
            stale: false,
        };

        let response = compute_digest_response(
            &creds,
            &challenge,
            "GET",
            "/dir/index.html",
            None,
            1,
        );

        // HA1 = MD5("Mufasa:testrealm@host.com:Circle Of Life")
        // HA2 = MD5("GET:/dir/index.html")
        // response = MD5(HA1:nonce:HA2) -- no qop
        assert!(!response.is_empty());
        assert_eq!(response.len(), 32); // MD5 hex is 32 chars
    }

    #[test]
    fn test_compute_digest_response_md5_with_qop() {
        let creds = DigestCredentials {
            username: "alice".to_string(),
            password: "password123".to_string(),
        };
        let challenge = DigestChallenge {
            realm: "example.com".to_string(),
            nonce: "abc123".to_string(),
            opaque: Some("opaque_val".to_string()),
            algorithm: Algorithm::Md5,
            qop_options: vec![Qop::Auth],
            stale: false,
        };

        let response = compute_digest_response(
            &creds,
            &challenge,
            "REGISTER",
            "sip:example.com",
            Some("cnonce_val"),
            1,
        );

        assert!(!response.is_empty());
        assert_eq!(response.len(), 32);
    }

    #[test]
    fn test_extract_challenge() {
        let challenge = Challenge::Digest {
            params: vec![
                DigestParam::Realm("example.com".to_string()),
                DigestParam::Nonce("abc123".to_string()),
                DigestParam::Algorithm(Algorithm::Md5),
                DigestParam::Qop(vec![Qop::Auth]),
            ],
        };

        let extracted = extract_challenge(&challenge);
        assert!(extracted.is_ok());
        let extracted = extracted.unwrap_or_else(|e| panic!("Failed: {}", e));
        assert_eq!(extracted.realm, "example.com");
        assert_eq!(extracted.nonce, "abc123");
        assert_eq!(extracted.algorithm, Algorithm::Md5);
        assert_eq!(extracted.qop_options, vec![Qop::Auth]);
    }
}

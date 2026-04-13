//! SIP Digest Authentication per RFC 2617 and RFC 3261
//!
//! This module provides SIP Digest authentication functionality for both
//! client and server implementations. It supports MD5 algorithm and can
//! be extended to support SHA-256 (RFC 7616).

use crate::error::{AuthError, Result};
use hex;
use rand::Rng;
use std::time::{SystemTime, UNIX_EPOCH};

/// Digest authentication algorithm
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestAlgorithm {
    /// MD5 algorithm (RFC 2617)
    MD5,
    /// SHA-256 algorithm (RFC 7616) - Future support
    SHA256,
}

impl DigestAlgorithm {
    pub fn as_str(&self) -> &'static str {
        match self {
            DigestAlgorithm::MD5 => "MD5",
            DigestAlgorithm::SHA256 => "SHA-256",
        }
    }
}

impl std::fmt::Display for DigestAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Digest challenge issued by server (401/407 response)
#[derive(Debug, Clone)]
pub struct DigestChallenge {
    pub realm: String,
    pub nonce: String,
    pub algorithm: DigestAlgorithm,
    pub qop: Option<Vec<String>>,  // "auth", "auth-int"
    pub opaque: Option<String>,
}

/// Digest response from client (Authorization header)
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

/// SIP Digest authenticator for generating challenges and validating responses
pub struct DigestAuthenticator {
    realm: String,
    algorithm: DigestAlgorithm,
}

impl DigestAuthenticator {
    /// Create a new digest authenticator with specified realm
    pub fn new(realm: impl Into<String>) -> Self {
        Self {
            realm: realm.into(),
            algorithm: DigestAlgorithm::MD5,
        }
    }

    /// Generate a new authentication challenge
    pub fn generate_challenge(&self) -> DigestChallenge {
        DigestChallenge {
            realm: self.realm.clone(),
            nonce: Self::generate_nonce(),
            algorithm: self.algorithm,
            qop: Some(vec!["auth".to_string()]),
            opaque: Some(Self::generate_opaque()),
        }
    }

    /// Format challenge as WWW-Authenticate header value
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

    /// Validate a digest response against stored password
    pub fn validate_response(
        &self,
        response: &DigestResponse,
        method: &str,
        password: &str,
    ) -> Result<bool> {
        // Compute expected response
        let ha1 = self.compute_ha1(&response.username, &response.realm, password);
        let ha2 = self.compute_ha2(method, &response.uri, response.qop.as_deref());
        
        tracing::info!("🔍 SERVER: Validating digest: method={}, uri={}", method, response.uri);
        tracing::info!("🔍 SERVER: HA1 inputs: username={}, realm={}, password={}", 
                       response.username, response.realm, password);
        tracing::info!("🔍 SERVER: Computed HA1: {}", ha1);
        tracing::info!("🔍 SERVER: Computed HA2: {}", ha2);
        
        let expected = if let (Some(ref qop), Some(ref nc), Some(ref cnonce)) = 
            (&response.qop, &response.nc, &response.cnonce) {
            // With qop
            let exp = self.compute_response_with_qop(&ha1, &response.nonce, nc, cnonce, qop, &ha2);
            tracing::info!("🔍 SERVER: Computed expected (with qop={}): {}", qop, exp);
            exp
        } else {
            // Without qop (legacy)
            let exp = self.compute_response(&ha1, &response.nonce, &ha2);
            tracing::info!("🔍 SERVER: Computed expected (no qop): {}", exp);
            exp
        };
        
        tracing::info!("🔍 SERVER: Client sent: {}", response.response);
        tracing::info!("🔍 SERVER: Expected:    {}", expected);
        tracing::info!("🔍 SERVER: Match: {}", expected == response.response);

        Ok(expected == response.response)
    }

    /// Parse WWW-Authenticate header to extract challenge
    pub fn parse_challenge(header: &str) -> Result<DigestChallenge> {
        let header = header.trim();
        
        // Remove "Digest " prefix
        let params_str = if header.starts_with("Digest ") {
            &header[7..]
        } else if header.starts_with("digest ") {
            &header[7..]
        } else {
            return Err(AuthError::InvalidChallenge("Missing 'Digest' prefix".into()));
        };

        // Parse key="value" pairs
        let mut realm = None;
        let mut nonce = None;
        let mut algorithm = DigestAlgorithm::MD5;
        let mut qop = None;
        let mut opaque = None;

        for param in params_str.split(',') {
            let param = param.trim();
            if let Some((key, value)) = param.split_once('=') {
                let key = key.trim();
                let value = value.trim().trim_matches('"');

                match key {
                    "realm" => realm = Some(value.to_string()),
                    "nonce" => nonce = Some(value.to_string()),
                    "algorithm" => {
                        algorithm = match value {
                            "MD5" => DigestAlgorithm::MD5,
                            "SHA-256" => DigestAlgorithm::SHA256,
                            _ => DigestAlgorithm::MD5,
                        };
                    }
                    "qop" => {
                        qop = Some(value.split(',').map(|s| s.trim().to_string()).collect());
                    }
                    "opaque" => opaque = Some(value.to_string()),
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

    /// Parse Authorization header to extract response
    pub fn parse_authorization(header: &str) -> Result<DigestResponse> {
        let header = header.trim();
        
        // Remove "Digest " prefix
        let params_str = if header.starts_with("Digest ") {
            &header[7..]
        } else if header.starts_with("digest ") {
            &header[7..]
        } else {
            return Err(AuthError::InvalidResponse("Missing 'Digest' prefix".into()));
        };

        // Parse key="value" pairs
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

        for param in params_str.split(',') {
            let param = param.trim();
            if let Some((key, value)) = param.split_once('=') {
                let key = key.trim();
                let value = value.trim().trim_matches('"');

                match key {
                    "username" => username = Some(value.to_string()),
                    "realm" => realm = Some(value.to_string()),
                    "nonce" => nonce = Some(value.to_string()),
                    "uri" => uri = Some(value.to_string()),
                    "response" => response = Some(value.to_string()),
                    "algorithm" => {
                        algorithm = match value {
                            "MD5" => DigestAlgorithm::MD5,
                            "SHA-256" => DigestAlgorithm::SHA256,
                            _ => DigestAlgorithm::MD5,
                        };
                    }
                    "cnonce" => cnonce = Some(value.to_string()),
                    "qop" => qop = Some(value.to_string()),
                    "nc" => nc = Some(value.to_string()),
                    "opaque" => opaque = Some(value.to_string()),
                    _ => {}
                }
            }
        }

        Ok(DigestResponse {
            username: username.ok_or_else(|| AuthError::InvalidResponse("Missing username".into()))?,
            realm: realm.ok_or_else(|| AuthError::InvalidResponse("Missing realm".into()))?,
            nonce: nonce.ok_or_else(|| AuthError::InvalidResponse("Missing nonce".into()))?,
            uri: uri.ok_or_else(|| AuthError::InvalidResponse("Missing uri".into()))?,
            response: response.ok_or_else(|| AuthError::InvalidResponse("Missing response".into()))?,
            algorithm,
            cnonce,
            qop,
            nc,
            opaque,
        })
    }

    /// Compute HA1 = MD5(username:realm:password)
    fn compute_ha1(&self, username: &str, realm: &str, password: &str) -> String {
        let data = format!("{}:{}:{}", username, realm, password);
        let digest = md5::compute(data.as_bytes());
        hex::encode(&digest[..])
    }

    /// Compute HA2 = MD5(method:uri) or MD5(method:uri:body) for qop=auth-int
    fn compute_ha2(&self, method: &str, uri: &str, qop: Option<&str>) -> String {
        let data = match qop {
            Some("auth-int") => {
                // For auth-int, we'd need the request body
                // For now, just use empty body
                format!("{}:{}:{}", method, uri, "")
            }
            _ => format!("{}:{}", method, uri),
        };
        
        let digest = md5::compute(data.as_bytes());
        hex::encode(&digest[..])
    }

    /// Compute response = MD5(HA1:nonce:HA2) (without qop)
    fn compute_response(&self, ha1: &str, nonce: &str, ha2: &str) -> String {
        let data = format!("{}:{}:{}", ha1, nonce, ha2);
        let digest = md5::compute(data.as_bytes());
        hex::encode(&digest[..])
    }

    /// Compute response = MD5(HA1:nonce:nc:cnonce:qop:HA2) (with qop)
    fn compute_response_with_qop(
        &self,
        ha1: &str,
        nonce: &str,
        nc: &str,
        cnonce: &str,
        qop: &str,
        ha2: &str,
    ) -> String {
        let data = format!("{}:{}:{}:{}:{}:{}", ha1, nonce, nc, cnonce, qop, ha2);
        let digest = md5::compute(data.as_bytes());
        hex::encode(&digest[..])
    }

    /// Generate a secure random nonce
    fn generate_nonce() -> String {
        let mut rng = rand::thread_rng();
        let random_bytes: [u8; 16] = rng.gen();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        let data = format!("{}{}", timestamp, hex::encode(random_bytes));
        let digest = md5::compute(data.as_bytes());
        hex::encode(&digest[..])
    }

    /// Generate opaque value
    fn generate_opaque() -> String {
        let mut rng = rand::thread_rng();
        let random_bytes: [u8; 16] = rng.gen();
        hex::encode(random_bytes)
    }
}

/// Client-side digest authentication helper
pub struct DigestClient;

impl DigestClient {
    /// Compute digest response for a challenge
    /// Returns (response_hash, optional_cnonce)
    pub fn compute_response(
        username: &str,
        password: &str,
        challenge: &DigestChallenge,
        method: &str,
        uri: &str,
    ) -> Result<(String, Option<String>)> {
        let ha1 = Self::compute_ha1(username, &challenge.realm, password);
        let ha2 = Self::compute_ha2(method, uri);

        tracing::info!("🔍 CLIENT: HA1 inputs: username={}, realm={}, password={}", username, challenge.realm, password);
        tracing::info!("🔍 CLIENT: Computed HA1: {}", ha1);
        tracing::info!("🔍 CLIENT: Computed HA2: {} (method={}, uri={})", ha2, method, uri);

        // Check if we need qop
        if let Some(ref qop_options) = challenge.qop {
            if qop_options.contains(&"auth".to_string()) {
                // Use qop=auth
                let nc = "00000001";
                let cnonce = Self::generate_cnonce();
                tracing::info!("🔍 CLIENT: Using qop=auth with nc={}, cnonce={}", nc, cnonce);
                let response = Self::compute_response_with_qop(
                    &ha1,
                    &challenge.nonce,
                    nc,
                    &cnonce,
                    "auth",
                    &ha2,
                );
                tracing::info!("🔍 CLIENT: Computed response (with qop): {}", response);
                return Ok((response, Some(cnonce)));  // Return cnonce!
            }
        }

        // Legacy mode without qop
        tracing::info!("🔍 CLIENT: Using legacy mode (no qop)");
        let data = format!("{}:{}:{}", ha1, challenge.nonce, ha2);
        let digest = md5::compute(data.as_bytes());
        let result = hex::encode(&digest[..]);
        tracing::info!("🔍 CLIENT: Computed response (no qop): {}", result);
        Ok((result, None))  // No cnonce in legacy mode
    }

    /// Format Authorization header
    pub fn format_authorization(
        username: &str,
        challenge: &DigestChallenge,
        uri: &str,
        response: &str,
        cnonce: Option<&str>,  // Use the cnonce from compute_response!
    ) -> String {
        let mut parts = vec![
            format!(r#"username="{}""#, username),
            format!(r#"realm="{}""#, challenge.realm),
            format!(r#"nonce="{}""#, challenge.nonce),
            format!(r#"uri="{}""#, uri),
            format!(r#"response="{}""#, response),
            format!(r#"algorithm={}"#, challenge.algorithm),
        ];

        // Add qop-related fields if present
        if let Some(ref qop_options) = challenge.qop {
            if qop_options.contains(&"auth".to_string()) {
                parts.push(r#"qop=auth"#.to_string());
                parts.push(r#"nc=00000001"#.to_string());
                // Use the cnonce that was used in computation, not a new one!
                if let Some(cn) = cnonce {
                    parts.push(format!(r#"cnonce="{}""#, cn));
                } else {
                    // Fallback if cnonce not provided (shouldn't happen)
                    parts.push(format!(r#"cnonce="{}""#, Self::generate_cnonce()));
                }
            }
        }

        if let Some(ref opaque) = challenge.opaque {
            parts.push(format!(r#"opaque="{}""#, opaque));
        }

        format!("Digest {}", parts.join(", "))
    }

    fn compute_ha1(username: &str, realm: &str, password: &str) -> String {
        let data = format!("{}:{}:{}", username, realm, password);
        let digest = md5::compute(data.as_bytes());
        hex::encode(&digest[..])
    }

    fn compute_ha2(method: &str, uri: &str) -> String {
        let data = format!("{}:{}", method, uri);
        let digest = md5::compute(data.as_bytes());
        hex::encode(&digest[..])
    }

    fn compute_response_with_qop(
        ha1: &str,
        nonce: &str,
        nc: &str,
        cnonce: &str,
        qop: &str,
        ha2: &str,
    ) -> String {
        let data = format!("{}:{}:{}:{}:{}:{}", ha1, nonce, nc, cnonce, qop, ha2);
        let digest = md5::compute(data.as_bytes());
        hex::encode(&digest[..])
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
    fn test_compute_ha1() {
        let auth = DigestAuthenticator::new("realm");
        let ha1 = auth.compute_ha1("user", "realm", "password");
        // This is a known MD5 hash for "user:realm:password"
        assert_eq!(ha1.len(), 32); // MD5 produces 32 hex characters
    }

    #[test]
    fn test_generate_nonce() {
        let nonce1 = DigestAuthenticator::generate_nonce();
        let nonce2 = DigestAuthenticator::generate_nonce();
        
        assert_eq!(nonce1.len(), 32);
        assert_ne!(nonce1, nonce2); // Should be different
    }

    #[test]
    fn test_parse_challenge() {
        let header = r#"Digest realm="test", nonce="abc123", algorithm=MD5, qop="auth""#;
        let challenge = DigestAuthenticator::parse_challenge(header).unwrap();
        
        assert_eq!(challenge.realm, "test");
        assert_eq!(challenge.nonce, "abc123");
        assert_eq!(challenge.algorithm, DigestAlgorithm::MD5);
        assert!(challenge.qop.is_some());
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

    #[test]
    fn test_client_compute_response() {
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
            "sip:registrar.example.com"
        ).unwrap();
        
        assert_eq!(response.len(), 32); // MD5 produces 32 hex characters
    }
}


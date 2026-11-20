//! `CertResolver` trait: fetch the X.509 certificate that signed a
//! PASSporT, given the `info=` URL on the SIP `Identity` header.
//!
//! Pluggable so applications can supply a cache, an in-tree stub for
//! tests, or a private repository fetcher. The reference impl
//! (`ReqwestCertResolver`) uses `reqwest` with rustls-tls.

use crate::errors::VerifierError;
use async_trait::async_trait;
use url::Url;

/// Fetch raw certificate bytes (DER or PEM) from the `info=` URL.
///
/// Implementations must:
/// - Reject non-HTTPS URLs (RFC 8224 §6.1 forbids fetching over
///   unauthenticated transports).
/// - Enforce a sensible size cap (a few hundred KB max).
/// - Time out reasonable: typical STIR/SHAKEN deployments expect
///   sub-second cert fetches and aggressive caching at the verifier.
#[async_trait]
pub trait CertResolver: Send + Sync {
    async fn fetch(&self, url: &Url) -> Result<Vec<u8>, VerifierError>;
}

/// Reference implementation using `reqwest`. HTTPS only, 256 KB cap,
/// 5-second timeout. Applications typically wrap this in a caching
/// layer keyed by URL.
pub struct ReqwestCertResolver {
    client: reqwest::Client,
    max_bytes: usize,
}

impl ReqwestCertResolver {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .expect("reqwest client should build with default config"),
            max_bytes: 256 * 1024,
        }
    }

    pub fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes;
        self
    }
}

impl Default for ReqwestCertResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CertResolver for ReqwestCertResolver {
    async fn fetch(&self, url: &Url) -> Result<Vec<u8>, VerifierError> {
        if url.scheme() != "https" {
            return Err(VerifierError::BadInfo(format!(
                "info= URL must use https scheme, got {}",
                url.scheme()
            )));
        }
        let resp = self
            .client
            .get(url.as_str())
            .send()
            .await
            .map_err(|e| VerifierError::CertFetch(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(VerifierError::CertFetch(format!(
                "HTTP {} from {}",
                resp.status(),
                url
            )));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| VerifierError::CertFetch(e.to_string()))?;
        if bytes.len() > self.max_bytes {
            return Err(VerifierError::CertFetch(format!(
                "cert too large: {} bytes exceeds cap {}",
                bytes.len(),
                self.max_bytes
            )));
        }
        Ok(bytes.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_non_https() {
        let resolver = ReqwestCertResolver::new();
        let url = Url::parse("http://example.org/cert.pem").unwrap();
        let err = resolver.fetch(&url).await.unwrap_err();
        match err {
            VerifierError::BadInfo(msg) => assert!(msg.contains("https")),
            other => panic!("expected BadInfo, got {:?}", other),
        }
    }
}

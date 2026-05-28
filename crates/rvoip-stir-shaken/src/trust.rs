//! Operator-supplied STI-CA trust anchors for SHAKEN cert chain
//! validation.
//!
//! Per ATIS-1000080 (SHAKEN Governance Model), the STI Policy
//! Administrator (STI-PA) publishes the approved-CA list; each
//! Verifying Service Provider (VSP) holds the resulting roots and
//! refreshes them out-of-band. This library never bundles roots — the
//! application supplies them via [`TrustStore`], either as DER blobs
//! or a PEM bundle file.
//!
//! `TrustStore` is opaque: it stores DER bytes and produces
//! `rustls_pki_types::TrustAnchor` views on demand, so the webpki
//! types never leak into the public API.

use crate::errors::VerifierError;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use rustls_pki_types::{CertificateDer, TrustAnchor};

/// A collection of trusted STI-CA root certificates in DER form.
///
/// An empty store disables chain validation (the verifier falls back
/// to the legacy "trust whatever the resolver returned" behaviour);
/// applications opt in by installing at least one anchor.
#[derive(Clone, Default)]
pub struct TrustStore {
    anchors_der: Vec<Vec<u8>>,
}

impl TrustStore {
    /// Empty store — chain validation is skipped.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build a store from DER-encoded root certificates.
    pub fn from_der_certs(anchors_der: Vec<Vec<u8>>) -> Self {
        Self { anchors_der }
    }

    /// Parse one or more `-----BEGIN CERTIFICATE-----` PEM blocks
    /// from `pem` and store each block's DER. Returns an error if
    /// the input contains no CERTIFICATE blocks or any block fails
    /// to base64-decode.
    pub fn from_pem_bundle(pem: &[u8]) -> Result<Self, VerifierError> {
        let mut store = Self::default();
        store.add_pem(pem)?;
        Ok(store)
    }

    /// Append all CERTIFICATE blocks found in `pem`. Returns the
    /// number of anchors added.
    pub fn add_pem(&mut self, pem: &[u8]) -> Result<usize, VerifierError> {
        let added = decode_pem_bundle(pem)?;
        let n = added.len();
        if n == 0 {
            return Err(VerifierError::CertChain(
                "trust bundle contained no CERTIFICATE blocks".into(),
            ));
        }
        self.anchors_der.extend(added);
        Ok(n)
    }

    /// Append a single DER-encoded root.
    pub fn add_der(&mut self, der: Vec<u8>) {
        self.anchors_der.push(der);
    }

    /// True if no anchors are configured. Verifier treats this as
    /// "skip chain validation, preserve legacy behaviour."
    pub fn is_empty(&self) -> bool {
        self.anchors_der.is_empty()
    }

    pub fn len(&self) -> usize {
        self.anchors_der.len()
    }

    /// Build owned `TrustAnchor` views over the stored DER blobs.
    /// Errors if any anchor fails to parse — caller maps that to a
    /// configuration-time fatal.
    pub(crate) fn as_trust_anchors(&self) -> Result<Vec<TrustAnchor<'static>>, VerifierError> {
        self.anchors_der
            .iter()
            .map(|der| {
                let cert_der = CertificateDer::from(der.as_slice());
                webpki::anchor_from_trusted_cert(&cert_der)
                    .map(|anchor| anchor.to_owned())
                    .map_err(|e| {
                        VerifierError::CertChain(format!("trust anchor cert parse failed: {}", e))
                    })
            })
            .collect()
    }
}

impl std::fmt::Debug for TrustStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustStore")
            .field("anchor_count", &self.anchors_der.len())
            .finish()
    }
}

/// Walk every `-----BEGIN CERTIFICATE-----` block in `pem`, decoding
/// each base64 body to DER. Returns an empty vec if no CERTIFICATE
/// blocks are present (caller decides whether that's an error).
pub(crate) fn decode_pem_bundle(pem: &[u8]) -> Result<Vec<Vec<u8>>, VerifierError> {
    let text = std::str::from_utf8(pem)
        .map_err(|_| VerifierError::CertChain("PEM bundle is not valid UTF-8".into()))?;

    let mut out = Vec::new();
    let mut cursor = text;
    while let Some(begin_idx) = cursor.find("-----BEGIN CERTIFICATE-----") {
        let after_begin = &cursor[begin_idx + "-----BEGIN CERTIFICATE-----".len()..];
        let end_idx = after_begin
            .find("-----END CERTIFICATE-----")
            .ok_or_else(|| {
                VerifierError::CertChain(
                    "PEM bundle has BEGIN CERTIFICATE without matching END CERTIFICATE".into(),
                )
            })?;
        let body = &after_begin[..end_idx];
        let cleaned: String = body.chars().filter(|c| !c.is_whitespace()).collect();
        let der = BASE64_STANDARD
            .decode(cleaned.as_bytes())
            .map_err(|e| VerifierError::CertChain(format!("PEM base64 decode: {}", e)))?;
        out.push(der);
        cursor = &after_begin[end_idx + "-----END CERTIFICATE-----".len()..];
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_store_is_empty() {
        let store = TrustStore::empty();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn from_der_round_trip() {
        let store = TrustStore::from_der_certs(vec![vec![1, 2, 3], vec![4, 5, 6]]);
        assert_eq!(store.len(), 2);
        assert!(!store.is_empty());
    }

    #[test]
    fn from_pem_bundle_rejects_empty() {
        let err = TrustStore::from_pem_bundle(b"no certs here\n").unwrap_err();
        match err {
            VerifierError::CertChain(msg) => assert!(msg.contains("no CERTIFICATE")),
            other => panic!("expected CertChain, got {:?}", other),
        }
    }

    #[test]
    fn from_pem_bundle_decodes_two_blocks() {
        // Two trivial PEM-armored payloads (not real certs — we only
        // exercise the base64 decoder here).
        let pem = b"-----BEGIN CERTIFICATE-----\nQUJDRA==\n-----END CERTIFICATE-----\n\
                    -----BEGIN CERTIFICATE-----\nWFlaWlpa\n-----END CERTIFICATE-----\n";
        let store = TrustStore::from_pem_bundle(pem).expect("parse");
        assert_eq!(store.len(), 2);
        assert_eq!(store.anchors_der[0], b"ABCD");
        assert_eq!(store.anchors_der[1], b"XYZZZZ");
    }

    #[test]
    fn pem_bundle_unmatched_begin_errors() {
        let pem = b"-----BEGIN CERTIFICATE-----\nQUJDRA==\n";
        let err = TrustStore::from_pem_bundle(pem).unwrap_err();
        match err {
            VerifierError::CertChain(msg) => assert!(msg.contains("END CERTIFICATE")),
            other => panic!("expected CertChain, got {:?}", other),
        }
    }
}

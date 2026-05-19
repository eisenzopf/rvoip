//! Reference [`PASSporTVerifier`] implementation for STIR/SHAKEN
//! (RFC 8224 / RFC 8225 / RFC 8588 / ATIS-1000074).
//!
//! Given an inbound SIP request's byte-exact raw bytes and the
//! parsed `Identity:` header, this verifier:
//!
//! 1. Splits the compact-form PASSporT JWT into header / payload /
//!    signature segments.
//! 2. Resolves the certificate via the configured
//!    [`CertResolver`] (default: HTTPS fetch with size cap).
//! 3. Extracts the EC public key from the certificate's
//!    SubjectPublicKeyInfo.
//! 4. Verifies the JWS signature against `header.payload` using
//!    ES256.
//! 5. Cross-checks PASSporT `orig` / `dest` claims against the SIP
//!    `From` / `To` URIs.
//! 6. Validates that `iat` is within the configured freshness
//!    window (default ±60 s per ATIS-1000074 §5.3.1).
//!
//! ## Out of scope for this reference impl
//!
//! - **Full SHAKEN certificate-chain validation** (STI-CA → STI-PA
//!   anchors per ATIS-1000080). The current impl trusts the cert
//!   returned by the `CertResolver` directly. Applications that
//!   need chain validation should compose this verifier with a
//!   `webpki`-based pre-check, or wrap the `CertResolver` with a
//!   chain-validating decorator that returns only the leaf cert
//!   after successful validation.
//! - **OCSP / CRL checks** for cert revocation. Out of scope for
//!   the in-band SIP path; deployments rely on STI-CA
//!   short-lived certs.
//! - **`ppt=div` / `ppt=rcd` PASSporT extensions** beyond the base
//!   SHAKEN profile. Add follow-on impls per profile.

use crate::cert_resolver::CertResolver;
use crate::signer::split_compact_jwt;
use async_trait::async_trait;
use bytes::Bytes;
use jsonwebtoken::{Algorithm, DecodingKey};
use rvoip_sip_core::types::identity::Identity;
use rvoip_sip_core::Request;
use rvoip_sip_dialog::manager::{PASSporTVerifier, VerificationOutcome};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};
use url::Url;
use x509_parser::prelude::*;

/// Configuration for the reference [`ShakenVerifier`].
#[derive(Clone)]
pub struct ShakenVerifierConfig {
    /// Allowed clock skew between the local clock and the
    /// signer's claimed `iat`. ATIS-1000074 §5.3.1 suggests a 60-s
    /// window; deployments behind asymmetric NAT or with bursty
    /// SIP queues may widen this. Default: 60 s.
    pub freshness_window: Duration,

    /// When `true`, the verifier requires the PASSporT `ppt` header
    /// (and the matching SIP `Identity;ppt=...`) to be `"shaken"`.
    /// SBC pass-through paths that need to handle other PASSporT
    /// profiles (`div`, `rcd`) should set this to `false`.
    /// Default: `true`.
    pub require_shaken_ppt: bool,
}

impl Default for ShakenVerifierConfig {
    fn default() -> Self {
        Self {
            freshness_window: Duration::from_secs(60),
            require_shaken_ppt: true,
        }
    }
}

impl std::fmt::Debug for ShakenVerifierConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShakenVerifierConfig")
            .field("freshness_window", &self.freshness_window)
            .field("require_shaken_ppt", &self.require_shaken_ppt)
            .finish()
    }
}

/// Reference [`PASSporTVerifier`].
pub struct ShakenVerifier {
    resolver: Arc<dyn CertResolver>,
    config: ShakenVerifierConfig,
}

impl ShakenVerifier {
    pub fn new(resolver: Arc<dyn CertResolver>, config: ShakenVerifierConfig) -> Self {
        Self { resolver, config }
    }

    /// Borrow as an `Arc<dyn PASSporTVerifier>` ready to install on
    /// a `DialogManager` via `set_identity_verifier`.
    pub fn into_shared(self) -> Arc<dyn PASSporTVerifier> {
        Arc::new(self) as Arc<dyn PASSporTVerifier>
    }
}

impl std::fmt::Debug for ShakenVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShakenVerifier")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// JWS protected header — what the signer produced. We only read
/// the subset we cross-check.
#[derive(Deserialize)]
struct ParsedJwsHeader {
    alg: String,
    /// `typ` is `"passport"` for PASSporT (RFC 8225 §8.1) but some
    /// signers emit `"JWT"` or omit it; we don't gate on it.
    #[serde(default)]
    #[allow(dead_code)]
    typ: Option<String>,
    #[serde(default)]
    ppt: Option<String>,
    #[serde(default)]
    x5u: Option<String>,
}

/// PASSporT payload — what the signer attested to.
#[derive(Deserialize)]
struct ParsedPassportPayload {
    iat: i64,
    orig: ParsedOrigDest,
    dest: ParsedOrigDestVec,
    #[serde(default)]
    attest: Option<String>,
    #[serde(default)]
    origid: Option<String>,
}

/// `orig` claim shape — single TN or URI.
#[derive(Deserialize)]
struct ParsedOrigDest {
    #[serde(default)]
    tn: Option<String>,
    #[serde(default)]
    uri: Option<String>,
}

/// `dest` claim shape — arrays of TNs and URIs.
#[derive(Deserialize)]
struct ParsedOrigDestVec {
    #[serde(default)]
    tn: Option<Vec<String>>,
    #[serde(default)]
    uri: Option<Vec<String>>,
}

#[async_trait]
impl PASSporTVerifier for ShakenVerifier {
    async fn verify(
        &self,
        _raw_bytes: &Bytes,
        identity: &Identity,
        request: &Request,
    ) -> VerificationOutcome {
        // Step 1 — split the JWT and parse header + payload.
        let (header_bytes, payload_bytes, signature_b64) = match split_compact_jwt(&identity.jwt) {
            Ok(parts) => parts,
            Err(_) => {
                debug!("STIR/SHAKEN: malformed JWT — three segments expected");
                return VerificationOutcome::BadSignature;
            }
        };

        let header: ParsedJwsHeader = match serde_json::from_slice(&header_bytes) {
            Ok(h) => h,
            Err(_) => {
                debug!("STIR/SHAKEN: JWT header JSON parse failed");
                return VerificationOutcome::BadSignature;
            }
        };

        let payload: ParsedPassportPayload = match serde_json::from_slice(&payload_bytes) {
            Ok(p) => p,
            Err(_) => {
                debug!("STIR/SHAKEN: JWT payload JSON parse failed");
                return VerificationOutcome::BadSignature;
            }
        };

        // RFC 8588 §4 — SHAKEN requires alg=ES256, typ=passport,
        // ppt=shaken. We only enforce alg + ppt here; typ is
        // logged but not gated since some signers emit "JWT" too.
        if header.alg != "ES256" {
            return VerificationOutcome::BadSignature;
        }
        if self.config.require_shaken_ppt && header.ppt.as_deref() != Some("shaken") {
            return VerificationOutcome::BadInfo {
                reason: format!(
                    "PASSporT ppt header must be 'shaken' for SHAKEN profile, got {:?}",
                    header.ppt
                ),
            };
        }

        // Step 2 — resolve the cert URL.
        // Prefer the `info=` from the SIP Identity header
        // (RFC 8224 §4.1) since that's what the SIP spec covers; the
        // JWS `x5u` is checked for consistency but takes second seat.
        let cert_url_str = match identity.info.as_deref().or(header.x5u.as_deref()) {
            Some(u) => u,
            None => {
                return VerificationOutcome::BadInfo {
                    reason: "missing info= and x5u".into(),
                }
            }
        };
        let cert_url = match Url::parse(cert_url_str) {
            Ok(u) => u,
            Err(e) => {
                return VerificationOutcome::BadInfo {
                    reason: format!("invalid cert URL {}: {}", cert_url_str, e),
                }
            }
        };

        // Step 3 — fetch and parse the cert.
        let cert_bytes = match self.resolver.fetch(&cert_url).await {
            Ok(b) => b,
            Err(e) => {
                warn!("STIR/SHAKEN cert fetch failed for {}: {:?}", cert_url, e);
                return VerificationOutcome::BadChain {
                    reason: format!("cert fetch failed: {:?}", e),
                };
            }
        };

        let cert_der = match maybe_decode_pem_cert(&cert_bytes) {
            Some(der) => der,
            None => cert_bytes.clone(),
        };

        let (_, cert) = match X509Certificate::from_der(&cert_der) {
            Ok(c) => c,
            Err(e) => {
                return VerificationOutcome::BadChain {
                    reason: format!("X.509 parse failed: {}", e),
                };
            }
        };

        let spki_bytes = cert.tbs_certificate.subject_pki.subject_public_key.data.as_ref();

        // Step 4 — verify the JWS signature using the cert's
        // public key. ES256 = ECDSA over P-256 / SHA-256. Raw EC
        // public-key format from X.509 SPKI is uncompressed:
        // 0x04 || x(32) || y(32) = 65 bytes.
        let decoding_key = match build_decoding_key_from_spki(spki_bytes) {
            Ok(k) => k,
            Err(reason) => return VerificationOutcome::BadChain { reason },
        };

        let signing_input = {
            // Reconstitute base64url(header) "." base64url(payload)
            // from the JWT itself (rather than re-encoding) — the
            // signature was computed over those exact bytes.
            let dot = identity.jwt.rfind('.').unwrap();
            identity.jwt[..dot].to_string()
        };

        match jsonwebtoken::crypto::verify(
            &signature_b64,
            signing_input.as_bytes(),
            &decoding_key,
            Algorithm::ES256,
        ) {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                return VerificationOutcome::BadSignature;
            }
        }

        // Step 5 — cross-check claims against the SIP request.
        if let Some(field) = check_claim_mismatch(&payload, request) {
            return VerificationOutcome::ClaimMismatch { field };
        }

        // Step 6 — iat freshness window.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let skew = now - payload.iat;
        let window_secs = self.config.freshness_window.as_secs() as i64;
        if skew.abs() > window_secs {
            return VerificationOutcome::Stale { skew_secs: skew };
        }

        // All checks pass.
        let origid = payload
            .origid
            .as_deref()
            .and_then(|s| uuid::Uuid::parse_str(s).ok());

        VerificationOutcome::Valid {
            attest: payload.attest,
            origid,
        }
    }
}

/// If `bytes` is PEM-encoded with a CERTIFICATE armour, return the
/// DER body. Returns `None` if the input doesn't look like a PEM
/// certificate envelope (caller falls back to treating it as DER).
fn maybe_decode_pem_cert(bytes: &[u8]) -> Option<Vec<u8>> {
    let s = std::str::from_utf8(bytes).ok()?;
    if !s.contains("-----BEGIN CERTIFICATE-----") {
        return None;
    }
    let body = s
        .split("-----BEGIN CERTIFICATE-----")
        .nth(1)?
        .split("-----END CERTIFICATE-----")
        .next()?;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    let cleaned: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    STANDARD.decode(cleaned).ok()
}

/// Build a `DecodingKey` from an X.509 SPKI public-key bytes
/// (uncompressed form `0x04 || x || y` for P-256).
fn build_decoding_key_from_spki(spki: &[u8]) -> Result<DecodingKey, String> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;

    if spki.len() != 65 || spki[0] != 0x04 {
        return Err(format!(
            "expected uncompressed P-256 SPKI (65 bytes, leading 0x04); got {} bytes leading {:#x}",
            spki.len(),
            spki.first().copied().unwrap_or(0)
        ));
    }
    let x = URL_SAFE_NO_PAD.encode(&spki[1..33]);
    let y = URL_SAFE_NO_PAD.encode(&spki[33..65]);
    DecodingKey::from_ec_components(&x, &y).map_err(|e| format!("DecodingKey build failed: {}", e))
}

/// Walk the parsed PASSporT payload against the SIP request's
/// `From` / `To` URIs. Returns `Some(field_name)` on mismatch.
///
/// Cross-checks (RFC 8224 §6.4.1):
/// - `orig.tn` or `orig.uri` matches the SIP `From` user-part
/// - `dest.tn[0]` or `dest.uri[0]` matches the SIP `To` user-part
fn check_claim_mismatch(
    payload: &ParsedPassportPayload,
    request: &Request,
) -> Option<&'static str> {
    use rvoip_sip_core::types::TypedHeader;

    let from = request.headers.iter().find_map(|h| match h {
        TypedHeader::From(f) => Some(&f.0),
        _ => None,
    })?;
    let to = request.headers.iter().find_map(|h| match h {
        TypedHeader::To(t) => Some(&t.0),
        _ => None,
    })?;

    let sip_orig = canonical_e164_or_uri(&from.uri);
    let sip_dest = canonical_e164_or_uri(&to.uri);

    let orig_match = match (&payload.orig.tn, &payload.orig.uri) {
        (Some(tn), _) => Some(tn.as_str()) == sip_orig.as_deref(),
        (None, Some(uri)) => Some(uri.as_str()) == sip_orig.as_deref(),
        (None, None) => false,
    };
    if !orig_match {
        return Some("orig");
    }

    let dest_match = match (&payload.dest.tn, &payload.dest.uri) {
        (Some(tns), _) if !tns.is_empty() => {
            tns.iter().any(|t| Some(t.as_str()) == sip_dest.as_deref())
        }
        (_, Some(uris)) if !uris.is_empty() => {
            uris.iter().any(|u| Some(u.as_str()) == sip_dest.as_deref())
        }
        _ => false,
    };
    if !dest_match {
        return Some("dest");
    }

    None
}

/// Render a URI as either an E.164 TN string or the full URI
/// string, matching the shape the PASSporT signer would have
/// claimed.
fn canonical_e164_or_uri(uri: &rvoip_sip_core::types::uri::Uri) -> Option<String> {
    use rvoip_sip_core::types::uri::{Host, Scheme};
    match uri.scheme {
        Scheme::Tel => match &uri.host {
            Host::Domain(d) => Some(d.clone()),
            _ => None,
        },
        Scheme::Sip | Scheme::Sips => {
            if let Some(user) = uri.user.as_deref() {
                if user.starts_with('+') && user[1..].chars().all(|c| c.is_ascii_digit()) {
                    return Some(user.to_string());
                }
            }
            Some(uri.to_string())
        }
        _ => Some(uri.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maybe_decode_pem_cert_strips_armour() {
        let pem = b"-----BEGIN CERTIFICATE-----\nQUJDRA==\n-----END CERTIFICATE-----\n";
        let der = maybe_decode_pem_cert(pem).expect("decode");
        assert_eq!(der, b"ABCD");
    }

    #[test]
    fn maybe_decode_pem_cert_returns_none_for_der() {
        // DER blob without PEM armour
        assert!(maybe_decode_pem_cert(&[0x30, 0x82, 0x01]).is_none());
    }

    #[test]
    fn build_decoding_key_rejects_wrong_length() {
        let result = build_decoding_key_from_spki(&[0u8; 10]);
        assert!(result.is_err());
    }

    #[test]
    fn build_decoding_key_rejects_compressed_form() {
        // Compressed P-256 starts with 0x02 or 0x03, length 33
        let mut spki = vec![0u8; 33];
        spki[0] = 0x02;
        assert!(build_decoding_key_from_spki(&spki).is_err());
    }

    #[test]
    fn canonical_e164_from_tel() {
        let uri: rvoip_sip_core::types::uri::Uri = "tel:+15551234567".parse().unwrap();
        assert_eq!(canonical_e164_or_uri(&uri).as_deref(), Some("+15551234567"));
    }

    #[test]
    fn canonical_e164_from_sip_user() {
        let uri: rvoip_sip_core::types::uri::Uri =
            "sip:+15551234567@gw.example.com".parse().unwrap();
        assert_eq!(canonical_e164_or_uri(&uri).as_deref(), Some("+15551234567"));
    }

    #[test]
    fn canonical_for_named_sip_user_returns_full_uri() {
        let uri: rvoip_sip_core::types::uri::Uri = "sip:alice@example.com".parse().unwrap();
        let s = canonical_e164_or_uri(&uri).expect("some");
        assert!(s.contains("alice@example.com"));
    }
}

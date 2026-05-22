//! Reference [`PASSporTSigner`] implementation for STIR/SHAKEN
//! (RFC 8224 / RFC 8225 / RFC 8588 / ATIS-1000074).
//!
//! Produces an ES256-signed PASSporT JWT from a
//! [`PassportClaimSummary`] supplied by the dialog layer's
//! `RequestLifecycle::pre_send_request` hook.
//!
//! ## Why a manual JWS rather than `jsonwebtoken::encode`
//!
//! PASSporT's header carries a non-standard `ppt` parameter
//! (RFC 8225 §8.1) and `x5u` URL. The `jsonwebtoken` crate's
//! `Header` struct is a closed set of named fields that does not
//! include `ppt`. To stay strictly RFC-conformant we build the
//! header JSON manually and use `jsonwebtoken::crypto::sign` for the
//! ES256 primitive only.

use crate::types::{Attestation, OrigDestField, PassportClaims, PptType};
use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use jsonwebtoken::{Algorithm, EncodingKey};
use rvoip_sip_dialog::manager::{
    IdentityHeaderValue, PASSporTSigner, PassportClaimSummary, SignerErrorKind,
};
use serde::Serialize;
use std::str::FromStr;
use std::sync::Arc;
use url::Url;
use uuid::Uuid;

/// JWS protected header for SHAKEN PASSporTs (RFC 8588 §4).
///
/// Wire form is `{"alg":"ES256","typ":"passport","ppt":"shaken","x5u":"https://..."}`.
/// `ppt` is omitted when signing a base PASSporT (RFC 8225 with no
/// extension); SHAKEN deployments always set `ppt="shaken"`.
#[derive(Serialize)]
struct PassportHeader<'a> {
    alg: &'static str,
    typ: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    ppt: Option<&'a str>,
    x5u: &'a str,
}

/// Configuration for the reference [`ShakenSigner`].
#[derive(Clone)]
pub struct ShakenSignerConfig {
    /// HTTPS URL of the certificate that signed this PASSporT.
    /// Embedded as `x5u` in the JWS header AND as the `info=`
    /// parameter on the SIP `Identity:` header (RFC 8224 §4.1).
    pub cert_url: Url,

    /// Default attestation level emitted when the call's
    /// `PassportClaimSummary` does not specify one. SHAKEN
    /// deployments typically default to `A` (full attestation) and
    /// let upstream call-control code override per call.
    pub default_attest: Attestation,

    /// PASSporT extension type. Almost always
    /// [`PptType::Shaken`] for ATIS-1000074 deployments.
    pub ppt: PptType,

    /// Allowed clock skew when emitting `iat`. Some deployments
    /// pre-date the iat by a few seconds to absorb downstream clock
    /// drift; default `0`.
    pub iat_skew_secs: i64,
}

impl ShakenSignerConfig {
    pub fn new(cert_url: Url) -> Self {
        Self {
            cert_url,
            default_attest: Attestation::Full,
            ppt: PptType::Shaken,
            iat_skew_secs: 0,
        }
    }
}

/// Reference [`PASSporTSigner`] backed by an ES256 private key.
///
/// Build with [`ShakenSigner::from_pem`] or [`ShakenSigner::from_der`]
/// and install on the [`rvoip_sip_dialog::manager::DialogManager`] via
/// `set_identity_signer`. Apps with HSM-backed keys should implement
/// `PASSporTSigner` directly rather than going through this type.
pub struct ShakenSigner {
    key: EncodingKey,
    config: ShakenSignerConfig,
}

impl std::fmt::Debug for ShakenSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never expose key material in Debug output.
        f.debug_struct("ShakenSigner")
            .field("cert_url", &self.config.cert_url.as_str())
            .field("default_attest", &self.config.default_attest)
            .field("ppt", &self.config.ppt)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for ShakenSignerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShakenSignerConfig")
            .field("cert_url", &self.cert_url.as_str())
            .field("default_attest", &self.default_attest)
            .field("ppt", &self.ppt)
            .field("iat_skew_secs", &self.iat_skew_secs)
            .finish()
    }
}

impl ShakenSigner {
    /// Build from a PEM-encoded EC private key (curve P-256). The
    /// PEM blob is consumed once and the key material is retained
    /// in [`EncodingKey`] form — call sites typically read the PEM
    /// from disk at boot and `Arc`-wrap the resulting signer.
    pub fn from_pem(pem: &[u8], config: ShakenSignerConfig) -> Result<Self, SignerErrorKind> {
        let key = EncodingKey::from_ec_pem(pem).map_err(|_| SignerErrorKind::KeyUnavailable)?;
        Ok(Self { key, config })
    }

    /// Build from a DER-encoded EC private key (curve P-256).
    pub fn from_der(der: &[u8], config: ShakenSignerConfig) -> Self {
        Self {
            key: EncodingKey::from_ec_der(der),
            config,
        }
    }

    /// Borrow the signer as an `Arc<dyn PASSporTSigner>` ready to
    /// install on a `DialogManager`.
    pub fn into_shared(self) -> Arc<dyn PASSporTSigner> {
        Arc::new(self) as Arc<dyn PASSporTSigner>
    }

    /// Construct the full RFC 8225 / RFC 8588 PASSporT claim object
    /// from the SIP-shaped summary handed in by the dialog layer.
    fn build_claims(
        &self,
        summary: &PassportClaimSummary,
        iat: u64,
    ) -> Result<PassportClaims, SignerErrorKind> {
        let orig = match (&summary.orig_tn, &summary.orig_uri) {
            (Some(tn), _) => OrigDestField::Tn { tn: tn.clone() },
            (None, Some(uri)) => OrigDestField::Uri { uri: uri.clone() },
            (None, None) => return Err(SignerErrorKind::InvalidClaims),
        };

        let mut dest = crate::types::OrigDest {
            tn: None,
            uri: None,
        };
        if let Some(tn) = &summary.dest_tn {
            dest.tn = Some(vec![tn.clone()]);
        }
        if let Some(uri) = &summary.dest_uri {
            dest.uri = Some(vec![uri.clone()]);
        }
        if dest.tn.is_none() && dest.uri.is_none() {
            return Err(SignerErrorKind::InvalidClaims);
        }

        let attest = match summary.attest.as_deref() {
            Some("A") | Some("a") => Some(Attestation::Full),
            Some("B") | Some("b") => Some(Attestation::Partial),
            Some("C") | Some("c") => Some(Attestation::Gateway),
            Some(_) => return Err(SignerErrorKind::InvalidClaims),
            None if matches!(self.config.ppt, PptType::Shaken) => Some(self.config.default_attest),
            None => None,
        };

        let origid = summary.origid.or_else(|| Some(Uuid::new_v4()));

        Ok(PassportClaims {
            orig,
            dest,
            iat,
            origid,
            attest,
        })
    }
}

#[async_trait]
impl PASSporTSigner for ShakenSigner {
    async fn sign(
        &self,
        claims: PassportClaimSummary,
    ) -> Result<IdentityHeaderValue, SignerErrorKind> {
        // RFC 8225 §5.2.4 — iat is seconds since the UNIX epoch at the
        // time the PASSporT is created. Honour the configured skew so
        // downstream verifiers with slightly slow clocks still see iat
        // as fresh.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let iat = (now + self.config.iat_skew_secs).max(0) as u64;

        let claims = self.build_claims(&claims, iat)?;

        // Header: ES256 + typ=passport + ppt + x5u.
        let header = PassportHeader {
            alg: "ES256",
            typ: "passport",
            ppt: if matches!(self.config.ppt, PptType::Shaken) {
                Some(self.config.ppt.as_str())
            } else {
                Some(self.config.ppt.as_str())
            },
            x5u: self.config.cert_url.as_str(),
        };

        let header_json =
            serde_json::to_vec(&header).map_err(|_| SignerErrorKind::SigningFailed)?;
        let payload_json =
            serde_json::to_vec(&claims).map_err(|_| SignerErrorKind::InvalidClaims)?;

        let mut signing_input = String::new();
        signing_input.push_str(&URL_SAFE_NO_PAD.encode(&header_json));
        signing_input.push('.');
        signing_input.push_str(&URL_SAFE_NO_PAD.encode(&payload_json));

        let signature =
            jsonwebtoken::crypto::sign(signing_input.as_bytes(), &self.key, Algorithm::ES256)
                .map_err(|_| SignerErrorKind::SigningFailed)?;

        let jwt = format!("{}.{}", signing_input, signature);

        Ok(IdentityHeaderValue {
            jwt,
            info: self.config.cert_url.to_string(),
            alg: "ES256".to_string(),
            ppt: Some(self.config.ppt.as_str().to_string()),
        })
    }
}

/// Parse a JWT into `(header_json, payload_json, signature_b64)`
/// without verifying the signature. Used by tests and by the
/// verifier-side claim-extraction code.
pub(crate) fn split_compact_jwt(jwt: &str) -> Result<(Vec<u8>, Vec<u8>, String), SignerErrorKind> {
    let mut parts = jwt.split('.');
    let header_b64 = parts.next().ok_or(SignerErrorKind::InvalidClaims)?;
    let payload_b64 = parts.next().ok_or(SignerErrorKind::InvalidClaims)?;
    let sig_b64 = parts.next().ok_or(SignerErrorKind::InvalidClaims)?;
    if parts.next().is_some() {
        return Err(SignerErrorKind::InvalidClaims);
    }
    let header = URL_SAFE_NO_PAD
        .decode(header_b64)
        .map_err(|_| SignerErrorKind::InvalidClaims)?;
    let payload = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| SignerErrorKind::InvalidClaims)?;
    Ok((header, payload, sig_b64.to_string()))
}

// Suppress unused imports warning when only a subset is needed.
#[allow(dead_code)]
fn _silence_unused() {
    let _ = Url::from_str;
}

#[cfg(test)]
mod tests {
    use super::*;

    // ES256 test vector — a self-signed P-256 key generated for
    // testing only. NEVER use in production.
    const TEST_EC_PEM: &[u8] = b"-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgevZzL1gdAFr88hb2\n\
OF/2NxApJCzGCEDdfSp6VQO30hyhRANCAAQRWz+jn65BtOMvdyHKcvjBeBSDZH2r\n\
1RTwjmYSi9R/zpBnuQ4EiMnCqfMPWiZqB4QdbAd0E7oH50VpuZ1P087G\n\
-----END PRIVATE KEY-----\n";

    fn test_config() -> ShakenSignerConfig {
        ShakenSignerConfig::new(Url::parse("https://cert.example.org/p.cer").expect("url"))
    }

    fn full_claim_summary() -> PassportClaimSummary {
        PassportClaimSummary {
            orig_tn: Some("+15551234567".into()),
            orig_uri: Some("tel:+15551234567".into()),
            dest_tn: Some("+15559876543".into()),
            dest_uri: Some("tel:+15559876543".into()),
            iat: 0, // ignored by signer; signer picks its own
            origid: Some(Uuid::nil()),
            attest: Some("A".into()),
            ppt: Some("shaken".into()),
        }
    }

    #[tokio::test]
    async fn signs_and_produces_three_segment_jwt() {
        let signer = ShakenSigner::from_pem(TEST_EC_PEM, test_config()).expect("load PEM");
        let value = signer.sign(full_claim_summary()).await.expect("sign");
        assert_eq!(value.alg, "ES256");
        assert_eq!(value.ppt.as_deref(), Some("shaken"));
        assert_eq!(value.info, "https://cert.example.org/p.cer");
        // Compact-form JWT has exactly two dots.
        assert_eq!(value.jwt.matches('.').count(), 2);
    }

    #[tokio::test]
    async fn jwt_header_carries_passport_ppt_and_x5u() {
        let signer = ShakenSigner::from_pem(TEST_EC_PEM, test_config()).expect("load PEM");
        let value = signer.sign(full_claim_summary()).await.expect("sign");

        let (header_bytes, _payload_bytes, _sig) = split_compact_jwt(&value.jwt).expect("split");
        let header: serde_json::Value = serde_json::from_slice(&header_bytes).expect("header json");

        assert_eq!(header["alg"], "ES256");
        assert_eq!(header["typ"], "passport");
        assert_eq!(header["ppt"], "shaken");
        assert_eq!(header["x5u"], "https://cert.example.org/p.cer");
    }

    #[tokio::test]
    async fn jwt_payload_carries_orig_dest_iat_attest() {
        let signer = ShakenSigner::from_pem(TEST_EC_PEM, test_config()).expect("load PEM");
        let value = signer.sign(full_claim_summary()).await.expect("sign");

        let (_header, payload_bytes, _sig) = split_compact_jwt(&value.jwt).expect("split");
        let payload: serde_json::Value =
            serde_json::from_slice(&payload_bytes).expect("payload json");

        // RFC 8225 §5.2 + RFC 8588 §4 claim set
        assert_eq!(payload["attest"], "A");
        assert_eq!(payload["orig"]["tn"], "+15551234567");
        assert_eq!(payload["dest"]["tn"][0], "+15559876543");
        assert!(payload["iat"].is_number());
        assert!(payload["origid"].is_string());
    }

    #[tokio::test]
    async fn default_attest_fills_in_when_summary_omits() {
        let mut summary = full_claim_summary();
        summary.attest = None; // signer should pick default

        let mut config = test_config();
        config.default_attest = Attestation::Gateway; // C

        let signer = ShakenSigner::from_pem(TEST_EC_PEM, config).expect("load PEM");
        let value = signer.sign(summary).await.expect("sign");

        let (_h, payload_bytes, _s) = split_compact_jwt(&value.jwt).expect("split");
        let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).expect("json");
        assert_eq!(payload["attest"], "C");
    }

    #[tokio::test]
    async fn invalid_attest_letter_rejected() {
        let mut summary = full_claim_summary();
        summary.attest = Some("X".into());

        let signer = ShakenSigner::from_pem(TEST_EC_PEM, test_config()).expect("load PEM");
        let err = signer.sign(summary).await.expect_err("should reject");
        assert_eq!(err, SignerErrorKind::InvalidClaims);
    }

    #[tokio::test]
    async fn missing_orig_or_dest_rejected() {
        // No orig info at all
        let summary = PassportClaimSummary {
            orig_tn: None,
            orig_uri: None,
            dest_tn: Some("+1".into()),
            dest_uri: None,
            iat: 0,
            origid: None,
            attest: None,
            ppt: None,
        };
        let signer = ShakenSigner::from_pem(TEST_EC_PEM, test_config()).expect("load PEM");
        let err = signer.sign(summary).await.expect_err("should reject");
        assert_eq!(err, SignerErrorKind::InvalidClaims);
    }

    #[tokio::test]
    async fn bad_pem_rejected_at_load_time() {
        let bad_pem = b"-----BEGIN PRIVATE KEY-----\ngarbage\n-----END PRIVATE KEY-----\n";
        let err = ShakenSigner::from_pem(bad_pem, test_config()).expect_err("should reject");
        assert_eq!(err, SignerErrorKind::KeyUnavailable);
    }

    #[test]
    fn split_compact_jwt_round_trips() {
        // header_b64 / payload_b64 / signature_b64 — header and
        // payload are returned decoded; signature is left in
        // base64url form so callers can decide what to do with it.
        let jwt = "aGVhZGVy.cGF5bG9hZA.c2ln";
        let (h, p, s) = split_compact_jwt(jwt).expect("split");
        assert_eq!(h, b"header");
        assert_eq!(p, b"payload");
        assert_eq!(s, "c2ln");
    }

    #[test]
    fn split_compact_jwt_rejects_two_dots() {
        // 3 dots = 4 segments — invalid
        assert!(split_compact_jwt("a.b.c.d").is_err());
        // 1 dot = 2 segments — invalid
        assert!(split_compact_jwt("a.b").is_err());
    }
}

//! End-to-end sign → verify round-trip for STIR/SHAKEN.
//!
//! Generates a fresh P-256 keypair + self-signed cert at test time,
//! signs a PASSporT with the matching `ShakenSigner`, then runs the
//! `ShakenVerifier` against a `CertResolver` stub that returns the
//! generated cert. This proves the JWS signature path is correct
//! end-to-end (header.payload computation, ES256 signing, SPKI
//! extraction, JWS verification) without any external dependencies.
//!
//! Chain-validation is out of scope for these tests (the reference
//! verifier trusts the cert returned by the resolver).

use async_trait::async_trait;
use bytes::Bytes;
use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};
use rvoip_sip_core::types::address::Address;
use rvoip_sip_core::types::from::From as FromHeader;
use rvoip_sip_core::types::identity::Identity;
use rvoip_sip_core::types::to::To as ToHeader;
use rvoip_sip_core::types::TypedHeader;
use rvoip_sip_core::{Method, Request};
use rvoip_sip_dialog::manager::{
    PASSporTSigner, PASSporTVerifier, PassportClaimSummary, VerificationOutcome,
};
use rvoip_stir_shaken::{
    CertResolver, ShakenSigner, ShakenSignerConfig, ShakenVerifier, ShakenVerifierConfig,
    VerifierError,
};
use std::sync::Arc;
use url::Url;

const CERT_URL: &str = "https://cert.test.example/p.cer";

/// Holds the freshly generated cert pair so signer + resolver can
/// share key material in a single test invocation.
struct TestCertPair {
    /// PEM-encoded EC private key — fed to ShakenSigner::from_pem.
    private_key_pem: String,
    /// DER-encoded X.509 cert — returned by the resolver stub.
    cert_der: Vec<u8>,
}

impl TestCertPair {
    fn generate() -> Self {
        let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("keypair");
        let private_key_pem = key_pair.serialize_pem();

        let mut params = CertificateParams::new(vec!["test.example".into()]).expect("params");
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "STIR-SHAKEN test cert");

        let cert = params
            .self_signed(&key_pair)
            .expect("self-sign");
        let cert_der = cert.der().to_vec();

        Self {
            private_key_pem,
            cert_der,
        }
    }
}

/// CertResolver stub that always returns the test cert.
struct StubResolver {
    cert_der: Vec<u8>,
}

#[async_trait]
impl CertResolver for StubResolver {
    async fn fetch(&self, _url: &Url) -> Result<Vec<u8>, VerifierError> {
        Ok(self.cert_der.clone())
    }
}

fn build_invite(from_tn: &str, to_tn: &str) -> Request {
    let from_uri: rvoip_sip_core::types::uri::Uri = format!("tel:{}", from_tn).parse().unwrap();
    let to_uri: rvoip_sip_core::types::uri::Uri = format!("tel:{}", to_tn).parse().unwrap();
    Request::new(Method::Invite, to_uri.clone())
        .with_header(TypedHeader::From(FromHeader::new(Address::new(from_uri))))
        .with_header(TypedHeader::To(ToHeader::new(Address::new(to_uri))))
}

fn matching_claims(from_tn: &str, to_tn: &str) -> PassportClaimSummary {
    PassportClaimSummary {
        orig_tn: Some(from_tn.to_string()),
        orig_uri: Some(format!("tel:{}", from_tn)),
        dest_tn: Some(to_tn.to_string()),
        dest_uri: Some(format!("tel:{}", to_tn)),
        iat: 0,
        origid: Some(uuid::Uuid::new_v4()),
        attest: Some("A".into()),
        ppt: Some("shaken".into()),
    }
}

#[tokio::test]
async fn full_sign_then_verify_round_trip_yields_valid() {
    let pair = TestCertPair::generate();
    let cert_url = Url::parse(CERT_URL).unwrap();

    let signer = ShakenSigner::from_pem(
        pair.private_key_pem.as_bytes(),
        ShakenSignerConfig::new(cert_url.clone()),
    )
    .expect("load signer");

    let claims = matching_claims("+15551234567", "+15559876543");
    let identity_value = signer.sign(claims).await.expect("sign");

    let identity = Identity::with_params(
        identity_value.jwt,
        Some(identity_value.info),
        Some(identity_value.alg),
        identity_value.ppt,
    );

    let resolver: Arc<dyn CertResolver> = Arc::new(StubResolver {
        cert_der: pair.cert_der,
    });
    let verifier = ShakenVerifier::new(resolver, ShakenVerifierConfig::default());

    let request = build_invite("+15551234567", "+15559876543");
    let raw_bytes = Bytes::from_static(b"INVITE\r\n\r\n");

    let outcome = verifier.verify(&raw_bytes, &identity, &request).await;
    assert!(
        matches!(outcome, VerificationOutcome::Valid { ref attest, .. } if attest.as_deref() == Some("A")),
        "expected Valid attest=A, got {:?}",
        outcome
    );
}

#[tokio::test]
async fn tampered_signature_yields_bad_signature() {
    let pair = TestCertPair::generate();
    let cert_url = Url::parse(CERT_URL).unwrap();

    let signer = ShakenSigner::from_pem(
        pair.private_key_pem.as_bytes(),
        ShakenSignerConfig::new(cert_url),
    )
    .expect("load signer");

    let identity_value = signer
        .sign(matching_claims("+15551234567", "+15559876543"))
        .await
        .expect("sign");

    // Flip a bit in the signature segment.
    let mut jwt = identity_value.jwt.clone();
    {
        // Get the signature index — text after the last `.`.
        let dot = jwt.rfind('.').unwrap();
        let sig_start = dot + 1;
        // SAFETY: ASCII string; flipping a base64url char to a
        // different valid one (A↔B) changes the signature without
        // breaking the JWT format.
        unsafe {
            let bytes = jwt.as_bytes_mut();
            let c = bytes[sig_start];
            bytes[sig_start] = if c == b'A' { b'B' } else { b'A' };
        }
    }

    let identity = Identity::with_params(
        jwt,
        Some(identity_value.info),
        Some(identity_value.alg),
        identity_value.ppt,
    );

    let resolver: Arc<dyn CertResolver> = Arc::new(StubResolver {
        cert_der: pair.cert_der,
    });
    let verifier = ShakenVerifier::new(resolver, ShakenVerifierConfig::default());
    let request = build_invite("+15551234567", "+15559876543");
    let raw_bytes = Bytes::from_static(b"INVITE\r\n\r\n");

    let outcome = verifier.verify(&raw_bytes, &identity, &request).await;
    assert!(
        matches!(outcome, VerificationOutcome::BadSignature),
        "expected BadSignature, got {:?}",
        outcome
    );
}

#[tokio::test]
async fn claim_mismatch_on_from_uri_rejects() {
    let pair = TestCertPair::generate();
    let cert_url = Url::parse(CERT_URL).unwrap();

    let signer = ShakenSigner::from_pem(
        pair.private_key_pem.as_bytes(),
        ShakenSignerConfig::new(cert_url),
    )
    .expect("load signer");

    // Sign claims for +1...4567, but submit a SIP request with a different From.
    let identity_value = signer
        .sign(matching_claims("+15551234567", "+15559876543"))
        .await
        .expect("sign");

    let identity = Identity::with_params(
        identity_value.jwt,
        Some(identity_value.info),
        Some(identity_value.alg),
        identity_value.ppt,
    );

    let resolver: Arc<dyn CertResolver> = Arc::new(StubResolver {
        cert_der: pair.cert_der,
    });
    let verifier = ShakenVerifier::new(resolver, ShakenVerifierConfig::default());

    // SIP From uses a DIFFERENT TN than the signed PASSporT claims.
    let request = build_invite("+15559999999", "+15559876543");
    let raw_bytes = Bytes::from_static(b"INVITE\r\n\r\n");

    let outcome = verifier.verify(&raw_bytes, &identity, &request).await;
    assert!(
        matches!(outcome, VerificationOutcome::ClaimMismatch { field: "orig" }),
        "expected ClaimMismatch on orig, got {:?}",
        outcome
    );
}

#[tokio::test]
async fn stale_iat_outside_window_rejects() {
    let pair = TestCertPair::generate();
    let cert_url = Url::parse(CERT_URL).unwrap();

    let mut signer_config = ShakenSignerConfig::new(cert_url);
    // Sign with iat way in the past (5 minutes ago).
    signer_config.iat_skew_secs = -300;
    let signer = ShakenSigner::from_pem(pair.private_key_pem.as_bytes(), signer_config)
        .expect("load signer");

    let identity_value = signer
        .sign(matching_claims("+15551234567", "+15559876543"))
        .await
        .expect("sign");

    let identity = Identity::with_params(
        identity_value.jwt,
        Some(identity_value.info),
        Some(identity_value.alg),
        identity_value.ppt,
    );

    let resolver: Arc<dyn CertResolver> = Arc::new(StubResolver {
        cert_der: pair.cert_der,
    });
    // Default verifier config: 60-s freshness window. The signed
    // PASSporT is 5 minutes stale → should be rejected.
    let verifier = ShakenVerifier::new(resolver, ShakenVerifierConfig::default());
    let request = build_invite("+15551234567", "+15559876543");
    let raw_bytes = Bytes::from_static(b"INVITE\r\n\r\n");

    let outcome = verifier.verify(&raw_bytes, &identity, &request).await;
    assert!(
        matches!(outcome, VerificationOutcome::Stale { skew_secs } if skew_secs > 60),
        "expected Stale with skew > 60, got {:?}",
        outcome
    );
}

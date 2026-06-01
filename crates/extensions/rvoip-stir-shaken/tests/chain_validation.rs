//! Acceptance suite for STI-CA chain validation and SHAKEN cert-profile
//! enforcement. All test certs are built at test time with rcgen — no
//! external SHAKEN material is required.
//!
//! Tests cover:
//! 1. Empty TrustStore → legacy passthrough.
//! 2. Valid 2-level chain + SPC TNAuthList → Valid.
//! 3. Untrusted root → BadChain.
//! 4. Expired leaf → BadChain.
//! 5. P-384 chain (non-ES256) → BadChain.
//! 6. TNAuthList extension missing → BadChain.
//! 7. TNAuthList TN-only, orig.tn unauthorised → BadChain.
//! 8. JWT Claim Constraints rejects out-of-set attest → BadChain.
//! 9. JWT Claim Constraints absent → not rejected.

mod common;

use bytes::Bytes;
use common::{resolver_for, JccSpec, LeafSpec, TestPki, TnAuthEntry};
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
    ShakenSigner, ShakenSignerConfig, ShakenVerifier, ShakenVerifierConfig, TrustStore,
};
use time::{Duration as TimeDuration, OffsetDateTime};
use url::Url;

const CERT_URL: &str = "https://cert.test.example/p.cer";
const ORIG_TN: &str = "+15551234567";
const DEST_TN: &str = "+15559876543";

fn build_invite(from_tn: &str, to_tn: &str) -> Request {
    let from_uri: rvoip_sip_core::types::uri::Uri = format!("tel:{}", from_tn).parse().unwrap();
    let to_uri: rvoip_sip_core::types::uri::Uri = format!("tel:{}", to_tn).parse().unwrap();
    Request::new(Method::Invite, to_uri.clone())
        .with_header(TypedHeader::From(FromHeader::new(Address::new(from_uri))))
        .with_header(TypedHeader::To(ToHeader::new(Address::new(to_uri))))
}

fn matching_claims(from_tn: &str, to_tn: &str, attest: &str) -> PassportClaimSummary {
    PassportClaimSummary {
        orig_tn: Some(from_tn.to_string()),
        orig_uri: Some(format!("tel:{}", from_tn)),
        dest_tn: Some(to_tn.to_string()),
        dest_uri: Some(format!("tel:{}", to_tn)),
        iat: 0,
        origid: Some(uuid::Uuid::new_v4()),
        attest: Some(attest.to_string()),
        ppt: Some("shaken".into()),
    }
}

async fn sign_and_verify(
    pki: &TestPki,
    config: ShakenVerifierConfig,
    orig_tn: &str,
    dest_tn: &str,
    attest: &str,
) -> VerificationOutcome {
    let cert_url = Url::parse(CERT_URL).unwrap();
    let signer = ShakenSigner::from_pem(
        pki.leaf_key_pem().as_bytes(),
        ShakenSignerConfig::new(cert_url),
    )
    .expect("load signer");

    let identity_value = signer
        .sign(matching_claims(orig_tn, dest_tn, attest))
        .await
        .expect("sign");

    let identity = Identity::with_params(
        identity_value.jwt,
        Some(identity_value.info),
        Some(identity_value.alg),
        identity_value.ppt,
    );

    let resolver = resolver_for(pki.leaf_der());
    let verifier = ShakenVerifier::new(resolver, config);

    let request = build_invite(orig_tn, dest_tn);
    let raw_bytes = Bytes::from_static(b"INVITE\r\n\r\n");
    verifier.verify(&raw_bytes, &identity, &request).await
}

// ---------- 1. No anchors → legacy passthrough ----------

#[tokio::test]
async fn empty_trust_store_preserves_legacy_behaviour() {
    // Leaf with no TNAuthList and no anchors — should still Valid,
    // because chain + profile checks are skipped entirely.
    let pki = TestPki::build(&LeafSpec {
        tnauth_entries: vec![],
        ..LeafSpec::default()
    });
    let outcome =
        sign_and_verify(&pki, ShakenVerifierConfig::default(), ORIG_TN, DEST_TN, "A").await;
    assert!(
        matches!(outcome, VerificationOutcome::Valid { .. }),
        "expected Valid (legacy passthrough), got {:?}",
        outcome
    );
}

// ---------- 2. Valid chain ----------

#[tokio::test]
async fn valid_chain_with_spc_tnauth_list_yields_valid() {
    let pki = TestPki::build(&LeafSpec::default());
    let config = ShakenVerifierConfig::default()
        .with_trust_anchors(TrustStore::from_der_certs(vec![pki.root_der()]));
    let outcome = sign_and_verify(&pki, config, ORIG_TN, DEST_TN, "A").await;
    assert!(
        matches!(outcome, VerificationOutcome::Valid { ref attest, .. } if attest.as_deref() == Some("A")),
        "expected Valid, got {:?}",
        outcome
    );
}

// ---------- 3. Untrusted root ----------

#[tokio::test]
async fn untrusted_root_yields_bad_chain() {
    let pki_a = TestPki::build(&LeafSpec::default());
    let pki_b = TestPki::build(&LeafSpec::default());
    // Trust only B's root; resolver returns A's leaf.
    let config = ShakenVerifierConfig::default()
        .with_trust_anchors(TrustStore::from_der_certs(vec![pki_b.root_der()]));
    let outcome = sign_and_verify(&pki_a, config, ORIG_TN, DEST_TN, "A").await;
    assert!(
        matches!(outcome, VerificationOutcome::BadChain { .. }),
        "expected BadChain (untrusted root), got {:?}",
        outcome
    );
}

// ---------- 4. Expired leaf ----------

#[tokio::test]
async fn expired_leaf_yields_bad_chain() {
    let pki = TestPki::build(&LeafSpec {
        not_after: Some(OffsetDateTime::now_utc() - TimeDuration::days(1)),
        ..LeafSpec::default()
    });
    let config = ShakenVerifierConfig::default()
        .with_trust_anchors(TrustStore::from_der_certs(vec![pki.root_der()]));
    let outcome = sign_and_verify(&pki, config, ORIG_TN, DEST_TN, "A").await;
    assert!(
        matches!(outcome, VerificationOutcome::BadChain { .. }),
        "expected BadChain (expired), got {:?}",
        outcome
    );
}

// ---------- 5. Non-ES256 chain ----------

#[tokio::test]
async fn p384_chain_yields_bad_chain() {
    // webpki's allowed algorithm list is ECDSA_P256_SHA256 only;
    // a chain signed with ECDSA_P384_SHA384 (root → leaf) must be
    // rejected. Leaf key stays P-256 so the JWS still signs ES256.
    let pki = TestPki::build(&LeafSpec {
        p384_root: true,
        ..LeafSpec::default()
    });
    let config = ShakenVerifierConfig::default()
        .with_trust_anchors(TrustStore::from_der_certs(vec![pki.root_der()]));
    let outcome = sign_and_verify(&pki, config, ORIG_TN, DEST_TN, "A").await;
    assert!(
        matches!(outcome, VerificationOutcome::BadChain { .. }),
        "expected BadChain (non-ES256 chain), got {:?}",
        outcome
    );
}

// ---------- 6. TNAuthList missing ----------

#[tokio::test]
async fn missing_tnauth_list_yields_bad_chain() {
    let pki = TestPki::build(&LeafSpec {
        tnauth_entries: vec![],
        ..LeafSpec::default()
    });
    let config = ShakenVerifierConfig::default()
        .with_trust_anchors(TrustStore::from_der_certs(vec![pki.root_der()]));
    let outcome = sign_and_verify(&pki, config, ORIG_TN, DEST_TN, "A").await;
    match outcome {
        VerificationOutcome::BadChain { reason } => {
            assert!(
                reason.contains("TNAuthList"),
                "BadChain reason should mention TNAuthList, got {}",
                reason
            );
        }
        other => panic!("expected BadChain (TNAuthList missing), got {:?}", other),
    }
}

// ---------- 7. TNAuthList TN-only, orig.tn unauthorised ----------

#[tokio::test]
async fn unauthorised_orig_tn_yields_bad_chain() {
    // Leaf authorises a specific TN only — not the one we sign as orig.
    let pki = TestPki::build(&LeafSpec {
        tnauth_entries: vec![TnAuthEntry::Tn("+15558881111".into())],
        ..LeafSpec::default()
    });
    let config = ShakenVerifierConfig::default()
        .with_trust_anchors(TrustStore::from_der_certs(vec![pki.root_der()]));
    let outcome = sign_and_verify(&pki, config, ORIG_TN, DEST_TN, "A").await;
    match outcome {
        VerificationOutcome::BadChain { reason } => {
            assert!(
                reason.contains("not authorised"),
                "BadChain reason should mention authorisation, got {}",
                reason
            );
        }
        other => panic!("expected BadChain (unauthorised orig.tn), got {:?}", other),
    }
}

// ---------- 8. JWT Claim Constraints rejects out-of-set attest ----------

#[tokio::test]
async fn attest_outside_jcc_yields_bad_chain() {
    let pki = TestPki::build(&LeafSpec {
        jcc: Some(JccSpec {
            permitted: vec![("attest".into(), vec!["A".into()])],
        }),
        ..LeafSpec::default()
    });
    let config = ShakenVerifierConfig::default()
        .with_trust_anchors(TrustStore::from_der_certs(vec![pki.root_der()]));
    // Sign with attest="B" while leaf only permits "A".
    let outcome = sign_and_verify(&pki, config, ORIG_TN, DEST_TN, "B").await;
    match outcome {
        VerificationOutcome::BadChain { reason } => {
            assert!(
                reason.contains("attest"),
                "BadChain reason should mention attest, got {}",
                reason
            );
        }
        other => panic!("expected BadChain (attest constraint), got {:?}", other),
    }
}

// ---------- 9. JCC absent → not rejected ----------

#[tokio::test]
async fn jcc_absent_does_not_block_arbitrary_attest() {
    let pki = TestPki::build(&LeafSpec {
        // SPC entry authorises any orig.tn; no JCC extension.
        ..LeafSpec::default()
    });
    let config = ShakenVerifierConfig::default()
        .with_trust_anchors(TrustStore::from_der_certs(vec![pki.root_der()]));
    let outcome = sign_and_verify(&pki, config, ORIG_TN, DEST_TN, "C").await;
    assert!(
        matches!(outcome, VerificationOutcome::Valid { ref attest, .. } if attest.as_deref() == Some("C")),
        "expected Valid with attest=C, got {:?}",
        outcome
    );
}

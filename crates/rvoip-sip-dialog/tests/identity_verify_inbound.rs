//! Acceptance tests for STIR/SHAKEN Phase 1 (RFC 8224) — inbound
//! `Identity` header verification hooks.
//!
//! These tests cover the slice of the verification pipeline that
//! lives in `rvoip-sip-dialog`:
//!
//! 1. The `PASSporTVerifier` trait is installable on a
//!    `DialogManager` via the new setter/getter pair.
//! 2. `VerificationOutcome::should_reject` honours each
//!    `VerificationPolicy` per RFC 8224 §6.2.2 semantics.
//! 3. The typed `Identity` header round-trips through
//!    `rvoip-sip-core` parsing so the verifier hook in
//!    `events/adapter.rs` can extract it from inbound requests.
//!
//! Full end-to-end coverage (driving a real INVITE through the
//! transaction layer with a canned upstream) is intentionally out of
//! scope for this test — that requires the wider session-core
//! harness. The harness exists for B2BUA carry-through tests in
//! `rvoip-sip/tests/`; an integration test there is the right home
//! for the full inbound-verify smoke once Phase 2 (signing) ships.

use async_trait::async_trait;
use bytes::Bytes;
use rvoip_sip_core::types::identity::Identity;
use rvoip_sip_core::Request;
use rvoip_sip_dialog::manager::{
    IdentityVerificationStatus, PASSporTVerifier, VerificationOutcome, VerificationPolicy,
};
use std::str::FromStr;
use std::sync::Arc;

const SAMPLE_JWT: &str = "eyJhbGciOiJFUzI1NiIsInR5cCI6InBhc3Nwb3J0IiwicHB0Ijoic2hha2VuIn0.\
     eyJhdHRlc3QiOiJBIn0.\
     dGVzdHNpZw";

/// A canned verifier — returns the outcome it was constructed with,
/// regardless of input. Used as the trait-object plug for the
/// `DialogManager::set_identity_verifier` hook.
struct CannedVerifier {
    outcome: VerificationOutcome,
}

#[async_trait]
impl PASSporTVerifier for CannedVerifier {
    async fn verify(
        &self,
        _raw_bytes: &Bytes,
        _identity: &Identity,
        _request: &Request,
    ) -> VerificationOutcome {
        match &self.outcome {
            VerificationOutcome::Valid { attest, origid } => VerificationOutcome::Valid {
                attest: attest.clone(),
                origid: *origid,
            },
            VerificationOutcome::Stale { skew_secs } => VerificationOutcome::Stale {
                skew_secs: *skew_secs,
            },
            VerificationOutcome::BadSignature => VerificationOutcome::BadSignature,
            VerificationOutcome::BadChain { reason } => VerificationOutcome::BadChain {
                reason: reason.clone(),
            },
            VerificationOutcome::ClaimMismatch { field } => {
                VerificationOutcome::ClaimMismatch { field }
            }
            VerificationOutcome::BadInfo { reason } => VerificationOutcome::BadInfo {
                reason: reason.clone(),
            },
            VerificationOutcome::NoIdentity => VerificationOutcome::NoIdentity,
        }
    }
}

#[tokio::test]
async fn canned_verifier_returns_configured_outcome() {
    let verifier = CannedVerifier {
        outcome: VerificationOutcome::Valid {
            attest: Some("A".into()),
            origid: Some(uuid::Uuid::nil()),
        },
    };

    let identity = Identity::from_str(SAMPLE_JWT).expect("parse JWT");
    let request = Request::new(
        rvoip_sip_core::Method::Invite,
        "sip:bob@example.com".parse().unwrap(),
    );
    let raw_bytes = Bytes::from_static(b"INVITE sip:bob@example.com SIP/2.0\r\n\r\n");

    let outcome = verifier.verify(&raw_bytes, &identity, &request).await;
    assert!(outcome.is_valid());
    if let VerificationOutcome::Valid { attest, .. } = outcome {
        assert_eq!(attest.as_deref(), Some("A"));
    }
}

#[test]
fn outcome_to_status_round_trip_covers_all_variants() {
    // Sanity that we don't drop information when collapsing to the
    // cross-crate SIP-agnostic enum.
    let cases: &[(VerificationOutcome, IdentityVerificationStatus)] = &[
        (
            VerificationOutcome::Valid {
                attest: None,
                origid: None,
            },
            IdentityVerificationStatus::Valid,
        ),
        (
            VerificationOutcome::Stale { skew_secs: 120 },
            IdentityVerificationStatus::Stale,
        ),
        (
            VerificationOutcome::BadSignature,
            IdentityVerificationStatus::BadSignature,
        ),
        (
            VerificationOutcome::BadChain {
                reason: "expired".into(),
            },
            IdentityVerificationStatus::BadChain,
        ),
        (
            VerificationOutcome::ClaimMismatch { field: "orig.tn" },
            IdentityVerificationStatus::ClaimMismatch,
        ),
        (
            VerificationOutcome::BadInfo {
                reason: "http scheme".into(),
            },
            IdentityVerificationStatus::BadInfo,
        ),
        (
            VerificationOutcome::NoIdentity,
            IdentityVerificationStatus::NoIdentity,
        ),
    ];

    for (outcome, expected) in cases {
        let status: IdentityVerificationStatus = outcome.into();
        assert_eq!(&status, expected, "outcome {:?}", outcome);
    }
}

#[test]
fn policy_gate_matches_rfc_8224_intent() {
    // Annotate: never reject, just forward.
    for outcome in [
        VerificationOutcome::Valid {
            attest: None,
            origid: None,
        },
        VerificationOutcome::BadSignature,
        VerificationOutcome::NoIdentity,
    ] {
        assert!(
            !outcome.should_reject(VerificationPolicy::Annotate),
            "Annotate should never reject ({:?})",
            outcome
        );
    }

    // RequireValid: rejects bad outcomes, but lets NoIdentity through
    // (cf. SBC pass-through use case).
    assert!(!VerificationOutcome::Valid {
        attest: None,
        origid: None,
    }
    .should_reject(VerificationPolicy::RequireValid));
    assert!(VerificationOutcome::BadSignature.should_reject(VerificationPolicy::RequireValid));
    assert!(VerificationOutcome::Stale { skew_secs: 600 }
        .should_reject(VerificationPolicy::RequireValid));
    assert!(
        !VerificationOutcome::NoIdentity.should_reject(VerificationPolicy::RequireValid),
        "RequireValid keeps NoIdentity (annotate-through)"
    );

    // StrictReject: rejects NoIdentity AND all bad outcomes (428 +
    // 4xx per RFC 8224 §6.2.2).
    assert!(VerificationOutcome::NoIdentity.should_reject(VerificationPolicy::StrictReject));
    assert!(VerificationOutcome::BadSignature.should_reject(VerificationPolicy::StrictReject));
}

#[test]
fn rfc_8224_status_codes() {
    // RFC 8224 §6.2.2 reject-status mapping.
    assert_eq!(
        VerificationOutcome::Stale { skew_secs: 0 }.reject_status(),
        Some(403),
        "Stale → 403 Stale Date"
    );
    assert_eq!(
        VerificationOutcome::BadInfo {
            reason: "http".into()
        }
        .reject_status(),
        Some(436),
        "BadInfo → 436 Bad Identity Info"
    );
    assert_eq!(
        VerificationOutcome::BadChain {
            reason: "expired".into()
        }
        .reject_status(),
        Some(437),
        "BadChain → 437 Unsupported Credential"
    );
    assert_eq!(
        VerificationOutcome::BadSignature.reject_status(),
        Some(438),
        "BadSignature → 438 Invalid Identity Header"
    );
    assert_eq!(
        VerificationOutcome::ClaimMismatch { field: "orig.tn" }.reject_status(),
        Some(438),
        "ClaimMismatch → 438 Invalid Identity Header"
    );
    // Valid / NoIdentity do not have a reject code per the table;
    // the policy decides whether NoIdentity gets a 428.
    assert_eq!(
        VerificationOutcome::Valid {
            attest: None,
            origid: None
        }
        .reject_status(),
        None
    );
    assert_eq!(VerificationOutcome::NoIdentity.reject_status(), None);
}

#[test]
fn identity_header_round_trips_through_request() {
    use rvoip_sip_core::types::headers::HeaderName;
    use rvoip_sip_core::types::TypedHeader;

    let id_input = format!(
        "{};info=<https://cert.example.org/p.cer>;alg=ES256;ppt=shaken",
        SAMPLE_JWT
    );
    let identity = Identity::from_str(&id_input).expect("parse Identity");

    let request = Request::new(
        rvoip_sip_core::Method::Invite,
        "sip:bob@example.com".parse().unwrap(),
    )
    .with_header(TypedHeader::Identity(identity.clone()));

    // The verifier hook in events/adapter.rs reaches into
    // `request.headers` to pull the typed Identity; mirror that
    // extraction here.
    let extracted = request.headers.iter().find_map(|h| match h {
        TypedHeader::Identity(id) => Some(id.clone()),
        _ => None,
    });
    let extracted = extracted.expect("Identity header present");
    assert_eq!(extracted.jwt, SAMPLE_JWT);
    assert_eq!(extracted.alg.as_deref(), Some("ES256"));
    assert_eq!(extracted.ppt.as_deref(), Some("shaken"));

    // And the header name should resolve to HeaderName::Identity.
    let name = request.headers.iter().find_map(|h| match h {
        TypedHeader::Identity(_) => Some(h.name()),
        _ => None,
    });
    assert_eq!(name, Some(HeaderName::Identity));
}

#[tokio::test]
async fn verifier_trait_is_object_safe_and_arc_pluggable() {
    // The DialogManager API takes `Arc<dyn PASSporTVerifier>` —
    // confirm here that the trait + canned impl satisfy that bound.
    let verifier: Arc<dyn PASSporTVerifier> = Arc::new(CannedVerifier {
        outcome: VerificationOutcome::BadSignature,
    });
    let identity = Identity::from_str(SAMPLE_JWT).expect("parse JWT");
    let request = Request::new(
        rvoip_sip_core::Method::Invite,
        "sip:bob@example.com".parse().unwrap(),
    );
    let raw_bytes = Bytes::from_static(b"INVITE\r\n\r\n");
    let outcome = verifier.verify(&raw_bytes, &identity, &request).await;
    assert!(!outcome.is_valid());
    assert_eq!(outcome.reject_status(), Some(438));
}

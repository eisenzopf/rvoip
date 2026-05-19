//! # STIR/SHAKEN Identity Verification (RFC 8224)
//!
//! Defines the pluggable trait surface for inbound `Identity` header
//! verification. The trait lives here (in `rvoip-sip-dialog`) because
//! the verifier hook attaches at the dialog-event adapter — same site
//! that already pulls the byte-exact `Arc<Bytes>` from
//! `TransactionManager::take_inbound_bytes`. The reference
//! implementations (signing, verification, certificate-chain
//! validation, JWS handling) live in the optional sibling crate
//! `rvoip-stir-shaken`.
//!
//! ## Why a trait
//!
//! Verification needs operator-supplied trust anchors (SHAKEN STI-CA
//! roots), policy decisions (iat freshness window, attestation-level
//! acceptance), and a certificate-fetch strategy (HTTPS GET with
//! caching). None of those belong in a SIP library; they belong in the
//! application. The trait lets the application plug in its own
//! verifier — typically the reference `rvoip_stir_shaken::ShakenVerifier`
//! wrapped in a caching layer.
//!
//! ## Lifecycle
//!
//! ```text
//! UAS inbound:
//!   socket → parse → TransactionManager.pending_inbound_bytes[tx_key]
//!         → dispatch to dialog handlers
//!         → events/adapter.rs.convert_coordination_to_cross_crate_event:
//!               raw = take_inbound_bytes(tx_id)
//!               identity = IdentityHeader from request (if present)
//!               outcome = verifier.verify(&raw, &identity, &request).await
//!               [policy check on VerificationOutcome]
//!         → DialogToSessionEvent::IncomingCall { raw_request, identity_verification }
//! ```
//!
//! ## Failure handling
//!
//! Verifiers return a [`VerificationOutcome`] rather than `Result`. The
//! dialog adapter consults [`VerificationPolicy`] to decide whether to
//! ship the outcome through to the session layer (`Annotate`) or to
//! short-circuit with a 4xx response (`RequireValid` /
//! `StrictReject`). The mapping to RFC 8224 §6.2.2 status codes:
//!
//! | Outcome                | RFC 8224 status |
//! |---|---|
//! | `BadSignature`         | 438 Invalid Identity Header |
//! | `BadInfo`              | 436 Bad Identity Info |
//! | `BadChain`             | 437 Unsupported Credential |
//! | `Stale`                | 403 Stale Date |
//! | `NoIdentity` (StrictReject only) | 428 Use Identity Header |
//! | `ClaimMismatch`        | 438 Invalid Identity Header |

use async_trait::async_trait;
use bytes::Bytes;
use rvoip_sip_core::types::identity::Identity;
use rvoip_sip_core::Request;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Pluggable verifier trait for RFC 8224 `Identity` headers.
///
/// Implementations receive:
/// - `raw_bytes` — the byte-exact upstream SIP request (preserved by
///   the transport layer; not re-serialized). Used so the JWS
///   signature input matches what the upstream signer produced.
/// - `identity` — the parsed `Identity` header (JWT + info/alg/ppt
///   parameters).
/// - `request` — the parsed `Request` for cross-checking PASSporT
///   `orig`/`dest` claims against SIP `From`/`To`/`P-Asserted-Identity`.
///
/// Implementations must NOT mutate any input.
#[async_trait]
pub trait PASSporTVerifier: Send + Sync {
    async fn verify(
        &self,
        raw_bytes: &Bytes,
        identity: &Identity,
        request: &Request,
    ) -> VerificationOutcome;
}

/// Pluggable signer trait for RFC 8224 `Identity` headers.
///
/// Called by the outbound request lifecycle (`RequestLifecycle::pre_send_request`)
/// after Via/Max-Forwards/Route stamping but before the message hits
/// the wire. Implementations build a JWS-signed PASSporT JWT from the
/// supplied claim values and the signer's configured private key.
///
/// The dialog layer wraps the returned `IdentityHeaderValue` in a
/// `TypedHeader::Identity` and appends it to the outbound request.
#[async_trait]
pub trait PASSporTSigner: Send + Sync {
    async fn sign(
        &self,
        claims: PassportClaimSummary,
    ) -> Result<IdentityHeaderValue, SignerErrorKind>;
}

/// Per-deployment policy for what to do with the verification outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerificationPolicy {
    /// Forward the outcome to the session layer without rejecting.
    /// The application decides how to react. This is the SBC
    /// pass-through default and is the safest behaviour for
    /// non-PSTN deployments.
    #[default]
    Annotate,

    /// Reject when verification fails (`BadSignature` / `BadChain` /
    /// `Stale` / `ClaimMismatch` / `BadInfo`). Missing-Identity
    /// (`NoIdentity`) is still annotated through.
    RequireValid,

    /// Reject on any non-`Valid` outcome including `NoIdentity`. Per
    /// RFC 8224, missing Identity becomes 428 Use Identity Header.
    StrictReject,
}

/// Outcome of a PASSporT verification attempt. Returned by
/// [`PASSporTVerifier::verify`] and (when the policy is `Annotate`)
/// carried unchanged to the session layer via
/// `DialogToSessionEvent::IncomingCall.identity_verification`.
#[derive(Debug, Clone)]
pub enum VerificationOutcome {
    /// Signature verified, cert chain valid, claims match the SIP
    /// request, iat within freshness window. Attestation level is
    /// surfaced for downstream policy.
    Valid {
        attest: Option<String>,
        origid: Option<uuid::Uuid>,
    },

    /// `iat` claim outside the deployment's freshness window. RFC 8224
    /// §6.2.2 — maps to 403 Stale Date.
    Stale { skew_secs: i64 },

    /// JWS signature did not verify against the certificate's public
    /// key. Maps to 438 Invalid Identity Header.
    BadSignature,

    /// Certificate chain failed to validate to a trusted SHAKEN root.
    /// Maps to 437 Unsupported Credential.
    BadChain { reason: String },

    /// PASSporT claim does not match the SIP request (e.g. `orig.tn`
    /// vs. `From` URI mismatch). Maps to 438 Invalid Identity Header.
    ClaimMismatch { field: &'static str },

    /// `info=` URL is malformed, missing, or uses an unsupported
    /// scheme (RFC 8224 §6.1 requires HTTPS). Maps to 436 Bad
    /// Identity Info.
    BadInfo { reason: String },

    /// Inbound request has no `Identity` header. Whether this
    /// short-circuits to a reject depends on the policy.
    NoIdentity,
}

impl VerificationOutcome {
    /// True if verification produced an unambiguously valid PASSporT.
    pub fn is_valid(&self) -> bool {
        matches!(self, VerificationOutcome::Valid { .. })
    }

    /// SIP status code per RFC 8224 §6.2.2 if this outcome should
    /// reject the request. `None` for `Valid` and `NoIdentity` —
    /// callers decide the reject behaviour for `NoIdentity` from the
    /// policy.
    pub fn reject_status(&self) -> Option<u16> {
        match self {
            VerificationOutcome::Valid { .. } | VerificationOutcome::NoIdentity => None,
            VerificationOutcome::Stale { .. } => Some(403),
            VerificationOutcome::BadInfo { .. } => Some(436),
            VerificationOutcome::BadChain { .. } => Some(437),
            VerificationOutcome::BadSignature | VerificationOutcome::ClaimMismatch { .. } => {
                Some(438)
            }
        }
    }

    /// True if the configured policy should reject this outcome.
    pub fn should_reject(&self, policy: VerificationPolicy) -> bool {
        match (self, policy) {
            (_, VerificationPolicy::Annotate) => false,
            (VerificationOutcome::Valid { .. }, _) => false,
            (VerificationOutcome::NoIdentity, VerificationPolicy::StrictReject) => true,
            (VerificationOutcome::NoIdentity, VerificationPolicy::RequireValid) => false,
            (_, VerificationPolicy::RequireValid | VerificationPolicy::StrictReject) => true,
        }
    }
}

/// Outcome status enum delivered through the cross-crate event bus.
/// Kept SIP-agnostic and small so `infra-common` doesn't pull rvoip
/// types in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentityVerificationStatus {
    Valid,
    Stale,
    BadSignature,
    BadChain,
    ClaimMismatch,
    BadInfo,
    NoIdentity,
}

impl From<&VerificationOutcome> for IdentityVerificationStatus {
    fn from(outcome: &VerificationOutcome) -> Self {
        match outcome {
            VerificationOutcome::Valid { .. } => IdentityVerificationStatus::Valid,
            VerificationOutcome::Stale { .. } => IdentityVerificationStatus::Stale,
            VerificationOutcome::BadSignature => IdentityVerificationStatus::BadSignature,
            VerificationOutcome::BadChain { .. } => IdentityVerificationStatus::BadChain,
            VerificationOutcome::ClaimMismatch { .. } => IdentityVerificationStatus::ClaimMismatch,
            VerificationOutcome::BadInfo { .. } => IdentityVerificationStatus::BadInfo,
            VerificationOutcome::NoIdentity => IdentityVerificationStatus::NoIdentity,
        }
    }
}

/// Minimal claim shape passed to a signer. Concrete signer
/// implementations (in `rvoip-stir-shaken`) build the full RFC 8225 /
/// RFC 8588 PASSporT from this.
#[derive(Debug, Clone)]
pub struct PassportClaimSummary {
    pub orig_tn: Option<String>,
    pub orig_uri: Option<String>,
    pub dest_tn: Option<String>,
    pub dest_uri: Option<String>,
    pub iat: u64,
    pub origid: Option<uuid::Uuid>,
    /// `"A"` / `"B"` / `"C"` for SHAKEN; `None` for non-SHAKEN profiles.
    pub attest: Option<String>,
    /// PASSporT extension type — `"shaken"` / `"div"` / `"rcd"` / `None`
    /// for base profile.
    pub ppt: Option<String>,
}

/// The result returned by a signer: the compact-form JWT plus the
/// well-known parameters that ride on the `Identity` header.
#[derive(Debug, Clone)]
pub struct IdentityHeaderValue {
    pub jwt: String,
    pub info: String,
    pub alg: String,
    pub ppt: Option<String>,
}

/// Stripped-down signer error. The full `SignerError` lives in
/// `rvoip-stir-shaken`; the dialog layer only needs to distinguish
/// "signer unavailable" (degrade and emit unsigned) from "signer
/// available but failed" (propagate as a 5xx).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignerErrorKind {
    KeyUnavailable,
    SigningFailed,
    InvalidClaims,
}

/// Convenience type alias for the trait-object form used by config.
pub type SharedVerifier = Arc<dyn PASSporTVerifier>;
pub type SharedSigner = Arc<dyn PASSporTSigner>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_reject_status_matches_rfc_8224() {
        assert_eq!(
            VerificationOutcome::Valid {
                attest: None,
                origid: None
            }
            .reject_status(),
            None
        );
        assert_eq!(VerificationOutcome::NoIdentity.reject_status(), None);
        assert_eq!(
            VerificationOutcome::Stale { skew_secs: 90 }.reject_status(),
            Some(403)
        );
        assert_eq!(
            VerificationOutcome::BadInfo {
                reason: "http".into()
            }
            .reject_status(),
            Some(436)
        );
        assert_eq!(
            VerificationOutcome::BadChain {
                reason: "expired".into()
            }
            .reject_status(),
            Some(437)
        );
        assert_eq!(
            VerificationOutcome::BadSignature.reject_status(),
            Some(438)
        );
        assert_eq!(
            VerificationOutcome::ClaimMismatch { field: "orig.tn" }.reject_status(),
            Some(438)
        );
    }

    #[test]
    fn policy_annotate_never_rejects() {
        for outcome in [
            VerificationOutcome::Valid {
                attest: None,
                origid: None,
            },
            VerificationOutcome::NoIdentity,
            VerificationOutcome::BadSignature,
            VerificationOutcome::Stale { skew_secs: 0 },
        ] {
            assert!(!outcome.should_reject(VerificationPolicy::Annotate));
        }
    }

    #[test]
    fn policy_require_valid_rejects_bad_keeps_noidentity() {
        assert!(
            !VerificationOutcome::NoIdentity.should_reject(VerificationPolicy::RequireValid),
            "RequireValid keeps NoIdentity (just annotate)"
        );
        assert!(VerificationOutcome::BadSignature.should_reject(VerificationPolicy::RequireValid));
        assert!(VerificationOutcome::Stale { skew_secs: 90 }
            .should_reject(VerificationPolicy::RequireValid));
        assert!(!VerificationOutcome::Valid {
            attest: None,
            origid: None,
        }
        .should_reject(VerificationPolicy::RequireValid));
    }

    #[test]
    fn policy_strict_reject_rejects_noidentity() {
        assert!(VerificationOutcome::NoIdentity.should_reject(VerificationPolicy::StrictReject));
        assert!(VerificationOutcome::BadSignature.should_reject(VerificationPolicy::StrictReject));
        assert!(!VerificationOutcome::Valid {
            attest: None,
            origid: None,
        }
        .should_reject(VerificationPolicy::StrictReject));
    }

    #[test]
    fn status_conversion_drops_payload() {
        let outcome = VerificationOutcome::Valid {
            attest: Some("A".into()),
            origid: Some(uuid::Uuid::nil()),
        };
        let status: IdentityVerificationStatus = (&outcome).into();
        assert_eq!(status, IdentityVerificationStatus::Valid);
    }
}

//! # rvoip-stir-shaken
//!
//! STIR/SHAKEN signing and verification for the rvoip stack.
//!
//! - **RFC 8224** — Authenticated Identity Management in SIP
//! - **RFC 8225** — PASSporT: Personal Assertion Token
//! - **RFC 8588** — PASSporT SHAKEN Extension
//! - **ATIS-1000074** — Signature-based Handling of Asserted information using toKENs (SHAKEN)
//!
//! ## What lives where
//!
//! - **`rvoip-sip-core`** — typed `Identity` header (RFC 8224 wire form)
//! - **`rvoip-sip-dialog`** — `PASSporTSigner` / `PASSporTVerifier` trait
//!   surface and the inbound-verify / outbound-sign hooks
//! - **`rvoip-stir-shaken`** (this crate) — reference implementations
//!   that wrap `jsonwebtoken`, `x509-parser`, `webpki`, and `reqwest`;
//!   plus the pluggable `CertResolver` trait for SHAKEN STI-CA cert
//!   fetching.
//!
//! Applications that need STIR/SHAKEN depend on both `rvoip-sip-dialog`
//! and `rvoip-stir-shaken`, then install an `Arc<dyn PASSporTVerifier>`
//! / `Arc<dyn PASSporTSigner>` on the `DialogConfig`. Library never
//! bundles SHAKEN root anchors (STI-PA approved-CA list) — those are
//! supplied at runtime by the operator.

pub mod cert_resolver;
pub mod errors;
pub mod profile;
pub mod signer;
pub mod trust;
pub mod types;
pub mod verifier;

// Re-exports for convenience
pub use cert_resolver::{CertResolver, ReqwestCertResolver};
pub use errors::{SignerError, VerifierError};
pub use profile::{
    JwtClaimConstraints, TNAuthList, JWT_CLAIM_CONSTRAINTS_OID, TN_AUTH_LIST_OID,
};
pub use signer::{ShakenSigner, ShakenSignerConfig};
pub use trust::TrustStore;
pub use types::{Attestation, OrigDest, OrigDestField, PassportClaims, PptType};
pub use verifier::{ShakenVerifier, ShakenVerifierConfig};

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
pub use profile::{JwtClaimConstraints, TNAuthList, JWT_CLAIM_CONSTRAINTS_OID, TN_AUTH_LIST_OID};
pub use signer::{ShakenSigner, ShakenSignerConfig};
pub use trust::TrustStore;
pub use types::{Attestation, OrigDest, OrigDestField, PassportClaims, PptType};
pub use verifier::{ShakenVerifier, ShakenVerifierConfig};

/// Start a reqwest client builder whose TLS runs on the ring rustls provider.
///
/// reqwest is built with `rustls-no-provider`, so on its own it would read the
/// process-level rustls default. It is handed a ready `ClientConfig` instead:
/// the provider is named explicitly, and nothing is read from or installed as
/// the process default. Server certificates are checked by the platform
/// verifier, as reqwest's own rustls backend does; if that verifier cannot be
/// initialised, `ClientBuilder::build` returns the error.
pub(crate) fn ring_http_client_builder() -> reqwest::ClientBuilder {
    match ring_https_client_config() {
        Ok(config) => reqwest::Client::builder().tls_backend_preconfigured(config),
        // An unrecognised preconfigured backend makes `build()` fail, which
        // is where reqwest reports its own TLS initialisation errors.
        Err(_) => reqwest::Client::builder().tls_backend_preconfigured(()),
    }
}

/// The HTTPS client configuration behind [`ring_http_client_builder`].
fn ring_https_client_config() -> std::result::Result<rustls::ClientConfig, rustls::Error> {
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let verifier = rustls_platform_verifier::Verifier::new(provider.clone())?;
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(verifier))
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

//! Vapi bidirectional WebSocket voice-agent adapter for rvoip.
//!
//! This crate lets rvoip retain ownership of a SIP or WebRTC caller leg while
//! Vapi owns ASR, dialog, synthesis, and interruption handling on a raw-audio
//! WebSocket call transport.

#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]
#![doc = include_str!("../README.md")]

mod client;
mod media;

pub mod adapter;
pub mod agent;
pub mod config;
pub mod error;
pub mod events;
pub mod health;
mod jitter;
pub mod types;

pub use adapter::{
    VapiAdapter, VapiTransportHandle, ADAPTER_EVENT_CAPACITY, VAPI_CALL_REFERENCE_KIND,
};
pub use agent::{VapiAgentCall, VapiAgentOutcome};
pub use config::{VapiApiKey, VapiConfig};
pub use error::{Result, VapiError};
pub use events::{VapiEvent, VapiEventEnvelope, VapiOrderedEvent};
pub use health::VapiMediaHealth;
pub use types::{
    VapiAssistant, VapiAudioFormat, VapiCallOptions, VapiExistingCall, VapiMediaDelivery,
    VapiPeerFailurePolicy,
};

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

/// A `wss` connector trusting the WebPKI roots, built on the ring rustls
/// provider explicitly.
///
/// Without an explicit connector tungstenite builds one on the process-level
/// rustls default, which rustls cannot infer when a dependency graph compiles
/// both its `ring` and `aws-lc-rs` providers (the AWS SDK brings `aws-lc-rs`).
/// Nothing here reads or installs the process default.
pub(crate) fn ring_websocket_connector(
) -> std::result::Result<tokio_tungstenite::Connector, rustls::Error> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(tokio_tungstenite::Connector::Rustls(std::sync::Arc::new(
        config,
    )))
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

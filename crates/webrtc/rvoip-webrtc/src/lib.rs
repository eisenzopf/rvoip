//! # rvoip-webrtc
//!
//! WebRTC **interop adapter** for [`rvoip_core::ConnectionAdapter`], built on
//! webrtc-rs **0.20.0-alpha.1**.
//!
//! See [`docs/archived/IMPLEMENTATION_PLAN.md`](docs/archived/IMPLEMENTATION_PLAN.md) for architecture
//! and [`docs/archived/HARDENING_PLAN.md`](docs/archived/HARDENING_PLAN.md) for the path to production.

// Phase H1: deny ad-hoc panic paths in the library. Tests / examples may still
// use unwrap/expect via `#[cfg(test)]` and the unwrap_used lint exemption.
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

pub mod adapter;
pub mod config;
pub mod data_message;
pub mod errors;
pub mod identity;
pub mod media;
pub mod observability;
pub mod originate;
#[cfg(feature = "signaling-whip")]
mod outbound_whep;
#[cfg(feature = "signaling-whip")]
mod outbound_whip;
#[cfg(feature = "signaling-ws")]
mod outbound_ws;
pub mod peer;
pub mod sdp;
#[cfg(feature = "tls-rustls")]
pub mod tls;
pub mod turn_rest;

#[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
pub mod signaling;

#[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
pub mod server;

#[cfg(feature = "client")]
pub mod client;

pub use adapter::{
    WebRtcAdapter, WebRtcMetrics, WebRtcTransportHandle, ADAPTER_EVENT_CAP,
    MAX_INBOUND_ADMISSION_CONFIRMATION_TIMEOUT,
};
pub use config::{
    IceServerConfig, Nat1To1CandidateType, OpusSettings, UdpPortRangeConfig, WebRtcConfig,
};
pub use errors::{Result, WebRtcError};
pub use media::WebRtcStatsSnapshot;
#[cfg(feature = "tls-rustls")]
pub use originate::WebRtcTlsClientTrust;
pub use originate::{
    StaticWebRtcBearerCredentialProvider, WebRtcAudioCodec, WebRtcBearerCredential,
    WebRtcBearerCredentialProvider, WebRtcIceExchangePolicy, WebRtcOriginateContext,
    WebRtcOriginateContextError, WebRtcSignalingMode, WebRtcTargetPolicy,
    MAX_WEBRTC_ORIGINATE_PREOPENED_DATA_CHANNELS,
};
pub use peer::{
    connect_loopback, DataChannelOptions, IceCandidateLog, LocalIceEvent, PeerRole,
    RvoipDataChannel, RvoipPeerConnection, WebRtcFeatureSupport,
};
pub use sdp::{sdp_has_inline_ice_candidates, sdp_has_media_line, sdp_indicates_simulcast};
pub use webrtc::peer_connection::{RTCIceCandidate, RTCIceCandidateInit};

#[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
pub use server::{WebRtcServer, WebRtcServerBuilder};
#[cfg(feature = "signaling-whip")]
pub use signaling::whip::WhepServerMode;

#[cfg(feature = "client")]
pub use client::{
    run_audio, Answer, AudioPacing, AudioSink, AudioSource, CallTarget, ComprehensiveReport,
    CountingAudioSink, FixtureAudioSource, IceCandidate, NegotiationAction, NullAudioSink, Offer,
    PerfectNegotiation, SessionHandle, SessionMedium, Signaler, SignalingPool, WebRtcClient,
};
#[cfg(all(feature = "client", feature = "signaling-ws"))]
pub use client::{WsSignaler, WsSignalerConfig};

/// The rustls provider every rvoip-webrtc TLS configuration is built on.
///
/// Configurations name this provider explicitly instead of reading the
/// process-level rustls default, which rustls cannot infer when a dependency
/// graph compiles both its `ring` and `aws-lc-rs` providers. Nothing here
/// installs a process default.
#[cfg(any(
    feature = "signaling-whip",
    feature = "signaling-ws",
    feature = "tls-rustls"
))]
pub(crate) fn rustls_crypto_provider() -> std::sync::Arc<rustls::crypto::CryptoProvider> {
    static PROVIDER: std::sync::OnceLock<std::sync::Arc<rustls::crypto::CryptoProvider>> =
        std::sync::OnceLock::new();
    PROVIDER
        .get_or_init(|| std::sync::Arc::new(rustls::crypto::ring::default_provider()))
        .clone()
}

/// Start a reqwest client builder whose TLS runs on [`rustls_crypto_provider`].
///
/// reqwest is built with `rustls-no-provider`, so on its own it would read the
/// process-level rustls default. It is handed a ready `ClientConfig` instead.
/// Server certificates are checked by the platform verifier, as reqwest's own
/// rustls backend does; if that verifier cannot be initialised,
/// `ClientBuilder::build` returns the error.
#[cfg(all(test, feature = "signaling-whip"))]
pub(crate) fn ring_http_client_builder() -> reqwest::ClientBuilder {
    ring_http_client_builder_with_roots(Vec::new())
}

/// [`ring_http_client_builder`] with extra trust roots added to the platform's.
#[cfg(feature = "signaling-whip")]
pub(crate) fn ring_http_client_builder_with_roots(
    extra_roots: Vec<rustls::pki_types::CertificateDer<'static>>,
) -> reqwest::ClientBuilder {
    match ring_https_client_config(extra_roots) {
        Ok(config) => reqwest::Client::builder().tls_backend_preconfigured(config),
        // An unrecognised preconfigured backend makes `build()` fail, which
        // is where reqwest reports its own TLS initialisation errors.
        Err(_) => reqwest::Client::builder().tls_backend_preconfigured(()),
    }
}

#[cfg(feature = "signaling-whip")]
fn ring_https_client_config(
    extra_roots: Vec<rustls::pki_types::CertificateDer<'static>>,
) -> std::result::Result<rustls::ClientConfig, rustls::Error> {
    let provider = rustls_crypto_provider();
    let verifier = if extra_roots.is_empty() {
        rustls_platform_verifier::Verifier::new(provider.clone())?
    } else {
        #[cfg(any(all(unix, not(target_os = "android")), target_os = "windows"))]
        {
            rustls_platform_verifier::Verifier::new_with_extra_roots(extra_roots, provider.clone())?
        }
        #[cfg(not(any(all(unix, not(target_os = "android")), target_os = "windows")))]
        {
            return Err(rustls::Error::General(
                "extra trust roots are not supported by the platform verifier on this target"
                    .into(),
            ));
        }
    };
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(verifier))
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

/// A WebSocket TLS connector on [`rustls_crypto_provider`] that trusts the
/// WebPKI roots plus `extra_roots`.
#[cfg(feature = "signaling-ws")]
pub(crate) fn ring_websocket_connector(
    extra_roots: &[rustls::pki_types::CertificateDer<'static>],
) -> std::result::Result<tokio_tungstenite::Connector, rustls::Error> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for certificate in extra_roots {
        roots.add(certificate.clone())?;
    }
    let config = rustls::ClientConfig::builder_with_provider(rustls_crypto_provider())
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(tokio_tungstenite::Connector::Rustls(std::sync::Arc::new(
        config,
    )))
}

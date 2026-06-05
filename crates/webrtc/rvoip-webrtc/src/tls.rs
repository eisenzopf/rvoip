//! TLS termination for in-process WHIP (HTTPS) and WebSocket (WSS) listeners
//! using `axum-server` + `tokio-rustls`. Feature-gated under `tls-rustls`.
//!
//! Production deployments often prefer terminating TLS at a reverse proxy
//! (nginx/Envoy/Traefik) and running rvoip-webrtc plaintext behind it —
//! that path needs no extra config. This module is for the single-process
//! deployment case where you want HTTPS/WSS on the same binary.
//!
//! ## Example
//!
//! ```no_run
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! use rvoip_webrtc::{WebRtcConfig, WebRtcServerBuilder};
//! use rvoip_webrtc::tls::TlsConfig;
//!
//! let tls = TlsConfig::from_pem_files("cert.pem", "key.pem").await?;
//! let server = WebRtcServerBuilder::new(WebRtcConfig::default())
//!     .with_whips("0.0.0.0:8443", tls.clone())
//!     .with_wss("0.0.0.0:8444", tls)
//!     .build()
//!     .await?;
//! # Ok(()) }
//! ```

#![cfg(feature = "tls-rustls")]

use std::path::Path;
use std::sync::Arc;

use axum_server::tls_rustls::RustlsConfig;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig;

use crate::errors::{Result, WebRtcError};

/// Loaded TLS material — cheap to clone (`Arc`-backed). Pass to
/// `WebRtcServerBuilder::with_whips` / `with_wss`.
#[derive(Clone)]
pub struct TlsConfig {
    pub(crate) axum: RustlsConfig,
    pub(crate) acceptor: tokio_rustls::TlsAcceptor,
}

impl TlsConfig {
    /// Load cert chain + private key from PEM files. Both files must be PEM
    /// (`-----BEGIN CERTIFICATE-----` and `-----BEGIN PRIVATE KEY-----`).
    pub async fn from_pem_files(cert: impl AsRef<Path>, key: impl AsRef<Path>) -> Result<Self> {
        let cert_path = cert.as_ref().to_path_buf();
        let key_path = key.as_ref().to_path_buf();

        // axum-server has a convenience loader for its own RustlsConfig.
        let axum = RustlsConfig::from_pem_file(&cert_path, &key_path)
            .await
            .map_err(|e| WebRtcError::Signaling(format!("load TLS cert/key: {e}")))?;

        // Build a separate ServerConfig for the WS path (no axum-server here).
        let cert_pem = tokio::fs::read(&cert_path)
            .await
            .map_err(|e| WebRtcError::Signaling(format!("read {cert_path:?}: {e}")))?;
        let key_pem = tokio::fs::read(&key_path)
            .await
            .map_err(|e| WebRtcError::Signaling(format!("read {key_path:?}: {e}")))?;
        let acceptor = build_acceptor(&cert_pem, &key_pem)?;

        Ok(Self { axum, acceptor })
    }

    /// Build directly from in-memory PEM bytes (handy for tests with `rcgen`).
    pub async fn from_pem_bytes(cert_pem: &[u8], key_pem: &[u8]) -> Result<Self> {
        let axum = RustlsConfig::from_pem(cert_pem.to_vec(), key_pem.to_vec())
            .await
            .map_err(|e| WebRtcError::Signaling(format!("load TLS cert/key (PEM): {e}")))?;
        let acceptor = build_acceptor(cert_pem, key_pem)?;
        Ok(Self { axum, acceptor })
    }
}

fn build_acceptor(cert_pem: &[u8], key_pem: &[u8]) -> Result<tokio_rustls::TlsAcceptor> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut cert_reader = std::io::BufReader::new(cert_pem);
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
        .filter_map(|c| c.ok())
        .collect();
    if certs.is_empty() {
        return Err(WebRtcError::Signaling("no certs in PEM".into()));
    }

    let mut key_reader = std::io::BufReader::new(key_pem);
    let key = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| WebRtcError::Signaling(format!("parse PEM key: {e}")))?
        .ok_or_else(|| WebRtcError::Signaling("no private key in PEM".into()))?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, PrivateKeyDer::from(key))
        .map_err(|e| WebRtcError::Signaling(format!("build ServerConfig: {e}")))?;

    Ok(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
}

//! TLS helpers for the UCTP substrate adapters.
//!
//! `self_signed_for_dev` produces an in-memory self-signed cert that the
//! `bridge` demo's orchestrator and agents use. `dev_client_config_trusting`
//! builds a `rustls::ClientConfig` that pins the orchestrator's cert as a
//! known trust anchor — the production-shape path for agent binaries.
//! `dangerous_no_verify` skips verification entirely; it's gated behind
//! the `dev-dangerous` feature so production builds can't depend on it.

use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::sync::Arc;

use crate::errors::SubstrateError;

/// Enable NSS-format TLS secret logging for an explicitly configured capture
/// run. Returns `false` and leaves the configuration unchanged unless
/// `SSLKEYLOGFILE` is present.
///
/// This is never called implicitly: production deployments must not emit QUIC
/// traffic secrets merely because a process inherited an unexpected
/// environment variable. Conformance harnesses opt in before constructing the
/// quinn endpoint, then provide the resulting file to Wireshark/tshark.
pub fn enable_server_key_log_from_env(config: &mut rustls::ServerConfig) -> bool {
    if std::env::var_os("SSLKEYLOGFILE").is_none() {
        return false;
    }
    config.key_log = Arc::new(rustls::KeyLogFile::new());
    true
}

/// Client-side counterpart of [`enable_server_key_log_from_env`].
pub fn enable_client_key_log_from_env(config: &mut rustls::ClientConfig) -> bool {
    if std::env::var_os("SSLKEYLOGFILE").is_none() {
        return false;
    }
    config.key_log = Arc::new(rustls::KeyLogFile::new());
    true
}

/// Generate a fresh self-signed certificate covering the listed
/// SAN domains. Used by demo orchestrators to bring up a QUIC endpoint
/// on `127.0.0.1`.
pub fn self_signed_for_dev(
    domains: &[String],
) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), SubstrateError> {
    let cert = generate_simple_self_signed(domains.to_vec())
        .map_err(|e| SubstrateError::Tls(rustls::Error::General(format!("rcgen: {}", e))))?;
    let der = cert.cert.der().clone();
    let key = PrivateKeyDer::Pkcs8(cert.signing_key.serialize_der().into());
    Ok((der, key))
}

/// Build a `rustls::ClientConfig` that trusts only the given certificate.
/// This is the production-shape way to connect an agent to a
/// pinned-cert orchestrator — preferred over `dangerous_no_verify`.
pub fn dev_client_config_trusting(
    cert: &CertificateDer<'_>,
) -> Result<rustls::ClientConfig, SubstrateError> {
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(cert.clone().into_owned())
        .map_err(|e| SubstrateError::Tls(rustls::Error::General(format!("trust anchor: {}", e))))?;
    let cfg = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(cfg)
}

/// Verification-disabled client config. Tests and demos only.
#[cfg(feature = "dev-dangerous")]
pub fn dangerous_no_verify() -> rustls::ClientConfig {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{DigitallySignedStruct, SignatureScheme};
    use std::sync::Arc;

    #[derive(Debug)]
    struct NoVerify;
    impl ServerCertVerifier for NoVerify {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![
                SignatureScheme::RSA_PKCS1_SHA256,
                SignatureScheme::ECDSA_NISTP256_SHA256,
                SignatureScheme::ED25519,
            ]
        }
    }

    rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerify))
        .with_no_client_auth()
}

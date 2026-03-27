//! Production-grade DTLS adapter backed by `webrtc-dtls` 0.12.
//!
//! This module wraps the [`webrtc_dtls`] crate behind an API that integrates
//! with the existing rvoip DTLS/SRTP pipeline. It replaces the self-built
//! DTLS implementation for new code while the old code is kept as deprecated.
//!
//! ## Key export for SRTP
//!
//! After the handshake, call [`DtlsConnectionAdapter::get_srtp_keys`] to
//! extract keying material compatible with [`SrtpContextAdapter`](super::super::srtp::adapter::SrtpContextAdapter).

use std::sync::Arc;

use tracing::{debug, info};
use webrtc_dtls::config::Config as WebrtcDtlsConfig;
use webrtc_dtls::conn::DTLSConn;
use webrtc_dtls::crypto::Certificate as WebrtcCertificate;
use webrtc_dtls::extension::extension_use_srtp::SrtpProtectionProfile as WebrtcSrtpProfile;
use webrtc_util_dtls::Conn as DtlsUtilConn;
use webrtc_util_dtls::KeyingMaterialExporter;

use crate::error::Error;

/// DTLS role — mirrors the existing [`super::DtlsRole`] but is self-contained
/// so the adapter can be used independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtlsRole {
    /// DTLS client (initiates handshake).
    Client,
    /// DTLS server (waits for handshake).
    Server,
}

impl From<super::DtlsRole> for DtlsRole {
    fn from(role: super::DtlsRole) -> Self {
        match role {
            super::DtlsRole::Client => DtlsRole::Client,
            super::DtlsRole::Server => DtlsRole::Server,
        }
    }
}

/// Keying material extracted from a completed DTLS-SRTP handshake.
///
/// Contains the master key and salt for one direction (local or remote).
/// Use [`DtlsConnectionAdapter::get_srtp_keys`] to obtain both directions.
#[derive(Debug, Clone)]
pub struct SrtpKeyMaterial {
    /// SRTP master key for the local (outbound/protect) direction.
    pub local_key: Vec<u8>,
    /// SRTP master salt for the local (outbound/protect) direction.
    pub local_salt: Vec<u8>,
    /// SRTP master key for the remote (inbound/unprotect) direction.
    pub remote_key: Vec<u8>,
    /// SRTP master salt for the remote (inbound/unprotect) direction.
    pub remote_salt: Vec<u8>,
    /// The negotiated SRTP protection profile.
    pub profile: WebrtcSrtpProfile,
}

/// Configuration for the DTLS adapter.
#[derive(Clone)]
pub struct DtlsAdapterConfig {
    /// SRTP protection profiles to negotiate.
    pub srtp_profiles: Vec<WebrtcSrtpProfile>,
    /// Whether to skip certificate verification (testing only).
    pub insecure_skip_verify: bool,
    /// Maximum transmission unit for DTLS records.
    pub mtu: usize,
}

impl Default for DtlsAdapterConfig {
    fn default() -> Self {
        Self {
            srtp_profiles: vec![
                WebrtcSrtpProfile::Srtp_Aes128_Cm_Hmac_Sha1_80,
                WebrtcSrtpProfile::Srtp_Aead_Aes_128_Gcm,
            ],
            insecure_skip_verify: false,
            mtu: 1200,
        }
    }
}

/// Production-grade DTLS connection adapter backed by `webrtc-dtls`.
///
/// Manages the lifecycle of a DTLS connection:
/// 1. Create with [`DtlsConnectionAdapter::new`]
/// 2. Perform handshake with [`DtlsConnectionAdapter::handshake`]
/// 3. Extract SRTP keys with [`DtlsConnectionAdapter::get_srtp_keys`]
/// 4. Optionally send/receive application data
/// 5. Close with [`DtlsConnectionAdapter::close`]
pub struct DtlsConnectionAdapter {
    /// The underlying webrtc-dtls connection (set after handshake).
    conn: Option<Arc<DTLSConn>>,
    /// DTLS role for this endpoint.
    role: DtlsRole,
    /// Local certificate fingerprint (SHA-256, colon-separated hex).
    fingerprint: Option<String>,
    /// The certificate used for this connection.
    certificate: Option<WebrtcCertificate>,
}

impl DtlsConnectionAdapter {
    /// Create a new adapter for the given DTLS role.
    ///
    /// Generates a self-signed certificate for the handshake.
    pub async fn new(role: DtlsRole) -> Result<Self, Error> {
        let cert = WebrtcCertificate::generate_self_signed(vec!["rvoip".to_string()])
            .map_err(|e| Error::CryptoError(format!("Failed to generate DTLS certificate: {e}")))?;

        // Compute SHA-256 fingerprint of the certificate DER.
        let fingerprint = compute_certificate_fingerprint(&cert)?;

        Ok(Self {
            conn: None,
            role,
            fingerprint: Some(fingerprint),
            certificate: Some(cert),
        })
    }

    /// Perform the DTLS handshake over the given UDP connection.
    ///
    /// The `conn` must implement the `Conn` trait from `webrtc-util` 0.11
    /// (the version that `webrtc-dtls` 0.12 uses internally). A connected
    /// `tokio::net::UdpSocket` satisfies this automatically via the blanket
    /// impl provided by that crate.
    pub async fn handshake(
        &mut self,
        conn: Arc<dyn DtlsUtilConn + Send + Sync>,
        config: &DtlsAdapterConfig,
    ) -> Result<(), Error> {
        let cert = self.certificate.clone().ok_or_else(|| {
            Error::InvalidState("No certificate available for DTLS handshake".to_string())
        })?;

        let is_client = self.role == DtlsRole::Client;

        let dtls_config = WebrtcDtlsConfig {
            certificates: vec![cert],
            srtp_protection_profiles: config.srtp_profiles.clone(),
            insecure_skip_verify: config.insecure_skip_verify,
            mtu: config.mtu,
            ..Default::default()
        };

        info!(
            role = ?self.role,
            is_client = is_client,
            "Starting webrtc-dtls handshake"
        );

        let dtls_conn = DTLSConn::new(conn, dtls_config, is_client, None)
            .await
            .map_err(|e| Error::DtlsHandshakeError(format!("webrtc-dtls handshake failed: {e}")))?;

        info!("webrtc-dtls handshake completed successfully");

        self.conn = Some(Arc::new(dtls_conn));
        Ok(())
    }

    /// Extract SRTP keying material from the completed DTLS handshake.
    ///
    /// Uses RFC 5764 `EXTRACTOR-dtls_srtp` label to derive keys.
    pub async fn get_srtp_keys(&self) -> Result<SrtpKeyMaterial, Error> {
        let conn = self.conn.as_ref().ok_or_else(|| {
            Error::InvalidState("DTLS handshake not completed".to_string())
        })?;

        let profile = conn.selected_srtpprotection_profile();
        if profile == WebrtcSrtpProfile::Unsupported {
            return Err(Error::NegotiationFailed(
                "No SRTP protection profile was negotiated".to_string(),
            ));
        }

        let state = conn.connection_state().await;

        // RFC 5764: key material length = 2 * (key_len + salt_len)
        let (key_len, salt_len) = srtp_profile_key_salt_len(profile)?;
        let material_len = 2 * (key_len + salt_len);

        let keying_material = state
            .export_keying_material("EXTRACTOR-dtls_srtp", &[], material_len)
            .await
            .map_err(|e| Error::CryptoError(format!("Failed to export SRTP keying material: {e}")))?;

        // RFC 5764 Section 4.2: key material layout:
        //   client_write_SRTP_master_key[key_len]
        //   server_write_SRTP_master_key[key_len]
        //   client_write_SRTP_master_salt[salt_len]
        //   server_write_SRTP_master_salt[salt_len]
        let mut offset = 0;
        let client_key = keying_material[offset..offset + key_len].to_vec();
        offset += key_len;
        let server_key = keying_material[offset..offset + key_len].to_vec();
        offset += key_len;
        let client_salt = keying_material[offset..offset + salt_len].to_vec();
        offset += salt_len;
        let server_salt = keying_material[offset..offset + salt_len].to_vec();

        let is_client = self.role == DtlsRole::Client;

        let (local_key, local_salt, remote_key, remote_salt) = if is_client {
            (client_key, client_salt, server_key, server_salt)
        } else {
            (server_key, server_salt, client_key, client_salt)
        };

        debug!(
            profile = ?profile,
            key_len = key_len,
            salt_len = salt_len,
            "Extracted SRTP keying material from DTLS"
        );

        Ok(SrtpKeyMaterial {
            local_key,
            local_salt,
            remote_key,
            remote_salt,
            profile,
        })
    }

    /// Get the local certificate fingerprint (SHA-256, colon-separated hex).
    pub fn local_fingerprint(&self) -> Option<&str> {
        self.fingerprint.as_deref()
    }

    /// Send application data over the DTLS connection.
    pub async fn send_application_data(&self, data: &[u8]) -> Result<(), Error> {
        let conn = self.conn.as_ref().ok_or_else(|| {
            Error::InvalidState("DTLS connection not established".to_string())
        })?;

        conn.write(data, None)
            .await
            .map_err(|e| Error::IoError(format!("DTLS send failed: {e}")))?;
        Ok(())
    }

    /// Receive application data from the DTLS connection.
    pub async fn recv_application_data(&self, buf: &mut [u8]) -> Result<usize, Error> {
        let conn = self.conn.as_ref().ok_or_else(|| {
            Error::InvalidState("DTLS connection not established".to_string())
        })?;

        let n = conn
            .read(buf, None)
            .await
            .map_err(|e| Error::IoError(format!("DTLS recv failed: {e}")))?;
        Ok(n)
    }

    /// Close the DTLS connection.
    pub async fn close(&self) -> Result<(), Error> {
        if let Some(conn) = &self.conn {
            conn.close()
                .await
                .map_err(|e| Error::IoError(format!("DTLS close failed: {e}")))?;
        }
        Ok(())
    }

    /// Get the underlying DTLSConn, if the handshake has completed.
    pub fn inner_conn(&self) -> Option<&Arc<DTLSConn>> {
        self.conn.as_ref()
    }

    /// Get the DTLS role.
    pub fn role(&self) -> DtlsRole {
        self.role
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Compute SHA-256 fingerprint from a webrtc-dtls Certificate.
fn compute_certificate_fingerprint(cert: &WebrtcCertificate) -> Result<String, Error> {
    use sha2::{Sha256, Digest};

    if cert.certificate.is_empty() {
        return Err(Error::CryptoError("Certificate has no DER data".to_string()));
    }

    let der_bytes = cert.certificate[0].as_ref();
    let digest = Sha256::digest(der_bytes);

    let hex_parts: Vec<String> = digest.iter().map(|b| format!("{b:02X}")).collect();
    Ok(hex_parts.join(":"))
}

/// Get key and salt lengths for a given SRTP protection profile.
fn srtp_profile_key_salt_len(profile: WebrtcSrtpProfile) -> Result<(usize, usize), Error> {
    match profile {
        WebrtcSrtpProfile::Srtp_Aes128_Cm_Hmac_Sha1_80
        | WebrtcSrtpProfile::Srtp_Aes128_Cm_Hmac_Sha1_32 => Ok((16, 14)),
        WebrtcSrtpProfile::Srtp_Aead_Aes_128_Gcm => Ok((16, 12)),
        WebrtcSrtpProfile::Srtp_Aead_Aes_256_Gcm => Ok((32, 12)),
        _ => Err(Error::NegotiationFailed(format!(
            "Unsupported SRTP protection profile: {profile:?}"
        ))),
    }
}

/// Convert a webrtc-dtls `SrtpProtectionProfile` to a `webrtc-srtp` `ProtectionProfile`.
pub fn to_srtp_protection_profile(
    dtls_profile: WebrtcSrtpProfile,
) -> Result<webrtc_srtp::protection_profile::ProtectionProfile, Error> {
    use webrtc_srtp::protection_profile::ProtectionProfile;
    match dtls_profile {
        WebrtcSrtpProfile::Srtp_Aes128_Cm_Hmac_Sha1_80 => Ok(ProtectionProfile::Aes128CmHmacSha1_80),
        WebrtcSrtpProfile::Srtp_Aes128_Cm_Hmac_Sha1_32 => Ok(ProtectionProfile::Aes128CmHmacSha1_32),
        WebrtcSrtpProfile::Srtp_Aead_Aes_128_Gcm => Ok(ProtectionProfile::AeadAes128Gcm),
        WebrtcSrtpProfile::Srtp_Aead_Aes_256_Gcm => Ok(ProtectionProfile::AeadAes256Gcm),
        _ => Err(Error::NegotiationFailed(format!(
            "Cannot convert DTLS SRTP profile to webrtc-srtp profile: {dtls_profile:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_srtp_profile_key_salt_len() {
        let (k, s) = srtp_profile_key_salt_len(WebrtcSrtpProfile::Srtp_Aes128_Cm_Hmac_Sha1_80)
            .expect("valid profile");
        assert_eq!(k, 16);
        assert_eq!(s, 14);

        let (k, s) = srtp_profile_key_salt_len(WebrtcSrtpProfile::Srtp_Aead_Aes_128_Gcm)
            .expect("valid profile");
        assert_eq!(k, 16);
        assert_eq!(s, 12);

        let (k, s) = srtp_profile_key_salt_len(WebrtcSrtpProfile::Srtp_Aead_Aes_256_Gcm)
            .expect("valid profile");
        assert_eq!(k, 32);
        assert_eq!(s, 12);
    }

    #[test]
    fn test_to_srtp_protection_profile() {
        use webrtc_srtp::protection_profile::ProtectionProfile;

        let p = to_srtp_protection_profile(WebrtcSrtpProfile::Srtp_Aes128_Cm_Hmac_Sha1_80)
            .expect("valid conversion");
        assert!(matches!(p, ProtectionProfile::Aes128CmHmacSha1_80));

        let p = to_srtp_protection_profile(WebrtcSrtpProfile::Srtp_Aead_Aes_256_Gcm)
            .expect("valid conversion");
        assert!(matches!(p, ProtectionProfile::AeadAes256Gcm));
    }

    #[test]
    fn test_dtls_role_conversion() {
        assert_eq!(DtlsRole::from(super::super::DtlsRole::Client), DtlsRole::Client);
        assert_eq!(DtlsRole::from(super::super::DtlsRole::Server), DtlsRole::Server);
    }

    #[tokio::test]
    async fn test_adapter_new() {
        let adapter = DtlsConnectionAdapter::new(DtlsRole::Client).await;
        assert!(adapter.is_ok());
        let adapter = adapter.expect("adapter creation should succeed");
        assert!(adapter.local_fingerprint().is_some());
        assert!(adapter.inner_conn().is_none()); // No handshake yet
        assert_eq!(adapter.role(), DtlsRole::Client);
    }
}

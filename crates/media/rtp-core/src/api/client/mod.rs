//! Client API for media transport
//!
//! This module provides client-side API components for media transport.

pub mod config;
pub mod security;
pub mod transport;

// Re-export public API
pub use config::{ClientConfig, ClientConfigBuilder};
pub use security::{ClientSecurityConfig, ClientSecurityContext};
pub use transport::MediaTransportClient;

// Re-export implementation files
pub use security::DefaultClientSecurityContext;
pub use transport::default::DefaultMediaTransportClient;

// Import errors
use crate::api::common::error::MediaTransportError;

/// Factory for creating media transport clients
pub struct ClientFactory;

impl ClientFactory {
    /// Create a new media transport client
    pub async fn create_client(
        config: ClientConfig,
    ) -> Result<DefaultMediaTransportClient, MediaTransportError> {
        // Create client transport
        let client = DefaultMediaTransportClient::new(config)
            .await
            .map_err(|e| {
                MediaTransportError::InitializationError(format!("Failed to create client: {}", e))
            })?;

        Ok(client)
    }

    /// Attempt to create a WebRTC client.
    ///
    /// DTLS-SRTP is unavailable in 0.3.5, so construction returns an error.
    pub async fn create_webrtc_client(
        remote_addr: std::net::SocketAddr,
    ) -> Result<DefaultMediaTransportClient, MediaTransportError> {
        // Create WebRTC-optimized config
        let config = ClientConfigBuilder::webrtc()
            .remote_address(remote_addr)
            .build();

        Self::create_client(config).await
    }

    /// Retained unprovisioned SIP convenience entry point.
    ///
    /// This signature has no key-material argument, so it returns a
    /// configuration error in 0.3.5 rather than inventing a direct-SRTP key.
    /// Use `ClientConfigBuilder::sip`, add an implemented profile and key, then
    /// call `try_build` and `create_client`.
    pub async fn create_sip_client(
        remote_addr: std::net::SocketAddr,
    ) -> Result<DefaultMediaTransportClient, MediaTransportError> {
        // Preserve the source-compatible helper while validation fails closed.
        let config = ClientConfigBuilder::sip()
            .remote_address(remote_addr)
            .build();

        Self::create_client(config).await
    }
}

// Update the ClientConfigBuilder to add a method for the unified security config
impl ClientConfigBuilder {
    /// Set the security configuration using the unified SecurityConfig
    /// This provides an easier way to configure security with predefined profiles
    pub fn with_security(
        mut self,
        security_config: crate::api::common::config::SecurityConfig,
    ) -> Self {
        let configuration_is_valid = security_config.validate().is_ok();
        if !configuration_is_valid
            && matches!(
                security_config.mode,
                crate::api::common::config::SecurityMode::None
                    | crate::api::common::config::SecurityMode::Srtp
            )
        {
            // The client-specific configuration cannot represent unified
            // `profile` or `required` fields. Encode any invalid plain/direct
            // SRTP request as an invalid SRTP configuration so this infallible
            // builder can never erase the validation failure.
            return self.security_config(crate::api::client::security::ClientSecurityConfig {
                security_mode: crate::api::common::config::SecurityMode::Srtp,
                srtp_profiles: Vec::new(),
                srtp_key: None,
                ..crate::api::client::security::ClientSecurityConfig::default()
            });
        }

        match security_config.mode {
            crate::api::common::config::SecurityMode::None => {
                self = self.security_config(
                    crate::api::client::security::ClientSecurityConfig::unsecured(),
                );
            }
            crate::api::common::config::SecurityMode::Srtp => {
                // Basic SRTP with pre-shared key
                // Convert to security config format expected by client
                let client_security_config = crate::api::client::security::ClientSecurityConfig {
                    security_mode: crate::api::common::config::SecurityMode::Srtp,
                    fingerprint_algorithm: security_config.fingerprint_algorithm,
                    remote_fingerprint: security_config.remote_fingerprint.clone(),
                    remote_fingerprint_algorithm: security_config
                        .remote_fingerprint_algorithm
                        .clone(),
                    validate_fingerprint: false, // Not used for SRTP mode
                    srtp_profiles: security_config.srtp_profiles,
                    certificate_path: None, // Not used for SRTP mode
                    private_key_path: None, // Not used for SRTP mode
                    srtp_key: security_config.srtp_key.clone(),
                };

                self = self.security_config(client_security_config);

                // If a key was provided, set it up for SRTP
                if let Some(_key) = security_config.srtp_key {
                    // Here you would set up the pre-shared key
                    // This might require additional implementation in your SRTP code
                }
            }
            crate::api::common::config::SecurityMode::DtlsSrtp => {
                // DTLS-SRTP mode
                // Convert to security config format expected by client
                let client_security_config = crate::api::client::security::ClientSecurityConfig {
                    security_mode: crate::api::common::config::SecurityMode::DtlsSrtp,
                    fingerprint_algorithm: security_config.fingerprint_algorithm,
                    remote_fingerprint: security_config.remote_fingerprint.clone(),
                    remote_fingerprint_algorithm: security_config
                        .remote_fingerprint_algorithm
                        .clone(),
                    validate_fingerprint: security_config.remote_fingerprint.is_some(),
                    srtp_profiles: security_config.srtp_profiles,
                    certificate_path: security_config.certificate_path,
                    private_key_path: security_config.private_key_path,
                    srtp_key: None, // Not used for DTLS-SRTP
                };

                self = self.security_config(client_security_config);
            }
            crate::api::common::config::SecurityMode::SdesSrtp
            | crate::api::common::config::SecurityMode::MikeySrtp
            | crate::api::common::config::SecurityMode::ZrtpSrtp => {
                // Preserve the requested mode. Direct media construction has
                // an explicit unsupported branch for these methods; leaving
                // the builder's plain-RTP default here would be a downgrade.
                let requested_mode = security_config.mode;
                self = self.security_config(crate::api::client::security::ClientSecurityConfig {
                    security_mode: requested_mode,
                    fingerprint_algorithm: security_config.fingerprint_algorithm,
                    remote_fingerprint: security_config.remote_fingerprint,
                    remote_fingerprint_algorithm: security_config.remote_fingerprint_algorithm,
                    validate_fingerprint: false,
                    srtp_profiles: security_config.srtp_profiles,
                    certificate_path: security_config.certificate_path,
                    private_key_path: security_config.private_key_path,
                    srtp_key: security_config.srtp_key,
                });
            }
        }

        self
    }

    /// Retain a WebRTC/DTLS-SRTP configuration request.
    ///
    /// Client construction rejects this unavailable mode in 0.3.5.
    pub fn with_webrtc_security(self) -> Self {
        let security_config = crate::api::common::config::SecurityConfig::webrtc_compatible();
        self.with_security(security_config)
    }

    /// Set up SRTP with a pre-shared key
    pub fn with_srtp_key(self, key: Vec<u8>) -> Self {
        let security_config = crate::api::common::config::SecurityConfig::srtp_with_key(key);
        self.with_security(security_config)
    }

    /// Set up plain RTP (no security)
    pub fn with_no_security(self) -> Self {
        let security_config = crate::api::common::config::SecurityConfig::unsecured();
        self.with_security(security_config)
    }

    /// Retain a certificate-based DTLS-SRTP configuration request.
    ///
    /// Client construction rejects this unavailable mode in 0.3.5.
    pub fn with_dtls_certificate(self, cert_path: String, key_path: String) -> Self {
        let security_config =
            crate::api::common::config::SecurityConfig::dtls_with_certificate(cert_path, key_path);
        self.with_security(security_config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::common::config::{SecurityConfig, SecurityMode, SrtpProfile};
    use crate::api::common::frame::{MediaFrame, MediaFrameType};

    #[test]
    fn unsecured_config_replaces_dtls_defaults() {
        for config in [
            ClientConfigBuilder::new()
                .with_security(SecurityConfig::unsecured())
                .build(),
            ClientConfigBuilder::new().with_no_security().build(),
        ] {
            assert_eq!(config.security_config.security_mode, SecurityMode::None);
            assert!(config.security_config.srtp_profiles.is_empty());
            assert!(!config.security_config.validate_fingerprint);
        }
    }

    #[tokio::test]
    async fn plain_rtp_client_constructs_without_dtls() {
        let config = ClientConfigBuilder::new().with_no_security().build();
        DefaultMediaTransportClient::new(config).await.unwrap();
    }

    #[tokio::test]
    async fn unavailable_security_builders_never_downgrade_to_plain_rtp() {
        for security in [
            SecurityConfig::sdes_srtp(),
            SecurityConfig::mikey_psk(),
            SecurityConfig::zrtp_p2p(),
        ] {
            let requested_mode = security.mode;
            let config = ClientConfigBuilder::new().with_security(security).build();

            assert_eq!(config.security_config.security_mode, requested_mode);
            assert!(DefaultMediaTransportClient::new(config).await.is_err());
        }

        let config = ClientConfigBuilder::new().with_webrtc_security().build();
        assert_eq!(config.security_config.security_mode, SecurityMode::DtlsSrtp);
        assert!(DefaultMediaTransportClient::new(config).await.is_err());
    }

    #[tokio::test]
    async fn invalid_unified_client_configurations_survive_infallible_builder_mapping() {
        let mut profile_mismatch = SecurityConfig::unsecured();
        profile_mismatch.mode = SecurityMode::Srtp;
        profile_mismatch.srtp_profiles = vec![SrtpProfile::AesCm128HmacSha1_80];
        profile_mismatch.srtp_key = Some(vec![0x41; 30]);

        let mut required_plaintext = SecurityConfig::unsecured();
        required_plaintext.required = true;

        let mut plaintext_with_certificate = SecurityConfig::unsecured();
        plaintext_with_certificate.certificate_path = Some("certificate.pem".to_string());

        let mut plaintext_with_fingerprint = SecurityConfig::unsecured();
        plaintext_with_fingerprint.remote_fingerprint = Some("AA:BB".to_string());

        for invalid in [
            profile_mismatch,
            required_plaintext,
            plaintext_with_certificate,
            plaintext_with_fingerprint,
        ] {
            assert!(invalid.validate().is_err());
            let config = ClientConfigBuilder::new().with_security(invalid).build();
            assert_eq!(config.security_config.security_mode, SecurityMode::Srtp);
            assert!(config.security_config.srtp_profiles.is_empty());
            assert!(config.security_config.srtp_key.is_none());
            assert!(DefaultMediaTransportClient::new(config).await.is_err());
        }
    }

    #[tokio::test]
    async fn direct_client_config_rejects_ignored_security_fields() {
        let mut plain = crate::api::client::security::ClientSecurityConfig::unsecured();
        plain.remote_fingerprint = Some("AA:BB".to_string());
        let config = ClientConfigBuilder::new().security_config(plain).build();
        assert!(DefaultMediaTransportClient::new(config).await.is_err());

        let direct = crate::api::client::security::ClientSecurityConfig {
            security_mode: SecurityMode::Srtp,
            srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
            srtp_key: Some(vec![0x52; 30]),
            certificate_path: Some("certificate.pem".to_string()),
            ..crate::api::client::security::ClientSecurityConfig::default()
        };
        let config = ClientConfigBuilder::new().security_config(direct).build();
        assert!(DefaultMediaTransportClient::new(config).await.is_err());
    }

    #[tokio::test]
    async fn direct_srtp_client_session_emits_authenticated_srtcp() {
        let peer = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut security = SecurityConfig::srtp_with_key(vec![0x44; 30]);
        security.srtp_profiles = vec![crate::api::common::config::SrtpProfile::AesCm128HmacSha1_32];
        let config = ClientConfigBuilder::new()
            .remote_address(peer.local_addr().unwrap())
            .with_security(security)
            .build();
        let client = DefaultMediaTransportClient::new(config).await.unwrap();
        let session = client.get_session().await.unwrap();

        session.lock().await.send_sender_report().await.unwrap();

        let mut wire = [0_u8; 2048];
        let (length, _) =
            tokio::time::timeout(std::time::Duration::from_secs(1), peer.recv_from(&mut wire))
                .await
                .unwrap()
                .unwrap();
        let mut peer_receive = crate::srtp::SrtpContext::new(
            crate::srtp::SRTP_AES128_CM_SHA1_32,
            crate::srtp::SrtpCryptoKey::new(vec![0x44; 16], vec![0x44; 14]),
        )
        .unwrap();
        let plaintext = peer_receive.unprotect_rtcp(&wire[..length]).unwrap();
        assert!(matches!(
            crate::packet::rtcp::RtcpPacket::parse(&plaintext).unwrap(),
            crate::packet::rtcp::RtcpPacket::SenderReport(_)
        ));
        assert_ne!(&wire[..length], plaintext.as_ref());
    }

    #[tokio::test]
    async fn direct_srtp_client_reports_secure_only_while_crypto_ready_and_connected() {
        let peer = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let config = ClientConfigBuilder::new()
            .remote_address(peer.local_addr().unwrap())
            .with_security(SecurityConfig::srtp_with_key(vec![0x75; 30]))
            .build();
        let client = DefaultMediaTransportClient::new(config).await.unwrap();

        assert!(!client.is_secure());
        client.connect().await.unwrap();
        assert!(client.is_connected().await.unwrap());
        assert!(client.is_secure());
        client.disconnect().await.unwrap();
        assert!(!client.is_connected().await.unwrap());
        assert!(!client.is_secure());
    }

    #[tokio::test]
    async fn direct_psk_client_rejects_ambiguous_profile_lists() {
        let peer = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut security = SecurityConfig::srtp_with_key(vec![0x55; 30]);
        security.srtp_profiles = vec![
            SrtpProfile::AesCm128HmacSha1_80,
            SrtpProfile::AesCm128HmacSha1_32,
        ];
        let config = ClientConfigBuilder::new()
            .remote_address(peer.local_addr().unwrap())
            .with_security(security)
            .build();

        assert!(DefaultMediaTransportClient::new(config).await.is_err());
    }

    #[tokio::test]
    async fn direct_psk_client_uses_configured_suite_on_wire() {
        for (profile, suite_name, tag_len) in [
            (
                SrtpProfile::AesCm128HmacSha1_80,
                "AES_CM_128_HMAC_SHA1_80",
                10,
            ),
            (
                SrtpProfile::AesCm128HmacSha1_32,
                "AES_CM_128_HMAC_SHA1_32",
                4,
            ),
        ] {
            let peer = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let mut security = SecurityConfig::srtp_with_key(vec![0x66; 30]);
            security.srtp_profiles = vec![profile];
            let config = ClientConfigBuilder::new()
                .remote_address(peer.local_addr().unwrap())
                .rtcp_mux(true)
                .with_security(security)
                .build();
            let client = DefaultMediaTransportClient::new(config).await.unwrap();
            client.connect().await.unwrap();

            let info = client.get_security_info().await.unwrap();
            assert_eq!(info.crypto_suites, [suite_name]);
            assert_eq!(info.srtp_profile.as_deref(), Some(suite_name));

            let payload = b"psk-suite";
            client
                .send_frame(MediaFrame::new(
                    MediaFrameType::Audio,
                    payload.as_slice(),
                    1234,
                    10,
                    false,
                    0,
                    0x1122_3344,
                ))
                .await
                .unwrap();

            let mut wire = [0_u8; 2048];
            let (wire_len, _) =
                tokio::time::timeout(std::time::Duration::from_secs(1), peer.recv_from(&mut wire))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(wire_len, 12 + payload.len() + tag_len);

            client.disconnect().await.unwrap();
        }
    }
}

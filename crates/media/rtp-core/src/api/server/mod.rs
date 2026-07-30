//! Server API for media transport
//!
//! This module provides server-side API components for media transport.

pub mod config;
pub mod security;
pub mod transport;

// Re-export public API
pub use config::{ServerConfig, ServerConfigBuilder};
pub use security::{ServerSecurityConfig, ServerSecurityContext};
pub use transport::{ClientInfo, MediaTransportServer};

// Re-export implementation files
pub use security::DefaultServerSecurityContext;
pub use transport::DefaultMediaTransportServer;

// Import errors
use crate::api::common::error::MediaTransportError;

/// Factory for creating media transport servers
pub struct ServerFactory;

impl ServerFactory {
    /// Create a new media transport server
    pub async fn create_server(
        config: ServerConfig,
    ) -> Result<DefaultMediaTransportServer, MediaTransportError> {
        // Create the server
        let server = DefaultMediaTransportServer::new(config).await?;
        Ok(server)
    }

    /// Attempt to create a WebRTC server.
    ///
    /// DTLS-SRTP is unavailable in 0.3.5, so construction returns an error.
    pub async fn create_webrtc_server(
        local_addr: std::net::SocketAddr,
    ) -> Result<DefaultMediaTransportServer, MediaTransportError> {
        // Create WebRTC-optimized config
        let config = ServerConfigBuilder::webrtc()
            .local_address(local_addr)
            .build()?;

        Self::create_server(config).await
    }

    /// Retained unprovisioned SIP convenience entry point.
    ///
    /// This signature has no key-material argument, so it returns a
    /// configuration error in 0.3.5 rather than inventing a direct-SRTP key.
    /// Use `ServerConfigBuilder::sip`, add an implemented profile and key, then
    /// call `build` and `create_server`.
    pub async fn create_sip_server(
        local_addr: std::net::SocketAddr,
    ) -> Result<DefaultMediaTransportServer, MediaTransportError> {
        // Preserve the source-compatible helper while validation fails closed.
        let config = ServerConfigBuilder::sip()
            .local_address(local_addr)
            .build()?;

        Self::create_server(config).await
    }

    /// Create a high-capacity server
    pub async fn create_high_capacity_server(
        local_addr: std::net::SocketAddr,
        max_clients: usize,
    ) -> Result<DefaultMediaTransportServer, MediaTransportError> {
        // Create high-capacity config
        let config = ServerConfigBuilder::new()
            .local_address(local_addr)
            .max_clients(max_clients)
            .build()?;

        Self::create_server(config).await
    }
}

// Update the ServerConfigBuilder to add a method for the unified security config
impl ServerConfigBuilder {
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
            // The server-specific configuration cannot represent unified
            // `profile` or `required` fields. Encode any invalid plain/direct
            // SRTP request as an invalid SRTP configuration so this infallible
            // builder can never erase the validation failure.
            return self.security_config(crate::api::server::security::ServerSecurityConfig {
                security_mode: crate::api::common::config::SecurityMode::Srtp,
                srtp_profiles: Vec::new(),
                srtp_key: None,
                ..crate::api::server::security::ServerSecurityConfig::default()
            });
        }

        match security_config.mode {
            crate::api::common::config::SecurityMode::None => {
                self = self.security_config(
                    crate::api::server::security::ServerSecurityConfig::unsecured(),
                );
            }
            crate::api::common::config::SecurityMode::Srtp => {
                // Basic SRTP with pre-shared key
                // Convert to security config format expected by server
                let server_security_config = crate::api::server::security::ServerSecurityConfig {
                    security_mode: crate::api::common::config::SecurityMode::Srtp,
                    fingerprint_algorithm: security_config.fingerprint_algorithm,
                    srtp_profiles: security_config.srtp_profiles,
                    certificate_path: None, // Not used for SRTP mode
                    private_key_path: None, // Not used for SRTP mode
                    require_client_certificate: false,
                    srtp_key: security_config.srtp_key.clone(),
                };

                self = self.security_config(server_security_config);

                // If a key was provided, set it up for SRTP
                if let Some(_key) = security_config.srtp_key {
                    // Here you would set up the pre-shared key
                    // This might require additional implementation in your SRTP code
                }
            }
            crate::api::common::config::SecurityMode::DtlsSrtp => {
                // DTLS-SRTP mode
                // Convert to security config format expected by server
                let server_security_config = crate::api::server::security::ServerSecurityConfig {
                    security_mode: crate::api::common::config::SecurityMode::DtlsSrtp,
                    fingerprint_algorithm: security_config.fingerprint_algorithm,
                    srtp_profiles: security_config.srtp_profiles,
                    certificate_path: security_config.certificate_path,
                    private_key_path: security_config.private_key_path,
                    require_client_certificate: security_config.require_client_certificate,
                    srtp_key: None, // Not used for DTLS-SRTP
                };

                self = self.security_config(server_security_config);
            }
            crate::api::common::config::SecurityMode::SdesSrtp
            | crate::api::common::config::SecurityMode::MikeySrtp
            | crate::api::common::config::SecurityMode::ZrtpSrtp => {
                let requested_mode = security_config.mode;
                self = self.security_config(crate::api::server::security::ServerSecurityConfig {
                    security_mode: requested_mode,
                    fingerprint_algorithm: security_config.fingerprint_algorithm,
                    certificate_path: security_config.certificate_path,
                    private_key_path: security_config.private_key_path,
                    srtp_profiles: security_config.srtp_profiles,
                    require_client_certificate: security_config.require_client_certificate,
                    srtp_key: security_config.srtp_key,
                });
            }
        }

        self
    }

    /// Retain a WebRTC/DTLS-SRTP configuration request.
    ///
    /// Server construction rejects this unavailable mode in 0.3.5.
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
    /// Server construction rejects this unavailable mode in 0.3.5.
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
    use crate::api::server::transport::MediaTransportServer;

    #[test]
    fn unsecured_config_replaces_dtls_defaults() {
        for config in [
            ServerConfigBuilder::new()
                .with_security(SecurityConfig::unsecured())
                .build()
                .unwrap(),
            ServerConfigBuilder::new()
                .with_no_security()
                .build()
                .unwrap(),
        ] {
            assert_eq!(config.security_config.security_mode, SecurityMode::None);
            assert!(config.security_config.srtp_profiles.is_empty());
        }
    }

    #[tokio::test]
    async fn plain_rtp_server_starts_without_dtls() {
        let config = ServerConfigBuilder::new()
            .with_no_security()
            .build()
            .unwrap();
        let server = DefaultMediaTransportServer::new(config).await.unwrap();
        server.start().await.unwrap();
        server.stop().await.unwrap();
    }

    #[test]
    fn unavailable_security_builders_never_downgrade_to_plain_rtp() {
        for security in [
            SecurityConfig::sdes_srtp(),
            SecurityConfig::mikey_psk(),
            SecurityConfig::zrtp_p2p(),
        ] {
            assert!(ServerConfigBuilder::new()
                .with_security(security)
                .build()
                .is_err());
        }

        assert!(ServerConfigBuilder::new()
            .with_webrtc_security()
            .build()
            .is_err());
    }

    #[test]
    fn invalid_unified_server_configurations_fail_at_builder_validation() {
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
            assert!(ServerConfigBuilder::new()
                .with_security(invalid)
                .build()
                .is_err());
        }
    }

    #[test]
    fn direct_server_config_rejects_ignored_security_fields() {
        let mut plain = crate::api::server::security::ServerSecurityConfig::unsecured();
        plain.require_client_certificate = true;
        assert!(ServerConfigBuilder::new()
            .security_config(plain)
            .build()
            .is_err());

        let direct = crate::api::server::security::ServerSecurityConfig {
            security_mode: SecurityMode::Srtp,
            srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
            srtp_key: Some(vec![0x63; 30]),
            certificate_path: Some("certificate.pem".to_string()),
            ..crate::api::server::security::ServerSecurityConfig::default()
        };
        assert!(ServerConfigBuilder::new()
            .security_config(direct)
            .build()
            .is_err());
    }

    #[test]
    fn direct_psk_server_rejects_ambiguous_profile_lists() {
        let mut security = SecurityConfig::srtp_with_key(vec![0x55; 30]);
        security.srtp_profiles = vec![
            SrtpProfile::AesCm128HmacSha1_80,
            SrtpProfile::AesCm128HmacSha1_32,
        ];
        assert!(ServerConfigBuilder::new()
            .local_address("127.0.0.1:0".parse().unwrap())
            .with_security(security)
            .build()
            .is_err());
    }
}

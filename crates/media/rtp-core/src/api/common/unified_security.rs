//! Unified Security Context
//!
//! This module provides one interface for the implemented SDES exchange and
//! directly provisioned PSK SRTP. Public DTLS-SRTP, MIKEY, and ZRTP variants
//! and factory methods are retained for compatibility, but construction fails
//! with a typed unsupported-feature error before state or key mutation.

#[cfg(test)]
use crate::api::common::config::SecurityMode;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::api::common::config::{KeyExchangeMethod, SecurityConfig};
use crate::api::common::error::SecurityError;
use crate::security::SecurityKeyExchange;
use crate::srtp::{crypto::SrtpCryptoKey, SrtpContext, SrtpCryptoSuite};

/// Security state for unified context
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityState {
    /// Initial state - not initialized
    Initial,
    /// Key exchange in progress
    Negotiating,
    /// Key exchange completed successfully
    Established,
    /// Key exchange failed
    Failed,
    /// Security disabled
    Disabled,
}

/// Configuration for specific key exchange methods
#[derive(Debug, Clone)]
pub enum KeyExchangeConfig {
    /// Retained DTLS-SRTP configuration shape; unavailable in 0.3.5.
    DtlsSrtp {
        certificate_path: Option<String>,
        private_key_path: Option<String>,
        fingerprint_algorithm: String,
    },
    /// SDES configuration
    Sdes {
        crypto_suites: Vec<SrtpCryptoSuite>,
        offer_count: usize,
    },
    /// Retained MIKEY configuration shape; unavailable in 0.3.5.
    Mikey {
        psk: Option<Vec<u8>>,
        identity: Option<String>,
        mode: MikeyMode,
    },
    /// Retained ZRTP configuration shape; unavailable in 0.3.5.
    Zrtp {
        enable_sas: bool,
        cache_expiry: std::time::Duration,
    },
    /// Pre-shared key configuration
    PreSharedKey { key: Vec<u8>, salt: Vec<u8> },
}

/// MIKEY operation mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MikeyMode {
    /// Pre-shared key mode
    Psk,
    /// Public key exchange mode
    Pke,
}

/// Unified security context for implemented SDES and direct PSK methods.
pub struct UnifiedSecurityContext {
    /// Security configuration
    config: SecurityConfig,
    /// Selected key exchange method
    method: KeyExchangeMethod,
    /// Method-specific configuration
    method_config: KeyExchangeConfig,
    /// Current security state
    state: Arc<RwLock<SecurityState>>,
    /// The underlying key exchange implementation
    key_exchange: Arc<RwLock<Option<Box<dyn SecurityKeyExchange + Send + Sync>>>>,
    /// SRTP context once keys are established
    srtp_context: Arc<RwLock<Option<Arc<RwLock<SrtpContext>>>>>,
}

impl std::fmt::Debug for UnifiedSecurityContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnifiedSecurityContext")
            .field("method", &self.method)
            .field("config", &self.config)
            .field("method_config", &self.method_config)
            .field("state", &"<RwLock<SecurityState>>")
            .field("key_exchange", &"<Box<dyn SecurityKeyExchange>>")
            .field(
                "srtp_context",
                &"<RwLock<Option<Arc<RwLock<SrtpContext>>>>>",
            )
            .finish()
    }
}

impl UnifiedSecurityContext {
    /// Create a new unified security context
    pub fn new(config: SecurityConfig) -> Result<Self, SecurityError> {
        config.validate()?;
        let method = match config.mode.key_exchange_method() {
            Some(method) => method,
            None => {
                return Err(SecurityError::Configuration(
                    "No key exchange method for security mode".to_string(),
                ))
            }
        };

        let method_config = Self::create_method_config(&config, method)?;

        Ok(Self {
            state: Arc::new(RwLock::new(SecurityState::Initial)),
            method,
            config,
            key_exchange: Arc::new(RwLock::new(None)),
            srtp_context: Arc::new(RwLock::new(None)),
            method_config,
        })
    }

    /// Create method-specific configuration
    fn create_method_config(
        config: &SecurityConfig,
        method: KeyExchangeMethod,
    ) -> Result<KeyExchangeConfig, SecurityError> {
        match method {
            KeyExchangeMethod::DtlsSrtp => Err(SecurityError::UnsupportedFeature(
                "DTLS-SRTP is not complete and is unavailable".to_string(),
            )),
            KeyExchangeMethod::Sdes => {
                let crypto_suites =
                    crate::api::common::config::implemented_srtp_suites(&config.srtp_profiles)?;

                Ok(KeyExchangeConfig::Sdes {
                    crypto_suites,
                    offer_count: 2,
                })
            }
            KeyExchangeMethod::Mikey => Err(SecurityError::UnsupportedFeature(
                "MIKEY key exchange is not complete and is unavailable".to_string(),
            )),
            KeyExchangeMethod::Zrtp => Err(SecurityError::UnsupportedFeature(
                "ZRTP key exchange is not complete and is unavailable".to_string(),
            )),
            KeyExchangeMethod::PreSharedKey => match &config.srtp_key {
                Some(key) => {
                    if key.len() != 30 {
                        return Err(SecurityError::Configuration(
                                format!(
                                    "Pre-shared AES-128 SRTP key material must be exactly a 16-byte key and 14-byte salt, got {} bytes",
                                    key.len()
                                ),
                            ));
                    }

                    Ok(KeyExchangeConfig::PreSharedKey {
                        key: key[..16].to_vec(),
                        salt: key[16..30].to_vec(),
                    })
                }
                None => Err(SecurityError::Configuration(
                    "Pre-shared key required for PSK mode".to_string(),
                )),
            },
        }
    }

    fn selected_srtp_suite(&self) -> Result<SrtpCryptoSuite, SecurityError> {
        self.config
            .srtp_profiles
            .first()
            .copied()
            .ok_or_else(|| {
                SecurityError::Configuration(
                    "at least one implemented SRTP profile is required".to_string(),
                )
            })?
            .crypto_suite()
            .map_err(SecurityError::from)
    }

    /// Initialize the key exchange process
    pub async fn initialize(&self) -> Result<(), SecurityError> {
        let mut state = self.state.write().await;
        if *state != SecurityState::Initial {
            return Err(SecurityError::InvalidState(
                "Context already initialized".to_string(),
            ));
        }

        // Create the appropriate key exchange implementation
        let key_exchange_impl: Box<dyn SecurityKeyExchange + Send + Sync> = match self.method {
            KeyExchangeMethod::DtlsSrtp => {
                // DTLS-SRTP is handled by existing security contexts, not here
                return Err(SecurityError::Configuration(
                    "DTLS-SRTP should use existing security contexts".to_string(),
                ));
            }
            KeyExchangeMethod::Sdes => {
                if let KeyExchangeConfig::Sdes {
                    crypto_suites,
                    offer_count,
                } = &self.method_config
                {
                    let sdes_config = crate::security::sdes::SdesConfig {
                        crypto_suites: crypto_suites.clone(),
                        offer_count: *offer_count,
                    };
                    let sdes = crate::security::sdes::Sdes::new(
                        sdes_config,
                        crate::security::sdes::SdesRole::Offerer,
                    );
                    Box::new(sdes)
                } else {
                    return Err(SecurityError::Configuration(
                        "Invalid SDES configuration".to_string(),
                    ));
                }
            }
            KeyExchangeMethod::Mikey => {
                if let KeyExchangeConfig::Mikey {
                    psk,
                    identity: _,
                    mode,
                } = &self.method_config
                {
                    match mode {
                        MikeyMode::Psk => {
                            // Create MIKEY-PSK configuration
                            let mikey_config = crate::security::mikey::MikeyConfig {
                                method: crate::security::mikey::MikeyKeyExchangeMethod::Psk,
                                psk: psk.clone(),
                                srtp_profile: self.selected_srtp_suite()?,
                                ..Default::default()
                            };

                            // Default to initiator role - would be determined by call setup in real usage
                            let mikey = crate::security::mikey::Mikey::try_new(
                                mikey_config,
                                crate::security::mikey::MikeyRole::Initiator,
                            )
                            .map_err(SecurityError::from)?;
                            Box::new(mikey)
                        }
                        MikeyMode::Pke => {
                            return Err(SecurityError::UnsupportedFeature(
                                "MIKEY public-key exchange is not complete and is unavailable"
                                    .to_string(),
                            ));
                        }
                    }
                } else {
                    return Err(SecurityError::Configuration(
                        "Invalid MIKEY configuration".to_string(),
                    ));
                }
            }
            KeyExchangeMethod::Zrtp => {
                return Err(SecurityError::UnsupportedFeature(
                    "ZRTP key exchange is not complete and is unavailable".to_string(),
                ));
            }
            KeyExchangeMethod::PreSharedKey => {
                if let KeyExchangeConfig::PreSharedKey { key, salt } = &self.method_config {
                    // For pre-shared keys, we can immediately set up SRTP
                    let srtp_key = SrtpCryptoKey::new(key.clone(), salt.clone());
                    let srtp_context = SrtpContext::new(self.selected_srtp_suite()?, srtp_key)
                        .map_err(|e| {
                            SecurityError::CryptoError(format!(
                                "Failed to create SRTP context: {}",
                                e
                            ))
                        })?;

                    *self.srtp_context.write().await = Some(Arc::new(RwLock::new(srtp_context)));
                    *state = SecurityState::Established;
                    return Ok(());
                } else {
                    return Err(SecurityError::Configuration(
                        "Invalid PSK configuration".to_string(),
                    ));
                }
            }
        };

        // Initialize the key exchange
        let mut key_exchange_mut = key_exchange_impl;
        key_exchange_mut.init().map_err(SecurityError::from)?;

        *self.key_exchange.write().await = Some(key_exchange_mut);
        *state = SecurityState::Negotiating;

        Ok(())
    }

    /// Process an incoming message for key exchange
    pub async fn process_message(&self, message: &[u8]) -> Result<Option<Vec<u8>>, SecurityError> {
        let state = self.state.read().await;
        if *state != SecurityState::Negotiating {
            return Err(SecurityError::InvalidState(
                "Key exchange not in progress".to_string(),
            ));
        }
        drop(state);

        let mut key_exchange_guard = self.key_exchange.write().await;
        let key_exchange = key_exchange_guard.as_mut().ok_or_else(|| {
            SecurityError::NotInitialized("Key exchange not initialized".to_string())
        })?;

        let response = key_exchange
            .process_message(message)
            .map_err(SecurityError::from)?;

        // Check if key exchange is complete
        if key_exchange.is_complete() {
            // Get the negotiated keys and set up SRTP
            if let (Some(srtp_key), Some(srtp_suite)) =
                (key_exchange.get_srtp_key(), key_exchange.get_srtp_suite())
            {
                let srtp_context = SrtpContext::new(srtp_suite, srtp_key).map_err(|e| {
                    SecurityError::CryptoError(format!("Failed to create SRTP context: {}", e))
                })?;

                *self.srtp_context.write().await = Some(Arc::new(RwLock::new(srtp_context)));
                *self.state.write().await = SecurityState::Established;
            } else {
                *self.state.write().await = SecurityState::Failed;
                return Err(SecurityError::CryptoError(
                    "Key exchange completed but no keys available".to_string(),
                ));
            }
        }

        Ok(response)
    }

    /// Generate a fresh SDES offer using the configured implemented suites.
    pub async fn create_sdes_offer(&self) -> Result<Vec<String>, SecurityError> {
        if self.method != KeyExchangeMethod::Sdes {
            return Err(SecurityError::UnsupportedFeature(
                "offer generation is only implemented for SDES".to_string(),
            ));
        }

        match self.get_state().await {
            SecurityState::Initial => self.initialize().await?,
            SecurityState::Negotiating => {}
            state => {
                return Err(SecurityError::InvalidState(format!(
                    "cannot generate an SDES offer while security is {state:?}"
                )))
            }
        }

        let offer = self.process_message(b"").await?.ok_or_else(|| {
            SecurityError::CryptoError("SDES did not generate an offer".to_string())
        })?;
        let offer = String::from_utf8(offer).map_err(|error| {
            SecurityError::CryptoError(format!("SDES generated non-UTF-8 signaling: {error}"))
        })?;
        let lines = offer
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if lines.is_empty()
            || lines
                .iter()
                .any(|line| !line.starts_with("a=crypto:") || line.contains("placeholder"))
        {
            return Err(SecurityError::CryptoError(
                "SDES generated an invalid security offer".to_string(),
            ));
        }
        Ok(lines)
    }

    /// Check if the security context is established
    pub async fn is_established(&self) -> bool {
        *self.state.read().await == SecurityState::Established
    }

    /// Get the current security state
    pub async fn get_state(&self) -> SecurityState {
        *self.state.read().await
    }

    /// Get the key exchange method being used
    pub fn get_method(&self) -> KeyExchangeMethod {
        self.method
    }

    /// Get access to the SRTP context (if established)
    pub async fn get_srtp_context(&self) -> Option<Arc<RwLock<SrtpContext>>> {
        self.srtp_context.read().await.clone()
    }

    /// Protect an RTP packet using SRTP
    pub async fn protect_rtp(
        &self,
        packet: &crate::packet::RtpPacket,
    ) -> Result<crate::srtp::ProtectedRtpPacket, SecurityError> {
        let srtp_context = self.get_srtp_context().await.ok_or_else(|| {
            SecurityError::NotInitialized("SRTP context not established".to_string())
        })?;

        let result = srtp_context
            .write()
            .await
            .protect(packet)
            .map_err(|e| SecurityError::CryptoError(format!("SRTP encryption failed: {}", e)));
        result
    }

    /// Unprotect an RTP packet using SRTP
    pub async fn unprotect_rtp(
        &self,
        data: &[u8],
    ) -> Result<crate::packet::RtpPacket, SecurityError> {
        let srtp_context = self.get_srtp_context().await.ok_or_else(|| {
            SecurityError::NotInitialized("SRTP context not established".to_string())
        })?;

        let result = srtp_context
            .write()
            .await
            .unprotect(data)
            .map_err(|e| SecurityError::CryptoError(format!("SRTP decryption failed: {}", e)));
        result
    }
}

/// Factory for implemented contexts and retained unavailable-method shims.
pub struct SecurityContextFactory;

impl SecurityContextFactory {
    /// Create a unified security context from a security configuration
    pub fn create_context(config: SecurityConfig) -> Result<UnifiedSecurityContext, SecurityError> {
        UnifiedSecurityContext::new(config)
    }

    /// Create a context for SDES-SRTP
    pub fn create_sdes_context() -> Result<UnifiedSecurityContext, SecurityError> {
        let config = SecurityConfig::sdes_srtp();
        Self::create_context(config)
    }

    /// Retained MIKEY-PSK factory; returns `UnsupportedFeature` in 0.3.5.
    pub fn create_mikey_psk_context(psk: Vec<u8>) -> Result<UnifiedSecurityContext, SecurityError> {
        let mut config = SecurityConfig::mikey_psk();
        config.srtp_key = Some(psk);
        Self::create_context(config)
    }

    /// Retained ZRTP factory; returns `UnsupportedFeature` in 0.3.5.
    pub fn create_zrtp_context() -> Result<UnifiedSecurityContext, SecurityError> {
        let config = SecurityConfig::zrtp_p2p();
        Self::create_context(config)
    }

    /// Create a context with pre-shared key
    pub fn create_psk_context(key: Vec<u8>) -> Result<UnifiedSecurityContext, SecurityError> {
        let config = SecurityConfig::srtp_with_key(key);
        Self::create_context(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::common::config::{SecurityConfig, SecurityProfile};

    /// Test data for SRTP keys
    fn test_srtp_key() -> Vec<u8> {
        vec![
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
            0x0F, 0x10, // Salt
            0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
        ]
    }

    #[tokio::test]
    async fn test_create_psk_context() {
        let key = test_srtp_key();
        let context = SecurityContextFactory::create_psk_context(key).unwrap();

        assert_eq!(context.get_method(), KeyExchangeMethod::PreSharedKey);
        assert_eq!(context.get_state().await, SecurityState::Initial);
    }

    #[tokio::test]
    async fn test_psk_initialization() {
        let key = test_srtp_key();
        let context = SecurityContextFactory::create_psk_context(key).unwrap();

        // Initialize the PSK context
        context.initialize().await.unwrap();

        // PSK should immediately establish security
        assert!(context.is_established().await);
        assert_eq!(context.get_state().await, SecurityState::Established);
    }

    #[test]
    fn test_create_sdes_context() {
        let context = SecurityContextFactory::create_sdes_context().unwrap();
        assert_eq!(context.get_method(), KeyExchangeMethod::Sdes);
    }

    #[test]
    fn test_create_mikey_context() {
        let key = test_srtp_key();
        assert!(matches!(
            SecurityContextFactory::create_mikey_psk_context(key),
            Err(SecurityError::UnsupportedFeature(_))
        ));
    }

    #[test]
    fn test_create_zrtp_context() {
        assert!(matches!(
            SecurityContextFactory::create_zrtp_context(),
            Err(SecurityError::UnsupportedFeature(_))
        ));
    }

    #[test]
    fn test_security_config_creation() {
        // Test SDES config
        let sdes_config = SecurityConfig::sdes_srtp();
        assert_eq!(sdes_config.mode, SecurityMode::SdesSrtp);
        assert_eq!(sdes_config.profile, SecurityProfile::SdesSrtp);

        // Test MIKEY config
        let mikey_config = SecurityConfig::mikey_psk();
        assert_eq!(mikey_config.mode, SecurityMode::MikeySrtp);
        assert_eq!(mikey_config.profile, SecurityProfile::MikeyPsk);
        assert!(matches!(
            mikey_config.validate(),
            Err(SecurityError::UnsupportedFeature(_))
        ));

        // Test ZRTP config
        let zrtp_config = SecurityConfig::zrtp_p2p();
        assert_eq!(zrtp_config.mode, SecurityMode::ZrtpSrtp);
        assert_eq!(zrtp_config.profile, SecurityProfile::ZrtpP2P);
    }

    #[test]
    fn test_key_exchange_method_properties() {
        // Test method properties
        assert!(KeyExchangeMethod::Sdes.requires_network_exchange());
        assert!(KeyExchangeMethod::Sdes.uses_signaling_exchange());
        assert!(!KeyExchangeMethod::Sdes.uses_media_exchange());

        assert!(KeyExchangeMethod::Zrtp.requires_network_exchange());
        assert!(!KeyExchangeMethod::Zrtp.uses_signaling_exchange());
        assert!(KeyExchangeMethod::Zrtp.uses_media_exchange());

        assert!(!KeyExchangeMethod::PreSharedKey.requires_network_exchange());
        assert!(!KeyExchangeMethod::PreSharedKey.uses_signaling_exchange());
        assert!(!KeyExchangeMethod::PreSharedKey.uses_media_exchange());
    }

    #[test]
    fn test_security_mode_conversions() {
        // Test mode to method conversion
        assert_eq!(
            SecurityMode::SdesSrtp.key_exchange_method(),
            Some(KeyExchangeMethod::Sdes)
        );
        assert_eq!(
            SecurityMode::MikeySrtp.key_exchange_method(),
            Some(KeyExchangeMethod::Mikey)
        );
        assert_eq!(
            SecurityMode::ZrtpSrtp.key_exchange_method(),
            Some(KeyExchangeMethod::Zrtp)
        );
        assert_eq!(SecurityMode::None.key_exchange_method(), None);

        // Test method to mode conversion
        assert_eq!(
            KeyExchangeMethod::Sdes.to_security_mode(),
            SecurityMode::SdesSrtp
        );
        assert_eq!(
            KeyExchangeMethod::Mikey.to_security_mode(),
            SecurityMode::MikeySrtp
        );
        assert_eq!(
            KeyExchangeMethod::Zrtp.to_security_mode(),
            SecurityMode::ZrtpSrtp
        );
    }

    #[test]
    fn test_invalid_psk_key() {
        for length in [3, 16, 29, 31, 32] {
            let result = SecurityContextFactory::create_psk_context(vec![0x01; length]);
            assert!(matches!(result, Err(SecurityError::Configuration(_))));
        }
    }

    #[tokio::test]
    async fn test_sdes_initialization() {
        // Test SDES initialization (should work once SDES is fully implemented)
        let context = SecurityContextFactory::create_sdes_context().unwrap();

        // Currently SDES initialization should work since we have the core implementation
        let result = context.initialize().await;
        assert!(result.is_ok());
        assert_eq!(context.get_state().await, SecurityState::Negotiating);
    }

    #[tokio::test]
    async fn test_mikey_initialization() {
        let key = test_srtp_key();
        assert!(matches!(
            SecurityContextFactory::create_mikey_psk_context(key),
            Err(SecurityError::UnsupportedFeature(_))
        ));
    }

    #[tokio::test]
    async fn test_zrtp_initialization_is_rejected() {
        assert!(matches!(
            SecurityContextFactory::create_zrtp_context(),
            Err(SecurityError::UnsupportedFeature(_))
        ));
    }

    #[test]
    fn test_method_config_creation() {
        let key = test_srtp_key();
        let config = SecurityConfig::srtp_with_key(key);
        let context = UnifiedSecurityContext::new(config).unwrap();

        // Verify the method config was created correctly
        assert_eq!(context.method, KeyExchangeMethod::PreSharedKey);

        // Check the internal method config
        match &context.method_config {
            KeyExchangeConfig::PreSharedKey { key, salt, .. } => {
                assert_eq!(key.len(), 16); // AES-128 key
                assert_eq!(salt.len(), 14); // Standard SRTP salt
            }
            _ => panic!("Expected PreSharedKey config"),
        }
    }

    #[test]
    fn every_implemented_key_exchange_method_preserves_the_configured_suite() {
        let mut configs = vec![
            SecurityConfig::sdes_srtp(),
            SecurityConfig::srtp_with_key(test_srtp_key()),
        ];
        for config in &mut configs {
            config.srtp_profiles =
                vec![crate::api::common::config::SrtpProfile::AesCm128HmacSha1_32];
            let context = UnifiedSecurityContext::new(config.clone()).unwrap();
            assert_eq!(
                context.selected_srtp_suite().unwrap(),
                crate::srtp::SRTP_AES128_CM_SHA1_32
            );
            if let KeyExchangeConfig::Sdes { crypto_suites, .. } = &context.method_config {
                assert_eq!(crypto_suites[0], crate::srtp::SRTP_AES128_CM_SHA1_32);
            }
        }

        for mut unavailable in [SecurityConfig::mikey_psk(), SecurityConfig::zrtp_p2p()] {
            unavailable.srtp_profiles =
                vec![crate::api::common::config::SrtpProfile::AesCm128HmacSha1_32];
            assert!(matches!(
                UnifiedSecurityContext::new(unavailable),
                Err(SecurityError::UnsupportedFeature(_))
            ));
        }
    }

    #[test]
    fn mixed_supported_and_gcm_profiles_are_rejected_as_a_whole() {
        let mut config = SecurityConfig::sdes_srtp();
        config.srtp_profiles = vec![
            crate::api::common::config::SrtpProfile::AesCm128HmacSha1_80,
            crate::api::common::config::SrtpProfile::AesGcm128,
        ];
        assert!(matches!(
            UnifiedSecurityContext::new(config),
            Err(SecurityError::UnsupportedFeature(_))
        ));
    }

    #[test]
    fn test_sip_scenario_configs() {
        // Test predefined SIP scenario configurations
        let enterprise = SecurityConfig::sip_enterprise();
        assert_eq!(enterprise.mode, SecurityMode::SdesSrtp);

        let operator = SecurityConfig::sip_operator();
        assert_eq!(operator.mode, SecurityMode::SdesSrtp);

        let p2p = SecurityConfig::sip_peer_to_peer();
        assert_eq!(p2p.mode, SecurityMode::SdesSrtp);

        let bridge = SecurityConfig::sip_webrtc_bridge();
        assert_eq!(bridge.mode, SecurityMode::SdesSrtp); // Primary method
    }

    #[test]
    fn test_multi_method_config() {
        let methods = vec![KeyExchangeMethod::Sdes, KeyExchangeMethod::DtlsSrtp];
        let config = SecurityConfig::multi_method(methods);
        assert_eq!(config.mode, SecurityMode::SdesSrtp); // Should use first method
    }
}

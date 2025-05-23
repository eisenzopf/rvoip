//! Unified Security Context
//!
//! This module provides a unified interface for all SRTP key exchange methods,
//! including DTLS-SRTP, SDES, MIKEY, and ZRTP. It abstracts away the differences
//! between these methods and provides a consistent API.

use std::sync::Arc;
use tokio::sync::RwLock;

use crate::api::common::config::{KeyExchangeMethod, SecurityConfig, SecurityMode};
use crate::api::common::error::SecurityError;
use crate::security::{SecurityKeyExchange, sdes::Sdes, mikey::Mikey, zrtp::Zrtp};
use crate::srtp::{SrtpContext, SrtpCryptoSuite, crypto::SrtpCryptoKey};

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
    /// DTLS-SRTP configuration (handled by existing security contexts)
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
    /// MIKEY configuration
    Mikey {
        psk: Option<Vec<u8>>,
        identity: Option<String>,
        mode: MikeyMode,
    },
    /// ZRTP configuration
    Zrtp {
        enable_sas: bool,
        cache_expiry: std::time::Duration,
    },
    /// Pre-shared key configuration
    PreSharedKey {
        key: Vec<u8>,
        salt: Vec<u8>,
    },
}

/// MIKEY operation mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MikeyMode {
    /// Pre-shared key mode
    Psk,
    /// Public key exchange mode
    Pke,
}

/// Unified security context that can handle multiple key exchange methods
pub struct UnifiedSecurityContext {
    /// Current security state
    state: Arc<RwLock<SecurityState>>,
    /// Key exchange method being used
    method: KeyExchangeMethod,
    /// Configuration for this context
    config: SecurityConfig,
    /// The underlying key exchange implementation
    key_exchange: Arc<RwLock<Option<Box<dyn SecurityKeyExchange + Send + Sync>>>>,
    /// SRTP context once keys are established
    srtp_context: Arc<RwLock<Option<SrtpContext>>>,
    /// Method-specific configuration
    method_config: KeyExchangeConfig,
}

impl UnifiedSecurityContext {
    /// Create a new unified security context
    pub fn new(config: SecurityConfig) -> Result<Self, SecurityError> {
        let method = match config.mode.key_exchange_method() {
            Some(method) => method,
            None => return Err(SecurityError::Configuration("No key exchange method for security mode".to_string())),
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
    fn create_method_config(config: &SecurityConfig, method: KeyExchangeMethod) -> Result<KeyExchangeConfig, SecurityError> {
        match method {
            KeyExchangeMethod::DtlsSrtp => {
                Ok(KeyExchangeConfig::DtlsSrtp {
                    certificate_path: config.certificate_path.clone(),
                    private_key_path: config.private_key_path.clone(),
                    fingerprint_algorithm: config.fingerprint_algorithm.clone(),
                })
            },
            KeyExchangeMethod::Sdes => {
                // Convert SrtpProfile to SrtpCryptoSuite
                let crypto_suites = config.srtp_profiles.iter()
                    .filter_map(|profile| match profile {
                        crate::api::common::config::SrtpProfile::AesCm128HmacSha1_80 => 
                            Some(crate::srtp::SRTP_AES128_CM_SHA1_80),
                        crate::api::common::config::SrtpProfile::AesCm128HmacSha1_32 => 
                            Some(crate::srtp::SRTP_AES128_CM_SHA1_32),
                        _ => None, // Other profiles not implemented yet
                    })
                    .collect();

                Ok(KeyExchangeConfig::Sdes {
                    crypto_suites,
                    offer_count: 2,
                })
            },
            KeyExchangeMethod::Mikey => {
                Ok(KeyExchangeConfig::Mikey {
                    psk: config.srtp_key.clone(),
                    identity: None,
                    mode: MikeyMode::Psk, // Default to PSK mode
                })
            },
            KeyExchangeMethod::Zrtp => {
                Ok(KeyExchangeConfig::Zrtp {
                    enable_sas: true,
                    cache_expiry: std::time::Duration::from_secs(3600), // 1 hour
                })
            },
            KeyExchangeMethod::PreSharedKey => {
                match &config.srtp_key {
                    Some(key) => {
                        // For simplicity, use the first 14 bytes as salt if key is long enough
                        let salt = if key.len() >= 30 {
                            key[16..30].to_vec()
                        } else {
                            vec![0u8; 14] // Default salt
                        };
                        let actual_key = if key.len() >= 16 {
                            key[0..16].to_vec()
                        } else {
                            return Err(SecurityError::Configuration("Pre-shared key too short".to_string()));
                        };

                        Ok(KeyExchangeConfig::PreSharedKey {
                            key: actual_key,
                            salt,
                        })
                    },
                    None => Err(SecurityError::Configuration("Pre-shared key required for PSK mode".to_string())),
                }
            },
        }
    }

    /// Initialize the key exchange process
    pub async fn initialize(&self) -> Result<(), SecurityError> {
        let mut state = self.state.write().await;
        if *state != SecurityState::Initial {
            return Err(SecurityError::InvalidState("Context already initialized".to_string()));
        }

        // Create the appropriate key exchange implementation
        let key_exchange_impl: Box<dyn SecurityKeyExchange + Send + Sync> = match self.method {
            KeyExchangeMethod::DtlsSrtp => {
                // DTLS-SRTP is handled by existing security contexts, not here
                return Err(SecurityError::Configuration("DTLS-SRTP should use existing security contexts".to_string()));
            },
            KeyExchangeMethod::Sdes => {
                if let KeyExchangeConfig::Sdes { crypto_suites, offer_count } = &self.method_config {
                    let sdes_config = crate::security::sdes::SdesConfig {
                        crypto_suites: crypto_suites.clone(),
                        offer_count: *offer_count,
                    };
                    let sdes = crate::security::sdes::Sdes::new(sdes_config, crate::security::sdes::SdesRole::Offerer);
                    Box::new(sdes)
                } else {
                    return Err(SecurityError::Configuration("Invalid SDES configuration".to_string()));
                }
            },
            KeyExchangeMethod::Mikey => {
                if let KeyExchangeConfig::Mikey { psk, identity, mode } = &self.method_config {
                    // Create MIKEY instance based on configuration
                    // This is a placeholder - actual MIKEY implementation would go here
                    return Err(SecurityError::Configuration("MIKEY implementation not yet complete".to_string()));
                } else {
                    return Err(SecurityError::Configuration("Invalid MIKEY configuration".to_string()));
                }
            },
            KeyExchangeMethod::Zrtp => {
                if let KeyExchangeConfig::Zrtp { enable_sas, cache_expiry } = &self.method_config {
                    // Create ZRTP configuration based on security config
                    let zrtp_config = crate::security::zrtp::ZrtpConfig {
                        ciphers: vec![crate::security::zrtp::ZrtpCipher::Aes1],
                        hashes: vec![crate::security::zrtp::ZrtpHash::S256],
                        auth_tags: vec![crate::security::zrtp::ZrtpAuthTag::HS80, crate::security::zrtp::ZrtpAuthTag::HS32],
                        key_agreements: vec![crate::security::zrtp::ZrtpKeyAgreement::EC25],
                        sas_types: if *enable_sas { 
                            vec![crate::security::zrtp::ZrtpSasType::B32] 
                        } else { 
                            vec![] 
                        },
                        client_id: "RVOIP Unified Security".to_string(),
                        srtp_profile: crate::srtp::SRTP_AES128_CM_SHA1_80,
                    };
                    
                    // Default to initiator role - would be determined by call setup in real usage
                    let zrtp = crate::security::zrtp::Zrtp::new(zrtp_config, crate::security::zrtp::ZrtpRole::Initiator);
                    Box::new(zrtp)
                } else {
                    return Err(SecurityError::Configuration("Invalid ZRTP configuration".to_string()));
                }
            },
            KeyExchangeMethod::PreSharedKey => {
                if let KeyExchangeConfig::PreSharedKey { key, salt } = &self.method_config {
                    // For pre-shared keys, we can immediately set up SRTP
                    let srtp_key = SrtpCryptoKey::new(key.clone(), salt.clone());
                    let srtp_context = SrtpContext::new(
                        crate::srtp::SRTP_AES128_CM_SHA1_80, // Default profile
                        srtp_key,
                    ).map_err(|e| SecurityError::CryptoError(format!("Failed to create SRTP context: {}", e)))?;

                    *self.srtp_context.write().await = Some(srtp_context);
                    *state = SecurityState::Established;
                    return Ok(());
                } else {
                    return Err(SecurityError::Configuration("Invalid PSK configuration".to_string()));
                }
            },
        };

        // Initialize the key exchange
        let mut key_exchange_mut = key_exchange_impl;
        key_exchange_mut.init()
            .map_err(|e| SecurityError::CryptoError(format!("Failed to initialize key exchange: {}", e)))?;

        *self.key_exchange.write().await = Some(key_exchange_mut);
        *state = SecurityState::Negotiating;

        Ok(())
    }

    /// Process an incoming message for key exchange
    pub async fn process_message(&self, message: &[u8]) -> Result<Option<Vec<u8>>, SecurityError> {
        let state = self.state.read().await;
        if *state != SecurityState::Negotiating {
            return Err(SecurityError::InvalidState("Key exchange not in progress".to_string()));
        }
        drop(state);

        let mut key_exchange_guard = self.key_exchange.write().await;
        let key_exchange = key_exchange_guard.as_mut()
            .ok_or_else(|| SecurityError::NotInitialized("Key exchange not initialized".to_string()))?;

        let response = key_exchange.process_message(message)
            .map_err(|e| SecurityError::CryptoError(format!("Key exchange failed: {}", e)))?;

        // Check if key exchange is complete
        if key_exchange.is_complete() {
            // Get the negotiated keys and set up SRTP
            if let (Some(srtp_key), Some(srtp_suite)) = (key_exchange.get_srtp_key(), key_exchange.get_srtp_suite()) {
                let srtp_context = SrtpContext::new(srtp_suite, srtp_key)
                    .map_err(|e| SecurityError::CryptoError(format!("Failed to create SRTP context: {}", e)))?;

                *self.srtp_context.write().await = Some(srtp_context);
                *self.state.write().await = SecurityState::Established;
            } else {
                *self.state.write().await = SecurityState::Failed;
                return Err(SecurityError::CryptoError("Key exchange completed but no keys available".to_string()));
            }
        }

        Ok(response)
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
        let guard = self.srtp_context.read().await;
        if guard.is_some() {
            // Return a clone of the Arc pointing to a new RwLock containing the context
            // This is a bit complex due to the nested locking structure
            // In practice, you might want to redesign this API
            None // Placeholder - would need better design for safe access
        } else {
            None
        }
    }

    /// Protect an RTP packet using SRTP
    pub async fn protect_rtp(&self, packet: &crate::packet::RtpPacket) -> Result<crate::srtp::ProtectedRtpPacket, SecurityError> {
        let mut srtp_guard = self.srtp_context.write().await;
        let srtp_context = srtp_guard.as_mut()
            .ok_or_else(|| SecurityError::NotInitialized("SRTP context not established".to_string()))?;

        srtp_context.protect(packet)
            .map_err(|e| SecurityError::CryptoError(format!("SRTP encryption failed: {}", e)))
    }

    /// Unprotect an RTP packet using SRTP
    pub async fn unprotect_rtp(&self, data: &[u8]) -> Result<crate::packet::RtpPacket, SecurityError> {
        let mut srtp_guard = self.srtp_context.write().await;
        let srtp_context = srtp_guard.as_mut()
            .ok_or_else(|| SecurityError::NotInitialized("SRTP context not established".to_string()))?;

        srtp_context.unprotect(data)
            .map_err(|e| SecurityError::CryptoError(format!("SRTP decryption failed: {}", e)))
    }
}

/// Factory for creating unified security contexts
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

    /// Create a context for MIKEY-SRTP with PSK
    pub fn create_mikey_psk_context(psk: Vec<u8>) -> Result<UnifiedSecurityContext, SecurityError> {
        let mut config = SecurityConfig::mikey_psk();
        config.srtp_key = Some(psk);
        Self::create_context(config)
    }

    /// Create a context for ZRTP
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
    use crate::api::common::config::{SecurityConfig, SecurityProfile, SrtpProfile};

    /// Test data for SRTP keys
    fn test_srtp_key() -> Vec<u8> {
        vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
             0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
             // Salt
             0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18,
             0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E]
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
        let context = SecurityContextFactory::create_mikey_psk_context(key).unwrap();
        assert_eq!(context.get_method(), KeyExchangeMethod::Mikey);
    }

    #[test]
    fn test_create_zrtp_context() {
        let context = SecurityContextFactory::create_zrtp_context().unwrap();
        assert_eq!(context.get_method(), KeyExchangeMethod::Zrtp);
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
        assert_eq!(SecurityMode::SdesSrtp.key_exchange_method(), Some(KeyExchangeMethod::Sdes));
        assert_eq!(SecurityMode::MikeySrtp.key_exchange_method(), Some(KeyExchangeMethod::Mikey));
        assert_eq!(SecurityMode::ZrtpSrtp.key_exchange_method(), Some(KeyExchangeMethod::Zrtp));
        assert_eq!(SecurityMode::None.key_exchange_method(), None);

        // Test method to mode conversion
        assert_eq!(KeyExchangeMethod::Sdes.to_security_mode(), SecurityMode::SdesSrtp);
        assert_eq!(KeyExchangeMethod::Mikey.to_security_mode(), SecurityMode::MikeySrtp);
        assert_eq!(KeyExchangeMethod::Zrtp.to_security_mode(), SecurityMode::ZrtpSrtp);
    }

    #[test]
    fn test_invalid_psk_key() {
        // Test with key that's too short
        let short_key = vec![0x01, 0x02, 0x03]; // Only 3 bytes
        let result = SecurityContextFactory::create_psk_context(short_key);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_sdes_initialization_placeholder() {
        // Test SDES initialization (should work once SDES is fully implemented)
        let context = SecurityContextFactory::create_sdes_context().unwrap();
        
        // Currently SDES initialization should work since we have the core implementation
        let result = context.initialize().await;
        assert!(result.is_ok());
        assert_eq!(context.get_state().await, SecurityState::Negotiating);
    }

    #[tokio::test]
    async fn test_mikey_initialization_placeholder() {
        // Test MIKEY initialization (currently returns error as placeholder)
        let key = test_srtp_key();
        let context = SecurityContextFactory::create_mikey_psk_context(key).unwrap();
        
        let result = context.initialize().await;
        assert!(result.is_err()); // Should error until MIKEY is fully implemented
    }

    #[tokio::test]
    async fn test_zrtp_initialization_success() {
        // Test ZRTP initialization (now should work with real implementation)
        let context = SecurityContextFactory::create_zrtp_context().unwrap();
        
        let result = context.initialize().await;
        assert!(result.is_ok()); // Should now succeed with actual ZRTP implementation
        assert_eq!(context.get_state().await, SecurityState::Negotiating);
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
            KeyExchangeConfig::PreSharedKey { key, salt } => {
                assert_eq!(key.len(), 16); // AES-128 key
                assert_eq!(salt.len(), 14); // Standard SRTP salt
            },
            _ => panic!("Expected PreSharedKey config"),
        }
    }

    #[test]
    fn test_sip_scenario_configs() {
        // Test predefined SIP scenario configurations
        let enterprise = SecurityConfig::sip_enterprise();
        assert_eq!(enterprise.mode, SecurityMode::MikeySrtp);

        let operator = SecurityConfig::sip_operator();
        assert_eq!(operator.mode, SecurityMode::SdesSrtp);

        let p2p = SecurityConfig::sip_peer_to_peer();
        assert_eq!(p2p.mode, SecurityMode::ZrtpSrtp);

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
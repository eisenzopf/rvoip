//! Security Context Manager
//!
//! This module provides high-level management of the implemented SDES and
//! directly provisioned PSK contexts. Retained DTLS-SRTP, MIKEY, and ZRTP
//! method identifiers fail closed. Once established, callers can route RTP
//! through this manager's protect/unprotect methods.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::api::client::security::ClientSecurityContext;
use crate::api::common::config::{KeyExchangeMethod, SecurityConfig, SecurityMode};
use crate::api::common::error::SecurityError;
use crate::api::common::unified_security::{SecurityContextFactory, UnifiedSecurityContext};
use crate::api::server::security::ServerSecurityContext;

/// High-level security context manager for implemented security methods.
pub struct SecurityContextManager {
    /// Available security contexts by method
    contexts: Arc<RwLock<HashMap<KeyExchangeMethod, SecurityContextType>>>,
    /// Preferred order of key exchange methods
    method_preference: Vec<KeyExchangeMethod>,
    /// Currently active security method
    active_method: Arc<RwLock<Option<KeyExchangeMethod>>>,
    /// Base security configuration
    config: SecurityConfig,
}

/// Type of security context wrapper
#[derive(Clone)]
pub enum SecurityContextType {
    /// Unified context. Only SDES and directly provisioned PSK are available.
    Unified(Arc<UnifiedSecurityContext>),
    /// Existing DTLS-SRTP client context
    DtlsClient(Arc<dyn ClientSecurityContext>),
    /// Existing DTLS-SRTP server context
    DtlsServer(Arc<dyn ServerSecurityContext>),
}

/// Security negotiation strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NegotiationStrategy {
    /// Use the first available method
    FirstAvailable,
    /// Try methods in preference order with fallback
    PreferenceWithFallback,
    /// Use only the specified method (no fallback)
    Strict,
    /// Auto-detect based on incoming signaling
    AutoDetect,
}

/// Security method capabilities
#[derive(Debug, Clone)]
pub struct SecurityCapabilities {
    /// Supported key exchange methods
    pub supported_methods: Vec<KeyExchangeMethod>,
    /// Whether method can act as offerer
    pub can_offer: bool,
    /// Whether method can act as answerer
    pub can_answer: bool,
    /// Supported SRTP profiles
    pub srtp_profiles: Vec<crate::api::common::config::SrtpProfile>,
}

impl SecurityContextManager {
    /// Create a new security context manager
    pub fn new(config: SecurityConfig) -> Self {
        // Derive preferences only from an implemented and provisioned method.
        // A manager must never advertise a fallback that cannot actually
        // establish the configured protection.
        let method_preference = match config.mode {
            SecurityMode::SdesSrtp => vec![KeyExchangeMethod::Sdes],
            SecurityMode::Srtp
                if config
                    .srtp_key
                    .as_ref()
                    .is_some_and(|key_material| key_material.len() == 30) =>
            {
                vec![KeyExchangeMethod::PreSharedKey]
            }
            _ => Vec::new(),
        };

        Self {
            contexts: Arc::new(RwLock::new(HashMap::new())),
            method_preference,
            active_method: Arc::new(RwLock::new(None)),
            config,
        }
    }

    /// Create a manager with custom method preference
    pub fn with_method_preference(
        config: SecurityConfig,
        preference: Vec<KeyExchangeMethod>,
    ) -> Self {
        Self {
            contexts: Arc::new(RwLock::new(HashMap::new())),
            method_preference: preference,
            active_method: Arc::new(RwLock::new(None)),
            config,
        }
    }

    /// Initialize security contexts for supported methods
    pub async fn initialize(&self) -> Result<(), SecurityError> {
        self.config.validate()?;
        let mut contexts = self.contexts.write().await;

        for method in &self.method_preference {
            match method {
                KeyExchangeMethod::DtlsSrtp => {
                    // DTLS-SRTP contexts are created separately via existing infrastructure
                    // We'll handle this when needed
                }
                KeyExchangeMethod::Sdes
                | KeyExchangeMethod::Mikey
                | KeyExchangeMethod::Zrtp
                | KeyExchangeMethod::PreSharedKey => {
                    // Create unified context for these methods
                    let method_config = match self.create_method_config(*method) {
                        Ok(config) => config,
                        Err(error) => {
                            eprintln!(
                                "Warning: Failed to configure {} context: {}",
                                self.method_name(*method),
                                error
                            );
                            continue;
                        }
                    };
                    match SecurityContextFactory::create_context(method_config) {
                        Ok(unified_context) => {
                            contexts.insert(
                                *method,
                                SecurityContextType::Unified(Arc::new(unified_context)),
                            );
                        }
                        Err(e) => {
                            // Log warning but continue with other methods
                            eprintln!(
                                "Warning: Failed to initialize {} context: {}",
                                self.method_name(*method),
                                e
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Create method-specific configuration
    fn create_method_config(
        &self,
        method: KeyExchangeMethod,
    ) -> Result<SecurityConfig, SecurityError> {
        let mut config = self.config.clone();
        config.mode = method.to_security_mode();
        config.validate()?;
        Ok(config)
    }

    /// Get human-readable method name
    fn method_name(&self, method: KeyExchangeMethod) -> &'static str {
        match method {
            KeyExchangeMethod::DtlsSrtp => "DTLS-SRTP",
            KeyExchangeMethod::Sdes => "SDES-SRTP",
            KeyExchangeMethod::Mikey => "MIKEY-SRTP",
            KeyExchangeMethod::Zrtp => "ZRTP-SRTP",
            KeyExchangeMethod::PreSharedKey => "PSK-SRTP",
        }
    }

    fn ensure_method_available(method: KeyExchangeMethod) -> Result<(), SecurityError> {
        match method {
            KeyExchangeMethod::Sdes | KeyExchangeMethod::PreSharedKey => Ok(()),
            KeyExchangeMethod::DtlsSrtp => Err(SecurityError::UnsupportedFeature(
                "DTLS-SRTP is not complete and is unavailable".to_string(),
            )),
            KeyExchangeMethod::Mikey => Err(SecurityError::UnsupportedFeature(
                "MIKEY key exchange is not complete and is unavailable".to_string(),
            )),
            KeyExchangeMethod::Zrtp => Err(SecurityError::UnsupportedFeature(
                "ZRTP key exchange is not complete and is unavailable".to_string(),
            )),
        }
    }

    fn method_is_available(method: KeyExchangeMethod) -> bool {
        Self::ensure_method_available(method).is_ok()
    }

    /// Add a DTLS-SRTP client context
    pub async fn add_dtls_client_context(&self, context: Arc<dyn ClientSecurityContext>) {
        let mut contexts = self.contexts.write().await;
        contexts.insert(
            KeyExchangeMethod::DtlsSrtp,
            SecurityContextType::DtlsClient(context),
        );
    }

    /// Add a DTLS-SRTP server context
    pub async fn add_dtls_server_context(&self, context: Arc<dyn ServerSecurityContext>) {
        let mut contexts = self.contexts.write().await;
        contexts.insert(
            KeyExchangeMethod::DtlsSrtp,
            SecurityContextType::DtlsServer(context),
        );
    }

    /// Start security negotiation with a specific method
    pub async fn start_negotiation(&self, method: KeyExchangeMethod) -> Result<(), SecurityError> {
        Self::ensure_method_available(method)?;
        let contexts = self.contexts.read().await;
        let context = contexts.get(&method).ok_or_else(|| {
            SecurityError::Configuration(format!(
                "Method {} not available",
                self.method_name(method)
            ))
        })?;

        match context {
            SecurityContextType::Unified(unified) => {
                unified.initialize().await?;
                *self.active_method.write().await = Some(method);
            }
            SecurityContextType::DtlsClient(_) | SecurityContextType::DtlsServer(_) => {
                // DTLS negotiation is handled by existing infrastructure
                *self.active_method.write().await = Some(method);
            }
        }

        Ok(())
    }

    /// Auto-negotiate security method based on available contexts and preference
    pub async fn auto_negotiate(
        &self,
        strategy: NegotiationStrategy,
    ) -> Result<KeyExchangeMethod, SecurityError> {
        let contexts = self.contexts.read().await;

        match strategy {
            NegotiationStrategy::FirstAvailable => {
                for method in &self.method_preference {
                    if contexts.contains_key(method) {
                        let selected_method = *method;
                        drop(contexts);
                        self.start_negotiation(selected_method).await?;
                        return Ok(selected_method);
                    }
                }
                Err(SecurityError::Configuration(
                    "No security methods available".to_string(),
                ))
            }
            NegotiationStrategy::PreferenceWithFallback => {
                // Try to initialize the first available method
                let available_methods: Vec<KeyExchangeMethod> = self
                    .method_preference
                    .iter()
                    .filter(|method| contexts.contains_key(method))
                    .copied()
                    .collect();
                drop(contexts);

                for method in available_methods {
                    match self.start_negotiation(method).await {
                        Ok(_) => return Ok(method),
                        Err(_) => {
                            // Continue to next method
                            continue;
                        }
                    }
                }
                Err(SecurityError::Configuration(
                    "All security methods failed".to_string(),
                ))
            }
            NegotiationStrategy::Strict => {
                // Use only the primary method from config
                let primary_method = self.config.mode.key_exchange_method().ok_or_else(|| {
                    SecurityError::Configuration("No primary method configured".to_string())
                })?;

                if contexts.contains_key(&primary_method) {
                    drop(contexts);
                    self.start_negotiation(primary_method).await?;
                    Ok(primary_method)
                } else {
                    Err(SecurityError::Configuration(format!(
                        "Primary method {} not available",
                        self.method_name(primary_method)
                    )))
                }
            }
            NegotiationStrategy::AutoDetect => {
                // This would analyze incoming signaling to determine the best method
                // For now, fall back to FirstAvailable
                drop(contexts);
                Box::pin(self.auto_negotiate(NegotiationStrategy::FirstAvailable)).await
            }
        }
    }

    /// Process incoming signaling for key exchange
    pub async fn process_signaling(
        &self,
        data: &[u8],
        method: Option<KeyExchangeMethod>,
    ) -> Result<Option<Vec<u8>>, SecurityError> {
        let method = match method {
            Some(m) => m,
            None => {
                // Try to auto-detect method from signaling
                self.detect_method_from_signaling(data)?
            }
        };

        Self::ensure_method_available(method)?;

        let contexts = self.contexts.read().await;
        let context = contexts.get(&method).ok_or_else(|| {
            SecurityError::Configuration(format!(
                "Method {} not available",
                self.method_name(method)
            ))
        })?;

        match context {
            SecurityContextType::Unified(unified) => unified.process_message(data).await,
            SecurityContextType::DtlsClient(_) | SecurityContextType::DtlsServer(_) => {
                // DTLS signaling is handled differently
                Err(SecurityError::Configuration(
                    "DTLS signaling should be handled by DTLS contexts".to_string(),
                ))
            }
        }
    }

    /// Detect key exchange method from signaling data
    fn detect_method_from_signaling(
        &self,
        data: &[u8],
    ) -> Result<KeyExchangeMethod, SecurityError> {
        let data_str = std::str::from_utf8(data).map_err(|error| {
            SecurityError::Configuration(format!("security signaling is not valid UTF-8: {error}"))
        })?;

        // Simple detection heuristics
        if data_str.contains("a=crypto:") {
            Ok(KeyExchangeMethod::Sdes)
        } else if data_str.contains("MIKEY") {
            Err(SecurityError::UnsupportedFeature(
                "MIKEY key exchange is not complete and is unavailable".to_string(),
            ))
        } else if data_str.contains("zrtp-version") {
            Err(SecurityError::UnsupportedFeature(
                "ZRTP key exchange is not complete and is unavailable".to_string(),
            ))
        } else {
            Err(SecurityError::Configuration(
                "security signaling does not identify a supported key-exchange method".to_string(),
            ))
        }
    }

    /// Get the currently active method
    pub async fn get_active_method(&self) -> Option<KeyExchangeMethod> {
        *self.active_method.read().await
    }

    /// Check if security is established
    pub async fn is_established(&self) -> Result<bool, SecurityError> {
        let active_method = self.get_active_method().await.ok_or_else(|| {
            SecurityError::NotInitialized("No active security method".to_string())
        })?;

        let contexts = self.contexts.read().await;
        let context = contexts.get(&active_method).ok_or_else(|| {
            SecurityError::NotInitialized("Active method context not found".to_string())
        })?;

        match context {
            SecurityContextType::Unified(unified) => Ok(unified.is_established().await),
            SecurityContextType::DtlsClient(client) => client
                .is_handshake_complete()
                .await
                .map_err(|e| SecurityError::CryptoError(format!("DTLS client error: {}", e))),
            SecurityContextType::DtlsServer(server) => {
                // Server readiness check - this might need adjustment based on server API
                server
                    .is_ready()
                    .await
                    .map_err(|e| SecurityError::CryptoError(format!("DTLS server error: {}", e)))
            }
        }
    }

    /// Protect an RTP packet with the currently established unified context.
    ///
    /// This is the media-crypto handoff for manager users; negotiation alone
    /// must not be treated as protection unless media is routed through this
    /// method (or through an explicitly installed `SrtpContext`).
    pub async fn protect_rtp(
        &self,
        packet: &crate::packet::RtpPacket,
    ) -> Result<crate::srtp::ProtectedRtpPacket, SecurityError> {
        let active_method = self.get_active_method().await.ok_or_else(|| {
            SecurityError::NotInitialized("No active security method".to_string())
        })?;
        let context = self
            .contexts
            .read()
            .await
            .get(&active_method)
            .cloned()
            .ok_or_else(|| {
                SecurityError::NotInitialized("Active method context not found".to_string())
            })?;

        match context {
            SecurityContextType::Unified(unified) => unified.protect_rtp(packet).await,
            SecurityContextType::DtlsClient(_) | SecurityContextType::DtlsServer(_) => {
                Err(SecurityError::UnsupportedFeature(
                    "DTLS-SRTP media protection is unavailable".to_string(),
                ))
            }
        }
    }

    /// Authenticate and unprotect RTP with the currently established context.
    pub async fn unprotect_rtp(
        &self,
        data: &[u8],
    ) -> Result<crate::packet::RtpPacket, SecurityError> {
        let active_method = self.get_active_method().await.ok_or_else(|| {
            SecurityError::NotInitialized("No active security method".to_string())
        })?;
        let context = self
            .contexts
            .read()
            .await
            .get(&active_method)
            .cloned()
            .ok_or_else(|| {
                SecurityError::NotInitialized("Active method context not found".to_string())
            })?;

        match context {
            SecurityContextType::Unified(unified) => unified.unprotect_rtp(data).await,
            SecurityContextType::DtlsClient(_) | SecurityContextType::DtlsServer(_) => {
                Err(SecurityError::UnsupportedFeature(
                    "DTLS-SRTP media unprotection is unavailable".to_string(),
                ))
            }
        }
    }

    /// Get security capabilities
    pub async fn get_capabilities(&self) -> SecurityCapabilities {
        let contexts = self.contexts.read().await;
        let supported_methods: Vec<KeyExchangeMethod> = contexts
            .keys()
            .copied()
            .filter(|method| Self::method_is_available(*method))
            .collect();
        let config_is_valid = self.config.validate().is_ok();
        let has_supported_method = config_is_valid && !supported_methods.is_empty();
        // SDES is currently initialized with the offerer role. Direct PSK is
        // usable for pre-provisioned media crypto, but it has no signaling
        // offer/answer exchange. Report those distinctions exactly.
        let can_offer =
            has_supported_method && supported_methods.contains(&KeyExchangeMethod::Sdes);

        SecurityCapabilities {
            supported_methods: if has_supported_method {
                supported_methods
            } else {
                Vec::new()
            },
            can_offer,
            can_answer: false,
            srtp_profiles: if has_supported_method {
                self.config.srtp_profiles.clone()
            } else {
                Vec::new()
            },
        }
    }

    /// Generate security offer (e.g., for SDP)
    pub async fn create_security_offer(
        &self,
        method: KeyExchangeMethod,
    ) -> Result<Vec<String>, SecurityError> {
        Self::ensure_method_available(method)?;
        let contexts = self.contexts.read().await;
        let context = contexts.get(&method).ok_or_else(|| {
            SecurityError::Configuration(format!(
                "Method {} not available",
                self.method_name(method)
            ))
        })?;

        match context {
            SecurityContextType::Unified(unified) => {
                if method == KeyExchangeMethod::Sdes {
                    unified.create_sdes_offer().await
                } else {
                    Err(SecurityError::UnsupportedFeature(
                        "Offer generation not implemented for this method".to_string(),
                    ))
                }
            }
            SecurityContextType::DtlsClient(client) => {
                // Get DTLS fingerprint for SDP
                let fingerprint = client.get_fingerprint().await.map_err(|e| {
                    SecurityError::CryptoError(format!("Failed to get fingerprint: {}", e))
                })?;

                Ok(vec![
                    format!("a=fingerprint:sha-256 {}", fingerprint),
                    "a=setup:actpass".to_string(),
                ])
            }
            SecurityContextType::DtlsServer(server) => {
                // Get DTLS fingerprint for SDP
                let fingerprint = server.get_fingerprint().await.map_err(|e| {
                    SecurityError::CryptoError(format!("Failed to get fingerprint: {}", e))
                })?;

                Ok(vec![
                    format!("a=fingerprint:sha-256 {}", fingerprint),
                    "a=setup:passive".to_string(),
                ])
            }
        }
    }

    /// List available security methods
    pub async fn list_available_methods(&self) -> Vec<KeyExchangeMethod> {
        let contexts = self.contexts.read().await;
        contexts
            .keys()
            .copied()
            .filter(|method| Self::method_is_available(*method))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::common::config::{SecurityConfig, SecurityMode, SrtpProfile};

    /// Test SRTP key for testing
    fn test_srtp_key() -> Vec<u8> {
        vec![
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
            0x0F, 0x10, // Salt
            0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
        ]
    }

    #[tokio::test]
    async fn test_create_security_manager() {
        let config = SecurityConfig::sdes_srtp();
        let manager = SecurityContextManager::new(config);

        // Should start with no active method
        assert_eq!(manager.get_active_method().await, None);
    }

    #[tokio::test]
    async fn test_initialize_manager() {
        let config = SecurityConfig::srtp_with_key(test_srtp_key());
        let manager = SecurityContextManager::new(config);

        // Initialize should work
        let result = manager.initialize().await;
        assert!(result.is_ok());

        // Should have at least PSK method available
        let methods = manager.list_available_methods().await;
        assert!(methods.contains(&KeyExchangeMethod::PreSharedKey));
    }

    #[tokio::test]
    async fn test_custom_method_preference() {
        let config = SecurityConfig::sdes_srtp();
        let preference = vec![
            KeyExchangeMethod::Sdes,
            KeyExchangeMethod::PreSharedKey,
            KeyExchangeMethod::DtlsSrtp,
        ];
        let manager = SecurityContextManager::with_method_preference(config, preference);

        assert_eq!(manager.method_preference[0], KeyExchangeMethod::Sdes);
        assert_eq!(
            manager.method_preference[1],
            KeyExchangeMethod::PreSharedKey
        );
    }

    #[tokio::test]
    async fn test_method_detection() {
        let config = SecurityConfig::sdes_srtp();
        let manager = SecurityContextManager::new(config);

        // Test SDES detection
        let sdes_sdp = b"a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:test";
        let detected = manager.detect_method_from_signaling(sdes_sdp).unwrap();
        assert_eq!(detected, KeyExchangeMethod::Sdes);

        // Incomplete MIKEY signaling must never become a negotiable method.
        let mikey_data = b"MIKEY message content";
        assert!(matches!(
            manager.detect_method_from_signaling(mikey_data),
            Err(SecurityError::UnsupportedFeature(_))
        ));

        // Incomplete ZRTP signaling must never become a negotiable method.
        let zrtp_data = b"zrtp-version: 1.10";
        assert!(matches!(
            manager.detect_method_from_signaling(zrtp_data),
            Err(SecurityError::UnsupportedFeature(_))
        ));

        // Unknown and non-text input must not silently become SDES.
        let unknown_data = b"random signaling data";
        assert!(matches!(
            manager.detect_method_from_signaling(unknown_data),
            Err(SecurityError::Configuration(_))
        ));
        assert!(matches!(
            manager.detect_method_from_signaling(&[0xff, 0xfe]),
            Err(SecurityError::Configuration(_))
        ));
    }

    #[tokio::test]
    async fn test_method_name_mapping() {
        let config = SecurityConfig::sdes_srtp();
        let manager = SecurityContextManager::new(config);

        assert_eq!(
            manager.method_name(KeyExchangeMethod::DtlsSrtp),
            "DTLS-SRTP"
        );
        assert_eq!(manager.method_name(KeyExchangeMethod::Sdes), "SDES-SRTP");
        assert_eq!(manager.method_name(KeyExchangeMethod::Mikey), "MIKEY-SRTP");
        assert_eq!(manager.method_name(KeyExchangeMethod::Zrtp), "ZRTP-SRTP");
        assert_eq!(
            manager.method_name(KeyExchangeMethod::PreSharedKey),
            "PSK-SRTP"
        );
    }

    #[tokio::test]
    async fn test_security_capabilities() {
        let config = SecurityConfig::srtp_with_key(test_srtp_key());
        let manager = SecurityContextManager::new(config);
        manager.initialize().await.unwrap();

        let capabilities = manager.get_capabilities().await;

        assert!(!capabilities.can_offer);
        assert!(!capabilities.can_answer);
        assert!(!capabilities.supported_methods.is_empty());
        assert!(!capabilities.srtp_profiles.is_empty());
    }

    #[tokio::test]
    async fn capabilities_distinguish_sdes_offering_from_unimplemented_answering() {
        let manager = SecurityContextManager::new(SecurityConfig::sdes_srtp());
        manager.initialize().await.unwrap();

        let capabilities = manager.get_capabilities().await;
        assert_eq!(
            capabilities.supported_methods,
            vec![KeyExchangeMethod::Sdes]
        );
        assert!(capabilities.can_offer);
        assert!(!capabilities.can_answer);
    }

    #[tokio::test]
    async fn invalid_profiles_are_neither_initialized_nor_advertised() {
        let mut config = SecurityConfig::sdes_srtp();
        config.srtp_profiles = vec![SrtpProfile::AesCm128HmacSha1_80, SrtpProfile::AesGcm128];
        let manager = SecurityContextManager::new(config);

        assert!(matches!(
            manager.initialize().await,
            Err(SecurityError::UnsupportedFeature(_))
        ));
        let capabilities = manager.get_capabilities().await;
        assert!(capabilities.supported_methods.is_empty());
        assert!(capabilities.srtp_profiles.is_empty());
        assert!(!capabilities.can_offer);
        assert!(!capabilities.can_answer);
    }

    #[tokio::test]
    async fn sdes_offer_contains_fresh_real_key_material() {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

        let manager = SecurityContextManager::with_method_preference(
            SecurityConfig::sdes_srtp(),
            vec![KeyExchangeMethod::Sdes],
        );
        manager.initialize().await.unwrap();
        let offer = manager
            .create_security_offer(KeyExchangeMethod::Sdes)
            .await
            .unwrap();

        assert!(!offer.is_empty());
        for line in offer {
            assert!(!line.contains("placeholder"));
            let attribute = crate::security::sdes::SdesCryptoAttribute::parse(
                line.strip_prefix("a=crypto:").unwrap(),
            )
            .unwrap();
            assert_eq!(BASE64.decode(attribute.key_info).unwrap().len(), 30);
        }
    }

    #[test]
    fn test_negotiation_strategy_enum() {
        // Test that all negotiation strategies exist
        let _ = NegotiationStrategy::FirstAvailable;
        let _ = NegotiationStrategy::PreferenceWithFallback;
        let _ = NegotiationStrategy::Strict;
        let _ = NegotiationStrategy::AutoDetect;
    }

    #[test]
    fn test_security_context_type_variants() {
        // Test that we can create different context types
        use crate::api::common::unified_security::SecurityContextFactory;
        use std::sync::Arc;

        let unified_context = SecurityContextFactory::create_sdes_context().unwrap();
        let _context_type = SecurityContextType::Unified(Arc::new(unified_context));

        // Test that the enum variants exist (can't easily create DTLS contexts in unit tests)
        // but we can verify the types compile
    }

    #[tokio::test]
    async fn test_psk_negotiation() {
        let config = SecurityConfig::srtp_with_key(test_srtp_key());
        let manager = SecurityContextManager::new(config);
        manager.initialize().await.unwrap();

        // Should be able to start PSK negotiation
        let result = manager
            .start_negotiation(KeyExchangeMethod::PreSharedKey)
            .await;
        assert!(result.is_ok());

        // Should now have an active method
        assert_eq!(
            manager.get_active_method().await,
            Some(KeyExchangeMethod::PreSharedKey)
        );
    }

    #[tokio::test]
    async fn established_psk_manager_exposes_real_media_crypto() {
        use bytes::Bytes;

        let sender = SecurityContextManager::new(SecurityConfig::srtp_with_key(test_srtp_key()));
        let receiver = SecurityContextManager::new(SecurityConfig::srtp_with_key(test_srtp_key()));
        for manager in [&sender, &receiver] {
            manager.initialize().await.unwrap();
            manager
                .start_negotiation(KeyExchangeMethod::PreSharedKey)
                .await
                .unwrap();
            assert!(manager.is_established().await.unwrap());
        }

        let packet = crate::packet::RtpPacket::new_with_payload(
            0,
            7,
            1_120,
            0x1020_3040,
            Bytes::from_static(b"manager-protected"),
        );
        let plaintext = packet.serialize().unwrap();
        let protected = sender.protect_rtp(&packet).await.unwrap();
        let wire = protected.serialize().unwrap();
        assert_ne!(wire, plaintext);

        let recovered = receiver.unprotect_rtp(&wire).await.unwrap();
        assert_eq!(recovered.payload, packet.payload);
        assert_eq!(
            recovered.header.sequence_number,
            packet.header.sequence_number
        );
    }

    #[tokio::test]
    async fn test_create_method_config() {
        let config = SecurityConfig::sdes_srtp();
        let manager = SecurityContextManager::new(config);

        let method_config = manager
            .create_method_config(KeyExchangeMethod::Sdes)
            .unwrap();
        assert_eq!(method_config.mode, SecurityMode::SdesSrtp);
    }

    #[tokio::test]
    async fn test_auto_negotiate_no_methods() {
        // Create manager with no available methods
        let config = SecurityConfig::sdes_srtp();
        let manager = SecurityContextManager::with_method_preference(config, vec![]);

        let result = manager
            .auto_negotiate(NegotiationStrategy::FirstAvailable)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_manager_initialization_warnings() {
        let config = SecurityConfig::zrtp_p2p();
        let manager = SecurityContextManager::new(config);

        assert!(matches!(
            manager.initialize().await,
            Err(SecurityError::UnsupportedFeature(_))
        ));
    }

    #[tokio::test]
    async fn incomplete_methods_are_not_available_or_negotiable() {
        let manager = SecurityContextManager::new(SecurityConfig::srtp_with_key(test_srtp_key()));
        manager.initialize().await.unwrap();

        for method in [
            KeyExchangeMethod::DtlsSrtp,
            KeyExchangeMethod::Mikey,
            KeyExchangeMethod::Zrtp,
        ] {
            assert!(!manager.list_available_methods().await.contains(&method));
            assert!(matches!(
                manager.start_negotiation(method).await,
                Err(SecurityError::UnsupportedFeature(_))
            ));
            assert!(matches!(
                manager.process_signaling(b"offer", Some(method)).await,
                Err(SecurityError::UnsupportedFeature(_))
            ));
            assert!(matches!(
                manager.create_security_offer(method).await,
                Err(SecurityError::UnsupportedFeature(_))
            ));
        }
    }

    #[test]
    fn built_in_preferences_only_include_configured_available_methods() {
        assert_eq!(
            SecurityContextManager::new(SecurityConfig::sdes_srtp()).method_preference,
            vec![KeyExchangeMethod::Sdes]
        );
        assert_eq!(
            SecurityContextManager::new(SecurityConfig::srtp_with_key(test_srtp_key()))
                .method_preference,
            vec![KeyExchangeMethod::PreSharedKey]
        );
        assert!(SecurityContextManager::new(SecurityConfig::default())
            .method_preference
            .is_empty());
        for config in [
            SecurityConfig::webrtc_compatible(),
            SecurityConfig::mikey_psk(),
            SecurityConfig::zrtp_p2p(),
        ] {
            assert!(SecurityContextManager::new(config)
                .method_preference
                .is_empty());
        }
    }

    #[test]
    fn test_security_capabilities_struct() {
        let capabilities = SecurityCapabilities {
            supported_methods: vec![KeyExchangeMethod::Sdes],
            can_offer: true,
            can_answer: false,
            srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
        };

        assert_eq!(capabilities.supported_methods.len(), 1);
        assert!(capabilities.can_offer);
        assert!(!capabilities.can_answer);
        assert_eq!(capabilities.srtp_profiles.len(), 1);
    }
}

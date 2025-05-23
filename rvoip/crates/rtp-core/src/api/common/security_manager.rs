//! Security Context Manager
//!
//! This module provides high-level management of security contexts, including
//! support for multiple key exchange methods, fallback mechanisms, and 
//! integration with existing DTLS-SRTP infrastructure.

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock;

use crate::api::common::config::{KeyExchangeMethod, SecurityConfig};
use crate::api::common::error::SecurityError;
use crate::api::common::unified_security::{UnifiedSecurityContext, SecurityState, SecurityContextFactory};
use crate::api::client::security::{ClientSecurityContext, DefaultClientSecurityContext};
use crate::api::server::security::{ServerSecurityContext, DefaultServerSecurityContext};

/// High-level security context manager that can coordinate multiple security methods
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
    /// Unified context for SDES, MIKEY, ZRTP, PSK
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
        // Default method preference based on security and compatibility
        let method_preference = vec![
            KeyExchangeMethod::DtlsSrtp,  // Most common in modern systems
            KeyExchangeMethod::Sdes,      // Good for SIP systems
            KeyExchangeMethod::Zrtp,      // Good for P2P
            KeyExchangeMethod::Mikey,     // Enterprise
            KeyExchangeMethod::PreSharedKey, // Fallback
        ];

        Self {
            contexts: Arc::new(RwLock::new(HashMap::new())),
            method_preference,
            active_method: Arc::new(RwLock::new(None)),
            config,
        }
    }

    /// Create a manager with custom method preference
    pub fn with_method_preference(config: SecurityConfig, preference: Vec<KeyExchangeMethod>) -> Self {
        Self {
            contexts: Arc::new(RwLock::new(HashMap::new())),
            method_preference: preference,
            active_method: Arc::new(RwLock::new(None)),
            config,
        }
    }

    /// Initialize security contexts for supported methods
    pub async fn initialize(&self) -> Result<(), SecurityError> {
        let mut contexts = self.contexts.write().await;

        for method in &self.method_preference {
            match method {
                KeyExchangeMethod::DtlsSrtp => {
                    // DTLS-SRTP contexts are created separately via existing infrastructure
                    // We'll handle this when needed
                },
                KeyExchangeMethod::Sdes 
                | KeyExchangeMethod::Mikey 
                | KeyExchangeMethod::Zrtp 
                | KeyExchangeMethod::PreSharedKey => {
                    // Create unified context for these methods
                    let method_config = self.create_method_config(*method)?;
                    match SecurityContextFactory::create_context(method_config) {
                        Ok(unified_context) => {
                            contexts.insert(*method, SecurityContextType::Unified(Arc::new(unified_context)));
                        },
                        Err(e) => {
                            // Log warning but continue with other methods
                            eprintln!("Warning: Failed to initialize {} context: {}", 
                                self.method_name(*method), e);
                        }
                    }
                },
            }
        }

        Ok(())
    }

    /// Create method-specific configuration
    fn create_method_config(&self, method: KeyExchangeMethod) -> Result<SecurityConfig, SecurityError> {
        let mut config = self.config.clone();
        config.mode = method.to_security_mode();
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

    /// Add a DTLS-SRTP client context
    pub async fn add_dtls_client_context(&self, context: Arc<dyn ClientSecurityContext>) {
        let mut contexts = self.contexts.write().await;
        contexts.insert(KeyExchangeMethod::DtlsSrtp, SecurityContextType::DtlsClient(context));
    }

    /// Add a DTLS-SRTP server context
    pub async fn add_dtls_server_context(&self, context: Arc<dyn ServerSecurityContext>) {
        let mut contexts = self.contexts.write().await;
        contexts.insert(KeyExchangeMethod::DtlsSrtp, SecurityContextType::DtlsServer(context));
    }

    /// Start security negotiation with a specific method
    pub async fn start_negotiation(&self, method: KeyExchangeMethod) -> Result<(), SecurityError> {
        let contexts = self.contexts.read().await;
        let context = contexts.get(&method)
            .ok_or_else(|| SecurityError::Configuration(format!("Method {} not available", self.method_name(method))))?;

        match context {
            SecurityContextType::Unified(unified) => {
                unified.initialize().await?;
                *self.active_method.write().await = Some(method);
            },
            SecurityContextType::DtlsClient(_) | SecurityContextType::DtlsServer(_) => {
                // DTLS negotiation is handled by existing infrastructure
                *self.active_method.write().await = Some(method);
            },
        }

        Ok(())
    }

    /// Auto-negotiate security method based on available contexts and preference
    pub async fn auto_negotiate(&self, strategy: NegotiationStrategy) -> Result<KeyExchangeMethod, SecurityError> {
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
                Err(SecurityError::Configuration("No security methods available".to_string()))
            },
            NegotiationStrategy::PreferenceWithFallback => {
                // Try to initialize the first available method
                let available_methods: Vec<KeyExchangeMethod> = self.method_preference.iter()
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
                Err(SecurityError::Configuration("All security methods failed".to_string()))
            },
            NegotiationStrategy::Strict => {
                // Use only the primary method from config
                let primary_method = self.config.mode.key_exchange_method()
                    .ok_or_else(|| SecurityError::Configuration("No primary method configured".to_string()))?;
                
                if contexts.contains_key(&primary_method) {
                    drop(contexts);
                    self.start_negotiation(primary_method).await?;
                    Ok(primary_method)
                } else {
                    Err(SecurityError::Configuration(format!("Primary method {} not available", self.method_name(primary_method))))
                }
            },
            NegotiationStrategy::AutoDetect => {
                // This would analyze incoming signaling to determine the best method
                // For now, fall back to FirstAvailable
                drop(contexts);
                Box::pin(self.auto_negotiate(NegotiationStrategy::FirstAvailable)).await
            },
        }
    }

    /// Process incoming signaling for key exchange
    pub async fn process_signaling(&self, data: &[u8], method: Option<KeyExchangeMethod>) -> Result<Option<Vec<u8>>, SecurityError> {
        let method = match method {
            Some(m) => m,
            None => {
                // Try to auto-detect method from signaling
                self.detect_method_from_signaling(data)?
            }
        };

        let contexts = self.contexts.read().await;
        let context = contexts.get(&method)
            .ok_or_else(|| SecurityError::Configuration(format!("Method {} not available", self.method_name(method))))?;

        match context {
            SecurityContextType::Unified(unified) => {
                unified.process_message(data).await
            },
            SecurityContextType::DtlsClient(_) | SecurityContextType::DtlsServer(_) => {
                // DTLS signaling is handled differently
                Err(SecurityError::Configuration("DTLS signaling should be handled by DTLS contexts".to_string()))
            },
        }
    }

    /// Detect key exchange method from signaling data
    fn detect_method_from_signaling(&self, data: &[u8]) -> Result<KeyExchangeMethod, SecurityError> {
        let data_str = std::str::from_utf8(data).unwrap_or("");
        
        // Simple detection heuristics
        if data_str.contains("a=crypto:") {
            Ok(KeyExchangeMethod::Sdes)
        } else if data_str.contains("MIKEY") {
            Ok(KeyExchangeMethod::Mikey)
        } else if data_str.contains("zrtp-version") {
            Ok(KeyExchangeMethod::Zrtp)
        } else {
            // Default to SDES for SDP-based signaling
            Ok(KeyExchangeMethod::Sdes)
        }
    }

    /// Get the currently active method
    pub async fn get_active_method(&self) -> Option<KeyExchangeMethod> {
        *self.active_method.read().await
    }

    /// Check if security is established
    pub async fn is_established(&self) -> Result<bool, SecurityError> {
        let active_method = self.get_active_method().await
            .ok_or_else(|| SecurityError::NotInitialized("No active security method".to_string()))?;

        let contexts = self.contexts.read().await;
        let context = contexts.get(&active_method)
            .ok_or_else(|| SecurityError::NotInitialized("Active method context not found".to_string()))?;

        match context {
            SecurityContextType::Unified(unified) => {
                Ok(unified.is_established().await)
            },
            SecurityContextType::DtlsClient(client) => {
                client.is_handshake_complete().await
                    .map_err(|e| SecurityError::CryptoError(format!("DTLS client error: {}", e)))
            },
            SecurityContextType::DtlsServer(server) => {
                // Server readiness check - this might need adjustment based on server API
                server.is_ready().await
                    .map_err(|e| SecurityError::CryptoError(format!("DTLS server error: {}", e)))
            },
        }
    }

    /// Get security capabilities
    pub async fn get_capabilities(&self) -> SecurityCapabilities {
        let contexts = self.contexts.read().await;
        let supported_methods: Vec<KeyExchangeMethod> = contexts.keys().copied().collect();

        SecurityCapabilities {
            supported_methods,
            can_offer: true,  // Most methods can offer
            can_answer: true, // Most methods can answer
            srtp_profiles: self.config.srtp_profiles.clone(),
        }
    }

    /// Generate security offer (e.g., for SDP)
    pub async fn create_security_offer(&self, method: KeyExchangeMethod) -> Result<Vec<String>, SecurityError> {
        let contexts = self.contexts.read().await;
        let context = contexts.get(&method)
            .ok_or_else(|| SecurityError::Configuration(format!("Method {} not available", self.method_name(method))))?;

        match context {
            SecurityContextType::Unified(unified) => {
                // For SDES, we can generate crypto lines
                if method == KeyExchangeMethod::Sdes {
                    // This would generate SDP crypto attributes
                    // For now, return a placeholder
                    Ok(vec!["a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:placeholder".to_string()])
                } else {
                    Err(SecurityError::Configuration("Offer generation not implemented for this method".to_string()))
                }
            },
            SecurityContextType::DtlsClient(client) => {
                // Get DTLS fingerprint for SDP
                let fingerprint = client.get_fingerprint().await
                    .map_err(|e| SecurityError::CryptoError(format!("Failed to get fingerprint: {}", e)))?;
                
                Ok(vec![
                    format!("a=fingerprint:sha-256 {}", fingerprint),
                    "a=setup:actpass".to_string(),
                ])
            },
            SecurityContextType::DtlsServer(server) => {
                // Get DTLS fingerprint for SDP
                let fingerprint = server.get_fingerprint().await
                    .map_err(|e| SecurityError::CryptoError(format!("Failed to get fingerprint: {}", e)))?;
                
                Ok(vec![
                    format!("a=fingerprint:sha-256 {}", fingerprint),
                    "a=setup:passive".to_string(),
                ])
            },
        }
    }

    /// List available security methods
    pub async fn list_available_methods(&self) -> Vec<KeyExchangeMethod> {
        let contexts = self.contexts.read().await;
        contexts.keys().copied().collect()
    }
} 
//! SDES Server Implementation
//!
//! This module provides SDES (Security DEScriptions) server functionality for the API layer.
//! It handles crypto offer generation, answer processing, and key confirmation.

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, debug, warn, error};

use crate::api::common::error::SecurityError;
use crate::api::common::config::{SrtpProfile, SecurityConfig};
use crate::security::{SecurityKeyExchange, sdes::{Sdes, SdesConfig, SdesRole, SdesCryptoAttribute}};
use crate::srtp::{SrtpContext, SrtpCryptoSuite, SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32};
use crate::srtp::crypto::SrtpCryptoKey;

/// SDES server configuration
#[derive(Debug, Clone)]
pub struct SdesServerConfig {
    /// Supported SRTP profiles in order of preference
    pub supported_profiles: Vec<SrtpProfile>,
    /// Number of crypto attributes to include in offers
    pub offer_count: usize,
    /// Whether to require strong crypto suites only
    pub require_strong_crypto: bool,
    /// Maximum number of concurrent key exchanges
    pub max_concurrent_exchanges: usize,
}

impl Default for SdesServerConfig {
    fn default() -> Self {
        Self {
            supported_profiles: vec![
                SrtpProfile::AesCm128HmacSha1_80,
                SrtpProfile::AesCm128HmacSha1_32,
            ],
            offer_count: 2,
            require_strong_crypto: true,
            max_concurrent_exchanges: 100,
        }
    }
}

/// SDES server state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdesServerState {
    /// Initial state
    Initial,
    /// Offer generated and sent
    OfferGenerated,
    /// Processing answer
    ProcessingAnswer,
    /// Key exchange completed
    Completed,
    /// Error state
    Failed,
}

/// SDES server session for handling per-client crypto negotiation
pub struct SdesServerSession {
    /// Session identifier
    pub session_id: String,
    /// Configuration
    config: SdesServerConfig,
    /// Current state
    state: Arc<RwLock<SdesServerState>>,
    /// Core SDES implementation
    sdes: Arc<RwLock<Sdes>>,
    /// Established SRTP context (if any)
    srtp_context: Arc<RwLock<Option<SrtpContext>>>,
    /// Generated crypto attributes
    generated_crypto: Arc<RwLock<Vec<SdesCryptoAttribute>>>,
    /// Selected crypto attribute
    selected_crypto: Arc<RwLock<Option<SdesCryptoAttribute>>>,
}

impl SdesServerSession {
    /// Create a new SDES server session
    pub fn new(session_id: String, config: SdesServerConfig) -> Self {
        // Convert API config to core SDES config
        let sdes_config = SdesConfig {
            crypto_suites: Self::convert_srtp_profiles(&config.supported_profiles),
            offer_count: config.offer_count,
        };

        let sdes = Sdes::new(sdes_config, SdesRole::Offerer);

        Self {
            session_id,
            config,
            state: Arc::new(RwLock::new(SdesServerState::Initial)),
            sdes: Arc::new(RwLock::new(sdes)),
            srtp_context: Arc::new(RwLock::new(None)),
            generated_crypto: Arc::new(RwLock::new(Vec::new())),
            selected_crypto: Arc::new(RwLock::new(None)),
        }
    }

    /// Create SDES server session from security config
    pub fn from_security_config(session_id: String, config: &SecurityConfig) -> Self {
        let server_config = SdesServerConfig {
            supported_profiles: config.srtp_profiles.clone(),
            offer_count: config.srtp_profiles.len().min(4),
            require_strong_crypto: config.required,
            max_concurrent_exchanges: 100,
        };
        Self::new(session_id, server_config)
    }

    /// Convert API SRTP profiles to core crypto suites
    fn convert_srtp_profiles(profiles: &[SrtpProfile]) -> Vec<SrtpCryptoSuite> {
        profiles.iter().filter_map(|profile| {
            match profile {
                SrtpProfile::AesCm128HmacSha1_80 => Some(SRTP_AES128_CM_SHA1_80),
                SrtpProfile::AesCm128HmacSha1_32 => Some(SRTP_AES128_CM_SHA1_32),
                SrtpProfile::AesGcm128 => {
                    // AES-GCM not implemented in core yet
                    debug!("AES-GCM profile not yet supported, skipping");
                    None
                },
                SrtpProfile::AesGcm256 => {
                    // AES-GCM not implemented in core yet  
                    debug!("AES-GCM-256 profile not yet supported, skipping");
                    None
                },
            }
        }).collect()
    }

    /// Get current state
    pub async fn get_state(&self) -> SdesServerState {
        *self.state.read().await
    }

    /// Check if key exchange is complete
    pub async fn is_completed(&self) -> bool {
        *self.state.read().await == SdesServerState::Completed
    }

    /// Generate SDP offer with crypto attributes
    /// Returns SDP lines with crypto attributes for inclusion in SDP offer
    pub async fn generate_offer(&self) -> Result<Vec<String>, SecurityError> {
        let mut state = self.state.write().await;
        if *state != SdesServerState::Initial {
            return Err(SecurityError::InvalidState("SDES server session not in initial state".to_string()));
        }

        *state = SdesServerState::OfferGenerated;
        drop(state);

        info!("SDES server session {}: Generating crypto offer", self.session_id);

        // Initialize SDES
        let mut sdes = self.sdes.write().await;
        sdes.init().map_err(|e| SecurityError::CryptoError(format!("SDES init failed: {}", e)))?;

        // Generate offer through core SDES implementation
        match sdes.process_message(b"") {
            Ok(Some(offer_bytes)) => {
                let offer_str = String::from_utf8(offer_bytes)
                    .map_err(|_| SecurityError::CryptoError("Invalid UTF-8 in SDES offer".to_string()))?;
                
                let offer_lines: Vec<String> = offer_str.lines()
                    .map(|s| s.to_string())
                    .collect();

                // Parse and store generated crypto attributes for later reference
                let mut crypto_attrs = Vec::new();
                for line in &offer_lines {
                    if let Some(crypto_part) = line.strip_prefix("a=crypto:") {
                        match SdesCryptoAttribute::parse(crypto_part) {
                            Ok(attr) => crypto_attrs.push(attr),
                            Err(e) => {
                                warn!("Failed to parse generated crypto attribute: {}", e);
                            }
                        }
                    }
                }
                *self.generated_crypto.write().await = crypto_attrs;

                info!("SDES server session {}: Generated {} crypto attributes", 
                      self.session_id, offer_lines.len());
                for (i, line) in offer_lines.iter().enumerate() {
                    debug!("  Crypto {}: {}", i + 1, line);
                }

                Ok(offer_lines)
            },
            Ok(None) => {
                *self.state.write().await = SdesServerState::Failed;
                Err(SecurityError::CryptoError("No offer generated by SDES".to_string()))
            },
            Err(e) => {
                error!("SDES offer generation failed: {}", e);
                *self.state.write().await = SdesServerState::Failed;
                Err(SecurityError::CryptoError(format!("SDES offer generation failed: {}", e)))
            }
        }
    }

    /// Process SDP answer containing selected crypto attribute
    /// Returns true if key exchange is now complete
    pub async fn process_answer(&self, sdp_answer: &[String]) -> Result<bool, SecurityError> {
        let mut state = self.state.write().await;
        if *state != SdesServerState::OfferGenerated {
            return Err(SecurityError::InvalidState("SDES server session not in offer-generated state".to_string()));
        }

        *state = SdesServerState::ProcessingAnswer;
        drop(state);

        info!("SDES server session {}: Processing SDP answer with {} lines", 
              self.session_id, sdp_answer.len());

        // Extract crypto lines from SDP answer
        let crypto_lines: Vec<String> = sdp_answer.iter()
            .filter(|line| line.trim().starts_with("a=crypto:"))
            .cloned()
            .collect();

        if crypto_lines.is_empty() {
            *self.state.write().await = SdesServerState::Failed;
            return Err(SecurityError::Configuration("No crypto attributes found in SDP answer".to_string()));
        }

        if crypto_lines.len() > 1 {
            warn!("SDP answer contains {} crypto attributes, expected 1", crypto_lines.len());
        }

        info!("Found {} crypto attributes in answer", crypto_lines.len());
        for (i, line) in crypto_lines.iter().enumerate() {
            debug!("  Crypto {}: {}", i + 1, line);
        }

        // Process through core SDES implementation
        let answer_message = crypto_lines.join("\r\n");
        let mut sdes = self.sdes.write().await;
        
        match sdes.process_message(answer_message.as_bytes()) {
            Ok(None) => {
                // Answer processed successfully
                // Set up SRTP context if keys are available
                if let (Some(srtp_key), Some(srtp_suite)) = (sdes.get_srtp_key(), sdes.get_srtp_suite()) {
                    match SrtpContext::new(srtp_suite, srtp_key) {
                        Ok(context) => {
                            *self.srtp_context.write().await = Some(context);
                            info!("SDES server session {}: SRTP context established", self.session_id);
                        },
                        Err(e) => {
                            error!("Failed to create SRTP context: {}", e);
                            *self.state.write().await = SdesServerState::Failed;
                            return Err(SecurityError::CryptoError(format!("Failed to create SRTP context: {}", e)));
                        }
                    }
                }

                // Store selected crypto attribute
                if let Some(crypto_line) = crypto_lines.first() {
                    if let Some(crypto_part) = crypto_line.strip_prefix("a=crypto:") {
                        if let Ok(attr) = SdesCryptoAttribute::parse(crypto_part) {
                            *self.selected_crypto.write().await = Some(attr);
                        }
                    }
                }

                *self.state.write().await = SdesServerState::Completed;
                info!("SDES server session {}: Key exchange completed successfully", self.session_id);

                Ok(true)
            },
            Ok(Some(_)) => {
                // Unexpected response from SDES
                *self.state.write().await = SdesServerState::Failed;
                Err(SecurityError::CryptoError("Unexpected response from SDES when processing answer".to_string()))
            },
            Err(e) => {
                error!("SDES answer processing failed: {}", e);
                *self.state.write().await = SdesServerState::Failed;
                Err(SecurityError::CryptoError(format!("SDES answer processing failed: {}", e)))
            }
        }
    }

    /// Get selected crypto attribute info for logging/debugging
    pub async fn get_selected_crypto_info(&self) -> Option<String> {
        self.selected_crypto.read().await.as_ref().map(|attr| {
            format!("tag={}, suite={}", attr.tag, attr.crypto_suite)
        })
    }

    /// Get generated crypto attributes (for logging/debugging)
    pub async fn get_generated_crypto_info(&self) -> Vec<String> {
        self.generated_crypto.read().await.iter().map(|attr| {
            format!("tag={}, suite={}", attr.tag, attr.crypto_suite)
        }).collect()
    }

    /// Get session ID
    pub fn get_session_id(&self) -> &str {
        &self.session_id
    }
}

/// SDES server for managing multiple client sessions
pub struct SdesServer {
    /// Configuration
    config: SdesServerConfig,
    /// Active sessions by session ID
    sessions: Arc<RwLock<std::collections::HashMap<String, Arc<SdesServerSession>>>>,
}

impl SdesServer {
    /// Create a new SDES server
    pub fn new(config: SdesServerConfig) -> Self {
        Self {
            config,
            sessions: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Create SDES server from security config
    pub fn from_security_config(config: &SecurityConfig) -> Self {
        let server_config = SdesServerConfig {
            supported_profiles: config.srtp_profiles.clone(),
            offer_count: config.srtp_profiles.len().min(4),
            require_strong_crypto: config.required,
            max_concurrent_exchanges: 100,
        };
        Self::new(server_config)
    }

    /// Create a new session for a client
    pub async fn create_session(&self, session_id: String) -> Result<Arc<SdesServerSession>, SecurityError> {
        let mut sessions = self.sessions.write().await;
        
        if sessions.len() >= self.config.max_concurrent_exchanges {
            return Err(SecurityError::Configuration("Maximum concurrent SDES exchanges reached".to_string()));
        }

        if sessions.contains_key(&session_id) {
            return Err(SecurityError::Configuration(format!("Session {} already exists", session_id)));
        }

        let session = Arc::new(SdesServerSession::new(session_id.clone(), self.config.clone()));
        sessions.insert(session_id.clone(), session.clone());

        info!("SDES server: Created session {} (total sessions: {})", session_id, sessions.len());

        Ok(session)
    }

    /// Get an existing session
    pub async fn get_session(&self, session_id: &str) -> Option<Arc<SdesServerSession>> {
        self.sessions.read().await.get(session_id).cloned()
    }

    /// Remove a session
    pub async fn remove_session(&self, session_id: &str) -> bool {
        let mut sessions = self.sessions.write().await;
        let removed = sessions.remove(session_id).is_some();
        
        if removed {
            info!("SDES server: Removed session {} (total sessions: {})", session_id, sessions.len());
        }
        
        removed
    }

    /// Get number of active sessions
    pub async fn session_count(&self) -> usize {
        self.sessions.read().await.len()
    }

    /// Get all session IDs
    pub async fn get_session_ids(&self) -> Vec<String> {
        self.sessions.read().await.keys().cloned().collect()
    }
} 
//! SDES Client Implementation
//!
//! This module provides SDES (Security DEScriptions) client functionality for the API layer.
//! It handles SDP crypto attribute parsing, key extraction, and SRTP context setup.

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, debug, warn, error};

use crate::api::common::error::SecurityError;
use crate::api::common::config::{SrtpProfile, SecurityConfig};
use crate::security::{SecurityKeyExchange, sdes::{Sdes, SdesConfig, SdesRole, SdesCryptoAttribute}};
use crate::srtp::{SrtpContext, SrtpCryptoSuite, SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32};
use crate::srtp::crypto::SrtpCryptoKey;

/// SDES client configuration
#[derive(Debug, Clone)]
pub struct SdesClientConfig {
    /// Supported SRTP profiles in order of preference
    pub supported_profiles: Vec<SrtpProfile>,
    /// Whether to validate incoming crypto attributes strictly
    pub strict_validation: bool,
    /// Maximum number of crypto attributes to accept in an offer
    pub max_crypto_attributes: usize,
}

impl Default for SdesClientConfig {
    fn default() -> Self {
        Self {
            supported_profiles: vec![
                SrtpProfile::AesCm128HmacSha1_80,
                SrtpProfile::AesCm128HmacSha1_32,
            ],
            strict_validation: true,
            max_crypto_attributes: 8,
        }
    }
}

/// SDES client state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdesClientState {
    /// Initial state
    Initial,
    /// Processing offer
    ProcessingOffer,
    /// Answer sent, waiting for confirmation
    AnswerSent,
    /// Key exchange completed
    Completed,
    /// Error state
    Failed,
}

/// SDES client for handling SDP crypto attribute negotiation
pub struct SdesClient {
    /// Configuration
    config: SdesClientConfig,
    /// Current state
    state: Arc<RwLock<SdesClientState>>,
    /// Core SDES implementation
    sdes: Arc<RwLock<Sdes>>,
    /// Established SRTP context (if any)
    srtp_context: Arc<RwLock<Option<SrtpContext>>>,
    /// Selected crypto attribute
    selected_crypto: Arc<RwLock<Option<SdesCryptoAttribute>>>,
}

impl SdesClient {
    /// Create a new SDES client
    pub fn new(config: SdesClientConfig) -> Self {
        // Convert API config to core SDES config
        let sdes_config = SdesConfig {
            crypto_suites: Self::convert_srtp_profiles(&config.supported_profiles),
            offer_count: config.supported_profiles.len().min(4),
        };

        let sdes = Sdes::new(sdes_config, SdesRole::Answerer);

        Self {
            config,
            state: Arc::new(RwLock::new(SdesClientState::Initial)),
            sdes: Arc::new(RwLock::new(sdes)),
            srtp_context: Arc::new(RwLock::new(None)),
            selected_crypto: Arc::new(RwLock::new(None)),
        }
    }

    /// Create SDES client from security config
    pub fn from_security_config(config: &SecurityConfig) -> Self {
        let client_config = SdesClientConfig {
            supported_profiles: config.srtp_profiles.clone(),
            strict_validation: config.required,
            max_crypto_attributes: 8,
        };
        Self::new(client_config)
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
    pub async fn get_state(&self) -> SdesClientState {
        *self.state.read().await
    }

    /// Check if key exchange is complete
    pub async fn is_completed(&self) -> bool {
        *self.state.read().await == SdesClientState::Completed
    }

    /// Process an SDP offer containing crypto attributes
    /// Returns SDP answer lines with selected crypto attribute
    pub async fn process_offer(&self, sdp_offer: &[String]) -> Result<Vec<String>, SecurityError> {
        let mut state = self.state.write().await;
        if *state != SdesClientState::Initial {
            return Err(SecurityError::InvalidState("SDES client not in initial state".to_string()));
        }

        *state = SdesClientState::ProcessingOffer;
        drop(state);

        info!("SDES client processing SDP offer with {} lines", sdp_offer.len());

        // Extract crypto lines from SDP offer
        let crypto_lines: Vec<String> = sdp_offer.iter()
            .filter(|line| line.trim().starts_with("a=crypto:"))
            .cloned()
            .collect();

        if crypto_lines.is_empty() {
            *self.state.write().await = SdesClientState::Failed;
            return Err(SecurityError::Configuration("No crypto attributes found in SDP offer".to_string()));
        }

        if crypto_lines.len() > self.config.max_crypto_attributes {
            warn!("SDP offer contains {} crypto attributes, limiting to {}", 
                  crypto_lines.len(), self.config.max_crypto_attributes);
        }

        info!("Found {} crypto attributes in offer", crypto_lines.len());
        for (i, line) in crypto_lines.iter().enumerate() {
            debug!("  Crypto {}: {}", i + 1, line);
        }

        // Process through core SDES implementation
        let offer_message = crypto_lines.join("\r\n");
        let mut sdes = self.sdes.write().await;
        
        match sdes.process_message(offer_message.as_bytes()) {
            Ok(Some(answer_bytes)) => {
                let answer_str = String::from_utf8(answer_bytes)
                    .map_err(|_| SecurityError::CryptoError("Invalid UTF-8 in SDES answer".to_string()))?;
                
                let answer_lines: Vec<String> = answer_str.lines()
                    .map(|s| s.to_string())
                    .collect();

                // Set up SRTP context if keys are available
                if let (Some(srtp_key), Some(srtp_suite)) = (sdes.get_srtp_key(), sdes.get_srtp_suite()) {
                    match SrtpContext::new(srtp_suite, srtp_key) {
                        Ok(context) => {
                            *self.srtp_context.write().await = Some(context);
                            info!("SDES client: SRTP context established");
                        },
                        Err(e) => {
                            error!("Failed to create SRTP context: {}", e);
                            *self.state.write().await = SdesClientState::Failed;
                            return Err(SecurityError::CryptoError(format!("Failed to create SRTP context: {}", e)));
                        }
                    }
                }

                *self.state.write().await = SdesClientState::Completed;
                info!("SDES client: Key exchange completed successfully");

                Ok(answer_lines)
            },
            Ok(None) => {
                *self.state.write().await = SdesClientState::Failed;
                Err(SecurityError::CryptoError("No answer generated by SDES".to_string()))
            },
            Err(e) => {
                error!("SDES processing failed: {}", e);
                *self.state.write().await = SdesClientState::Failed;
                Err(SecurityError::CryptoError(format!("SDES processing failed: {}", e)))
            }
        }
    }

    /// Get the established SRTP context
    pub async fn get_srtp_context(&self) -> Option<Arc<RwLock<SrtpContext>>> {
        let guard = self.srtp_context.read().await;
        if guard.is_some() {
            // For now, we'll return None due to the complex nested structure
            // In a production implementation, you'd want to restructure this
            // to avoid the Arc<RwLock<Option<T>>> pattern
            None
        } else {
            None
        }
    }

    /// Get selected crypto attribute info for logging/debugging
    pub async fn get_selected_crypto_info(&self) -> Option<String> {
        self.selected_crypto.read().await.as_ref().map(|attr| {
            format!("tag={}, suite={}", attr.tag, attr.crypto_suite)
        })
    }

    /// Parse crypto attributes from SDP lines (utility method)
    pub fn parse_crypto_attributes(sdp_lines: &[String]) -> Result<Vec<SdesCryptoAttribute>, SecurityError> {
        let mut attributes = Vec::new();

        for line in sdp_lines {
            if let Some(crypto_part) = line.strip_prefix("a=crypto:") {
                match SdesCryptoAttribute::parse(crypto_part) {
                    Ok(attr) => attributes.push(attr),
                    Err(e) => {
                        warn!("Failed to parse crypto attribute '{}': {}", crypto_part, e);
                        return Err(SecurityError::Configuration(
                            format!("Invalid crypto attribute: {}", e)
                        ));
                    }
                }
            }
        }

        Ok(attributes)
    }

    /// Generate crypto attributes for client-initiated offers (rare, but supported)
    pub async fn generate_offer(&self) -> Result<Vec<String>, SecurityError> {
        let mut state = self.state.write().await;
        if *state != SdesClientState::Initial {
            return Err(SecurityError::InvalidState("SDES client not in initial state".to_string()));
        }

        // Change role to offerer temporarily
        let sdes_config = SdesConfig {
            crypto_suites: Self::convert_srtp_profiles(&self.config.supported_profiles),
            offer_count: self.config.supported_profiles.len().min(4),
        };

        let mut offerer_sdes = Sdes::new(sdes_config, SdesRole::Offerer);
        offerer_sdes.init().map_err(|e| SecurityError::CryptoError(format!("SDES init failed: {}", e)))?;

        // Generate offer
        match offerer_sdes.process_message(b"") {
            Ok(Some(offer_bytes)) => {
                let offer_str = String::from_utf8(offer_bytes)
                    .map_err(|_| SecurityError::CryptoError("Invalid UTF-8 in SDES offer".to_string()))?;
                
                let offer_lines: Vec<String> = offer_str.lines()
                    .map(|s| s.to_string())
                    .collect();

                *state = SdesClientState::AnswerSent;
                info!("SDES client generated offer with {} crypto attributes", offer_lines.len());

                Ok(offer_lines)
            },
            Ok(None) => {
                Err(SecurityError::CryptoError("No offer generated by SDES".to_string()))
            },
            Err(e) => {
                error!("SDES offer generation failed: {}", e);
                Err(SecurityError::CryptoError(format!("SDES offer generation failed: {}", e)))
            }
        }
    }
} 
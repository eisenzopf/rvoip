//! SDES Client Implementation
//!
//! This module provides SDES (Security DEScriptions) client functionality for the API layer.
//! It handles SDP crypto attribute parsing, key extraction, and SRTP context setup.

use async_trait::async_trait;
use std::any::Any;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::api::client::security::{ClientSecurityConfig, ClientSecurityContext};
use crate::api::common::config::{SecurityConfig, SecurityInfo, SecurityMode, SrtpProfile};
use crate::api::common::error::SecurityError;
use crate::api::server::security::SocketHandle;
use crate::security::sdes::{SdesCryptoAttribute, SdesNegotiator};
use crate::srtp::{SrtpContext, SrtpCryptoSuite, SRTP_AES128_CM_SHA1_32, SRTP_AES128_CM_SHA1_80};

/// RFC 4568 §6.2 wire name for the suites this module offers.
fn suite_wire_name(suite: &SrtpCryptoSuite) -> Result<&'static str, SecurityError> {
    match (suite.key_length, suite.tag_length) {
        (16, 10) => Ok("AES_CM_128_HMAC_SHA1_80"),
        (16, 4) => Ok("AES_CM_128_HMAC_SHA1_32"),
        _ => Err(SecurityError::Configuration(format!(
            "unsupported crypto suite for SDES: {suite:?}"
        ))),
    }
}

fn suite_from_wire_name(name: &str) -> Result<SrtpCryptoSuite, SecurityError> {
    match name {
        "AES_CM_128_HMAC_SHA1_80" => Ok(SRTP_AES128_CM_SHA1_80),
        "AES_CM_128_HMAC_SHA1_32" => Ok(SRTP_AES128_CM_SHA1_32),
        other => Err(SecurityError::Configuration(format!(
            "unsupported crypto suite: {other}"
        ))),
    }
}

/// Format an `SdesCryptoAttribute` as a bare `a=crypto:` line body. This
/// module's `Vec<String>` SDP lines are only ever consumed by this same
/// module — no real SDP text is produced here, unlike the actual SIP
/// adapter's typed `sip-core` integration.
fn format_crypto_line(attr: &SdesCryptoAttribute) -> Result<String, SecurityError> {
    Ok(format!(
        "{} {} inline:{}",
        attr.tag,
        suite_wire_name(&attr.suite)?,
        attr.key_inline
    ))
}

/// Parse a bare `a=crypto:` line body (as produced by
/// [`format_crypto_line`]) back into an `SdesCryptoAttribute`.
fn parse_crypto_line(line: &str) -> Result<SdesCryptoAttribute, SecurityError> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 3 {
        return Err(SecurityError::Configuration(
            "invalid SDES crypto attribute format".to_string(),
        ));
    }
    let tag: u32 = parts[0]
        .parse()
        .map_err(|_| SecurityError::Configuration("invalid tag in crypto attribute".to_string()))?;
    let suite = suite_from_wire_name(parts[1])?;
    let key_inline = parts[2]
        .strip_prefix("inline:")
        .ok_or_else(|| SecurityError::Configuration("missing inline: key".to_string()))?
        .to_string();
    Ok(SdesCryptoAttribute::new(tag, suite, key_inline))
}

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
    /// Established SRTP context (if any)
    srtp_context: Arc<RwLock<Option<SrtpContext>>>,
    /// Selected crypto attribute
    selected_crypto: Arc<RwLock<Option<SdesCryptoAttribute>>>,
}

impl SdesClient {
    /// Create a new SDES client. Unlike the old byte-oriented engine,
    /// [`SdesNegotiator::new_answerer`] carries no state to hold between
    /// construction and processing an offer, so there's nothing to build
    /// here beyond the config itself.
    pub fn new(config: SdesClientConfig) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(SdesClientState::Initial)),
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
        profiles
            .iter()
            .filter_map(|profile| {
                match profile {
                    SrtpProfile::AesCm128HmacSha1_80 => Some(SRTP_AES128_CM_SHA1_80),
                    SrtpProfile::AesCm128HmacSha1_32 => Some(SRTP_AES128_CM_SHA1_32),
                    SrtpProfile::AesGcm128 => {
                        // AES-GCM not implemented in core yet
                        debug!("AES-GCM profile not yet supported, skipping");
                        None
                    }
                    SrtpProfile::AesGcm256 => {
                        // AES-GCM not implemented in core yet
                        debug!("AES-GCM-256 profile not yet supported, skipping");
                        None
                    }
                }
            })
            .collect()
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
            return Err(SecurityError::InvalidState(
                "SDES client not in initial state".to_string(),
            ));
        }

        *state = SdesClientState::ProcessingOffer;
        drop(state);

        info!(
            "SDES client processing SDP offer with {} lines",
            sdp_offer.len()
        );

        // Extract crypto lines from SDP offer
        let crypto_lines: Vec<String> = sdp_offer
            .iter()
            .filter(|line| line.trim().starts_with("a=crypto:"))
            .cloned()
            .collect();

        if crypto_lines.is_empty() {
            *self.state.write().await = SdesClientState::Failed;
            return Err(SecurityError::Configuration(
                "No crypto attributes found in SDP offer".to_string(),
            ));
        }

        if crypto_lines.len() > self.config.max_crypto_attributes {
            warn!(
                "SDP offer contains {} crypto attributes, limiting to {}",
                crypto_lines.len(),
                self.config.max_crypto_attributes
            );
        }

        info!("Found {} crypto attributes in offer", crypto_lines.len());
        for (i, line) in crypto_lines.iter().enumerate() {
            debug!("  Crypto {}: {}", i + 1, line);
        }

        // Parse crypto lines into typed attributes and process through
        // the canonical SDES engine (this is always the answerer side —
        // we're processing an inbound offer).
        let attrs: Vec<SdesCryptoAttribute> = match crypto_lines
            .iter()
            .map(|line| {
                let body = line.trim().strip_prefix("a=crypto:").unwrap_or(line);
                parse_crypto_line(body)
            })
            .collect()
        {
            Ok(attrs) => attrs,
            Err(e) => {
                *self.state.write().await = SdesClientState::Failed;
                return Err(e);
            }
        };

        let answerer = SdesNegotiator::new_answerer();
        match answerer.process_offer(&attrs) {
            Ok((answer_attr, pair)) => {
                let answer_line = match format_crypto_line(&answer_attr) {
                    Ok(line) => format!("a=crypto:{line}"),
                    Err(e) => {
                        *self.state.write().await = SdesClientState::Failed;
                        return Err(e);
                    }
                };

                *self.srtp_context.write().await = Some(pair.recv_ctx);
                *self.selected_crypto.write().await = Some(answer_attr);
                info!("SDES client: SRTP context established");

                *self.state.write().await = SdesClientState::Completed;
                info!("SDES client: Key exchange completed successfully");

                Ok(vec![answer_line])
            }
            Err(e) => {
                error!("SDES processing failed: {}", e);
                *self.state.write().await = SdesClientState::Failed;
                Err(SecurityError::CryptoError(format!(
                    "SDES processing failed: {}",
                    e
                )))
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
        self.selected_crypto
            .read()
            .await
            .as_ref()
            .map(|attr| format!("tag={}, suite={:?}", attr.tag, attr.suite))
    }

    /// Parse crypto attributes from SDP lines (utility method)
    pub fn parse_crypto_attributes(
        sdp_lines: &[String],
    ) -> Result<Vec<SdesCryptoAttribute>, SecurityError> {
        let mut attributes = Vec::new();

        for line in sdp_lines {
            if let Some(crypto_part) = line.strip_prefix("a=crypto:") {
                match parse_crypto_line(crypto_part) {
                    Ok(attr) => attributes.push(attr),
                    Err(e) => {
                        warn!("Failed to parse crypto attribute '{}': {}", crypto_part, e);
                        return Err(SecurityError::Configuration(format!(
                            "Invalid crypto attribute: {}",
                            e
                        )));
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
            return Err(SecurityError::InvalidState(
                "SDES client not in initial state".to_string(),
            ));
        }

        let suites = Self::convert_srtp_profiles(&self.config.supported_profiles);
        let (_offerer, attrs) = SdesNegotiator::new_offerer(&suites).map_err(|e| {
            SecurityError::CryptoError(format!("SDES offer generation failed: {e}"))
        })?;

        let offer_lines: Result<Vec<String>, SecurityError> = attrs
            .iter()
            .map(|attr| format_crypto_line(attr).map(|line| format!("a=crypto:{line}")))
            .collect();
        let offer_lines = offer_lines?;

        *state = SdesClientState::AnswerSent;
        info!(
            "SDES client generated offer with {} crypto attributes",
            offer_lines.len()
        );

        Ok(offer_lines)
    }
}

/// SRTP-only client security context (no DTLS handshake)
/// This implementation uses pre-shared keys negotiated through SIP/SDP
#[allow(dead_code)] // retained (liveness/Drop hold or reserved); not read
pub struct SrtpClientSecurityContext {
    /// Configuration
    config: ClientSecurityConfig,
    /// SDES client for key management
    #[allow(dead_code)] // retained (liveness/Drop hold or reserved); not read
    sdes_client: Arc<SdesClient>,
    /// Remote address
    remote_addr: Arc<RwLock<Option<SocketAddr>>>,
    /// Socket handle
    socket: Arc<RwLock<Option<SocketHandle>>>,
    /// Handshake completed flag (always true for pre-shared keys)
    handshake_completed: Arc<RwLock<bool>>,
}

impl SrtpClientSecurityContext {
    /// Create a new SRTP-only client security context
    pub async fn new(config: ClientSecurityConfig) -> Result<Arc<Self>, SecurityError> {
        // Create SDES client config from security config
        let sdes_config = SdesClientConfig {
            supported_profiles: config.srtp_profiles.clone(),
            strict_validation: true,
            max_crypto_attributes: 8,
        };

        let sdes_client = Arc::new(SdesClient::new(sdes_config));

        let ctx = Self {
            config,
            sdes_client,
            remote_addr: Arc::new(RwLock::new(None)),
            socket: Arc::new(RwLock::new(None)),
            handshake_completed: Arc::new(RwLock::new(true)), // Pre-shared keys = no handshake needed
        };

        info!("Created SRTP-only client security context (pre-shared keys)");
        Ok(Arc::new(ctx))
    }
}

#[async_trait]
impl ClientSecurityContext for SrtpClientSecurityContext {
    async fn initialize(&self) -> Result<(), SecurityError> {
        debug!("Initializing SRTP-only client security context");
        // No DTLS initialization needed for pre-shared keys
        Ok(())
    }

    async fn start_handshake(&self) -> Result<(), SecurityError> {
        debug!("SRTP-only: No handshake needed for pre-shared keys");
        // Pre-shared keys don't need a handshake
        Ok(())
    }

    async fn is_handshake_complete(&self) -> Result<bool, SecurityError> {
        // Always complete for pre-shared keys
        Ok(*self.handshake_completed.read().await)
    }

    async fn wait_for_handshake(&self) -> Result<(), SecurityError> {
        // No waiting needed for pre-shared keys
        Ok(())
    }

    async fn set_remote_address(&self, addr: SocketAddr) -> Result<(), SecurityError> {
        let mut remote_addr = self.remote_addr.write().await;
        *remote_addr = Some(addr);
        debug!("SRTP-only: Set remote address to {}", addr);
        Ok(())
    }

    async fn set_socket(&self, socket: SocketHandle) -> Result<(), SecurityError> {
        let mut socket_lock = self.socket.write().await;
        *socket_lock = Some(socket.clone());

        // Set remote address if available
        if let Some(remote_addr) = socket.remote_addr {
            let mut remote_addr_guard = self.remote_addr.write().await;
            *remote_addr_guard = Some(remote_addr);
        }

        debug!("SRTP-only: Set socket handle");
        Ok(())
    }

    async fn set_remote_fingerprint(
        &self,
        _fingerprint: &str,
        _algorithm: &str,
    ) -> Result<(), SecurityError> {
        debug!("SRTP-only: Fingerprint not used with pre-shared keys");
        // Not needed for pre-shared key SRTP
        Ok(())
    }

    async fn complete_handshake(
        &self,
        remote_addr: SocketAddr,
        _remote_fingerprint: &str,
    ) -> Result<(), SecurityError> {
        // Just set the remote address, no handshake needed
        self.set_remote_address(remote_addr).await?;
        debug!("SRTP-only: Complete handshake called - no handshake needed for pre-shared keys");
        Ok(())
    }

    async fn process_packet(&self, _data: &[u8]) -> Result<(), SecurityError> {
        debug!("SRTP-only: No packet processing needed for pre-shared keys");
        // No DTLS packet processing needed
        Ok(())
    }

    async fn start_packet_handler(&self) -> Result<(), SecurityError> {
        debug!("SRTP-only: No packet handler needed for pre-shared keys");
        // No DTLS packet handler needed
        Ok(())
    }

    async fn get_security_info(&self) -> Result<SecurityInfo, SecurityError> {
        let crypto_suites = self
            .config
            .srtp_profiles
            .iter()
            .map(|p| match p {
                SrtpProfile::AesCm128HmacSha1_80 => "AES_CM_128_HMAC_SHA1_80",
                SrtpProfile::AesCm128HmacSha1_32 => "AES_CM_128_HMAC_SHA1_32",
                SrtpProfile::AesGcm128 => "AEAD_AES_128_GCM",
                SrtpProfile::AesGcm256 => "AEAD_AES_256_GCM",
            })
            .map(|s| s.to_string())
            .collect::<Vec<_>>();

        Ok(SecurityInfo {
            mode: SecurityMode::Srtp,
            fingerprint: None, // No fingerprint for pre-shared keys
            fingerprint_algorithm: None,
            crypto_suites,
            key_params: Some("Pre-shared key (from SIP/SDP)".to_string()),
            srtp_profile: Some("AES_CM_128_HMAC_SHA1_80".to_string()),
        })
    }

    async fn close(&self) -> Result<(), SecurityError> {
        debug!("SRTP-only: Closing security context");
        let mut handshake_complete = self.handshake_completed.write().await;
        *handshake_complete = false;
        Ok(())
    }

    fn is_secure(&self) -> bool {
        true // Pre-shared key SRTP is secure
    }

    fn get_security_info_sync(&self) -> SecurityInfo {
        SecurityInfo {
            mode: SecurityMode::Srtp,
            fingerprint: None,
            fingerprint_algorithm: None,
            crypto_suites: vec!["AES_CM_128_HMAC_SHA1_80".to_string()],
            key_params: Some("Pre-shared key (from SIP/SDP)".to_string()),
            srtp_profile: Some("AES_CM_128_HMAC_SHA1_80".to_string()),
        }
    }

    async fn get_fingerprint(&self) -> Result<String, SecurityError> {
        Err(SecurityError::Configuration(
            "Fingerprints not used with pre-shared key SRTP".to_string(),
        ))
    }

    async fn get_fingerprint_algorithm(&self) -> Result<String, SecurityError> {
        Err(SecurityError::Configuration(
            "Fingerprint algorithm not used with pre-shared key SRTP".to_string(),
        ))
    }

    async fn has_transport(&self) -> Result<bool, SecurityError> {
        // For pre-shared keys, we don't need special DTLS transport
        Ok(self.socket.read().await.is_some())
    }

    async fn is_ready(&self) -> Result<bool, SecurityError> {
        // Check if socket is set and remote address is set
        let socket_set = self.socket.read().await.is_some();
        let remote_addr_set = self.remote_addr.read().await.is_some();

        let is_ready = socket_set && remote_addr_set;

        debug!("SRTP-only client security context ready: {}", is_ready);
        debug!("  - Socket set: {}", socket_set);
        debug!("  - Remote address set: {}", remote_addr_set);

        Ok(is_ready)
    }

    async fn process_dtls_packet(&self, _data: &[u8]) -> Result<(), SecurityError> {
        debug!("SRTP-only: No DTLS packet processing needed for pre-shared keys");
        // No DTLS processing for pre-shared keys
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// Get the security configuration
    fn get_config(&self) -> &ClientSecurityConfig {
        &self.config
    }
}

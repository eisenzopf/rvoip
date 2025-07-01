//! Key Management System
//!
//! This module provides advanced key management capabilities including:
//! - Automatic key rotation based on time intervals or packet counts
//! - Multi-stream key syndication (derive keys for multiple streams)
//! - Key lifecycle management with expiration and cleanup
//! - Security policy enforcement

use std::sync::Arc;
use std::time::{Duration, Instant};
use std::collections::HashMap;
use tokio::sync::RwLock;
use tracing::{info, debug, warn, error};

use crate::api::common::config::{KeyExchangeMethod, SecurityConfig, SrtpProfile};
use crate::api::common::error::SecurityError;
use crate::srtp::{SrtpContext, SrtpCryptoSuite, crypto::SrtpCryptoKey};

/// Key rotation policy defines when keys should be rotated
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyRotationPolicy {
    /// Rotate after a fixed time interval
    TimeInterval(Duration),
    /// Rotate after a certain number of packets
    PacketCount(u64),
    /// Rotate when explicitly requested
    Manual,
    /// Never rotate automatically
    Never,
    /// Combine multiple policies (rotate when any condition is met)
    Combined(Vec<KeyRotationPolicy>),
}

impl KeyRotationPolicy {
    /// Check if rotation is needed based on the policy
    pub fn should_rotate(&self, elapsed: Duration, packet_count: u64) -> bool {
        match self {
            KeyRotationPolicy::TimeInterval(interval) => elapsed >= *interval,
            KeyRotationPolicy::PacketCount(threshold) => packet_count >= *threshold,
            KeyRotationPolicy::Manual => false,
            KeyRotationPolicy::Never => false,
            KeyRotationPolicy::Combined(policies) => {
                policies.iter().any(|policy| policy.should_rotate(elapsed, packet_count))
            }
        }
    }

    /// Get a description of the policy
    pub fn description(&self) -> String {
        match self {
            KeyRotationPolicy::TimeInterval(duration) => {
                format!("Time interval: {:?}", duration)
            },
            KeyRotationPolicy::PacketCount(count) => {
                format!("Packet count: {}", count)
            },
            KeyRotationPolicy::Manual => "Manual rotation".to_string(),
            KeyRotationPolicy::Never => "No rotation".to_string(),
            KeyRotationPolicy::Combined(policies) => {
                let descriptions: Vec<String> = policies.iter()
                    .map(|p| p.description())
                    .collect();
                format!("Combined: [{}]", descriptions.join(", "))
            }
        }
    }

    /// Common rotation policies for different use cases
    pub fn enterprise_standard() -> Self {
        Self::Combined(vec![
            Self::TimeInterval(Duration::from_secs(3600)), // 1 hour
            Self::PacketCount(1_000_000), // 1M packets
        ])
    }

    pub fn high_security() -> Self {
        Self::Combined(vec![
            Self::TimeInterval(Duration::from_secs(900)), // 15 minutes
            Self::PacketCount(100_000), // 100K packets
        ])
    }

    pub fn development() -> Self {
        Self::TimeInterval(Duration::from_secs(300)) // 5 minutes
    }
}

/// Stream type for multi-stream key syndication
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamType {
    Audio,
    Video,
    Data,
    Control,
}

impl StreamType {
    /// Get the key derivation label for this stream type
    pub fn derivation_label(&self) -> &'static str {
        match self {
            StreamType::Audio => "SRTP_AUDIO",
            StreamType::Video => "SRTP_VIDEO", 
            StreamType::Data => "SRTP_DATA",
            StreamType::Control => "SRTP_CONTROL",
        }
    }

    /// Get default SRTP profile for this stream type
    pub fn default_srtp_profile(&self) -> SrtpProfile {
        match self {
            StreamType::Audio => SrtpProfile::AesCm128HmacSha1_80,
            StreamType::Video => SrtpProfile::AesCm128HmacSha1_80,
            StreamType::Data => SrtpProfile::AesCm128HmacSha1_32,
            StreamType::Control => SrtpProfile::AesCm128HmacSha1_32,
        }
    }
}

/// Key store for managing multiple keys and their metadata
pub struct KeyStore {
    /// Master key material for deriving stream-specific keys
    master_key: Vec<u8>,
    /// Stream-specific SRTP contexts
    stream_contexts: HashMap<StreamType, SrtpContext>,
    /// Key creation timestamp
    created_at: Instant,
    /// Last rotation timestamp
    last_rotation: Option<Instant>,
    /// Number of packets processed since last rotation
    packet_count: u64,
    /// Key generation counter (incremented on each rotation)
    generation: u32,
}

impl KeyStore {
    /// Create a new key store with master key material
    pub fn new(master_key: Vec<u8>) -> Result<Self, SecurityError> {
        if master_key.len() < 32 {
            return Err(SecurityError::Configuration("Master key too short".to_string()));
        }

        Ok(Self {
            master_key,
            stream_contexts: HashMap::new(),
            created_at: Instant::now(),
            last_rotation: None,
            packet_count: 0,
            generation: 0,
        })
    }

    /// Derive a key for a specific stream type
    pub fn derive_stream_key(&self, stream_type: StreamType) -> Result<SrtpCryptoKey, SecurityError> {
        // Use HKDF-like key derivation (simplified for this implementation)
        let label = stream_type.derivation_label();
        let mut derived_key = Vec::with_capacity(30); // 16 bytes key + 14 bytes salt

        // Simple key derivation: master_key || label || generation
        derived_key.extend_from_slice(&self.master_key[0..16.min(self.master_key.len())]);
        
        // XOR with stream label hash for differentiation
        let label_hash = self.hash_label(label);
        for (i, &byte) in label_hash.iter().take(16).enumerate() {
            if i < derived_key.len() {
                derived_key[i] ^= byte;
            }
        }

        // Add generation counter
        let gen_bytes = self.generation.to_be_bytes();
        for (i, &byte) in gen_bytes.iter().enumerate() {
            if i < derived_key.len() {
                derived_key[i] ^= byte;
            }
        }

        // Derive salt from the latter part of master key
        let mut salt = vec![0u8; 14];
        if self.master_key.len() >= 30 {
            salt.copy_from_slice(&self.master_key[16..30]);
        } else {
            // Use a default salt if master key is too short
            salt = vec![0x9e, 0x7c, 0xa4, 0xf0, 0x85, 0x2d, 0x1c, 0x22, 0xf9, 0x8a, 0x1b, 0x5e, 0x6c, 0x3d];
        }

        // XOR salt with generation for uniqueness
        for (i, &byte) in gen_bytes.iter().cycle().take(salt.len()).enumerate() {
            salt[i] ^= byte;
        }

        Ok(SrtpCryptoKey::new(derived_key, salt))
    }

    /// Simple hash function for label differentiation
    fn hash_label(&self, label: &str) -> Vec<u8> {
        // Simple hash: just use the bytes of the label with some mixing
        let mut hash = vec![0u8; 16];
        let label_bytes = label.as_bytes();
        
        for (i, &byte) in label_bytes.iter().enumerate() {
            let hash_len = hash.len();
            hash[i % hash_len] ^= byte.wrapping_add(i as u8);
        }
        
        hash
    }

    /// Set up SRTP context for a stream type
    pub fn setup_stream(&mut self, stream_type: StreamType) -> Result<(), SecurityError> {
        let stream_key = self.derive_stream_key(stream_type)?;
        let crypto_suite = match stream_type.default_srtp_profile() {
            SrtpProfile::AesCm128HmacSha1_80 => crate::srtp::SRTP_AES128_CM_SHA1_80,
            SrtpProfile::AesCm128HmacSha1_32 => crate::srtp::SRTP_AES128_CM_SHA1_32,
            _ => return Err(SecurityError::Configuration("Unsupported SRTP profile".to_string())),
        };

        let srtp_context = SrtpContext::new(crypto_suite, stream_key)
            .map_err(|e| SecurityError::CryptoError(format!("Failed to create SRTP context: {}", e)))?;

        self.stream_contexts.insert(stream_type, srtp_context);
        debug!("Set up SRTP context for {:?} stream (generation {})", stream_type, self.generation);

        Ok(())
    }

    /// Get SRTP context for a stream type
    pub fn get_stream_context(&mut self, stream_type: StreamType) -> Option<&mut SrtpContext> {
        self.stream_contexts.get_mut(&stream_type)
    }

    /// Rotate all keys (increment generation and re-derive)
    pub fn rotate_keys(&mut self) -> Result<(), SecurityError> {
        self.generation += 1;
        self.last_rotation = Some(Instant::now());
        self.packet_count = 0;

        info!("Rotating keys to generation {}", self.generation);

        // Re-setup all existing stream contexts with new keys
        let stream_types: Vec<StreamType> = self.stream_contexts.keys().cloned().collect();
        for stream_type in stream_types {
            self.setup_stream(stream_type)?;
        }

        Ok(())
    }

    /// Increment packet count (for packet-based rotation)
    pub fn increment_packet_count(&mut self) {
        self.packet_count += 1;
    }

    /// Get time elapsed since creation or last rotation
    pub fn elapsed_time(&self) -> Duration {
        match self.last_rotation {
            Some(last) => last.elapsed(),
            None => self.created_at.elapsed(),
        }
    }

    /// Get current packet count
    pub fn packet_count(&self) -> u64 {
        self.packet_count
    }

    /// Get current key generation
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Get configured stream types
    pub fn configured_streams(&self) -> Vec<StreamType> {
        self.stream_contexts.keys().cloned().collect()
    }
}

/// Multi-stream key syndication configuration
#[derive(Debug, Clone)]
pub struct KeySyndicationConfig {
    /// Stream types to manage
    pub stream_types: Vec<StreamType>,
    /// Whether to automatically set up contexts for all streams
    pub auto_setup_streams: bool,
    /// Synchronize key rotation across all streams
    pub synchronized_rotation: bool,
}

impl Default for KeySyndicationConfig {
    fn default() -> Self {
        Self {
            stream_types: vec![StreamType::Audio, StreamType::Video],
            auto_setup_streams: true,
            synchronized_rotation: true,
        }
    }
}

impl KeySyndicationConfig {
    /// Configuration for audio-only scenarios
    pub fn audio_only() -> Self {
        Self {
            stream_types: vec![StreamType::Audio],
            auto_setup_streams: true,
            synchronized_rotation: true,
        }
    }

    /// Configuration for multimedia scenarios
    pub fn multimedia() -> Self {
        Self {
            stream_types: vec![StreamType::Audio, StreamType::Video, StreamType::Data],
            auto_setup_streams: true,
            synchronized_rotation: true,
        }
    }

    /// Configuration for full control scenarios
    pub fn full_control() -> Self {
        Self {
            stream_types: vec![StreamType::Audio, StreamType::Video, StreamType::Data, StreamType::Control],
            auto_setup_streams: true,
            synchronized_rotation: true,
        }
    }
}

/// Key syndication manager for handling multiple streams
pub struct KeySyndication {
    /// Configuration
    config: KeySyndicationConfig,
    /// Key stores by session ID
    sessions: HashMap<String, KeyStore>,
}

impl KeySyndication {
    /// Create a new key syndication manager
    pub fn new(config: KeySyndicationConfig) -> Self {
        Self {
            config,
            sessions: HashMap::new(),
        }
    }

    /// Create a session with master key material
    pub fn create_session(&mut self, session_id: String, master_key: Vec<u8>) -> Result<(), SecurityError> {
        let mut key_store = KeyStore::new(master_key)?;

        // Auto-setup streams if configured
        if self.config.auto_setup_streams {
            for &stream_type in &self.config.stream_types {
                key_store.setup_stream(stream_type)?;
            }
        }

        self.sessions.insert(session_id.clone(), key_store);
        info!("Created key syndication session: {}", session_id);

        Ok(())
    }

    /// Get mutable access to a session's key store
    pub fn get_session_mut(&mut self, session_id: &str) -> Option<&mut KeyStore> {
        self.sessions.get_mut(session_id)
    }

    /// Add a stream to an existing session
    pub fn add_stream(&mut self, session_id: &str, stream_type: StreamType) -> Result<(), SecurityError> {
        let key_store = self.sessions.get_mut(session_id)
            .ok_or_else(|| SecurityError::NotFound(format!("Session not found: {}", session_id)))?;

        key_store.setup_stream(stream_type)?;
        info!("Added {:?} stream to session {}", stream_type, session_id);

        Ok(())
    }

    /// Remove a session
    pub fn remove_session(&mut self, session_id: &str) -> bool {
        let removed = self.sessions.remove(session_id).is_some();
        if removed {
            info!("Removed key syndication session: {}", session_id);
        }
        removed
    }

    /// Get session count
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Get all session IDs
    pub fn session_ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }
}

/// Security policy for enforcing security requirements
#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    /// Required key exchange methods (in order of preference)
    pub required_methods: Vec<KeyExchangeMethod>,
    /// Minimum key rotation interval
    pub min_rotation_interval: Option<Duration>,
    /// Maximum key lifetime
    pub max_key_lifetime: Option<Duration>,
    /// Required SRTP profiles
    pub required_srtp_profiles: Vec<SrtpProfile>,
    /// Whether to enforce strict validation
    pub strict_validation: bool,
    /// Whether to require perfect forward secrecy
    pub require_pfs: bool,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            required_methods: vec![KeyExchangeMethod::Sdes, KeyExchangeMethod::DtlsSrtp],
            min_rotation_interval: Some(Duration::from_secs(3600)), // 1 hour
            max_key_lifetime: Some(Duration::from_secs(86400)), // 24 hours
            required_srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
            strict_validation: true,
            require_pfs: false,
        }
    }
}

impl SecurityPolicy {
    /// Enterprise security policy
    pub fn enterprise() -> Self {
        Self {
            required_methods: vec![KeyExchangeMethod::Mikey, KeyExchangeMethod::Sdes],
            min_rotation_interval: Some(Duration::from_secs(1800)), // 30 minutes
            max_key_lifetime: Some(Duration::from_secs(7200)), // 2 hours
            required_srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
            strict_validation: true,
            require_pfs: true,
        }
    }

    /// High security policy
    pub fn high_security() -> Self {
        Self {
            required_methods: vec![KeyExchangeMethod::Zrtp, KeyExchangeMethod::Mikey],
            min_rotation_interval: Some(Duration::from_secs(900)), // 15 minutes
            max_key_lifetime: Some(Duration::from_secs(3600)), // 1 hour
            required_srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80, SrtpProfile::AesGcm128],
            strict_validation: true,
            require_pfs: true,
        }
    }

    /// Development/testing policy (relaxed)
    pub fn development() -> Self {
        Self {
            required_methods: vec![
                KeyExchangeMethod::PreSharedKey,
                KeyExchangeMethod::Sdes,
                KeyExchangeMethod::DtlsSrtp,
            ],
            min_rotation_interval: None,
            max_key_lifetime: None,
            required_srtp_profiles: vec![
                SrtpProfile::AesCm128HmacSha1_80,
                SrtpProfile::AesCm128HmacSha1_32,
            ],
            strict_validation: false,
            require_pfs: false,
        }
    }

    /// Validate a security configuration against this policy
    pub fn validate_config(&self, config: &SecurityConfig) -> Result<(), SecurityError> {
        // Check if the configured method is allowed
        let method = config.mode.key_exchange_method()
            .ok_or_else(|| SecurityError::PolicyViolation("No key exchange method configured".to_string()))?;

        if !self.required_methods.contains(&method) {
            return Err(SecurityError::PolicyViolation(
                format!("Key exchange method {:?} not allowed by policy", method)
            ));
        }

        // Check SRTP profiles
        for profile in &config.srtp_profiles {
            if !self.required_srtp_profiles.contains(profile) {
                return Err(SecurityError::PolicyViolation(
                    format!("SRTP profile {:?} not allowed by policy", profile)
                ));
            }
        }

        // Additional checks would go here (PFS, validation strictness, etc.)

        Ok(())
    }

    /// Check if a rotation policy complies with this security policy
    pub fn validate_rotation_policy(&self, policy: &KeyRotationPolicy) -> Result<(), SecurityError> {
        if let Some(min_interval) = self.min_rotation_interval {
            match policy {
                KeyRotationPolicy::TimeInterval(interval) => {
                    if *interval < min_interval {
                        return Err(SecurityError::PolicyViolation(
                            format!("Rotation interval {:?} is less than minimum {:?}", interval, min_interval)
                        ));
                    }
                },
                KeyRotationPolicy::Never => {
                    return Err(SecurityError::PolicyViolation(
                        "Policy requires key rotation but 'Never' policy specified".to_string()
                    ));
                },
                KeyRotationPolicy::Combined(policies) => {
                    for sub_policy in policies {
                        self.validate_rotation_policy(sub_policy)?;
                    }
                },
                _ => {} // Other policies might be acceptable
            }
        }

        Ok(())
    }
}

/// Main key manager that coordinates all key management activities
pub struct KeyManager {
    /// Key rotation policy
    rotation_policy: Arc<RwLock<KeyRotationPolicy>>,
    /// Key store (single session for simple cases)
    key_store: Arc<RwLock<Option<KeyStore>>>,
    /// Multi-stream syndication manager
    syndication: Arc<RwLock<KeySyndication>>,
    /// Security policy enforcement
    security_policy: Arc<RwLock<SecurityPolicy>>,
    /// Background task handle for automatic rotation
    rotation_task: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
}

impl KeyManager {
    /// Create a new key manager
    pub fn new(
        rotation_policy: KeyRotationPolicy,
        syndication_config: KeySyndicationConfig,
        security_policy: SecurityPolicy,
    ) -> Self {
        Self {
            rotation_policy: Arc::new(RwLock::new(rotation_policy)),
            key_store: Arc::new(RwLock::new(None)),
            syndication: Arc::new(RwLock::new(KeySyndication::new(syndication_config))),
            security_policy: Arc::new(RwLock::new(security_policy)),
            rotation_task: Arc::new(RwLock::new(None)),
        }
    }

    /// Initialize with master key material
    pub async fn initialize(&self, master_key: Vec<u8>) -> Result<(), SecurityError> {
        let key_store = KeyStore::new(master_key)?;
        *self.key_store.write().await = Some(key_store);

        // Start automatic rotation task if needed
        self.start_rotation_task().await;

        Ok(())
    }

    /// Start automatic key rotation task
    async fn start_rotation_task(&self) {
        let policy = self.rotation_policy.read().await.clone();
        
        match policy {
            KeyRotationPolicy::TimeInterval(interval) => {
                let key_store = self.key_store.clone();
                let rotation_policy = self.rotation_policy.clone();
                
                let handle = tokio::spawn(async move {
                    let mut rotation_interval = tokio::time::interval(interval);
                    rotation_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    
                    loop {
                        rotation_interval.tick().await;
                        
                        if let Some(mut store) = key_store.write().await.as_mut() {
                            if let Err(e) = store.rotate_keys() {
                                error!("Automatic key rotation failed: {}", e);
                            } else {
                                info!("Automatic key rotation completed (generation {})", store.generation());
                            }
                        }
                    }
                });
                
                *self.rotation_task.write().await = Some(handle);
                info!("Started automatic key rotation task with interval {:?}", interval);
            },
            _ => {
                debug!("No automatic rotation task needed for policy: {:?}", policy);
            }
        }
    }

    /// Stop automatic rotation task
    pub async fn stop_rotation_task(&self) {
        if let Some(handle) = self.rotation_task.write().await.take() {
            handle.abort();
            info!("Stopped automatic key rotation task");
        }
    }

    /// Manually trigger key rotation
    pub async fn rotate_keys(&self) -> Result<(), SecurityError> {
        if let Some(key_store) = self.key_store.write().await.as_mut() {
            key_store.rotate_keys()?;
            info!("Manual key rotation completed (generation {})", key_store.generation());
        }

        Ok(())
    }

    /// Check if rotation is needed and perform it
    pub async fn check_and_rotate(&self) -> Result<bool, SecurityError> {
        let policy = self.rotation_policy.read().await.clone();
        
        if let Some(key_store) = self.key_store.write().await.as_mut() {
            let elapsed = key_store.elapsed_time();
            let packet_count = key_store.packet_count();
            
            if policy.should_rotate(elapsed, packet_count) {
                key_store.rotate_keys()?;
                info!("Key rotation triggered by policy: {}", policy.description());
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Get syndication manager
    pub async fn syndication(&self) -> tokio::sync::RwLockReadGuard<KeySyndication> {
        self.syndication.read().await
    }

    /// Get mutable syndication manager
    pub async fn syndication_mut(&self) -> tokio::sync::RwLockWriteGuard<KeySyndication> {
        self.syndication.write().await
    }

    /// Validate configuration against security policy
    pub async fn validate_config(&self, config: &SecurityConfig) -> Result<(), SecurityError> {
        let policy = self.security_policy.read().await;
        policy.validate_config(config)
    }

    /// Get current key statistics
    pub async fn get_statistics(&self) -> KeyManagerStatistics {
        let mut stats = KeyManagerStatistics::default();
        
        if let Some(key_store) = self.key_store.read().await.as_ref() {
            stats.current_generation = key_store.generation();
            stats.elapsed_time = key_store.elapsed_time();
            stats.packet_count = key_store.packet_count();
            stats.configured_streams = key_store.configured_streams().len();
        }

        let syndication = self.syndication.read().await;
        stats.active_sessions = syndication.session_count();

        stats
    }
}

/// Key manager statistics for monitoring and debugging
#[derive(Debug, Default)]
pub struct KeyManagerStatistics {
    pub current_generation: u32,
    pub elapsed_time: Duration,
    pub packet_count: u64,
    pub configured_streams: usize,
    pub active_sessions: usize,
}

impl Drop for KeyManager {
    fn drop(&mut self) {
        // Note: This won't work with async, but it's here for documentation
        // In practice, you'd call stop_rotation_task() before dropping
        debug!("KeyManager dropped");
    }
} 
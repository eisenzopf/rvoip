//! Error Recovery and Fallback System
//!
//! This module provides robust error recovery and fallback mechanisms for security contexts.
//! It handles various failure scenarios and provides automatic fallback to alternative
//! security methods when the primary method fails.

use std::sync::Arc;
use std::time::{Duration, Instant};
use std::collections::VecDeque;
use tokio::sync::RwLock;
use tracing::{info, debug, warn, error};

use crate::api::common::config::{KeyExchangeMethod, SecurityConfig};
use crate::api::common::error::SecurityError;
use crate::api::common::unified_security::{UnifiedSecurityContext, SecurityState};

/// Recovery strategy defines how to handle security failures
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryStrategy {
    /// Immediately failover to the next available method
    ImmediateFallback,
    /// Retry the current method with exponential backoff before falling back
    RetryWithBackoff {
        max_retries: u32,
        initial_delay: Duration,
        max_delay: Duration,
    },
    /// Wait for manual intervention (don't auto-recover)
    Manual,
    /// Fail completely (no recovery attempt)
    Fail,
}

impl RecoveryStrategy {
    /// Enterprise-grade recovery strategy
    pub fn enterprise() -> Self {
        Self::RetryWithBackoff {
            max_retries: 3,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
        }
    }

    /// High-availability recovery strategy
    pub fn high_availability() -> Self {
        Self::RetryWithBackoff {
            max_retries: 5,
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(10),
        }
    }

    /// Development/testing strategy (immediate fallback)
    pub fn development() -> Self {
        Self::ImmediateFallback
    }

    /// Get a description of the strategy
    pub fn description(&self) -> String {
        match self {
            Self::ImmediateFallback => "Immediate fallback".to_string(),
            Self::RetryWithBackoff { max_retries, initial_delay, max_delay } => {
                format!("Retry with backoff: {} retries, {:.1}s-{:.1}s delay", 
                       max_retries, initial_delay.as_secs_f64(), max_delay.as_secs_f64())
            },
            Self::Manual => "Manual intervention required".to_string(),
            Self::Fail => "No recovery".to_string(),
        }
    }
}

/// Types of security failures that can be recovered from
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureType {
    /// Key exchange initialization failed
    InitializationFailure,
    /// Network connectivity issues
    NetworkFailure,
    /// Cryptographic validation failed
    CryptoFailure,
    /// Timeout during key exchange
    TimeoutFailure,
    /// Authentication failed
    AuthenticationFailure,
    /// Configuration error
    ConfigurationFailure,
    /// Unknown/unclassified failure
    UnknownFailure,
}

impl FailureType {
    /// Check if this failure type is recoverable
    pub fn is_recoverable(&self) -> bool {
        match self {
            Self::InitializationFailure => true,
            Self::NetworkFailure => true,
            Self::CryptoFailure => false, // Usually indicates a more serious issue
            Self::TimeoutFailure => true,
            Self::AuthenticationFailure => false, // Usually indicates credential issues
            Self::ConfigurationFailure => false, // Needs manual intervention
            Self::UnknownFailure => true, // Err on the side of trying recovery
        }
    }

    /// Get the severity level of this failure
    pub fn severity(&self) -> FailureSeverity {
        match self {
            Self::InitializationFailure => FailureSeverity::Medium,
            Self::NetworkFailure => FailureSeverity::Low,
            Self::CryptoFailure => FailureSeverity::High,
            Self::TimeoutFailure => FailureSeverity::Low,
            Self::AuthenticationFailure => FailureSeverity::High,
            Self::ConfigurationFailure => FailureSeverity::Medium,
            Self::UnknownFailure => FailureSeverity::Medium,
        }
    }

    /// Classify an error into a failure type
    pub fn from_error(error: &SecurityError) -> Self {
        match error {
            SecurityError::NotInitialized(_) => Self::InitializationFailure,
            SecurityError::CryptoError(_) => Self::CryptoFailure,
            SecurityError::InvalidState(_) => Self::ConfigurationFailure,
            SecurityError::Configuration(_) => Self::ConfigurationFailure,
            SecurityError::NotFound(_) => Self::ConfigurationFailure,
            SecurityError::PolicyViolation(_) => Self::ConfigurationFailure,
            SecurityError::Timeout(_) => Self::TimeoutFailure,
            SecurityError::Network(_) => Self::NetworkFailure,
            SecurityError::Authentication(_) => Self::AuthenticationFailure,
            // Handle DTLS-specific errors
            SecurityError::Handshake(_) => Self::CryptoFailure,
            SecurityError::HandshakeVerification(_) => Self::CryptoFailure,
            SecurityError::HandshakeError(_) => Self::CryptoFailure,
            SecurityError::Internal(_) => Self::UnknownFailure,
        }
    }
}

/// Severity level of failures
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FailureSeverity {
    Low,
    Medium,
    High,
    Critical,
}

/// Fallback configuration for method prioritization
#[derive(Debug, Clone)]
pub struct FallbackConfig {
    /// Ordered list of methods to try (primary first)
    pub method_priority: Vec<KeyExchangeMethod>,
    /// Recovery strategy per method
    pub recovery_strategies: std::collections::HashMap<KeyExchangeMethod, RecoveryStrategy>,
    /// Maximum number of total fallback attempts
    pub max_fallback_attempts: u32,
    /// Cooldown period before retrying a failed method
    pub method_cooldown: Duration,
    /// Whether to automatically fall back on any failure
    pub auto_fallback: bool,
}

impl Default for FallbackConfig {
    fn default() -> Self {
        let mut recovery_strategies = std::collections::HashMap::new();
        recovery_strategies.insert(KeyExchangeMethod::Sdes, RecoveryStrategy::enterprise());
        recovery_strategies.insert(KeyExchangeMethod::DtlsSrtp, RecoveryStrategy::enterprise());
        recovery_strategies.insert(KeyExchangeMethod::PreSharedKey, RecoveryStrategy::ImmediateFallback);

        Self {
            method_priority: vec![
                KeyExchangeMethod::Sdes,
                KeyExchangeMethod::DtlsSrtp,
                KeyExchangeMethod::PreSharedKey,
            ],
            recovery_strategies,
            max_fallback_attempts: 3,
            method_cooldown: Duration::from_secs(60),
            auto_fallback: true,
        }
    }
}

impl FallbackConfig {
    /// Configuration for enterprise environments
    pub fn enterprise() -> Self {
        let mut recovery_strategies = std::collections::HashMap::new();
        recovery_strategies.insert(KeyExchangeMethod::Mikey, RecoveryStrategy::enterprise());
        recovery_strategies.insert(KeyExchangeMethod::Sdes, RecoveryStrategy::enterprise());
        recovery_strategies.insert(KeyExchangeMethod::DtlsSrtp, RecoveryStrategy::high_availability());

        Self {
            method_priority: vec![
                KeyExchangeMethod::Mikey,
                KeyExchangeMethod::Sdes,
                KeyExchangeMethod::DtlsSrtp,
            ],
            recovery_strategies,
            max_fallback_attempts: 5,
            method_cooldown: Duration::from_secs(30),
            auto_fallback: true,
        }
    }

    /// Configuration for peer-to-peer scenarios
    pub fn peer_to_peer() -> Self {
        let mut recovery_strategies = std::collections::HashMap::new();
        recovery_strategies.insert(KeyExchangeMethod::Zrtp, RecoveryStrategy::high_availability());
        recovery_strategies.insert(KeyExchangeMethod::Sdes, RecoveryStrategy::enterprise());
        recovery_strategies.insert(KeyExchangeMethod::PreSharedKey, RecoveryStrategy::ImmediateFallback);

        Self {
            method_priority: vec![
                KeyExchangeMethod::Zrtp,
                KeyExchangeMethod::Sdes,
                KeyExchangeMethod::PreSharedKey,
            ],
            recovery_strategies,
            max_fallback_attempts: 3,
            method_cooldown: Duration::from_secs(45),
            auto_fallback: true,
        }
    }

    /// Configuration for development/testing
    pub fn development() -> Self {
        let mut recovery_strategies = std::collections::HashMap::new();
        recovery_strategies.insert(KeyExchangeMethod::PreSharedKey, RecoveryStrategy::development());
        recovery_strategies.insert(KeyExchangeMethod::Sdes, RecoveryStrategy::development());

        Self {
            method_priority: vec![
                KeyExchangeMethod::PreSharedKey,
                KeyExchangeMethod::Sdes,
            ],
            recovery_strategies,
            max_fallback_attempts: 2,
            method_cooldown: Duration::from_secs(5),
            auto_fallback: true,
        }
    }
}

/// Record of a failure event for analysis and monitoring
#[derive(Debug, Clone)]
pub struct FailureRecord {
    pub timestamp: Instant,
    pub method: KeyExchangeMethod,
    pub failure_type: FailureType,
    pub error_message: String,
    pub retry_attempt: u32,
    pub recovery_action: RecoveryAction,
}

/// Actions taken during recovery
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Retrying the same method
    Retry,
    /// Falling back to a different method
    Fallback(KeyExchangeMethod),
    /// Manual intervention required
    ManualIntervention,
    /// Recovery abandoned
    Abandoned,
}

/// State of the error recovery system
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryState {
    /// Normal operation (no recovery in progress)
    Normal,
    /// Currently retrying a failed method
    Retrying,
    /// Currently falling back to alternative method
    FallingBack,
    /// Waiting for manual intervention
    WaitingForIntervention,
    /// All recovery attempts exhausted
    Exhausted,
}

impl Default for RecoveryState {
    fn default() -> Self {
        Self::Normal
    }
}

/// Error recovery manager that handles failures and orchestrates fallbacks
pub struct ErrorRecoveryManager {
    /// Fallback configuration
    config: FallbackConfig,
    /// Current recovery state
    state: Arc<RwLock<RecoveryState>>,
    /// History of failures for analysis
    failure_history: Arc<RwLock<VecDeque<FailureRecord>>>,
    /// Currently attempted methods and their cooldown expiry
    method_cooldowns: Arc<RwLock<std::collections::HashMap<KeyExchangeMethod, Instant>>>,
    /// Current fallback attempt count
    fallback_attempts: Arc<RwLock<u32>>,
    /// Active security context being managed
    security_context: Arc<RwLock<Option<UnifiedSecurityContext>>>,
}

impl ErrorRecoveryManager {
    /// Create a new error recovery manager
    pub fn new(config: FallbackConfig) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(RecoveryState::Normal)),
            failure_history: Arc::new(RwLock::new(VecDeque::new())),
            method_cooldowns: Arc::new(RwLock::new(std::collections::HashMap::new())),
            fallback_attempts: Arc::new(RwLock::new(0)),
            security_context: Arc::new(RwLock::new(None)),
        }
    }

    /// Set the security context to manage
    pub async fn set_security_context(&self, context: UnifiedSecurityContext) {
        *self.security_context.write().await = Some(context);
    }

    /// Handle a security failure and attempt recovery
    pub async fn handle_failure(
        &self,
        method: KeyExchangeMethod,
        error: SecurityError,
    ) -> Result<RecoveryAction, SecurityError> {
        let failure_type = FailureType::from_error(&error);
        
        info!("Handling security failure: {:?} method failed with {:?}", method, failure_type);
        debug!("Error details: {}", error);

        // Record the failure
        self.record_failure(method, failure_type, error.to_string(), 0).await;

        // Check if this failure type is recoverable
        if !failure_type.is_recoverable() {
            warn!("Failure type {:?} is not recoverable", failure_type);
            *self.state.write().await = RecoveryState::WaitingForIntervention;
            return Ok(RecoveryAction::ManualIntervention);
        }

        // Get recovery strategy for this method
        let strategy = self.config.recovery_strategies
            .get(&method)
            .cloned()
            .unwrap_or(RecoveryStrategy::ImmediateFallback);

        match strategy {
            RecoveryStrategy::ImmediateFallback => {
                self.attempt_fallback(method).await
            },
            RecoveryStrategy::RetryWithBackoff { max_retries, initial_delay, max_delay } => {
                self.attempt_retry_with_backoff(method, max_retries, initial_delay, max_delay).await
            },
            RecoveryStrategy::Manual => {
                *self.state.write().await = RecoveryState::WaitingForIntervention;
                Ok(RecoveryAction::ManualIntervention)
            },
            RecoveryStrategy::Fail => {
                *self.state.write().await = RecoveryState::Exhausted;
                Ok(RecoveryAction::Abandoned)
            },
        }
    }

    /// Attempt to retry the current method with backoff
    async fn attempt_retry_with_backoff(
        &self,
        method: KeyExchangeMethod,
        max_retries: u32,
        initial_delay: Duration,
        max_delay: Duration,
    ) -> Result<RecoveryAction, SecurityError> {
        *self.state.write().await = RecoveryState::Retrying;

        for retry_attempt in 1..=max_retries {
            // Calculate backoff delay (exponential with max cap)
            let delay = std::cmp::min(
                initial_delay * (2_u32.pow(retry_attempt - 1)),
                max_delay,
            );

            info!("Retrying {:?} method (attempt {}/{}) after {:?} delay", 
                  method, retry_attempt, max_retries, delay);

            // Wait for backoff delay
            tokio::time::sleep(delay).await;

            // Attempt to reinitialize the security context
            if let Some(context) = self.security_context.read().await.as_ref() {
                match context.initialize().await {
                    Ok(()) => {
                        info!("Retry successful for {:?} method", method);
                        *self.state.write().await = RecoveryState::Normal;
                        return Ok(RecoveryAction::Retry);
                    },
                    Err(e) => {
                        let failure_type = FailureType::from_error(&e);
                        self.record_failure(method, failure_type, e.to_string(), retry_attempt).await;
                        warn!("Retry {}/{} failed for {:?} method: {}", 
                              retry_attempt, max_retries, method, e);
                    }
                }
            }
        }

        // All retries exhausted, attempt fallback
        warn!("All retries exhausted for {:?} method, attempting fallback", method);
        self.attempt_fallback(method).await
    }

    /// Attempt to fallback to the next available method
    fn attempt_fallback(&self, failed_method: KeyExchangeMethod) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<RecoveryAction, SecurityError>> + Send + '_>> {
        Box::pin(async move {
            let mut fallback_attempts = self.fallback_attempts.write().await;
            
            if *fallback_attempts >= self.config.max_fallback_attempts {
                warn!("Maximum fallback attempts ({}) reached", self.config.max_fallback_attempts);
                *self.state.write().await = RecoveryState::Exhausted;
                return Ok(RecoveryAction::Abandoned);
            }

            *fallback_attempts += 1;
            *self.state.write().await = RecoveryState::FallingBack;

            // Find the next available method
            let next_method = self.find_next_available_method(failed_method).await;
            
            match next_method {
                Some(method) => {
                    info!("Attempting fallback to {:?} method (attempt {}/{})", 
                          method, *fallback_attempts, self.config.max_fallback_attempts);

                    // Set cooldown for the failed method
                    self.set_method_cooldown(failed_method).await;

                    // Attempt to create and initialize new context with fallback method
                    match self.create_fallback_context(method).await {
                        Ok(()) => {
                            info!("Successfully fell back to {:?} method", method);
                            *self.state.write().await = RecoveryState::Normal;
                            Ok(RecoveryAction::Fallback(method))
                        },
                        Err(e) => {
                            warn!("Fallback to {:?} method failed: {}", method, e);
                            // Try the next method recursively
                            self.attempt_fallback(method).await
                        }
                    }
                },
                None => {
                    warn!("No more methods available for fallback");
                    *self.state.write().await = RecoveryState::Exhausted;
                    Ok(RecoveryAction::Abandoned)
                }
            }
        })
    }

    /// Find the next available method in the priority list
    async fn find_next_available_method(&self, current_method: KeyExchangeMethod) -> Option<KeyExchangeMethod> {
        let cooldowns = self.method_cooldowns.read().await;
        let now = Instant::now();

        // Find current method in priority list
        let current_index = self.config.method_priority.iter()
            .position(|&m| m == current_method)?;

        // Look for next available method (not on cooldown)
        for &method in &self.config.method_priority[current_index + 1..] {
            if let Some(cooldown_expiry) = cooldowns.get(&method) {
                if now < *cooldown_expiry {
                    debug!("Method {:?} is on cooldown until {:?}", method, cooldown_expiry);
                    continue;
                }
            }
            return Some(method);
        }

        // If no method found after current, wrap around to beginning
        for &method in &self.config.method_priority[..current_index] {
            if let Some(cooldown_expiry) = cooldowns.get(&method) {
                if now < *cooldown_expiry {
                    continue;
                }
            }
            return Some(method);
        }

        None
    }

    /// Set cooldown for a method
    async fn set_method_cooldown(&self, method: KeyExchangeMethod) {
        let expiry = Instant::now() + self.config.method_cooldown;
        self.method_cooldowns.write().await.insert(method, expiry);
        debug!("Set cooldown for {:?} method until {:?}", method, expiry);
    }

    /// Create a new security context for fallback method
    async fn create_fallback_context(&self, method: KeyExchangeMethod) -> Result<(), SecurityError> {
        // This is a simplified implementation
        // In practice, you'd need to create appropriate SecurityConfig for the method
        let config = match method {
            KeyExchangeMethod::PreSharedKey => SecurityConfig::srtp_with_key(vec![0u8; 32]),
            KeyExchangeMethod::Sdes => SecurityConfig::sdes_srtp(),
            KeyExchangeMethod::DtlsSrtp => SecurityConfig::webrtc_compatible(),
            KeyExchangeMethod::Mikey => SecurityConfig::mikey_psk(),
            KeyExchangeMethod::Zrtp => SecurityConfig::zrtp_p2p(),
        };

        let new_context = UnifiedSecurityContext::new(config)?;
        new_context.initialize().await?;
        
        *self.security_context.write().await = Some(new_context);
        Ok(())
    }

    /// Record a failure for analysis
    async fn record_failure(
        &self,
        method: KeyExchangeMethod,
        failure_type: FailureType,
        error_message: String,
        retry_attempt: u32,
    ) {
        let record = FailureRecord {
            timestamp: Instant::now(),
            method,
            failure_type,
            error_message,
            retry_attempt,
            recovery_action: RecoveryAction::Retry, // Will be updated later
        };

        let mut history = self.failure_history.write().await;
        history.push_back(record);

        // Keep only the last 100 failure records
        while history.len() > 100 {
            history.pop_front();
        }
    }

    /// Get current recovery state
    pub async fn get_state(&self) -> RecoveryState {
        *self.state.read().await
    }

    /// Get failure statistics
    pub async fn get_failure_statistics(&self) -> FailureStatistics {
        let history = self.failure_history.read().await;
        let mut stats = FailureStatistics::default();
        
        for record in history.iter() {
            stats.total_failures += 1;
            
            let method_stats = stats.failures_by_method.entry(record.method)
                .or_insert(0);
            *method_stats += 1;
            
            let type_stats = stats.failures_by_type.entry(record.failure_type)
                .or_insert(0);
            *type_stats += 1;
        }

        if let Some(latest) = history.back() {
            stats.last_failure_time = Some(latest.timestamp);
        }

        stats.current_state = *self.state.read().await;
        stats.fallback_attempts = *self.fallback_attempts.read().await;

        stats
    }

    /// Reset recovery state (for manual intervention)
    pub async fn reset(&self) {
        *self.state.write().await = RecoveryState::Normal;
        *self.fallback_attempts.write().await = 0;
        self.method_cooldowns.write().await.clear();
        info!("Error recovery manager reset");
    }

    /// Check if recovery is possible
    pub async fn can_recover(&self) -> bool {
        let state = *self.state.read().await;
        matches!(state, RecoveryState::Normal | RecoveryState::Retrying | RecoveryState::FallingBack)
    }
}

/// Statistics about failures and recovery attempts
#[derive(Debug, Default)]
pub struct FailureStatistics {
    pub total_failures: u32,
    pub failures_by_method: std::collections::HashMap<KeyExchangeMethod, u32>,
    pub failures_by_type: std::collections::HashMap<FailureType, u32>,
    pub last_failure_time: Option<Instant>,
    pub current_state: RecoveryState,
    pub fallback_attempts: u32,
}

impl FailureStatistics {
    /// Get the most problematic method
    pub fn most_problematic_method(&self) -> Option<(KeyExchangeMethod, u32)> {
        self.failures_by_method.iter()
            .max_by_key(|(_, count)| *count)
            .map(|(&method, &count)| (method, count))
    }

    /// Get the most common failure type
    pub fn most_common_failure_type(&self) -> Option<(FailureType, u32)> {
        self.failures_by_type.iter()
            .max_by_key(|(_, count)| *count)
            .map(|(&failure_type, &count)| (failure_type, count))
    }

    /// Calculate failure rate for a specific method
    pub fn failure_rate_for_method(&self, method: KeyExchangeMethod) -> f64 {
        if self.total_failures == 0 {
            return 0.0;
        }
        
        let method_failures = self.failures_by_method.get(&method).unwrap_or(&0);
        (*method_failures as f64) / (self.total_failures as f64)
    }
} 
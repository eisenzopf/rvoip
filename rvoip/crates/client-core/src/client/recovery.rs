//! Error recovery and retry mechanisms for client operations
//! 
//! This module provides utilities for handling transient failures and
//! implementing recovery strategies for various error scenarios.

use crate::error::{ClientError, ClientResult};
use std::future::Future;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, warn, error};

/// Configuration for retry behavior
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_attempts: u32,
    /// Initial delay between retries
    pub initial_delay: Duration,
    /// Maximum delay between retries
    pub max_delay: Duration,
    /// Multiplier for exponential backoff
    pub backoff_multiplier: f64,
    /// Whether to add jitter to delays
    pub use_jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            use_jitter: true,
        }
    }
}

impl RetryConfig {
    /// Create a configuration for quick retries (e.g., network operations)
    pub fn quick() -> Self {
        Self {
            max_attempts: 5,
            initial_delay: Duration::from_millis(50),
            max_delay: Duration::from_secs(5),
            backoff_multiplier: 1.5,
            use_jitter: true,
        }
    }
    
    /// Create a configuration for slow retries (e.g., registration)
    pub fn slow() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            backoff_multiplier: 3.0,
            use_jitter: false,
        }
    }
}

/// Retry an operation with exponential backoff
pub async fn retry_with_backoff<T, F, Fut>(
    operation_name: &str,
    config: RetryConfig,
    mut operation: F,
) -> ClientResult<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ClientResult<T>>,
{
    let mut attempt = 0;
    let mut delay = config.initial_delay;
    
    loop {
        attempt += 1;
        debug!(
            operation = operation_name,
            attempt = attempt,
            max_attempts = config.max_attempts,
            "Attempting operation"
        );
        
        match operation().await {
            Ok(result) => {
                if attempt > 1 {
                    debug!(
                        operation = operation_name,
                        attempt = attempt,
                        "Operation succeeded after retries"
                    );
                }
                return Ok(result);
            }
            Err(e) if e.is_recoverable() && attempt < config.max_attempts => {
                warn!(
                    operation = operation_name,
                    attempt = attempt,
                    error = %e,
                    category = e.category(),
                    next_delay_ms = delay.as_millis(),
                    "Recoverable error, will retry"
                );
                
                // Apply jitter if configured
                let actual_delay = if config.use_jitter {
                    let jitter = (rand::random::<f64>() - 0.5) * 0.2; // ±10% jitter
                    let millis = delay.as_millis() as f64;
                    Duration::from_millis((millis * (1.0 + jitter)) as u64)
                } else {
                    delay
                };
                
                sleep(actual_delay).await;
                
                // Calculate next delay with exponential backoff
                let next_delay_ms = (delay.as_millis() as f64 * config.backoff_multiplier) as u64;
                delay = Duration::from_millis(next_delay_ms).min(config.max_delay);
            }
            Err(e) => {
                if attempt >= config.max_attempts {
                    error!(
                        operation = operation_name,
                        attempts = attempt,
                        error = %e,
                        "Operation failed after all retry attempts"
                    );
                } else {
                    error!(
                        operation = operation_name,
                        error = %e,
                        category = e.category(),
                        "Non-recoverable error, not retrying"
                    );
                }
                return Err(e);
            }
        }
    }
}

/// Recovery strategies for specific error scenarios
pub struct RecoveryStrategies;

impl RecoveryStrategies {
    /// Recover from network errors
    pub async fn recover_network_error(
        error: &ClientError,
        _context: &str,
    ) -> Option<RecoveryAction> {
        match error {
            ClientError::NetworkError { reason } => {
                if reason.contains("timeout") {
                    Some(RecoveryAction::RetryWithBackoff(RetryConfig::quick()))
                } else if reason.contains("connection refused") {
                    Some(RecoveryAction::WaitAndRetry(Duration::from_secs(5)))
                } else {
                    Some(RecoveryAction::CheckConnectivityAndRetry)
                }
            }
            ClientError::ConnectionTimeout => {
                Some(RecoveryAction::RetryWithBackoff(RetryConfig::slow()))
            }
            ClientError::ServerUnreachable { .. } => {
                Some(RecoveryAction::TryAlternateServer)
            }
            _ => None,
        }
    }
    
    /// Recover from registration errors
    pub async fn recover_registration_error(
        error: &ClientError,
        _context: &str,
    ) -> Option<RecoveryAction> {
        match error {
            ClientError::RegistrationFailed { reason } => {
                if reason.contains("401") || reason.contains("unauthorized") {
                    Some(RecoveryAction::UpdateCredentials)
                } else if reason.contains("timeout") {
                    Some(RecoveryAction::RetryWithBackoff(RetryConfig::slow()))
                } else if reason.contains("503") {
                    Some(RecoveryAction::WaitAndRetry(Duration::from_secs(30)))
                } else {
                    None
                }
            }
            ClientError::AuthenticationFailed { .. } => {
                Some(RecoveryAction::UpdateCredentials)
            }
            ClientError::RegistrationExpired => {
                Some(RecoveryAction::Reregister)
            }
            _ => None,
        }
    }
    
    /// Recover from media errors
    pub async fn recover_media_error(
        error: &ClientError,
        _context: &str,
    ) -> Option<RecoveryAction> {
        match error {
            ClientError::MediaNegotiationFailed { reason } => {
                if reason.contains("codec") {
                    Some(RecoveryAction::RenegotiateCodecs)
                } else if reason.contains("port") {
                    Some(RecoveryAction::ReallocatePorts)
                } else {
                    Some(RecoveryAction::RestartMediaSession)
                }
            }
            ClientError::NoCompatibleCodecs => {
                Some(RecoveryAction::UseDefaultCodec)
            }
            ClientError::AudioDeviceError { .. } => {
                Some(RecoveryAction::ReinitializeAudioDevice)
            }
            _ => None,
        }
    }
}

/// Actions that can be taken to recover from errors
#[derive(Debug, Clone)]
pub enum RecoveryAction {
    /// Retry with exponential backoff
    RetryWithBackoff(RetryConfig),
    /// Wait for a fixed duration then retry
    WaitAndRetry(Duration),
    /// Check network connectivity before retrying
    CheckConnectivityAndRetry,
    /// Try an alternate server
    TryAlternateServer,
    /// Update credentials and retry
    UpdateCredentials,
    /// Re-register with the server
    Reregister,
    /// Renegotiate media codecs
    RenegotiateCodecs,
    /// Reallocate media ports
    ReallocatePorts,
    /// Restart the media session
    RestartMediaSession,
    /// Use a default/fallback codec
    UseDefaultCodec,
    /// Reinitialize audio device
    ReinitializeAudioDevice,
}

/// Context-aware error wrapper that adds context using anyhow
pub trait ErrorContext<T> {
    /// Add context to the error
    fn context(self, context: &str) -> ClientResult<T>;
    
    /// Add context with lazy evaluation
    fn with_context<F>(self, f: F) -> ClientResult<T>
    where
        F: FnOnce() -> String;
}

impl<T> ErrorContext<T> for ClientResult<T> {
    fn context(self, context: &str) -> ClientResult<T> {
        self.map_err(|e| {
            error!(
                error = %e,
                context = context,
                category = e.category(),
                "Operation failed with context"
            );
            ClientError::InternalError {
                message: format!("{}: {}", context, e)
            }
        })
    }
    
    fn with_context<F>(self, f: F) -> ClientResult<T>
    where
        F: FnOnce() -> String,
    {
        self.map_err(|e| {
            let context = f();
            error!(
                error = %e,
                context = %context,
                category = e.category(),
                "Operation failed with context"
            );
            ClientError::InternalError {
                message: format!("{}: {}", context, e)
            }
        })
    }
}

/// Helper to add operation timeout with proper error context
pub async fn with_timeout<T, F>(
    operation_name: &str,
    timeout: Duration,
    future: F,
) -> ClientResult<T>
where
    F: Future<Output = ClientResult<T>>,
{
    match tokio::time::timeout(timeout, future).await {
        Ok(result) => result,
        Err(_) => {
            error!(
                operation = operation_name,
                timeout_ms = timeout.as_millis(),
                "Operation timed out"
            );
            Err(ClientError::OperationTimeout {
                duration_ms: timeout.as_millis() as u64,
            })
        }
    }
}

// Use rand for jitter in retry logic

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_retry_with_backoff_success() {
        let mut attempts = 0;
        let result = retry_with_backoff(
            "test_operation",
            RetryConfig::quick(),
            || async {
                attempts += 1;
                if attempts < 3 {
                    Err(ClientError::NetworkError {
                        reason: "temporary failure".to_string()
                    })
                } else {
                    Ok(42)
                }
            }
        ).await;
        
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts, 3);
    }
    
    #[tokio::test]
    async fn test_retry_non_recoverable() {
        let mut attempts = 0;
        let result = retry_with_backoff(
            "test_operation",
            RetryConfig::default(),
            || async {
                attempts += 1;
                Err(ClientError::InvalidConfiguration {
                    field: "test".to_string(),
                    reason: "bad config".to_string()
                })
            }
        ).await;
        
        assert!(result.is_err());
        assert_eq!(attempts, 1); // Should not retry
    }
} 
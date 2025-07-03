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
/// 
/// Defines the parameters for retry operations including maximum attempts, delay strategies,
/// and backoff behavior. This configuration is used by retry mechanisms to control how
/// operations are retried when they encounter recoverable errors.
/// 
/// # Examples
/// 
/// ```rust
/// # use rvoip_client_core::client::recovery::RetryConfig;
/// # use std::time::Duration;
/// # fn main() {
/// // Create a default retry configuration
/// let config = RetryConfig::default();
/// assert_eq!(config.max_attempts, 3);
/// assert_eq!(config.initial_delay, Duration::from_millis(100));
/// assert_eq!(config.backoff_multiplier, 2.0);
/// assert!(config.use_jitter);
/// 
/// // Create a custom configuration
/// let custom_config = RetryConfig {
///     max_attempts: 5,
///     initial_delay: Duration::from_millis(200),
///     max_delay: Duration::from_secs(10),
///     backoff_multiplier: 1.5,
///     use_jitter: false,
/// };
/// 
/// println!("Custom config allows {} attempts", custom_config.max_attempts);
/// # }
/// ```
/// 
/// ```rust
/// # use rvoip_client_core::client::recovery::RetryConfig;
/// # use std::time::Duration;
/// # fn main() {
/// // Configuration for different scenarios
/// let network_config = RetryConfig {
///     max_attempts: 3,
///     initial_delay: Duration::from_millis(100),
///     max_delay: Duration::from_secs(5),
///     backoff_multiplier: 2.0,
///     use_jitter: true,
/// };
/// 
/// let registration_config = RetryConfig {
///     max_attempts: 5,
///     initial_delay: Duration::from_secs(1),
///     max_delay: Duration::from_secs(30),
///     backoff_multiplier: 2.5,
///     use_jitter: false,
/// };
/// 
/// println!("Network retries: {} attempts", network_config.max_attempts);
/// println!("Registration retries: {} attempts", registration_config.max_attempts);
/// # }
/// ```
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
    /// 
    /// Returns a retry configuration optimized for fast, transient operations like
    /// network requests. This configuration uses shorter delays and more attempts
    /// to quickly recover from temporary network issues.
    /// 
    /// # Returns
    /// 
    /// A `RetryConfig` with:
    /// - 5 maximum attempts
    /// - 50ms initial delay
    /// - 5 second maximum delay
    /// - 1.5x backoff multiplier
    /// - Jitter enabled
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::recovery::RetryConfig;
    /// # use std::time::Duration;
    /// # fn main() {
    /// // Quick retry configuration for network operations
    /// let config = RetryConfig::quick();
    /// 
    /// assert_eq!(config.max_attempts, 5);
    /// assert_eq!(config.initial_delay, Duration::from_millis(50));
    /// assert_eq!(config.max_delay, Duration::from_secs(5));
    /// assert_eq!(config.backoff_multiplier, 1.5);
    /// assert!(config.use_jitter);
    /// 
    /// println!("Quick config: {} attempts with {}ms initial delay", 
    ///          config.max_attempts, config.initial_delay.as_millis());
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::recovery::RetryConfig;
    /// # fn main() {
    /// // Comparison with default configuration
    /// let quick = RetryConfig::quick();
    /// let default = RetryConfig::default();
    /// 
    /// println!("Quick config attempts: {}", quick.max_attempts);
    /// println!("Default config attempts: {}", default.max_attempts);
    /// println!("Quick is more aggressive: {}", quick.max_attempts > default.max_attempts);
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::recovery::RetryConfig;
    /// # fn main() {
    /// // Use case scenarios
    /// let config = RetryConfig::quick();
    /// 
    /// // Suitable for:
    /// println!("Suitable for API calls: {}", config.max_attempts >= 3);
    /// println!("Suitable for DNS lookups: {}", config.initial_delay.as_millis() < 100);
    /// println!("Suitable for HTTP requests: {}", config.max_delay.as_secs() <= 10);
    /// # }
    /// ```
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
    /// 
    /// Returns a retry configuration optimized for slower, more deliberate operations
    /// like SIP registration or authentication. This configuration uses longer delays
    /// and fewer attempts to avoid overwhelming servers or triggering rate limits.
    /// 
    /// # Returns
    /// 
    /// A `RetryConfig` with:
    /// - 3 maximum attempts
    /// - 1 second initial delay
    /// - 60 second maximum delay
    /// - 3.0x backoff multiplier
    /// - Jitter disabled
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::recovery::RetryConfig;
    /// # use std::time::Duration;
    /// # fn main() {
    /// // Slow retry configuration for registration operations
    /// let config = RetryConfig::slow();
    /// 
    /// assert_eq!(config.max_attempts, 3);
    /// assert_eq!(config.initial_delay, Duration::from_secs(1));
    /// assert_eq!(config.max_delay, Duration::from_secs(60));
    /// assert_eq!(config.backoff_multiplier, 3.0);
    /// assert!(!config.use_jitter);
    /// 
    /// println!("Slow config: {} attempts with {}s initial delay", 
    ///          config.max_attempts, config.initial_delay.as_secs());
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::recovery::RetryConfig;
    /// # fn main() {
    /// // Compare different retry strategies
    /// let quick = RetryConfig::quick();
    /// let slow = RetryConfig::slow();
    /// 
    /// println!("Quick initial delay: {}ms", quick.initial_delay.as_millis());
    /// println!("Slow initial delay: {}s", slow.initial_delay.as_secs());
    /// println!("Slow is more conservative: {}", 
    ///          slow.initial_delay > quick.initial_delay);
    /// # }
    /// ```
    /// 
    /// ```rust
/// # use rvoip_client_core::client::recovery::RetryConfig;
/// # use std::time::Duration;
/// # fn main() {
/// // Backoff progression example
    /// let config = RetryConfig::slow();
    /// let mut delay = config.initial_delay;
    /// 
    /// println!("Retry delay progression:");
    /// for attempt in 1..=config.max_attempts {
    ///     println!("  Attempt {}: {}s", attempt, delay.as_secs());
    ///     let next_delay_secs = (delay.as_secs() as f64 * config.backoff_multiplier) as u64;
    ///     delay = Duration::from_secs(next_delay_secs.min(config.max_delay.as_secs()));
    /// }
    /// # }
    /// ```
    /// 
    /// # Use Cases
    /// 
    /// - SIP registration operations
    /// - Authentication and credential refresh
    /// - Server discovery and configuration
    /// - License validation and activation
    /// - Database connection establishment
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
/// 
/// Executes an async operation with automatic retry logic using exponential backoff.
/// The function will retry the operation if it fails with a recoverable error,
/// using the configured retry strategy. Non-recoverable errors immediately return.
/// 
/// # Arguments
/// 
/// * `operation_name` - A descriptive name for the operation (used in logging)
/// * `config` - Retry configuration specifying attempts, delays, and backoff behavior
/// * `operation` - A closure that returns a future representing the operation to retry
/// 
/// # Returns
/// 
/// Returns the successful result of the operation, or the final error if all retries fail.
/// 
/// # Examples
/// 
/// ```rust
/// # use rvoip_client_core::client::recovery::{retry_with_backoff, RetryConfig};
/// # use rvoip_client_core::error::{ClientError, ClientResult};
/// # use std::sync::atomic::{AtomicU32, Ordering};
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // Simulate a flaky network operation
/// let attempts = AtomicU32::new(0);
/// 
/// let result = retry_with_backoff(
///     "network_request",
///     RetryConfig::quick(),
///     || async {
///         let current = attempts.fetch_add(1, Ordering::SeqCst) + 1;
///         if current < 3 {
///             // Simulate temporary failure
///             Err(ClientError::NetworkError {
///                 reason: "Connection timeout".to_string()
///             })
///         } else {
///             // Succeed on 3rd attempt
///             Ok("Success!".to_string())
///         }
///     }
/// ).await?;
/// 
/// assert_eq!(result, "Success!");
/// assert_eq!(attempts.load(Ordering::SeqCst), 3);
/// # Ok(())
/// # }
/// ```
/// 
/// ```rust
/// # use rvoip_client_core::client::recovery::{retry_with_backoff, RetryConfig};
/// # use rvoip_client_core::error::{ClientError, ClientResult};
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // Non-recoverable error example
/// let result: ClientResult<i32> = retry_with_backoff(
///     "validation",
///     RetryConfig::default(),
///     || async {
///         Err(ClientError::InvalidConfiguration {
///             field: "username".to_string(),
///             reason: "cannot be empty".to_string()
///         })
///     }
/// ).await;
/// 
/// // Should fail immediately (non-recoverable error)
/// assert!(result.is_err());
/// # Ok(())
/// # }
/// ```
/// 
/// ```rust
/// # use rvoip_client_core::client::recovery::{retry_with_backoff, RetryConfig};
/// # use rvoip_client_core::error::{ClientError, ClientResult};
/// # use std::time::Duration;
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // Custom retry configuration
/// let custom_config = RetryConfig {
///     max_attempts: 2,
///     initial_delay: Duration::from_millis(10),
///     max_delay: Duration::from_secs(1),
///     backoff_multiplier: 2.0,
///     use_jitter: false,
/// };
/// 
/// let result = retry_with_backoff(
///     "custom_operation",
///     custom_config,
///     || async {
///         Ok(42i32)
///     }
/// ).await?;
/// 
/// assert_eq!(result, 42);
/// # Ok(())
/// # }
/// ```
/// 
/// # Error Recovery
/// 
/// The function uses `ClientError::is_recoverable()` to determine if an error
/// should trigger a retry. Recoverable errors include:
/// - Network timeouts and connectivity issues
/// - Temporary server unavailability (5xx errors)
/// - Rate limiting responses
/// 
/// Non-recoverable errors include:
/// - Configuration errors
/// - Authentication failures
/// - Invalid requests (4xx errors except rate limiting)
/// 
/// # Backoff Strategy
/// 
/// The delay between retries follows an exponential backoff pattern:
/// 1. Start with `initial_delay`
/// 2. Multiply by `backoff_multiplier` after each failure
/// 3. Cap at `max_delay`
/// 4. Optionally add random jitter to avoid thundering herd
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
/// 
/// Provides intelligent recovery suggestions for different types of errors that
/// can occur during VoIP client operations. Each recovery strategy analyzes
/// the error type and context to recommend appropriate recovery actions.
/// 
/// # Examples
/// 
/// ```rust
/// # use rvoip_client_core::client::recovery::{RecoveryStrategies, RecoveryAction};
/// # use rvoip_client_core::error::ClientError;
/// # #[tokio::main]
/// # async fn main() {
/// // Analyze a network error
/// let network_error = ClientError::NetworkError {
///     reason: "Connection timeout".to_string()
/// };
/// 
/// let recovery = RecoveryStrategies::recover_network_error(&network_error, "API call").await;
/// match recovery {
///     Some(RecoveryAction::RetryWithBackoff(_)) => {
///         println!("Recommended: Retry with backoff");
///     }
///     Some(action) => {
///         println!("Recommended recovery: {:?}", action);
///     }
///     None => {
///         println!("No recovery strategy available");
///     }
/// }
/// # }
/// ```
/// 
/// ```rust
/// # use rvoip_client_core::client::recovery::{RecoveryStrategies, RecoveryAction};
/// # use rvoip_client_core::error::ClientError;
/// # #[tokio::main]
/// # async fn main() {
/// // Analyze a registration error
/// let reg_error = ClientError::RegistrationFailed {
///     reason: "401 Unauthorized".to_string()
/// };
/// 
/// let recovery = RecoveryStrategies::recover_registration_error(&reg_error, "SIP registration").await;
/// if let Some(RecoveryAction::UpdateCredentials) = recovery {
///     println!("Authentication issue detected - update credentials needed");
/// }
/// # }
/// ```
/// 
/// ```rust
/// # use rvoip_client_core::client::recovery::{RecoveryStrategies, RecoveryAction};
/// # use rvoip_client_core::error::ClientError;
/// # #[tokio::main]
/// # async fn main() {
/// // Analyze a media error
/// let media_error = ClientError::MediaNegotiationFailed {
///     reason: "No compatible codec found".to_string()
/// };
/// 
/// let recovery = RecoveryStrategies::recover_media_error(&media_error, "call setup").await;
/// match recovery {
///     Some(RecoveryAction::RenegotiateCodecs) => {
///         println!("Codec negotiation issue - trying different codecs");
///     }
///     Some(RecoveryAction::UseDefaultCodec) => {
///         println!("Using fallback codec");
///     }
///     _ => {
///         println!("No specific media recovery available");
///     }
/// }
/// # }
/// ```
/// 
/// # Recovery Strategy Types
/// 
/// The `RecoveryStrategies` provides specialized recovery logic for:
/// - **Network Errors**: Connection issues, timeouts, server unreachability
/// - **Registration Errors**: Authentication, authorization, server responses
/// - **Media Errors**: Codec negotiation, port allocation, device issues
/// 
/// Each strategy analyzes error details and suggests appropriate recovery actions.
pub struct RecoveryStrategies;

impl RecoveryStrategies {
    /// Recover from network errors
    /// 
    /// Analyzes network-related errors and suggests appropriate recovery actions.
    /// This method examines the error type and reason to determine the best
    /// strategy for recovering from network connectivity issues.
    /// 
    /// # Arguments
    /// 
    /// * `error` - The network error to analyze
    /// * `_context` - Additional context about the operation (currently unused)
    /// 
    /// # Returns
    /// 
    /// Returns `Some(RecoveryAction)` with a suggested recovery strategy,
    /// or `None` if no specific recovery is recommended.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::recovery::{RecoveryStrategies, RecoveryAction};
    /// # use rvoip_client_core::error::ClientError;
    /// # #[tokio::main]
    /// # async fn main() {
    /// // Timeout error recovery
    /// let timeout_error = ClientError::NetworkError {
    ///     reason: "Request timeout".to_string()
    /// };
    /// 
    /// let recovery = RecoveryStrategies::recover_network_error(&timeout_error, "API call").await;
    /// if let Some(RecoveryAction::RetryWithBackoff(_)) = recovery {
    ///     println!("Timeout detected - will retry with backoff");
    /// }
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::recovery::{RecoveryStrategies, RecoveryAction};
    /// # use rvoip_client_core::error::ClientError;
    /// # #[tokio::main]
    /// # async fn main() {
    /// // Connection refused error
    /// let conn_error = ClientError::NetworkError {
    ///     reason: "Connection refused".to_string()
    /// };
    /// 
    /// let recovery = RecoveryStrategies::recover_network_error(&conn_error, "connect").await;
    /// if let Some(RecoveryAction::WaitAndRetry(_)) = recovery {
    ///     println!("Connection refused - will wait before retry");
    /// }
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::recovery::{RecoveryStrategies, RecoveryAction};
    /// # use rvoip_client_core::error::ClientError;
    /// # #[tokio::main]
    /// # async fn main() {
    /// // Server unreachable error
    /// let server_error = ClientError::ServerUnreachable {
    ///     server: "sip.example.com".to_string()
    /// };
    /// 
    /// let recovery = RecoveryStrategies::recover_network_error(&server_error, "registration").await;
    /// if let Some(RecoveryAction::TryAlternateServer) = recovery {
    ///     println!("Server unreachable - trying alternate server");
    /// }
    /// # }
    /// ```
    /// 
    /// # Recovery Strategies
    /// 
    /// - **Timeout errors**: Quick retry with backoff
    /// - **Connection refused**: Wait before retry
    /// - **Server unreachable**: Try alternate server
    /// - **General network errors**: Check connectivity and retry
    /// - **Connection timeout**: Slow retry with longer delays
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
    /// 
    /// Analyzes SIP registration errors and suggests appropriate recovery actions.
    /// This method examines registration failure reasons to determine the best
    /// strategy for recovering from authentication, authorization, and server issues.
    /// 
    /// # Arguments
    /// 
    /// * `error` - The registration error to analyze
    /// * `_context` - Additional context about the operation (currently unused)
    /// 
    /// # Returns
    /// 
    /// Returns `Some(RecoveryAction)` with a suggested recovery strategy,
    /// or `None` if no specific recovery is recommended.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::recovery::{RecoveryStrategies, RecoveryAction};
    /// # use rvoip_client_core::error::ClientError;
    /// # #[tokio::main]
    /// # async fn main() {
    /// // Authentication failure
    /// let auth_error = ClientError::RegistrationFailed {
    ///     reason: "401 Unauthorized - Invalid credentials".to_string()
    /// };
    /// 
    /// let recovery = RecoveryStrategies::recover_registration_error(&auth_error, "register").await;
    /// if let Some(RecoveryAction::UpdateCredentials) = recovery {
    ///     println!("Authentication failed - credentials need updating");
    /// }
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::recovery::{RecoveryStrategies, RecoveryAction};
    /// # use rvoip_client_core::error::ClientError;
    /// # #[tokio::main]
    /// # async fn main() {
    /// // Server busy error
    /// let busy_error = ClientError::RegistrationFailed {
    ///     reason: "503 Service Unavailable".to_string()
    /// };
    /// 
    /// let recovery = RecoveryStrategies::recover_registration_error(&busy_error, "register").await;
    /// if let Some(RecoveryAction::WaitAndRetry(_)) = recovery {
    ///     println!("Server busy - will wait before retrying");
    /// }
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::recovery::{RecoveryStrategies, RecoveryAction};
    /// # use rvoip_client_core::error::ClientError;
    /// # #[tokio::main]
    /// # async fn main() {
    /// // Registration expired
    /// let expired_error = ClientError::RegistrationExpired;
    /// 
    /// let recovery = RecoveryStrategies::recover_registration_error(&expired_error, "register").await;
    /// if let Some(RecoveryAction::Reregister) = recovery {
    ///     println!("Registration expired - will re-register");
    /// }
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::recovery::{RecoveryStrategies, RecoveryAction};
    /// # use rvoip_client_core::error::ClientError;
    /// # #[tokio::main]
    /// # async fn main() {
    /// // Authentication failed directly
    /// let auth_failed = ClientError::AuthenticationFailed {
    ///     reason: "Invalid password".to_string()
    /// };
    /// 
    /// let recovery = RecoveryStrategies::recover_registration_error(&auth_failed, "auth").await;
    /// if let Some(RecoveryAction::UpdateCredentials) = recovery {
    ///     println!("Direct authentication failure - update credentials");
    /// }
    /// # }
    /// ```
    /// 
    /// # Recovery Strategies
    /// 
    /// - **401 Unauthorized**: Update credentials and retry
    /// - **503 Service Unavailable**: Wait 30 seconds before retry
    /// - **Timeout errors**: Retry with slow backoff
    /// - **Registration expired**: Perform fresh registration
    /// - **Authentication failed**: Update credentials
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
    /// 
    /// Analyzes media-related errors and suggests appropriate recovery actions.
    /// This method examines media negotiation, codec, port allocation, and device
    /// errors to determine the best strategy for recovering media functionality.
    /// 
    /// # Arguments
    /// 
    /// * `error` - The media error to analyze
    /// * `_context` - Additional context about the operation (currently unused)
    /// 
    /// # Returns
    /// 
    /// Returns `Some(RecoveryAction)` with a suggested recovery strategy,
    /// or `None` if no specific recovery is recommended.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::recovery::{RecoveryStrategies, RecoveryAction};
    /// # use rvoip_client_core::error::ClientError;
    /// # #[tokio::main]
    /// # async fn main() {
    /// // Codec negotiation failure
    /// let codec_error = ClientError::MediaNegotiationFailed {
    ///     reason: "No compatible codec found".to_string()
    /// };
    /// 
    /// let recovery = RecoveryStrategies::recover_media_error(&codec_error, "call setup").await;
    /// if let Some(RecoveryAction::RenegotiateCodecs) = recovery {
    ///     println!("Codec negotiation failed - will try different codecs");
    /// }
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::recovery::{RecoveryStrategies, RecoveryAction};
    /// # use rvoip_client_core::error::ClientError;
    /// # #[tokio::main]
    /// # async fn main() {
    /// // Port allocation failure
    /// let port_error = ClientError::MediaNegotiationFailed {
    ///     reason: "Port allocation failed - no available ports".to_string()
    /// };
    /// 
    /// let recovery = RecoveryStrategies::recover_media_error(&port_error, "media setup").await;
    /// if let Some(RecoveryAction::ReallocatePorts) = recovery {
    ///     println!("Port allocation failed - will try different ports");
    /// }
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::recovery::{RecoveryStrategies, RecoveryAction};
    /// # use rvoip_client_core::error::ClientError;
    /// # #[tokio::main]
    /// # async fn main() {
    /// // No compatible codecs
    /// let no_codecs_error = ClientError::NoCompatibleCodecs;
    /// 
    /// let recovery = RecoveryStrategies::recover_media_error(&no_codecs_error, "codec selection").await;
    /// if let Some(RecoveryAction::UseDefaultCodec) = recovery {
    ///     println!("No compatible codecs - falling back to default");
    /// }
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::recovery::{RecoveryStrategies, RecoveryAction};
    /// # use rvoip_client_core::error::ClientError;
    /// # #[tokio::main]
    /// # async fn main() {
    /// // Audio device error
/// let device_error = ClientError::AudioDeviceError {
///     reason: "Device not available".to_string()
/// };
    /// 
    /// let recovery = RecoveryStrategies::recover_media_error(&device_error, "audio init").await;
    /// if let Some(RecoveryAction::ReinitializeAudioDevice) = recovery {
    ///     println!("Audio device error - will reinitialize");
    /// }
    /// # }
    /// ```
    /// 
    /// # Recovery Strategies
    /// 
    /// - **Codec issues**: Renegotiate codecs or use default codec
    /// - **Port allocation**: Reallocate media ports
    /// - **Media negotiation**: Restart media session
    /// - **Audio device errors**: Reinitialize audio device
    /// - **General media failures**: Restart media session
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
/// 
/// Represents the different recovery actions that can be recommended by
/// recovery strategies. Each action corresponds to a specific approach
/// for handling different types of errors in VoIP operations.
/// 
/// # Examples
/// 
/// ```rust
/// # use rvoip_client_core::client::recovery::{RecoveryAction, RetryConfig};
/// # use std::time::Duration;
/// # fn main() {
/// // Create different recovery actions
/// let retry_action = RecoveryAction::RetryWithBackoff(RetryConfig::quick());
/// let wait_action = RecoveryAction::WaitAndRetry(Duration::from_secs(5));
/// let cred_action = RecoveryAction::UpdateCredentials;
/// 
/// // Pattern match on actions
/// match retry_action {
///     RecoveryAction::RetryWithBackoff(config) => {
///         println!("Will retry {} times", config.max_attempts);
///     }
///     _ => {
///         println!("Different action type");
///     }
/// }
/// # }
/// ```
/// 
/// ```rust
/// # use rvoip_client_core::client::recovery::{RecoveryAction, RetryConfig};
/// # use std::time::Duration;
/// # fn main() {
/// // Handle different recovery actions
/// let actions = vec![
///     RecoveryAction::CheckConnectivityAndRetry,
///     RecoveryAction::TryAlternateServer,
///     RecoveryAction::Reregister,
///     RecoveryAction::RenegotiateCodecs,
/// ];
/// 
/// for action in actions {
///     match action {
///         RecoveryAction::CheckConnectivityAndRetry => {
///             println!("Action: Check network connectivity");
///         }
///         RecoveryAction::TryAlternateServer => {
///             println!("Action: Try alternate server");
///         }
///         RecoveryAction::Reregister => {
///             println!("Action: Re-register with server");
///         }
///         RecoveryAction::RenegotiateCodecs => {
///             println!("Action: Renegotiate media codecs");
///         }
///         _ => {
///             println!("Action: {:?}", action);
///         }
///     }
/// }
/// # }
/// ```
/// 
/// ```rust
/// # use rvoip_client_core::client::recovery::{RecoveryAction, RetryConfig};
/// # use std::time::Duration;
/// # fn main() {
/// // Recovery action priorities
/// let high_priority = vec![
///     RecoveryAction::UpdateCredentials,
///     RecoveryAction::ReinitializeAudioDevice,
/// ];
/// 
/// let medium_priority = vec![
///     RecoveryAction::RetryWithBackoff(RetryConfig::quick()),
///     RecoveryAction::WaitAndRetry(Duration::from_secs(1)),
/// ];
/// 
/// let low_priority = vec![
///     RecoveryAction::TryAlternateServer,
///     RecoveryAction::UseDefaultCodec,
/// ];
/// 
/// println!("High priority actions: {}", high_priority.len());
/// println!("Medium priority actions: {}", medium_priority.len());
/// println!("Low priority actions: {}", low_priority.len());
/// # }
/// ```
/// 
/// # Action Categories
/// 
/// - **Retry Actions**: `RetryWithBackoff`, `WaitAndRetry`
/// - **Network Actions**: `CheckConnectivityAndRetry`, `TryAlternateServer`
/// - **Authentication Actions**: `UpdateCredentials`, `Reregister`
/// - **Media Actions**: `RenegotiateCodecs`, `ReallocatePorts`, `RestartMediaSession`, `UseDefaultCodec`
/// - **Device Actions**: `ReinitializeAudioDevice`
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
/// 
/// Provides methods to add contextual information to errors, making them more
/// informative for debugging and logging. This trait extends `ClientResult<T>`
/// with context-adding capabilities.
/// 
/// # Examples
/// 
/// ```rust
/// # use rvoip_client_core::client::recovery::ErrorContext;
/// # use rvoip_client_core::error::{ClientError, ClientResult};
/// # fn main() {
/// fn might_fail() -> ClientResult<String> {
///     Err(ClientError::NetworkError {
///         reason: "Connection timeout".to_string()
///     })
/// }
/// 
/// // Add context to an error
/// let result = might_fail().context("Failed to connect to SIP server");
/// 
/// if let Err(e) = result {
///     println!("Error with context: {}", e);
/// }
/// # }
/// ```
/// 
/// ```rust
/// # use rvoip_client_core::client::recovery::ErrorContext;
/// # use rvoip_client_core::error::{ClientError, ClientResult};
/// # fn main() {
/// fn get_user_id() -> i32 { 42 }
/// 
/// fn register_user() -> ClientResult<()> {
///     Err(ClientError::RegistrationFailed {
///         reason: "Invalid credentials".to_string()
///     })
/// }
/// 
/// // Add context with lazy evaluation
/// let result = register_user().with_context(|| {
///     format!("Failed to register user {}", get_user_id())
/// });
/// 
/// assert!(result.is_err());
/// # }
/// ```
/// 
/// ```rust
/// # use rvoip_client_core::client::recovery::ErrorContext;
/// # use rvoip_client_core::error::{ClientError, ClientResult};
/// # fn main() {
/// // Chain multiple context layers
/// fn inner_operation() -> ClientResult<String> {
///     Err(ClientError::InvalidConfiguration {
///         field: "port".to_string(),
///         reason: "out of range".to_string()
///     })
/// }
/// 
/// fn outer_operation() -> ClientResult<String> {
///     inner_operation().context("Inner operation failed")
/// }
/// 
/// let result = outer_operation().context("Outer operation failed");
/// assert!(result.is_err());
/// # }
/// ```
/// 
/// # Benefits
/// 
/// - **Better debugging**: Adds operation context to errors
/// - **Error tracking**: Maintains error chains for complex operations
/// - **Logging integration**: Automatic structured logging of context
/// - **Lazy evaluation**: Context strings computed only when needed
pub trait ErrorContext<T> {
    /// Add context to the error
    /// 
    /// Adds a static context string to the error. The context is evaluated
    /// immediately and included in the error message if the result is an error.
    /// 
    /// # Arguments
    /// 
    /// * `context` - A string describing the operation that failed
    /// 
    /// # Returns
    /// 
    /// Returns the original result if successful, or an error with added context.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::recovery::ErrorContext;
    /// # use rvoip_client_core::error::{ClientError, ClientResult};
    /// # fn main() {
    /// fn network_operation() -> ClientResult<String> {
    ///     Err(ClientError::NetworkError {
    ///         reason: "DNS resolution failed".to_string()
    ///     })
    /// }
    /// 
    /// let result = network_operation().context("Failed to resolve server address");
    /// assert!(result.is_err());
    /// # }
    /// ```
    fn context(self, context: &str) -> ClientResult<T>;
    
    /// Add context with lazy evaluation
    /// 
    /// Adds context to the error using a closure that is only evaluated
    /// if the result is an error. This is useful for expensive context
    /// computation that should only happen when needed.
    /// 
    /// # Arguments
    /// 
    /// * `f` - A closure that returns the context string
    /// 
    /// # Returns
    /// 
    /// Returns the original result if successful, or an error with added context.
    /// 
    /// # Examples
    /// 
    /// ```rust
/// # use rvoip_client_core::client::recovery::ErrorContext;
/// # use rvoip_client_core::error::{ClientError, ClientResult};
/// # use uuid::Uuid;
/// # fn main() {
/// fn get_call_id() -> String { "call-123".to_string() }
    /// 
    /// fn end_call() -> ClientResult<()> {
///     Err(ClientError::CallNotFound {
///         call_id: uuid::Uuid::new_v4()
///     })
/// }
    /// 
    /// // Context is only computed if there's an error
    /// let result = end_call().with_context(|| {
    ///     format!("Failed to end call {}", get_call_id())
    /// });
    /// 
    /// assert!(result.is_err());
    /// # }
    /// ```
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
/// 
/// Wraps an async operation with a timeout, converting timeout errors into
/// appropriate `ClientError` variants with proper logging and context.
/// This function provides a consistent way to handle operation timeouts
/// across the VoIP client.
/// 
/// # Arguments
/// 
/// * `operation_name` - A descriptive name for the operation (used in logging)
/// * `timeout` - The maximum duration to wait for the operation
/// * `future` - The async operation to execute with timeout
/// 
/// # Returns
/// 
/// Returns the result of the operation if it completes within the timeout,
/// or a `ClientError::OperationTimeout` if the timeout is exceeded.
/// 
/// # Examples
/// 
/// ```rust
/// # use rvoip_client_core::client::recovery::with_timeout;
/// # use rvoip_client_core::error::{ClientError, ClientResult};
/// # use std::time::Duration;
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // Fast operation that completes within timeout
/// let result = with_timeout(
///     "quick_operation",
///     Duration::from_secs(5),
///     async {
///         tokio::time::sleep(Duration::from_millis(100)).await;
///         Ok::<String, ClientError>("Success".to_string())
///     }
/// ).await?;
/// 
/// assert_eq!(result, "Success");
/// # Ok(())
/// # }
/// ```
/// 
/// ```rust
/// # use rvoip_client_core::client::recovery::with_timeout;
/// # use rvoip_client_core::error::{ClientError, ClientResult};
/// # use std::time::Duration;
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // Operation that times out
/// let result: ClientResult<String> = with_timeout(
///     "slow_operation",
///     Duration::from_millis(100),
///     async {
///         tokio::time::sleep(Duration::from_secs(1)).await;
///         Ok("Should not reach here".to_string())
///     }
/// ).await;
/// 
/// // Should be a timeout error
/// match result {
///     Err(ClientError::OperationTimeout { duration_ms }) => {
///         assert_eq!(duration_ms, 100);
///         println!("Operation timed out after {}ms", duration_ms);
///     }
///     _ => panic!("Expected timeout error"),
/// }
/// # Ok(())
/// # }
/// ```
/// 
/// ```rust
/// # use rvoip_client_core::client::recovery::with_timeout;
/// # use rvoip_client_core::error::{ClientError, ClientResult};
/// # use std::time::Duration;
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // Network operation with timeout
/// async fn network_request() -> ClientResult<String> {
///     // Simulate network operation
///     tokio::time::sleep(Duration::from_millis(50)).await;
///     Ok("Network response".to_string())
/// }
/// 
/// let result = with_timeout(
///     "network_request",
///     Duration::from_secs(1),
///     network_request()
/// ).await?;
/// 
/// assert_eq!(result, "Network response");
/// # Ok(())
/// # }
/// ```
/// 
/// ```rust
/// # use rvoip_client_core::client::recovery::with_timeout;
/// # use rvoip_client_core::error::{ClientError, ClientResult};
/// # use std::time::Duration;
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // Operation that fails before timeout
/// let result: ClientResult<String> = with_timeout(
///     "failing_operation",
///     Duration::from_secs(5),
///     async {
///         Err(ClientError::NetworkError {
///             reason: "Connection failed".to_string()
///         })
///     }
/// ).await;
/// 
/// // Should preserve the original error
/// match result {
///     Err(ClientError::NetworkError { reason }) => {
///         assert_eq!(reason, "Connection failed");
///     }
///     _ => panic!("Expected network error"),
/// }
/// # Ok(())
/// # }
/// ```
/// 
/// # Use Cases
/// 
/// - Network operations (API calls, DNS lookups)
/// - SIP registration and authentication
/// - Media negotiation and setup
/// - Database operations and queries
/// - File I/O and resource allocation
/// 
/// # Logging
/// 
/// The function automatically logs timeout events with structured logging,
/// including the operation name and timeout duration for debugging purposes.
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
        use std::sync::atomic::{AtomicU32, Ordering};
        let attempts = AtomicU32::new(0);
        
        let result = retry_with_backoff(
            "test_operation",
            RetryConfig::quick(),
            || async {
                let current = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                if current < 3 {
                    Err(ClientError::NetworkError {
                        reason: "temporary failure".to_string()
                    })
                } else {
                    Ok(42)
                }
            }
        ).await;
        
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }
    
    #[tokio::test]
    async fn test_retry_non_recoverable() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let attempts = AtomicU32::new(0);
        
        let result: Result<i32, _> = retry_with_backoff(
            "test_operation",
            RetryConfig::default(),
            || async {
                attempts.fetch_add(1, Ordering::SeqCst);
                Err(ClientError::InvalidConfiguration {
                    field: "test".to_string(),
                    reason: "bad config".to_string()
                })
            }
        ).await;
        
        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 1); // Should not retry
    }
} 
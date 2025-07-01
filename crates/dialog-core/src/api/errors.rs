//! API Error Types
//!
//! This module provides simplified error types for the dialog-core API layer,
//! abstracting internal complexity and providing clear error categories for
//! application developers.
//!
//! ## Error Categories
//!
//! - **Configuration**: Invalid configuration or setup parameters
//! - **Network**: Network connectivity or transport issues  
//! - **Protocol**: SIP protocol violations or parsing errors
//! - **Dialog**: Dialog state or lifecycle errors
//! - **Internal**: Internal implementation errors
//!
//! ## Usage Examples
//!
//! ### Basic Error Handling
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{ApiError, ApiResult};
//!
//! async fn handle_api_error(result: ApiResult<String>) {
//!     match result {
//!         Ok(value) => println!("Success: {}", value),
//!         Err(ApiError::Configuration { message }) => {
//!             eprintln!("Please check your configuration: {}", message);
//!         },
//!         Err(ApiError::Network { message }) => {
//!             eprintln!("Network problem (check connectivity): {}", message);
//!         },
//!         Err(ApiError::Protocol { message }) => {
//!             eprintln!("SIP protocol issue: {}", message);
//!         },
//!         Err(ApiError::Dialog { message }) => {
//!             eprintln!("Dialog state problem: {}", message);
//!         },
//!         Err(ApiError::Internal { message }) => {
//!             eprintln!("Internal error (please report): {}", message);
//!         },
//!     }
//! }
//! ```
//!
//! ### Error Propagation
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{ApiError, ApiResult};
//!
//! async fn example_function() -> ApiResult<String> {
//!     // This automatically converts from internal errors
//!     let dialog = some_dialog_operation().await?;
//!     Ok(format!("Dialog created: {}", dialog))
//! }
//!
//! # async fn some_dialog_operation() -> Result<String, rvoip_dialog_core::errors::DialogError> {
//! #     Ok("test".to_string())
//! # }
//! ```

use std::fmt;

use crate::errors::DialogError;

/// High-level result type for API operations
///
/// This is the standard Result type used throughout the dialog-core API,
/// providing simplified error handling for application developers.
///
/// ## Examples
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::{ApiResult, ApiError};
///
/// async fn example_function() -> ApiResult<String> {
///     Ok("Success".to_string())
/// }
///
/// # async fn usage() {
/// match example_function().await {
///     Ok(result) => println!("Got: {}", result),
///     Err(ApiError::Configuration { message }) => {
///         eprintln!("Config error: {}", message);
///     },
///     Err(e) => eprintln!("Other error: {}", e),
/// }
/// # }
/// ```
pub type ApiResult<T> = Result<T, ApiError>;

/// Simplified error type for API consumers
///
/// Provides high-level error categories that applications can handle
/// appropriately without needing to understand internal dialog-core details.
///
/// ## Design Principles
///
/// - **User-friendly**: Clear, actionable error messages
/// - **Categorized**: Logical grouping for appropriate handling
/// - **Abstracted**: Hides internal implementation complexity
/// - **Consistent**: Uniform error handling across all APIs
///
/// ## Error Categories
///
/// ### Configuration Errors
/// Issues with setup, parameters, or invalid configurations:
/// - Invalid URIs or addresses
/// - Missing required parameters
/// - Incompatible configuration combinations
///
/// ### Network Errors
/// Connectivity and transport-related issues:
/// - Connection failures
/// - Transport errors
/// - Timeout issues
///
/// ### Protocol Errors
/// SIP protocol violations and parsing errors:
/// - Malformed SIP messages
/// - Protocol state violations
/// - Unsupported SIP features
///
/// ### Dialog Errors
/// Dialog state and lifecycle issues:
/// - Dialog not found
/// - Invalid dialog state transitions
/// - Dialog termination errors
///
/// ### Internal Errors
/// Implementation or system-level errors:
/// - Unexpected internal states
/// - System resource issues
/// - Programming errors
#[derive(Debug, Clone)]
pub enum ApiError {
    /// Configuration error
    ///
    /// Indicates an issue with configuration parameters, setup, or initialization.
    /// These errors typically require user intervention to fix the configuration.
    ///
    /// ## Common Causes
    /// - Invalid URI formats
    /// - Missing required parameters
    /// - Incompatible configuration options
    /// - Invalid network addresses
    ///
    /// ## Example Response
    /// Review and correct the configuration parameters.
    Configuration { 
        /// Human-readable error message
        message: String 
    },
    
    /// Network error
    ///
    /// Indicates connectivity or transport-related issues.
    /// These errors may be transient and worth retrying.
    ///
    /// ## Common Causes
    /// - Network connectivity issues
    /// - Server unavailable
    /// - Connection timeouts
    /// - Transport layer failures
    ///
    /// ## Example Response
    /// Check network connectivity and retry the operation.
    Network { 
        /// Human-readable error message
        message: String 
    },
    
    /// SIP protocol error
    ///
    /// Indicates violations of the SIP protocol or parsing errors.
    /// These errors suggest malformed messages or protocol misuse.
    ///
    /// ## Common Causes
    /// - Malformed SIP messages
    /// - Invalid SIP headers
    /// - Protocol state violations
    /// - Unsupported SIP extensions
    ///
    /// ## Example Response
    /// Review SIP message formatting and protocol compliance.
    Protocol { 
        /// Human-readable error message
        message: String 
    },
    
    /// Dialog error
    ///
    /// Indicates issues with dialog state, lifecycle, or operations.
    /// These errors suggest problems with dialog management.
    ///
    /// ## Common Causes
    /// - Dialog not found
    /// - Invalid state transitions
    /// - Dialog already terminated
    /// - Concurrent access issues
    ///
    /// ## Example Response
    /// Check dialog state and ensure proper lifecycle management.
    Dialog { 
        /// Human-readable error message
        message: String 
    },
    
    /// Internal error
    ///
    /// Indicates unexpected internal errors or system issues.
    /// These errors suggest bugs or system-level problems.
    ///
    /// ## Common Causes
    /// - Programming errors
    /// - System resource exhaustion
    /// - Unexpected internal states
    /// - Concurrency issues
    ///
    /// ## Example Response
    /// These errors should be reported as potential bugs.
    Internal { 
        /// Human-readable error message
        message: String 
    },
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Configuration { message } => {
                write!(f, "Configuration error: {}", message)
            },
            ApiError::Network { message } => {
                write!(f, "Network error: {}", message)
            },
            ApiError::Protocol { message } => {
                write!(f, "SIP protocol error: {}", message)
            },
            ApiError::Dialog { message } => {
                write!(f, "Dialog error: {}", message)
            },
            ApiError::Internal { message } => {
                write!(f, "Internal error: {}", message)
            },
        }
    }
}

impl std::error::Error for ApiError {}

impl ApiError {
    /// Create a configuration error
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::Configuration { message: message.into() }
    }
    
    /// Create a network error
    pub fn network(message: impl Into<String>) -> Self {
        Self::Network { message: message.into() }
    }
    
    /// Create a protocol error
    pub fn protocol(message: impl Into<String>) -> Self {
        Self::Protocol { message: message.into() }
    }
    
    /// Create a dialog error
    pub fn dialog(message: impl Into<String>) -> Self {
        Self::Dialog { message: message.into() }
    }
    
    /// Create an internal error
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal { message: message.into() }
    }
}

/// Convert from internal DialogError to public ApiError
///
/// This conversion abstracts internal error details and provides
/// user-friendly error categories for API consumers.
impl From<DialogError> for ApiError {
    fn from(error: DialogError) -> Self {
        match error {
            // Configuration-related errors
            DialogError::ConfigError { message, .. } => {
                ApiError::Configuration { message }
            },
            
            // Network and transport errors
            DialogError::NetworkError { message, .. } => {
                ApiError::Network { message }
            },
            
            // SIP protocol errors
            DialogError::ProtocolError { message, .. } => {
                ApiError::Protocol { message }
            },
            DialogError::RoutingError { message, .. } => {
                ApiError::Protocol { message }
            },
            
            // Dialog-specific errors
            DialogError::DialogNotFound { id, .. } => {
                ApiError::Dialog { 
                    message: format!("Dialog not found: {}", id) 
                }
            },
            DialogError::InvalidState { expected, actual, .. } => {
                ApiError::Dialog { 
                    message: format!("Invalid dialog state: expected {}, got {}", expected, actual) 
                }
            },
            DialogError::DialogAlreadyExists { id, .. } => {
                ApiError::Dialog { 
                    message: format!("Dialog already exists: {}", id) 
                }
            },
            
            // Transaction errors (map to internal for simplicity)
            DialogError::TransactionError { message, .. } => {
                ApiError::Internal { 
                    message: format!("Transaction error: {}", message) 
                }
            },
            
            // SDP and other internal errors
            DialogError::SdpError { message, .. } => {
                ApiError::Internal { 
                    message: format!("SDP error: {}", message) 
                }
            },
            DialogError::InternalError { message, .. } => {
                ApiError::Internal { message }
            },
            DialogError::TimeoutError { operation, .. } => {
                ApiError::Internal { 
                    message: format!("Timeout error: {}", operation) 
                }
            },
        }
    }
}

/// Convert from standard io::Error to ApiError
impl From<std::io::Error> for ApiError {
    fn from(error: std::io::Error) -> Self {
        ApiError::Network { 
            message: format!("I/O error: {}", error) 
        }
    }
}

/// Convert from serialization errors to ApiError
impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        ApiError::Configuration { 
            message: format!("Serialization error: {}", error) 
        }
    }
}

/// Convert from address parsing errors to ApiError
impl From<std::net::AddrParseError> for ApiError {
    fn from(error: std::net::AddrParseError) -> Self {
        ApiError::Configuration { 
            message: format!("Invalid address: {}", error) 
        }
    }
}

/// Convert from URI parsing errors to ApiError
impl From<http::uri::InvalidUri> for ApiError {
    fn from(error: http::uri::InvalidUri) -> Self {
        ApiError::Configuration { 
            message: format!("Invalid URI: {}", error) 
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_error_display() {
        let config_error = ApiError::Configuration { 
            message: "Invalid config".to_string() 
        };
        assert_eq!(format!("{}", config_error), "Configuration error: Invalid config");
        
        let network_error = ApiError::Network { 
            message: "Connection failed".to_string() 
        };
        assert_eq!(format!("{}", network_error), "Network error: Connection failed");
    }
    
    #[test]
    fn test_error_constructors() {
        let error = ApiError::configuration("test config error");
        match error {
            ApiError::Configuration { message } => assert_eq!(message, "test config error"),
            _ => panic!("Expected configuration error"),
        }
        
        let error = ApiError::network("test network error");
        match error {
            ApiError::Network { message } => assert_eq!(message, "test network error"),
            _ => panic!("Expected network error"),
        }
    }
    
    #[test]
    fn test_io_error_conversion() {
        let io_error = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "connection refused");
        let api_error: ApiError = io_error.into();
        
        match api_error {
            ApiError::Network { message } => assert!(message.contains("connection refused")),
            _ => panic!("Expected network error"),
        }
    }
    
    #[test]
    fn test_addr_parse_error_conversion() {
        let parse_error: std::net::AddrParseError = "invalid:address".parse::<std::net::SocketAddr>().unwrap_err();
        let api_error: ApiError = parse_error.into();
        
        match api_error {
            ApiError::Configuration { message } => assert!(message.contains("Invalid address")),
            _ => panic!("Expected configuration error"),
        }
    }
} 
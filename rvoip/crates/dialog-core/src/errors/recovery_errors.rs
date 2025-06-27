//! Recovery-specific error types
//!
//! This module defines error types for dialog recovery operations including
//! failure detection, recovery strategies, and recovery coordination.

use thiserror::Error;

/// Result type for recovery operations
pub type RecoveryResult<T> = Result<T, RecoveryError>;

/// Error types for dialog recovery operations
#[derive(Error, Debug, Clone)]
pub enum RecoveryError {
    /// Recovery not needed for this dialog
    #[error("Recovery not needed for dialog: {dialog_id}")]
    RecoveryNotNeeded { dialog_id: String },
    
    /// Recovery already in progress
    #[error("Recovery already in progress for dialog: {dialog_id}")]
    RecoveryInProgress { dialog_id: String },
    
    /// Recovery failed after maximum attempts
    #[error("Recovery failed after {attempts} attempts for dialog: {dialog_id}")]
    RecoveryFailed { dialog_id: String, attempts: u32 },
    
    /// Recovery strategy not available
    #[error("Recovery strategy not available: {strategy}")]
    StrategyNotAvailable { strategy: String },
    
    /// Failure detection error
    #[error("Failure detection error: {message}")]
    FailureDetectionError { message: String },
    
    /// Recovery coordination error
    #[error("Recovery coordination error: {message}")]
    CoordinationError { message: String },
    
    /// Dialog state incompatible with recovery
    #[error("Dialog state incompatible with recovery: {state}")]
    IncompatibleState { state: String },
    
    /// Recovery timeout
    #[error("Recovery timed out for dialog: {dialog_id}")]
    RecoveryTimeout { dialog_id: String },
}

impl RecoveryError {
    /// Create a recovery not needed error
    pub fn recovery_not_needed(dialog_id: &str) -> Self {
        Self::RecoveryNotNeeded {
            dialog_id: dialog_id.to_string(),
        }
    }
    
    /// Create a recovery in progress error
    pub fn recovery_in_progress(dialog_id: &str) -> Self {
        Self::RecoveryInProgress {
            dialog_id: dialog_id.to_string(),
        }
    }
    
    /// Create a recovery failed error
    pub fn recovery_failed(dialog_id: &str, attempts: u32) -> Self {
        Self::RecoveryFailed {
            dialog_id: dialog_id.to_string(),
            attempts,
        }
    }
    
    /// Create a strategy not available error
    pub fn strategy_not_available(strategy: &str) -> Self {
        Self::StrategyNotAvailable {
            strategy: strategy.to_string(),
        }
    }
} 
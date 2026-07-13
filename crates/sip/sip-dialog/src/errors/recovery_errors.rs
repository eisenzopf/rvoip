//! Recovery-specific error types
//!
//! This module defines error types for dialog recovery operations including
//! failure detection, recovery strategies, and recovery coordination.

use std::{error, fmt};

/// Result type for recovery operations
pub type RecoveryResult<T> = Result<T, RecoveryError>;

/// Error types for dialog recovery operations
#[derive(Clone)]
pub enum RecoveryError {
    /// Recovery not needed for this dialog
    RecoveryNotNeeded { dialog_id: String },

    /// Recovery already in progress
    RecoveryInProgress { dialog_id: String },

    /// Recovery failed after maximum attempts
    RecoveryFailed { dialog_id: String, attempts: u32 },

    /// Recovery strategy not available
    StrategyNotAvailable { strategy: String },

    /// Failure detection error
    FailureDetectionError { message: String },

    /// Recovery coordination error
    CoordinationError { message: String },

    /// Dialog state incompatible with recovery
    IncompatibleState { state: String },

    /// Recovery timeout
    RecoveryTimeout { dialog_id: String },
}

impl RecoveryError {
    pub const fn diagnostic_class(&self) -> &'static str {
        match self {
            Self::RecoveryNotNeeded { .. } => "not-needed",
            Self::RecoveryInProgress { .. } => "in-progress",
            Self::RecoveryFailed { .. } => "failed",
            Self::StrategyNotAvailable { .. } => "strategy-unavailable",
            Self::FailureDetectionError { .. } => "failure-detection",
            Self::CoordinationError { .. } => "coordination",
            Self::IncompatibleState { .. } => "incompatible-state",
            Self::RecoveryTimeout { .. } => "timeout",
        }
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn every_recovery_error_variant_is_payload_free() {
        const CANARY: &str = "recovery-error-direct-secret-canary";
        let errors = vec![
            RecoveryError::RecoveryNotNeeded {
                dialog_id: CANARY.into(),
            },
            RecoveryError::RecoveryInProgress {
                dialog_id: CANARY.into(),
            },
            RecoveryError::RecoveryFailed {
                dialog_id: CANARY.into(),
                attempts: 3,
            },
            RecoveryError::StrategyNotAvailable {
                strategy: CANARY.into(),
            },
            RecoveryError::FailureDetectionError {
                message: CANARY.into(),
            },
            RecoveryError::CoordinationError {
                message: CANARY.into(),
            },
            RecoveryError::IncompatibleState {
                state: CANARY.into(),
            },
            RecoveryError::RecoveryTimeout {
                dialog_id: CANARY.into(),
            },
        ];

        for error in errors {
            let rendered = format!("{error:?} {error}");
            assert!(!rendered.contains(CANARY), "payload leaked: {rendered}");
            assert!(!error.diagnostic_class().is_empty());
            assert!(std::error::Error::source(&error).is_none());
        }
    }
}

impl fmt::Display for RecoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "SIP dialog recovery failed (class={}",
            self.diagnostic_class()
        )?;
        if let Self::RecoveryFailed { attempts, .. } = self {
            write!(formatter, ", attempts={attempts}")?;
        }
        formatter.write_str(")")
    }
}

impl fmt::Debug for RecoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveryError")
            .field("class", &self.diagnostic_class())
            .field(
                "attempts",
                &match self {
                    Self::RecoveryFailed { attempts, .. } => Some(*attempts),
                    _ => None,
                },
            )
            .finish()
    }
}

impl error::Error for RecoveryError {}

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

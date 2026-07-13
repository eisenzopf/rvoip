//! Error types for registrar-core

use std::{error, fmt};

/// Result type alias for registrar operations
pub type Result<T> = std::result::Result<T, RegistrarError>;

/// Main error type for registrar operations
#[derive(Clone)]
pub enum RegistrarError {
    /// User not found in registry
    UserNotFound(String),

    /// Contact not found for user
    ContactNotFound { user: String, uri: String },

    /// Registration has expired
    RegistrationExpired(String),

    /// Invalid registration parameters
    InvalidRegistration(String),

    /// Maximum contacts exceeded
    MaxContactsExceeded { user: String, max: usize },

    /// Subscription not found
    SubscriptionNotFound(String),

    /// Invalid subscription
    InvalidSubscription(String),

    /// Maximum subscriptions exceeded
    MaxSubscriptionsExceeded { user: String, max: usize },

    /// Presence not found
    PresenceNotFound(String),

    /// Invalid presence data
    InvalidPresence(String),

    /// PIDF XML parsing error
    PidfError(String),

    /// Event bus error
    EventBusError(String),

    /// Configuration error
    ConfigError(String),

    /// Storage error (for future persistent storage)
    StorageError(String),

    /// Timeout error
    Timeout(String),

    /// Internal error
    Internal(String),

    /// Other errors
    Other(String),
}

impl RegistrarError {
    pub const fn diagnostic_class(&self) -> &'static str {
        match self {
            Self::UserNotFound(_) => "user-not-found",
            Self::ContactNotFound { .. } => "contact-not-found",
            Self::RegistrationExpired(_) => "registration-expired",
            Self::InvalidRegistration(_) => "invalid-registration",
            Self::MaxContactsExceeded { .. } => "max-contacts-exceeded",
            Self::SubscriptionNotFound(_) => "subscription-not-found",
            Self::InvalidSubscription(_) => "invalid-subscription",
            Self::MaxSubscriptionsExceeded { .. } => "max-subscriptions-exceeded",
            Self::PresenceNotFound(_) => "presence-not-found",
            Self::InvalidPresence(_) => "invalid-presence",
            Self::PidfError(_) => "pidf",
            Self::EventBusError(_) => "event-bus",
            Self::ConfigError(_) => "configuration",
            Self::StorageError(_) => "storage",
            Self::Timeout(_) => "timeout",
            Self::Internal(_) => "internal",
            Self::Other(_) => "other",
        }
    }
}

impl fmt::Display for RegistrarError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "SIP registrar operation failed (class={}",
            self.diagnostic_class()
        )?;
        match self {
            Self::MaxContactsExceeded { max, .. } | Self::MaxSubscriptionsExceeded { max, .. } => {
                write!(formatter, ", max={max}")?
            }
            _ => {}
        }
        formatter.write_str(")")
    }
}

impl fmt::Debug for RegistrarError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegistrarError")
            .field("class", &self.diagnostic_class())
            .field(
                "max",
                &match self {
                    Self::MaxContactsExceeded { max, .. }
                    | Self::MaxSubscriptionsExceeded { max, .. } => Some(*max),
                    _ => None,
                },
            )
            .finish()
    }
}

impl error::Error for RegistrarError {}

impl From<std::io::Error> for RegistrarError {
    fn from(err: std::io::Error) -> Self {
        RegistrarError::Internal(err.to_string())
    }
}

impl From<serde_json::Error> for RegistrarError {
    fn from(err: serde_json::Error) -> Self {
        RegistrarError::Internal(format!("JSON error: {}", err))
    }
}

impl From<quick_xml::Error> for RegistrarError {
    fn from(err: quick_xml::Error) -> Self {
        RegistrarError::PidfError(err.to_string())
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn every_registrar_error_variant_is_payload_free() {
        const CANARY: &str = "registrar-error-direct-secret-canary";
        let errors = vec![
            RegistrarError::UserNotFound(CANARY.into()),
            RegistrarError::ContactNotFound {
                user: CANARY.into(),
                uri: CANARY.into(),
            },
            RegistrarError::RegistrationExpired(CANARY.into()),
            RegistrarError::InvalidRegistration(CANARY.into()),
            RegistrarError::MaxContactsExceeded {
                user: CANARY.into(),
                max: 10,
            },
            RegistrarError::SubscriptionNotFound(CANARY.into()),
            RegistrarError::InvalidSubscription(CANARY.into()),
            RegistrarError::MaxSubscriptionsExceeded {
                user: CANARY.into(),
                max: 20,
            },
            RegistrarError::PresenceNotFound(CANARY.into()),
            RegistrarError::InvalidPresence(CANARY.into()),
            RegistrarError::PidfError(CANARY.into()),
            RegistrarError::EventBusError(CANARY.into()),
            RegistrarError::ConfigError(CANARY.into()),
            RegistrarError::StorageError(CANARY.into()),
            RegistrarError::Timeout(CANARY.into()),
            RegistrarError::Internal(CANARY.into()),
            RegistrarError::Other(CANARY.into()),
        ];

        for error in errors {
            let rendered = format!("{error:?} {error}");
            assert!(!rendered.contains(CANARY), "payload leaked: {rendered}");
            assert!(!error.diagnostic_class().is_empty());
            assert!(std::error::Error::source(&error).is_none());
        }
    }
}

//! Error types for users-core

use std::fmt;

pub enum Error {
    Database(sqlx_core::Error),

    InvalidCredentials,

    UserNotFound(String),

    UserAlreadyExists(String),

    InvalidPassword(String),

    Jwt(jsonwebtoken::errors::Error),

    ApiKeyNotFound,

    ApiKeyExpired,

    /// A security-sensitive operation was attempted on an authentication
    /// service constructed without its durable security store.
    SecurityStoreUnavailable {
        operation: &'static str,
    },

    /// API-key-to-JWT exchange is intentionally disabled until a versioned
    /// contract can retain key scopes and revocation lineage end to end.
    ApiKeyTokenExchangeDisabled {
        contract: &'static str,
    },

    Config(String),

    Validation(String),

    Internal(anyhow::Error),
}

impl Error {
    fn diagnostic_class(&self) -> &'static str {
        match self {
            Self::Database(_) => "database",
            Self::InvalidCredentials => "invalid-credentials",
            Self::UserNotFound(_) => "user-not-found",
            Self::UserAlreadyExists(_) => "user-already-exists",
            Self::InvalidPassword(_) => "invalid-password",
            Self::Jwt(_) => "jwt",
            Self::ApiKeyNotFound => "api-key-not-found",
            Self::ApiKeyExpired => "api-key-expired",
            Self::SecurityStoreUnavailable { .. } => "security-store-unavailable",
            Self::ApiKeyTokenExchangeDisabled { .. } => "api-key-token-exchange-disabled",
            Self::Config(_) => "configuration",
            Self::Validation(_) => "validation",
            Self::Internal(_) => "internal",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "users operation failed (class={})",
            self.diagnostic_class()
        )
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UsersError")
            .field("class", &self.diagnostic_class())
            .finish()
    }
}

impl std::error::Error for Error {}

impl From<sqlx_core::Error> for Error {
    fn from(error: sqlx_core::Error) -> Self {
        Self::Database(error)
    }
}

impl From<jsonwebtoken::errors::Error> for Error {
    fn from(error: jsonwebtoken::errors::Error) -> Self {
        Self::Jwt(error)
    }
}

impl From<anyhow::Error> for Error {
    fn from(error: anyhow::Error) -> Self {
        Self::Internal(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

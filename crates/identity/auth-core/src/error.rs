//! Error types for authentication operations

use std::fmt;

pub enum AuthError {
    InvalidToken(String),

    TokenExpired,

    InsufficientPermissions(String),

    ProviderError(String),

    ConfigError(String),

    NetworkError(String),

    CacheError(String),

    InternalError(String),

    InvalidChallenge(String),

    InvalidResponse(String),

    DigestError(String),
}

impl AuthError {
    fn diagnostic_class(&self) -> &'static str {
        match self {
            Self::InvalidToken(_) => "invalid-token",
            Self::TokenExpired => "token-expired",
            Self::InsufficientPermissions(_) => "insufficient-permissions",
            Self::ProviderError(_) => "provider",
            Self::ConfigError(_) => "configuration",
            Self::NetworkError(_) => "network",
            Self::CacheError(_) => "cache",
            Self::InternalError(_) => "internal",
            Self::InvalidChallenge(_) => "invalid-challenge",
            Self::InvalidResponse(_) => "invalid-response",
            Self::DigestError(_) => "digest-computation",
        }
    }
}

impl fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "authentication failed (class={})",
            self.diagnostic_class()
        )
    }
}

impl fmt::Debug for AuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthError")
            .field("class", &self.diagnostic_class())
            .finish()
    }
}

impl std::error::Error for AuthError {}

pub type Result<T> = std::result::Result<T, AuthError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbitrary_lower_error_values_are_never_diagnostic_content() {
        const SECRET: &str = "auth-core-lower-error-secret-canary";
        let errors = [
            AuthError::InvalidToken(SECRET.into()),
            AuthError::InsufficientPermissions(SECRET.into()),
            AuthError::ProviderError(SECRET.into()),
            AuthError::ConfigError(SECRET.into()),
            AuthError::NetworkError(SECRET.into()),
            AuthError::CacheError(SECRET.into()),
            AuthError::InternalError(SECRET.into()),
            AuthError::InvalidChallenge(SECRET.into()),
            AuthError::InvalidResponse(SECRET.into()),
            AuthError::DigestError(SECRET.into()),
        ];

        for error in errors {
            let rendered = format!("{error} {error:?}");
            assert!(!rendered.contains(SECRET), "lower error leaked: {rendered}");
            match error {
                AuthError::InvalidToken(value)
                | AuthError::InsufficientPermissions(value)
                | AuthError::ProviderError(value)
                | AuthError::ConfigError(value)
                | AuthError::NetworkError(value)
                | AuthError::CacheError(value)
                | AuthError::InternalError(value)
                | AuthError::InvalidChallenge(value)
                | AuthError::InvalidResponse(value)
                | AuthError::DigestError(value) => assert_eq!(value, SECRET),
                AuthError::TokenExpired => unreachable!(),
            }
        }
    }
}

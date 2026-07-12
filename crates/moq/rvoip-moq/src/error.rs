use rvoip_core_traits::error::RvoipError;

use crate::{LocError, MoqCompatibilityError, MoqNamespaceError, MsfCatalogError};

/// Bounded, non-sensitive relay failure categories.
///
/// These values are safe to expose in diagnostics and metric labels. The wire
/// adapter deliberately does not retain transport error strings because they
/// can contain relay URLs or credential material.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum MoqRelayFailure {
    ConnectFailed,
    ConnectTimeout,
    SessionEnded,
    PublicationEnded,
    ReconnectExhausted,
    TaskFailed,
}

impl MoqRelayFailure {
    pub(crate) const fn metric_label(self) -> &'static str {
        match self {
            Self::ConnectFailed => "connect-failed",
            Self::ConnectTimeout => "connect-timeout",
            Self::SessionEnded => "session-ended",
            Self::PublicationEnded => "publication-ended",
            Self::ReconnectExhausted => "reconnect-exhausted",
            Self::TaskFailed => "task-failed",
        }
    }
}

impl std::fmt::Display for MoqRelayFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.metric_label())
    }
}

/// Stable rvoip-owned error surface for the MOQT adapter.
///
/// Wire-engine errors are rendered into the `Wire` variant so `moq-rs` types
/// never become part of rvoip-moq's public API.
#[derive(Debug, thiserror::Error)]
pub enum MoqError {
    #[error("invalid MOQT publisher configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("MOQT publisher construction requires an active Tokio runtime")]
    RuntimeUnavailable,
    #[error(transparent)]
    Namespace(#[from] MoqNamespaceError),
    #[error(transparent)]
    Compatibility(#[from] MoqCompatibilityError),
    #[error(transparent)]
    Loc(#[from] LocError),
    #[error(transparent)]
    CatalogModel(#[from] MsfCatalogError),
    #[error("MOQT tracks are closed")]
    Closed,
    #[error("MOQT wire error: {0}")]
    Wire(String),
    #[error("MSF catalog encoding failed: {0}")]
    Catalog(#[from] serde_json::Error),
    #[error("MOQT relay failed: {0}")]
    RelayFailure(MoqRelayFailure),
    #[error("invalid MOQT TLS configuration: {0}")]
    TlsConfiguration(&'static str),
}

impl From<MoqError> for RvoipError {
    fn from(error: MoqError) -> Self {
        RvoipError::Adapter(error.to_string())
    }
}

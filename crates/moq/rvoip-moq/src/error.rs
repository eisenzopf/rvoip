use rvoip_core_traits::error::RvoipError;

use crate::{LocError, MoqCompatibilityError, MoqNamespaceError, MsfCatalogError};

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
    #[error("MOQT relay error: {0}")]
    Relay(String),
}

impl From<MoqError> for RvoipError {
    fn from(error: MoqError) -> Self {
        RvoipError::Adapter(error.to_string())
    }
}

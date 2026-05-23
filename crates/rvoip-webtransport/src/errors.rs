//! `UctpWtError` per design doc §3.2.1.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum UctpWtError {
    #[error(transparent)]
    Uctp(#[from] rvoip_uctp::errors::UctpError),

    #[error(transparent)]
    Substrate(#[from] rvoip_uctp::errors::SubstrateError),

    #[error("wt session error: {0}")]
    Session(String),

    #[error("adapter not started")]
    NotStarted,

    #[error("adapter shutdown")]
    Shutdown,
}

pub type Result<T> = std::result::Result<T, UctpWtError>;

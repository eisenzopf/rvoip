//! `UctpQuicError` per `UCTP_IMPLEMENTATION_PLAN.md` §3.2.1.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum UctpQuicError {
    #[error(transparent)]
    Uctp(#[from] rvoip_uctp::errors::UctpError),

    #[error(transparent)]
    Substrate(#[from] rvoip_uctp::errors::SubstrateError),

    #[error("adapter not started")]
    NotStarted,

    #[error("adapter shutdown")]
    Shutdown,
}

pub type Result<T> = std::result::Result<T, UctpQuicError>;

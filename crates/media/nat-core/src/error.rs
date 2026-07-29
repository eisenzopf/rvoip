//! Error type for this crate.

use thiserror::Error as ThisError;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, ThisError)]
pub enum Error {
    /// Wraps a `webrtc_ice::Error` (feature `ice`), keeping that
    /// dependency out of this crate's public error surface.
    #[error("ICE agent error: {0}")]
    Agent(String),

    #[error("invalid ICE candidate: {0}")]
    InvalidCandidate(String),

    #[error("candidate gathering completed without producing any candidates")]
    NoCandidatesGathered,

    #[error("ICE connectivity check failed or timed out")]
    ConnectFailed,

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(feature = "ice")]
impl From<webrtc_ice::Error> for Error {
    fn from(e: webrtc_ice::Error) -> Self {
        Error::Agent(e.to_string())
    }
}

//! `UctpWtError` per design doc §3.2.1.

pub enum UctpWtError {
    Uctp(rvoip_uctp::errors::UctpError),

    Substrate(rvoip_uctp::errors::SubstrateError),

    Session(String),

    NotStarted,

    Shutdown,
}

impl UctpWtError {
    pub const fn diagnostic_class(&self) -> &'static str {
        match self {
            Self::Uctp(_) => "uctp",
            Self::Substrate(_) => "substrate",
            Self::Session(_) => "session",
            Self::NotStarted => "not-started",
            Self::Shutdown => "shutdown",
        }
    }
}

impl std::fmt::Display for UctpWtError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "UCTP WebTransport error (class={})",
            self.diagnostic_class()
        )
    }
}

impl std::fmt::Debug for UctpWtError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UctpWtError")
            .field("class", &self.diagnostic_class())
            .finish()
    }
}

impl std::error::Error for UctpWtError {}

impl From<rvoip_uctp::errors::UctpError> for UctpWtError {
    fn from(error: rvoip_uctp::errors::UctpError) -> Self {
        Self::Uctp(error)
    }
}

impl From<rvoip_uctp::errors::SubstrateError> for UctpWtError {
    fn from(error: rvoip_uctp::errors::SubstrateError) -> Self {
        Self::Substrate(error)
    }
}

pub type Result<T> = std::result::Result<T, UctpWtError>;

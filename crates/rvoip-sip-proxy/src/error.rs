use std::net::SocketAddr;
use thiserror::Error;

pub type ProxyResult<T> = std::result::Result<T, ProxyError>;

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("transaction error: {0}")]
    Transaction(String),

    #[error("transport error: {0}")]
    Transport(String),

    #[error("Max-Forwards exhausted; sending 483 Too Many Hops")]
    MaxForwardsExhausted,

    #[error("loop detected via branch collision")]
    LoopDetected,

    #[error("no route decision returned by application for {0}")]
    NoRoute(SocketAddr),

    #[error("Timer C fired on stalled INVITE — sending 408 Request Timeout")]
    TimerCExpired,

    #[error("proxy is shut down")]
    Shutdown,
}

impl From<rvoip_sip_transport::Error> for ProxyError {
    fn from(e: rvoip_sip_transport::Error) -> Self {
        ProxyError::Transport(e.to_string())
    }
}

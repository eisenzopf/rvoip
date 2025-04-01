use std::io;
use std::net::SocketAddr;
use thiserror::Error;

/// A type alias for handling `Result`s with `Error`
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur in SIP transport handling
#[derive(Error, Debug)]
pub enum Error {
    /// Failed to bind to the specified address
    #[error("Failed to bind to {0}: {1}")]
    BindFailed(SocketAddr, #[source] io::Error),

    /// Error receiving a packet
    #[error("Error receiving packet: {0}")]
    ReceiveFailed(#[source] io::Error),

    /// Error sending a packet
    #[error("Error sending packet to {0}: {1}")]
    SendFailed(SocketAddr, #[source] io::Error),

    /// Packet too large for transport
    #[error("Packet too large for transport (size: {0} bytes, max: {1} bytes)")]
    PacketTooLarge(usize, usize),

    /// Error in SIP message processing
    #[error("SIP message error: {0}")]
    SipMessageError(#[from] rvoip_sip_core::Error),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// Channel error (receiver dropped)
    #[error("Channel closed")]
    ChannelClosed,

    /// Transport is closed
    #[error("Transport is closed")]
    TransportClosed,

    /// Other error
    #[error("{0}")]
    Other(String),
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error::Other(s.to_string())
    }
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error::Other(s)
    }
}

impl<T> From<tokio::sync::mpsc::error::SendError<T>> for Error {
    fn from(_: tokio::sync::mpsc::error::SendError<T>) -> Self {
        Error::ChannelClosed
    }
} 
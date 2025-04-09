use std::fmt;
use std::io;
use thiserror::Error;

/// A type alias for handling `Result`s with `Error`
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur in SIP protocol handling
#[derive(Error, Debug)]
pub enum Error {
    /// Invalid SIP method
    #[error("Invalid SIP method")]
    InvalidMethod,

    /// Invalid SIP header syntax
    #[error("Invalid SIP header: {0}")]
    InvalidHeader(String),

    /// Invalid SIP URI
    #[error("Invalid SIP URI: {0}")]
    InvalidUri(String),

    /// Invalid SIP version
    #[error("Invalid SIP version")]
    InvalidVersion,

    /// Invalid status code
    #[error("Invalid status code: {0}")]
    InvalidStatusCode(u16),

    /// Invalid message format
    #[error("Invalid message format: {0}")]
    InvalidFormat(String),

    /// Parser error
    #[error("Parser error: {0}")]
    Parser(String),
    
    /// Parse error
    #[error("Parse error: {0}")]
    ParseError(String),

    /// Input/output error
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// Other error with message
    #[error("{0}")]
    Other(String),
}

impl From<nom::Err<nom::error::Error<&str>>> for Error {
    fn from(err: nom::Err<nom::error::Error<&str>>) -> Self {
        Error::Parser(format!("Parsing failed: {err}"))
    }
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
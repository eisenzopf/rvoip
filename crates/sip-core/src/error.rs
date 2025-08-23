use std::fmt;
use std::io;
use thiserror::Error;
use std::str::Utf8Error;
use nom::error::{Error as NomError};

/// A type alias for handling `Result`s with `Error`
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur in SIP protocol handling
#[derive(Error, Debug, Clone)]
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

    /// Parser error with location information
    #[error("Parser error at line {line}, column {column}: {message}")]
    ParserWithLocation {
        /// Line number where the error occurred (1-indexed)
        line: usize,
        /// Column number where the error occurred (1-indexed)
        column: usize,
        /// Error message
        message: String,
    },
    
    /// Parser error
    #[error("Parser error: {0}")]
    Parser(String),
    
    /// Parse error
    #[error("Parse error: {0}")]
    ParseError(String),

    /// Content-Length mismatch
    #[error("Content-Length mismatch: expected {expected}, got {actual}")]
    ContentLengthMismatch {
        /// Expected length as stated in Content-Length header
        expected: usize,
        /// Actual length of body
        actual: usize,
    },

    /// Missing required header
    #[error("Missing required header: {0}")]
    MissingHeader(String),

    /// Unsupported media type
    #[error("Unsupported media type: {0}")]
    UnsupportedMediaType(String),

    /// Malformed URI component
    #[error("Malformed URI component: {component} - {message}")]
    MalformedUriComponent {
        /// URI component that is malformed (e.g., "host", "port")
        component: String,
        /// Error message
        message: String, 
    },

    /// Error related to SDP processing
    #[error("SDP error: {0}")]
    SdpError(String), // Generic SDP error

    /// Specific SDP parsing error
    #[error("SDP parsing error: {0}")]
    SdpParsingError(String),

    /// SDP validation error
    #[error("SDP validation error: {0}")]
    SdpValidationError(String),

    /// Message validation error
    #[error("Message validation error: {0}")]
    ValidationError(String),

    /// Transport-specific error
    #[error("Transport error: {0}")]
    Transport(String),

    /// Incremental parsing error - not enough data
    #[error("Incremental parsing error: {0}")]
    IncompleteParse(String),

    /// Input/output error
    #[error("I/O Error: {0}")]
    IoError(String),
    
    /// Line too long in SIP message
    #[error("Line too long: {0} characters")]
    LineTooLong(usize),

    /// Too many headers in SIP message
    #[error("Too many headers: {0}")]
    TooManyHeaders(usize),

    /// Body too large in SIP message
    #[error("Body too large: {0} bytes")]
    BodyTooLarge(usize),

    /// Other error with message
    #[error("{0}")]
    Other(String),

    /// Invalid input value
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// SDP generation error
    #[error("SDP Generation Error: {0}")]
    SdpFormatError(String),

    /// Invalid UTF-8 sequence
    #[error("Invalid UTF-8 sequence: {0}")]
    Utf8Error(#[from] Utf8Error),

    /// Internal error
    #[error("Internal Error: {0}")]
    InternalError(String),

    /// Error related to message or header building
    #[error("Builder error: {0}")]
    BuilderError(String),
}

impl From<nom::Err<nom::error::Error<&str>>> for Error {
    fn from(err: nom::Err<nom::error::Error<&str>>) -> Self {
        match err {
            nom::Err::Error(e) | nom::Err::Failure(e) => {
                let (line, column) = calculate_position(e.input);
                Error::ParserWithLocation {
                    line,
                    column,
                    message: format!("Failed to parse at position {}: {:?}", e.input.len(), e.code),
                }
            },
            nom::Err::Incomplete(_) => Error::IncompleteParse("Need more data".to_string()),
        }
    }
}

impl From<nom::Err<(&str, nom::error::ErrorKind)>> for Error {
    fn from(err: nom::Err<(&str, nom::error::ErrorKind)>) -> Self {
        match err {
            nom::Err::Error((input, kind)) | nom::Err::Failure((input, kind)) => {
                let (line, column) = calculate_position(input);
                Error::ParserWithLocation {
                    line,
                    column,
                    message: format!("Parser error: {:?}", kind),
                }
            },
            nom::Err::Incomplete(_) => Error::IncompleteParse("Need more data".to_string()),
        }
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

/// Calculate the line and column position from an input string
fn calculate_position(input: &str) -> (usize, usize) {
    let mut line = 1;
    let mut last_line_start = 0;
    
    for (i, c) in input.char_indices() {
        if c == '\n' {
            line += 1;
            last_line_start = i + 1;
        }
    }
    
    let column = input.len() - last_line_start + 1;
    (line, column)
}

/// LocationAwareError for tracking precise error locations
#[derive(Debug)]
pub struct LocationAwareError {
    /// Line where the error occurred
    pub line: usize,
    /// Column where the error occurred
    pub column: usize,
    /// Error message
    pub message: String,
    /// Input at the error location (small snippet)
    pub context: String,
}

impl fmt::Display for LocationAwareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Error at line {}, column {}: {}\nContext: '{}'", 
               self.line, self.column, self.message, self.context)
    }
}

// Convert Nom errors to our custom Error type
// This allows using `?` with nom parsers in functions returning `Result<T>`.
impl<'a> From<nom::Err<NomError<&'a [u8]>>> for Error {
    fn from(err: nom::Err<NomError<&'a [u8]>>) -> Self {
        Error::ParseError(format!("Nom parsing error: {:?}", err))
    }
}

// Add conversion from NomError directly if needed within map_res closures
impl<'a> From<NomError<&'a [u8]>> for Error {
    fn from(err: NomError<&'a [u8]>) -> Self {
        Error::ParseError(format!("Nom error detail: {:?}", err))
    }
}

// Add conversion from ParseIntError
impl From<std::num::ParseIntError> for Error {
    fn from(err: std::num::ParseIntError) -> Self {
        Error::ParseError(format!("Failed to parse integer: {}", err))
    }
}

// Convert owned Nom errors (Vec<u8>) to our custom Error type
impl From<nom::Err<NomError<Vec<u8>>>> for Error {
    fn from(err: nom::Err<NomError<Vec<u8>>>) -> Self {
        // We lose positional info here as Vec<u8> doesn't track original input slice easily
        Error::ParseError(format!("Nom parsing error (owned): {:?}", err))
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::IoError(err.to_string())
    }
} 
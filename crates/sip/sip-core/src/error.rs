use nom::error::Error as NomError;
use std::fmt;
use std::io;
use std::str::Utf8Error;

/// A type alias for handling `Result`s with `Error`
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur in SIP protocol handling
#[derive(Clone)]
pub enum Error {
    /// Invalid SIP method
    InvalidMethod,

    /// Invalid SIP header syntax
    InvalidHeader(String),

    /// Invalid SIP URI
    InvalidUri(String),

    /// Invalid SIP version
    InvalidVersion,

    /// Invalid status code
    InvalidStatusCode(u16),

    /// Invalid message format
    InvalidFormat(String),

    /// Parser error with location information
    ParserWithLocation {
        /// Line number where the error occurred (1-indexed)
        line: usize,
        /// Column number where the error occurred (1-indexed)
        column: usize,
        /// Error message
        message: String,
    },

    /// Parser error
    Parser(String),

    /// Parse error
    ParseError(String),

    /// Content-Length mismatch
    ContentLengthMismatch {
        /// Expected length as stated in Content-Length header
        expected: usize,
        /// Actual length of body
        actual: usize,
    },

    /// Missing required header
    MissingHeader(String),

    /// Unsupported media type
    UnsupportedMediaType(String),

    /// Malformed URI component
    MalformedUriComponent {
        /// URI component that is malformed (e.g., "host", "port")
        component: String,
        /// Error message
        message: String,
    },

    /// Error related to SDP processing
    SdpError(String), // Generic SDP error

    /// Specific SDP parsing error
    SdpParsingError(String),

    /// SDP validation error
    SdpValidationError(String),

    /// Message validation error
    ValidationError(String),

    /// Transport-specific error
    Transport(String),

    /// Incremental parsing error - not enough data
    IncompleteParse(String),

    /// Input/output error
    IoError(String),

    /// Line too long in SIP message
    LineTooLong(usize),

    /// Too many headers in SIP message
    TooManyHeaders(usize),

    /// Body too large in SIP message
    BodyTooLarge(usize),

    /// Other error with message
    Other(String),

    /// Invalid input value
    InvalidInput(String),

    /// SDP generation error
    SdpFormatError(String),

    /// Invalid UTF-8 sequence
    Utf8Error(Utf8Error),

    /// Internal error
    InternalError(String),

    /// Error related to message or header building
    BuilderError(String),
}

impl Error {
    /// Stable, payload-free class for logs, metrics, and public diagnostics.
    pub const fn diagnostic_class(&self) -> &'static str {
        match self {
            Self::InvalidMethod => "invalid-method",
            Self::InvalidHeader(_) => "invalid-header",
            Self::InvalidUri(_) => "invalid-uri",
            Self::InvalidVersion => "invalid-version",
            Self::InvalidStatusCode(_) => "invalid-status-code",
            Self::InvalidFormat(_) => "invalid-format",
            Self::ParserWithLocation { .. } => "parser-with-location",
            Self::Parser(_) => "parser",
            Self::ParseError(_) => "parse",
            Self::ContentLengthMismatch { .. } => "content-length-mismatch",
            Self::MissingHeader(_) => "missing-header",
            Self::UnsupportedMediaType(_) => "unsupported-media-type",
            Self::MalformedUriComponent { .. } => "malformed-uri-component",
            Self::SdpError(_) => "sdp",
            Self::SdpParsingError(_) => "sdp-parsing",
            Self::SdpValidationError(_) => "sdp-validation",
            Self::ValidationError(_) => "validation",
            Self::Transport(_) => "transport",
            Self::IncompleteParse(_) => "incomplete-parse",
            Self::IoError(_) => "io",
            Self::LineTooLong(_) => "line-too-long",
            Self::TooManyHeaders(_) => "too-many-headers",
            Self::BodyTooLarge(_) => "body-too-large",
            Self::Other(_) => "other",
            Self::InvalidInput(_) => "invalid-input",
            Self::SdpFormatError(_) => "sdp-format",
            Self::Utf8Error(_) => "utf8",
            Self::InternalError(_) => "internal",
            Self::BuilderError(_) => "builder",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "SIP operation failed (class={}",
            self.diagnostic_class()
        )?;
        match self {
            Self::InvalidStatusCode(code) => write!(formatter, ", status_code={code}")?,
            Self::ParserWithLocation { line, column, .. } => {
                write!(formatter, ", line={line}, column={column}")?
            }
            Self::ContentLengthMismatch { expected, actual } => {
                write!(formatter, ", expected={expected}, actual={actual}")?
            }
            Self::LineTooLong(length) => write!(formatter, ", characters={length}")?,
            Self::TooManyHeaders(count) => write!(formatter, ", count={count}")?,
            Self::BodyTooLarge(bytes) => write!(formatter, ", bytes={bytes}")?,
            Self::Utf8Error(error) => write!(
                formatter,
                ", valid_up_to={}, error_len={:?}",
                error.valid_up_to(),
                error.error_len()
            )?,
            _ => {}
        }
        formatter.write_str(")")
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Error")
            .field("class", &self.diagnostic_class())
            .field(
                "detail_bytes",
                &match self {
                    Self::InvalidHeader(value)
                    | Self::InvalidUri(value)
                    | Self::InvalidFormat(value)
                    | Self::Parser(value)
                    | Self::ParseError(value)
                    | Self::MissingHeader(value)
                    | Self::UnsupportedMediaType(value)
                    | Self::SdpError(value)
                    | Self::SdpParsingError(value)
                    | Self::SdpValidationError(value)
                    | Self::ValidationError(value)
                    | Self::Transport(value)
                    | Self::IncompleteParse(value)
                    | Self::IoError(value)
                    | Self::Other(value)
                    | Self::InvalidInput(value)
                    | Self::SdpFormatError(value)
                    | Self::InternalError(value)
                    | Self::BuilderError(value) => Some(value.len()),
                    Self::ParserWithLocation { message, .. } => Some(message.len()),
                    Self::MalformedUriComponent { component, message } => {
                        Some(component.len().saturating_add(message.len()))
                    }
                    _ => None,
                },
            )
            .finish()
    }
}

impl std::error::Error for Error {}

impl From<Utf8Error> for Error {
    fn from(error: Utf8Error) -> Self {
        Self::Utf8Error(error)
    }
}

impl From<nom::Err<nom::error::Error<&str>>> for Error {
    fn from(err: nom::Err<nom::error::Error<&str>>) -> Self {
        match err {
            nom::Err::Error(e) | nom::Err::Failure(e) => {
                let (line, column) = calculate_position(e.input);
                Error::ParserWithLocation {
                    line,
                    column,
                    message: format!(
                        "Failed to parse at position {}: {:?}",
                        e.input.len(),
                        e.code
                    ),
                }
            }
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
            }
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
        write!(
            f,
            "SIP parse operation failed (class=location-aware, line={}, column={}, message_bytes={}, context_bytes={})",
            self.line,
            self.column,
            self.message.len(),
            self.context.len()
        )
    }
}

impl fmt::Debug for LocationAwareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocationAwareError")
            .field("line", &self.line)
            .field("column", &self.column)
            .field("message_bytes", &self.message.len())
            .field("context_bytes", &self.context.len())
            .finish()
    }
}

impl std::error::Error for LocationAwareError {}

// Convert Nom errors to our custom Error type
// This allows using `?` with nom parsers in functions returning `Result<T>`.
impl<'a> From<nom::Err<NomError<&'a [u8]>>> for Error {
    fn from(err: nom::Err<NomError<&'a [u8]>>) -> Self {
        nom_byte_error(err)
    }
}

// Add conversion from NomError directly if needed within map_res closures
impl<'a> From<NomError<&'a [u8]>> for Error {
    fn from(err: NomError<&'a [u8]>) -> Self {
        Error::ParseError(format!(
            "SIP parser rejected input (class={:?}, remaining_bytes={})",
            err.code,
            err.input.len()
        ))
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
        match err {
            nom::Err::Error(error) | nom::Err::Failure(error) => Error::ParseError(format!(
                "SIP parser rejected owned input (class={:?}, remaining_bytes={})",
                error.code,
                error.input.len()
            )),
            // Preserve the public conversion contract from before diagnostic
            // hardening: byte-oriented nom errors, including Incomplete, map
            // to ParseError. Do not include nom's input in the diagnostic.
            nom::Err::Incomplete(needed) => Error::ParseError(format!(
                "SIP parser rejected owned input (class=Incomplete, needed={needed:?})"
            )),
        }
    }
}

fn nom_byte_error(err: nom::Err<NomError<&[u8]>>) -> Error {
    match err {
        nom::Err::Error(error) | nom::Err::Failure(error) => Error::ParseError(format!(
            "SIP parser rejected input (class={:?}, remaining_bytes={})",
            error.code,
            error.input.len()
        )),
        // Preserve the public conversion contract from before diagnostic
        // hardening: byte-oriented nom errors, including Incomplete, map to
        // ParseError. Incomplete carries no input buffer to report.
        nom::Err::Incomplete(needed) => Error::ParseError(format!(
            "SIP parser rejected input (class=Incomplete, needed={needed:?})"
        )),
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::IoError(err.to_string())
    }
}

#[cfg(test)]
mod diagnostic_safety_tests {
    use super::*;
    use nom::error::ErrorKind;

    #[test]
    fn byte_nom_error_conversions_never_embed_input() {
        const SECRET: &[u8] = b"Digest username=alice,response=credential-secret";

        let borrowed = Error::from(nom::Err::Failure(NomError::new(SECRET, ErrorKind::Verify)));
        let owned = Error::from(nom::Err::Failure(NomError::new(
            SECRET.to_vec(),
            ErrorKind::Verify,
        )));
        let direct = Error::from(NomError::new(SECRET, ErrorKind::Verify));

        for error in [borrowed, owned, direct] {
            let Error::ParseError(detail) = &error else {
                panic!("nom conversion must retain the public ParseError variant")
            };
            assert!(detail.contains(&format!("remaining_bytes={}", SECRET.len())));
            assert!(detail.contains("Verify"));
            let rendered = format!("{error:?} {error}");
            assert!(!rendered.contains("credential-secret"));
            assert!(!rendered.contains("username=alice"));
            assert!(!rendered.contains("remaining_bytes="));
            assert!(!rendered.contains("Verify"));
            assert!(rendered.contains("class"));
        }
    }

    #[test]
    fn byte_nom_incomplete_preserves_parse_error_variant() {
        let borrowed = Error::from(nom::Err::<NomError<&[u8]>>::Incomplete(nom::Needed::new(7)));
        let owned = Error::from(nom::Err::<NomError<Vec<u8>>>::Incomplete(nom::Needed::new(
            11,
        )));

        for error in [borrowed, owned] {
            let Error::ParseError(detail) = &error else {
                panic!("incomplete nom input must retain ParseError")
            };
            assert!(detail.contains("class=Incomplete"));
            let rendered = format!("{error:?} {error}");
            assert!(!rendered.contains("class=Incomplete"));
            assert!(!rendered.contains("IncompleteParse"));
        }
    }

    #[test]
    #[allow(invalid_from_utf8)]
    fn every_public_error_variant_has_payload_free_diagnostics() {
        const CANARY: &str = "sip-core-direct-diagnostic-secret-canary";
        let utf8 = std::str::from_utf8(&[0xff]).unwrap_err();
        let errors = vec![
            Error::InvalidMethod,
            Error::InvalidHeader(CANARY.into()),
            Error::InvalidUri(CANARY.into()),
            Error::InvalidVersion,
            Error::InvalidStatusCode(799),
            Error::InvalidFormat(CANARY.into()),
            Error::ParserWithLocation {
                line: 2,
                column: 3,
                message: CANARY.into(),
            },
            Error::Parser(CANARY.into()),
            Error::ParseError(CANARY.into()),
            Error::ContentLengthMismatch {
                expected: 10,
                actual: 9,
            },
            Error::MissingHeader(CANARY.into()),
            Error::UnsupportedMediaType(CANARY.into()),
            Error::MalformedUriComponent {
                component: CANARY.into(),
                message: CANARY.into(),
            },
            Error::SdpError(CANARY.into()),
            Error::SdpParsingError(CANARY.into()),
            Error::SdpValidationError(CANARY.into()),
            Error::ValidationError(CANARY.into()),
            Error::Transport(CANARY.into()),
            Error::IncompleteParse(CANARY.into()),
            Error::IoError(CANARY.into()),
            Error::LineTooLong(1_024),
            Error::TooManyHeaders(513),
            Error::BodyTooLarge(65_536),
            Error::Other(CANARY.into()),
            Error::InvalidInput(CANARY.into()),
            Error::SdpFormatError(CANARY.into()),
            Error::Utf8Error(utf8),
            Error::InternalError(CANARY.into()),
            Error::BuilderError(CANARY.into()),
        ];

        for error in errors {
            let rendered = format!("{error:?} {error}");
            assert!(!rendered.contains(CANARY), "payload leaked: {rendered}");
            assert!(!error.diagnostic_class().is_empty());
            assert!(std::error::Error::source(&error).is_none());
        }

        let location = LocationAwareError {
            line: 7,
            column: 11,
            message: CANARY.into(),
            context: CANARY.into(),
        };
        let rendered = format!("{location:?} {location}");
        assert!(!rendered.contains(CANARY));
        assert!(std::error::Error::source(&location).is_none());
    }

    #[test]
    fn source_never_debug_formats_nom_byte_input() {
        let source = include_str!("error.rs");
        for fragments in [
            ["Nom parsing error", ": {:?}"],
            ["Nom parsing error (owned)", ": {:?}"],
            ["Nom error detail", ": {:?}"],
        ] {
            assert!(!source.contains(&fragments.concat()));
        }
    }
}

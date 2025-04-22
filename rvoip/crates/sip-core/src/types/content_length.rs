use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use crate::parser;
use crate::error::{Result, Error};
use crate::parser::headers::parse_content_length;
use std::ops::Deref;
use nom::combinator::all_consuming;

/// Represents the Content-Length header field (RFC 3261 Section 7.3.2).
/// Indicates the size of the message body in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ContentLength(pub usize);

impl ContentLength {
    /// Creates a new Content-Length header value.
    pub fn new(length: usize) -> Self {
        Self(length)
    }
}

impl Deref for ContentLength {
    type Target = usize;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for ContentLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ContentLength {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        use crate::parser::headers::content_length::parse_content_length;

        all_consuming(parse_content_length)(s.as_bytes())
            .map_err(Error::from)
            .and_then(|(_, value)| {
                Ok(ContentLength(value.try_into().map_err(|_| Error::ParseError("Content-Length value out of range for usize".to_string()))?))
            })
    }
}

// TODO: Implement methods if needed 
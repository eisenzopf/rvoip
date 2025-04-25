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
pub struct ContentLength(pub u32);

impl ContentLength {
    /// Creates a new Content-Length header value.
    pub fn new(length: u32) -> Self {
        Self(length)
    }
}

impl Deref for ContentLength {
    type Target = u32;

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
        match s.trim().parse::<u32>() {
            Ok(len) => Ok(ContentLength(len)),
            Err(_) => Err(Error::ParseError(format!("Invalid Content-Length value: {}", s)))
        }
    }
}

// TODO: Implement methods if needed 
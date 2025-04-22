use crate::types::media_type::MediaType;
use crate::parser::headers::parse_content_type;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use crate::types::param::Param;
use bytes::Bytes;
use std::collections::HashMap;
use crate::parser;

/// Typed Content-Type header.
#[derive(Debug, Clone, PartialEq, Eq)] // Add derives as needed
pub struct ContentType(pub MediaType);

impl ContentType {
    /// Creates a new Content-Type header.
    pub fn new(media_type: MediaType) -> Self {
        Self(media_type)
    }
}

impl fmt::Display for ContentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0) // Delegate to MediaType display
    }
}

impl FromStr for ContentType {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        use crate::parser::headers::content_type::parse_content_type;

        all_consuming(parse_content_type)(s.as_bytes())
            .map_err(Error::from)
            .map(|(_, value)| ContentType(value))
    }
}

// TODO: Implement methods if needed 
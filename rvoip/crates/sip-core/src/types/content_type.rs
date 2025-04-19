use crate::types::media_type::MediaType;
use crate::parser::headers::parse_content_type;
use crate::error::Result;
use std::fmt;
use std::str::FromStr;

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
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        // Delegate to the parser function and wrap the result
        parse_content_type(s).map(ContentType)
    }
}

// TODO: Implement methods if needed 
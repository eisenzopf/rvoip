use std::fmt;
use std::str::FromStr;
use crate::error::Result;
use crate::parser::headers::parse_content_length;

/// Typed Content-Length header value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)] // Add derives as needed
pub struct ContentLength(pub usize);

impl fmt::Display for ContentLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ContentLength {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        parse_content_length(s)
    }
}

// TODO: Implement methods if needed 
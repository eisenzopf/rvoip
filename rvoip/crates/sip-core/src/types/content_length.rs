use std::fmt;
use std::str::FromStr;
use crate::error::Result;
use crate::parser::headers::parse_content_length;
use std::ops::Deref;
use nom::combinator::all_consuming;

/// Typed Content-Length header value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)] // Add derives as needed
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

        match all_consuming(parse_content_length)(s.as_bytes()) {
            Ok((_, value)) => Ok(ContentLength(value)),
            Err(e) => Err(Error::ParsingError{ 
                message: format!("Failed to parse Content-Length header: {:?}", e), 
                source: None 
            })
        }
    }
}

// TODO: Implement methods if needed 
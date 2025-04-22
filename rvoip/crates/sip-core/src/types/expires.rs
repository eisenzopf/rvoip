use crate::parser;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use crate::parser::headers::parse_expires;
use nom::combinator::all_consuming;

/// Typed Expires header value (seconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)] // Add derives as needed
pub struct Expires(pub u32);

impl Expires {
    /// Creates a new Expires header value.
    pub fn new(seconds: u32) -> Self {
        Self(seconds)
    }
}

impl fmt::Display for Expires {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Expires {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        use crate::parser::headers::expires::parse_expires;
        
        // Use map_err and From to convert Nom error to crate::Error::ParseError
        all_consuming(parse_expires)(s.as_bytes())
            .map_err(Error::from)
            .map(|(_, value)| Expires(value))
    }
}

// TODO: Implement methods if needed 
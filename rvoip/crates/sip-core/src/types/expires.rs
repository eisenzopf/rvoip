use std::fmt;
use std::str::FromStr;
use crate::error::Result;
use crate::parser::headers::parse_expires;

/// Typed Expires header value (seconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)] // Add derives as needed
pub struct Expires(pub u32);

impl fmt::Display for Expires {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Expires {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        parse_expires(s)
    }
}

// TODO: Implement methods if needed 
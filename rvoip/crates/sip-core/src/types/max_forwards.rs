use std::fmt;
use std::str::FromStr;
use crate::error::Result;
use crate::parser::headers::parse_max_forwards;

/// Typed Max-Forwards header value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)] // Add derives as needed
pub struct MaxForwards(pub u8);

impl fmt::Display for MaxForwards {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for MaxForwards {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        parse_max_forwards(s)
    }
}

// TODO: Implement methods (e.g., decrement, is_zero) 
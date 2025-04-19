use std::fmt;
use std::str::FromStr;
use crate::error::Result;
use crate::parser::headers::parse_max_forwards;

/// Typed Max-Forwards header value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)] // Add derives as needed
pub struct MaxForwards(pub u8);

impl MaxForwards {
    /// Creates a new Max-Forwards header value.
    pub fn new(hops: u8) -> Self {
        Self(hops)
    }

    /// Decrements the Max-Forwards value, returning None if it reaches zero.
    pub fn decrement(self) -> Option<Self> {
        if self.0 > 0 {
            Some(Self(self.0 - 1))
        } else {
            None
        }
    }

    /// Checks if the value is zero.
    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }
}

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
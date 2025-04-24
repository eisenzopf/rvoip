use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use crate::error::{Error, Result};

/// Represents the Max-Forwards header field (RFC 3261 Section 8.1.1.4).
/// Limits the number of proxies a request can traverse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        s.trim().parse::<u8>()
            .map(MaxForwards)
            .map_err(|e| Error::ParseError(
                format!("Invalid Max-Forwards value: {}", e)
            ))
    }
} 
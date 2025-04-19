use crate::types::address::Address; // Or maybe UriWithParams?
use crate::parser::headers::parse_reply_to; // Use the parser
use crate::error::Result;
use std::fmt;
use std::str::FromStr;

/// Typed Reply-To header.
#[derive(Debug, Clone, PartialEq, Eq)] // Add derives as needed
pub struct ReplyTo(pub Address); // Or UriWithParams

impl ReplyTo {
    /// Creates a new ReplyTo header.
    pub fn new(address: Address) -> Self {
        Self(address)
    }
}

impl fmt::Display for ReplyTo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0) // Delegate to Address display
    }
}

impl FromStr for ReplyTo {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        // Assuming parse_reply_to is the correct entry point
        parse_reply_to(s)
    }
}

// TODO: Implement methods if needed 
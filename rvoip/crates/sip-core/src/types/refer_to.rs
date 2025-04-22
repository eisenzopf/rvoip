use crate::types::address::Address; 
use crate::parser::headers::parse_refer_to; // Will be implemented in the parser
use crate::error::{Error, Result};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use serde::{Deserialize, Serialize};

/// Typed Refer-To header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferTo(pub Address);

impl ReferTo {
    /// Creates a new ReferTo header.
    pub fn new(address: Address) -> Self {
        Self(address)
    }
}

impl fmt::Display for ReferTo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0) // Delegate to Address display
    }
}

impl FromStr for ReferTo {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        use crate::parser::parse_address;
        // Use all_consuming, handle input type, map result and error
        all_consuming(parse_address)(s.as_bytes())
            .map(|(_rem, addr)| ReferTo(addr))
            .map_err(|e| Error::from(e.to_owned())) // Convert nom::Err to crate::error::Error
    }
} 
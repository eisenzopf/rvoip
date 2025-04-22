use crate::types::{HeaderName, HeaderValue, Param, TypedHeader, ParseTypedHeader};
use crate::types::address::Address;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use crate::error::{Error, Result};
use crate::parser::parse_address; // For FromStr
use std::ops::Deref;
use nom::combinator;

/// Represents the From header field (RFC 3261 Section 8.1.1.3).
/// Contains the logical identity of the initiator of the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct From(pub Address);

impl From {
    /// Creates a new From header.
    pub fn new(address: Address) -> Self {
        Self(address)
    }

    /// Gets the tag parameter value.
    pub fn tag(&self) -> Option<&str> {
        self.0.tag()
    }

    /// Sets or replaces the tag parameter.
    pub fn set_tag(&mut self, tag: impl Into<String>) {
        self.0.set_tag(tag)
    }
}

// Delegate Display and FromStr to Address
impl fmt::Display for From {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for From {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> {
        // Use all_consuming, handle input type, map result and error
        nom::combinator::all_consuming(parse_address)(s.as_bytes())
            .map(|(_rem, addr)| From(addr))
            .map_err(|e| Error::from(e.to_owned())) // Convert nom::Err to crate::error::Error
    }
}

// Optionally implement Deref to access all Address methods directly
impl Deref for From {
    type Target = Address;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// ... Display/FromStr impls ... 
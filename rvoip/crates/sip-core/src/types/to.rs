use crate::types::address::Address;
use std::fmt;
use std::str::FromStr;
use crate::error::Result;
use crate::parser::headers::parse_address; // For FromStr
use std::ops::Deref;
use serde::{Serialize, Deserialize};

/// Represents the To header field (RFC 3261 Section 8.1.1.3).
/// Contains the logical recipient of the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] // Added Serialize, Deserialize
pub struct To(pub Address);

impl To {
    /// Creates a new To header.
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
impl fmt::Display for To {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for To {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> {
        parse_address(s).map(To)
    }
}

// Optionally implement Deref to access all Address methods directly
impl Deref for To {
    type Target = Address;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// ... Display/FromStr impls ... 
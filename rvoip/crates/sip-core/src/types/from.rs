use crate::types::address::Address;
use std::fmt;
use std::str::FromStr;
use crate::error::Result;
use crate::parser::headers::parse_address; // For FromStr
use std::ops::Deref;

/// Typed From header.
#[derive(Debug, Clone, PartialEq, Eq)] // Add derives as needed
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
        parse_address(s).map(From)
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
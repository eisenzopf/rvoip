use crate::types::address::Address;
// use crate::types::Param; // Removed duplicate import
use std::fmt;
use std::str::FromStr;
use crate::error::Result;
use crate::parser::headers::parse_contact; // For FromStr
use std::ops::Deref;
use crate::types::param::Param;
use ordered_float::NotNan;

/// Represents the value within a Contact header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContactValue {
    /// A standard SIP address.
    Address(Address),
    /// The wildcard value "*".
    Wildcard,
}

/// Typed Contact header.
/// Note: RFC 3261 allows multiple Contact values in a single header line (comma-separated)
/// or multiple Contact header lines. This struct represents a SINGLE Contact value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact(pub ContactValue);

impl Contact {
    /// Creates a new Contact header from an Address.
    pub fn new_address(address: Address) -> Self {
        Self(ContactValue::Address(address))
    }

    /// Creates a new wildcard Contact header.
    pub fn new_wildcard() -> Self {
        Self(ContactValue::Wildcard)
    }

    /// Gets the expires parameter value, if present.
    /// Returns None for wildcard contacts.
    pub fn expires(&self) -> Option<u32> {
        match &self.0 {
            ContactValue::Address(addr) => addr.params.iter().find_map(|p| match p {
                Param::Expires(val) => Some(*val),
                _ => None,
            }),
            ContactValue::Wildcard => None,
        }
    }
    
    /// Sets or replaces the expires parameter.
    /// Panics if called on a wildcard contact.
    pub fn set_expires(&mut self, expires: u32) {
        match &mut self.0 {
            ContactValue::Address(addr) => {
                addr.params.retain(|p| !matches!(p, Param::Expires(_)));
                addr.params.push(Param::Expires(expires));
            },
            ContactValue::Wildcard => panic!("Cannot set expires on wildcard Contact"),
        }
    }

    /// Gets the q parameter value, if present.
    /// Returns None for wildcard contacts.
    pub fn q(&self) -> Option<NotNan<f32>> {
        match &self.0 {
            ContactValue::Address(addr) => addr.params.iter().find_map(|p| match p {
                Param::Q(val) => Some(val),
                _ => None,
            }).copied(),
            ContactValue::Wildcard => None,
        }
    }
    
    /// Sets or replaces the q parameter.
    /// Panics if called on a wildcard contact.
    pub fn set_q(&mut self, q: f32) {
        let clamped_q = q.max(0.0).min(1.0);
        match &mut self.0 {
            ContactValue::Address(addr) => {
                addr.params.retain(|p| !matches!(p, Param::Q(_)));
                addr.params.push(Param::Q(NotNan::try_from(clamped_q).expect("Clamped q value should not be NaN")));
            },
            ContactValue::Wildcard => panic!("Cannot set q on wildcard Contact"),
        }
    }
    
    /// Gets the tag parameter value.
    /// Returns None for wildcard contacts.
    pub fn tag(&self) -> Option<&str> {
        match &self.0 {
            ContactValue::Address(addr) => addr.tag(),
            ContactValue::Wildcard => None,
        }
    }
    
    /// Sets or replaces the tag parameter.
    /// Panics if called on a wildcard contact.
    pub fn set_tag(&mut self, tag: impl Into<String>) {
        match &mut self.0 {
            ContactValue::Address(addr) => addr.set_tag(tag),
            ContactValue::Wildcard => panic!("Cannot set tag on wildcard Contact"),
        }
    }

    /// Checks if this Contact represents the wildcard (*).
    pub fn is_wildcard(&self) -> bool {
        matches!(self.0, ContactValue::Wildcard)
    }
    
    /// Returns the underlying Address if this is not a wildcard contact.
    pub fn address(&self) -> Option<&Address> {
        match &self.0 {
            ContactValue::Address(addr) => Some(addr),
            ContactValue::Wildcard => None,
        }
    }
}

impl fmt::Display for Contact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ContactValue::Address(addr) => write!(f, "{}", addr), // Delegate to Address display
            ContactValue::Wildcard => write!(f, "*"),
        }
    }
}

impl FromStr for Contact {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> {
        let trimmed = s.trim();
        if trimmed == "*" {
            Ok(Contact::new_wildcard())
        } else {
            // Assumes parse_contact returns Vec<Address>, we take the first.
            parse_contact(trimmed)?
                .into_iter()
                .next()
                .map(Contact::new_address) // Use new_address constructor
                .ok_or_else(|| crate::error::Error::Parser("No valid contact value found".into()))
        }
    }
}

// Remove Deref implementation as it no longer directly wraps Address
/*
impl Deref for Contact {
    type Target = Address;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
*/

// TODO: Implement specific Contact logic/helpers (e.g., getting expires, q) 
use crate::types::address::Address;
// use crate::types::Param; // Removed duplicate import
use std::fmt;
use std::str::FromStr;
use crate::error::Result;
use crate::parser::headers::parse_contact; // For FromStr
use std::ops::Deref;
use crate::types::param::Param;
use ordered_float::NotNan;

/// Typed Contact header.
/// Note: RFC 3261 allows multiple Contact values in a single header line (comma-separated)
/// or multiple Contact header lines. This struct represents a SINGLE Contact value.
/// Use Vec<Contact> or similar for multiple contacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact(pub Address);

impl Contact {
    /// Creates a new Contact header.
    pub fn new(address: Address) -> Self {
        Self(address)
    }

    /// Gets the expires parameter value, if present.
    pub fn expires(&self) -> Option<u32> {
        self.0.params.iter().find_map(|p| match p {
            Param::Expires(val) => Some(*val),
            _ => None,
        })
    }
    
    /// Sets or replaces the expires parameter.
    pub fn set_expires(&mut self, expires: u32) {
        self.0.params.retain(|p| !matches!(p, Param::Expires(_)));
        self.0.params.push(Param::Expires(expires));
    }

    /// Gets the q parameter value, if present.
    pub fn q(&self) -> Option<NotNan<f32>> {
        self.0.params.iter().find_map(|p| match p {
            Param::Q(val) => Some(val),
            _ => None,
        }).copied()
    }
    
    /// Sets or replaces the q parameter.
    pub fn set_q(&mut self, q: f32) {
        let clamped_q = q.max(0.0).min(1.0);
        self.0.params.retain(|p| !matches!(p, Param::Q(_)));
        self.0.params.push(Param::Q(NotNan::try_from(clamped_q).expect("Clamped q value should not be NaN")));
    }
    
    // Delegate other Address methods if needed (e.g., tag)
    pub fn tag(&self) -> Option<&str> {
        self.0.tag()
    }
    
    pub fn set_tag(&mut self, tag: impl Into<String>) {
        self.0.set_tag(tag)
    }

    /// Checks if this Contact represents the wildcard (*).
    pub fn is_wildcard(&self) -> bool {
        self.0.uri.to_string() == "*" // Simplistic check, might need refinement if URI struct changes
    }
}

impl fmt::Display for Contact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Special case for wildcard contact
        if self.is_wildcard() {
            write!(f, "*")
        } else {
            write!(f, "{}", self.0) // Delegate to Address display
        }
    }
}

impl FromStr for Contact {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> {
        if s.trim() == "*" {
            // Handle wildcard contact
            Ok(Contact::new(Address::new(None::<String>, crate::uri::Uri::from_str("*").unwrap()))) // Add ::<String>
        } else {
            // Assumes parse_contact returns Vec<Address>, we take the first.
            // This might need adjustment if parse_contact changes or if we want to enforce single-value parsing here.
            parse_contact(s)?
                .into_iter()
                .next()
                .map(Contact::new)
                .ok_or_else(|| crate::error::Error::Parser("No valid contact value found".into()))
        }
    }
}

impl Deref for Contact {
    type Target = Address;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// TODO: Implement specific Contact logic/helpers (e.g., getting expires, q) 
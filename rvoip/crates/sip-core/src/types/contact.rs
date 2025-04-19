use crate::types::address::Address;
use crate::types::Param;
use std::fmt;
use std::str::FromStr;
use crate::error::Result;
use crate::parser::headers::parse_contact; // For FromStr

/// Typed Contact header.
#[derive(Debug, Clone, PartialEq, Eq)] // Add derives as needed
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
    pub fn q(&self) -> Option<f32> {
        self.0.params.iter().find_map(|p| match p {
            Param::Q(val) => Some(*val),
            _ => None,
        })
    }
    
    /// Sets or replaces the q parameter.
    pub fn set_q(&mut self, q: f32) {
        // Clamp q value between 0.0 and 1.0
        let clamped_q = q.max(0.0).min(1.0);
        self.0.params.retain(|p| !matches!(p, Param::Q(_)));
        self.0.params.push(Param::Q(clamped_q));
    }
    
    // Delegate other Address methods if needed (e.g., tag)
    pub fn tag(&self) -> Option<&str> {
        self.0.tag()
    }
    
    pub fn set_tag(&mut self, tag: impl Into<String>) {
        self.0.set_tag(tag)
    }
}

// TODO: Implement Display and FromStr for Contact

// TODO: Implement specific Contact logic/helpers (e.g., getting expires, q) 
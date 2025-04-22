use crate::types::address::Address;
// use crate::types::Param; // Removed duplicate import
use std::fmt;
use std::str::FromStr;
use crate::error::Result;
use crate::parser::headers::parse_contact; // For FromStr
use std::ops::Deref;
use crate::types::param::Param;
use ordered_float::NotNan;
use serde::{Serialize, Deserialize};

/// Represents a single parsed contact-param item (address + params)
/// Used by the parser and the updated ContactValue enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactParamInfo {
    pub address: Address, // Contains URI, display name, and address parameters
}

/// Represents the value within a Contact header, aligning with parser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactValue {
    /// The wildcard value "*".
    Star,
    /// One or more contact-param values (name-addr/addr-spec with parameters).
    Params(Vec<ContactParamInfo>),
}

/// Typed Contact header.
/// Represents the *entire* header value, which could be STAR or a list of contacts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact(pub Vec<ContactValue>);

impl Contact {
    /// Creates a new Contact header from a list of ContactParamInfo.
    pub fn new_params(params: Vec<ContactParamInfo>) -> Self {
        if params.is_empty() {
            // RFC allows empty Contact header, but usually implies registration removal.
            // Representing as empty Params list might be okay, or maybe a dedicated Empty variant?
            // Sticking with empty Params for now.
            println!("Warning: Creating Contact with empty parameter list.");
        }
        Self(vec![ContactValue::Params(params)])
    }

    /// Creates a new wildcard Contact header.
    pub fn new_star() -> Self {
        Self(vec![ContactValue::Star])
    }

    /// Gets the first Address from the Params variant, if present.
    /// Useful for single-valued Contact headers.
    /// Returns None for wildcard contacts or empty Params list.
    pub fn address(&self) -> Option<&Address> {
        self.0.first().and_then(|value| match value {
            ContactValue::Params(params) => params.first().map(|cp| &cp.address),
            ContactValue::Star => None,
        })
    }

    /// Gets a mutable reference to the first Address from the Params variant.
    /// Returns None for wildcard contacts or empty Params list.
    pub fn address_mut(&mut self) -> Option<&mut Address> {
        self.0.first_mut().and_then(|value| match value {
            ContactValue::Params(params) => params.first_mut().map(|cp| &mut cp.address),
            ContactValue::Star => None,
        })
    }

    /// Gets all addresses from the Params variant.
    /// Returns an empty iterator for wildcard contacts or empty lists.
    pub fn addresses(&self) -> impl Iterator<Item = &Address> {
        // Use Box to erase the specific Map type from each arm
        self.0.iter().flat_map(|value| -> Box<dyn Iterator<Item = &Address>> { 
            match value {
                ContactValue::Params(params) => Box::new(params.iter().map(|cp| &cp.address)),
                ContactValue::Star => Box::new(std::iter::empty()), // Use std::iter::empty for efficiency
            }
        })
    }
    
     /// Gets mutable references to all addresses from the Params variant.
    /// Returns an empty iterator for wildcard contacts or empty lists.
    pub fn addresses_mut(&mut self) -> impl Iterator<Item = &mut Address> {
        // Use Box to erase the specific Map type from each arm
        self.0.iter_mut().flat_map(|value| -> Box<dyn Iterator<Item = &mut Address>> { 
            match value {
                ContactValue::Params(params) => Box::new(params.iter_mut().map(|cp| &mut cp.address)),
                ContactValue::Star => Box::new(std::iter::empty()), // Use std::iter::empty
            }
        })
    }

    /// Gets the expires parameter value from the *first* contact, if present.
    pub fn expires(&self) -> Option<u32> {
        self.address().and_then(|addr| addr.get_param("expires"))
            .flatten()
            .and_then(|s| s.parse::<u32>().ok())
    }
    
    /// Sets or replaces the expires parameter on the *first* contact.
    /// Adds the first contact if the list is empty.
    /// Panics if called on a Star contact.
    pub fn set_expires(&mut self, expires: u32) {
        if let Some(value) = self.0.first_mut() {
            match value {
                ContactValue::Params(params) => {
                    if let Some(cp_info) = params.first_mut() {
                        cp_info.address.set_param("expires", Some(expires.to_string()));
                    } else {
                        // Handle case where Params list is empty? This seems unlikely for set_expires.
                        panic!("Cannot set expires on an empty Contact parameter list");
                    }
                },
                ContactValue::Star => panic!("Cannot set expires on star Contact"),
            }
        }
    }

    /// Gets the q parameter value from the *first* contact, if present.
    pub fn q(&self) -> Option<NotNan<f32>> {
        self.address().and_then(|addr| addr.get_param("q"))
            .flatten()
            .and_then(|s| s.parse::<f32>().ok())
            .and_then(|f| NotNan::new(f).ok())
    }
    
    /// Sets or replaces the q parameter on the *first* contact.
    /// Panics if called on a Star contact or if list is empty.
    pub fn set_q(&mut self, q: f32) {
        let clamped_q = q.max(0.0).min(1.0);
        if clamped_q.is_nan() { panic!("q value cannot be NaN"); }
        let q_value_str = clamped_q.to_string();
        
        if let Some(value) = self.0.first_mut() {
            match value {
                ContactValue::Params(params) => {
                    if let Some(cp_info) = params.first_mut() {
                        cp_info.address.set_param("q", Some(q_value_str));
                    } else {
                        panic!("Cannot set q on an empty Contact parameter list");
                    }
                },
                ContactValue::Star => panic!("Cannot set q on star Contact"),
            }
        }
    }
    
    /// Gets the tag parameter value from the *first* contact.
    pub fn tag(&self) -> Option<&str> {
        self.address().and_then(|addr| addr.tag())
    }
    
    /// Sets or replaces the tag parameter on the *first* contact.
    /// Panics if called on a Star contact or if list is empty.
    pub fn set_tag(&mut self, tag: impl Into<String>) {
        if let Some(value) = self.0.first_mut() {
            match value {
                ContactValue::Params(params) => {
                    if let Some(addr) = params.first_mut() {
                        addr.address.set_tag(tag);
                    } else {
                        panic!("Cannot set tag on an empty Contact parameter list");
                    }
                },
                ContactValue::Star => panic!("Cannot set tag on star Contact"),
            }
        }
    }

    /// Checks if this Contact represents the star (*).
    pub fn is_star(&self) -> bool {
        self.0.iter().any(|value| matches!(value, ContactValue::Star))
    }
}

impl fmt::Display for Contact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for value in &self.0 {
            match value {
                ContactValue::Params(params) => {
                    if !first {
                        write!(f, ", ")?;
                    }
                    for cp in params {
                        write!(f, "{}", cp.address)?;
                    }
                    first = false;
                }
                ContactValue::Star => write!(f, "*")?,
            }
        }
        Ok(())
    }
}

impl FromStr for Contact {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> {
        use nom::combinator::all_consuming;
        // The parser `parse_contact` returns a ContactValue, we need to wrap it in a Vec
        match all_consuming(crate::parser::headers::contact::parse_contact)(s.as_bytes()) {
            // Wrap the ContactValue in a Vec for the Contact constructor
            Ok((_, value)) => Ok(Contact(vec![value])),
            Err(e) => Err(crate::error::Error::from(e)), // Use From trait
        }
    }
}

// Remove Deref as Contact no longer directly wraps Address/Wildcard

// TODO: Review if ContactParamInfo is the best structure or if Address is sufficient.
// TODO: Re-evaluate handling of empty Contact header. 
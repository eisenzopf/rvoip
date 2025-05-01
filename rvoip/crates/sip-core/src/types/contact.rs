//! # SIP Contact Header
//! 
//! This module provides an implementation of the SIP Contact header as defined in
//! [RFC 3261 Section 20.10](https://datatracker.ietf.org/doc/html/rfc3261#section-20.10).
//!
//! The Contact header is used in various SIP messages to convey URI(s) at which a user
//! can be contacted directly. Its main purposes include:
//!
//! - In REGISTER requests, it indicates where the user can be reached
//! - In INVITE requests, it indicates where the caller can be reached
//! - In responses to INVITE, it indicates where the callee can be reached
//! - In 3xx responses, it provides alternative locations for the request
//!
//! ## Format
//!
//! The Contact header can take two main forms:
//!
//! 1. A list of name-address or address specs with parameters:
//!
//! ```text
//! Contact: "John Doe" <sip:john@example.com>;expires=3600;q=0.7,
//!          <sip:jane@example.com>;q=0.5
//! ```
//!
//! 2. A wildcard (*), typically used in REGISTER requests to remove all registrations:
//!
//! ```text
//! Contact: *
//! ```
//!
//! ## Examples
//!
//! ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a Contact with an address
/// let uri = Uri::from_str("sip:john@example.com").unwrap();
/// let address = Address::new_with_display_name("John Doe", uri);
/// let contact_info = ContactParamInfo { address };
/// let contact = Contact::new_params(vec![contact_info]);
///
/// // Create a wildcard Contact
/// let wildcard = Contact::new_star();
/// assert!(wildcard.is_star());
///
/// // Parse a Contact from a string
/// let contact = Contact::from_str("\"Alice\" <sip:alice@example.com>;expires=3600").unwrap();
/// assert_eq!(contact.expires(), Some(3600));
/// ```

use crate::types::address::Address;
// use crate::types::Param; // Removed duplicate import
use std::fmt;
use std::str::FromStr;
use crate::parser::headers::parse_contact; // For FromStr
use std::ops::Deref;
use crate::types::param::Param;
use ordered_float::NotNan;
use serde::{Serialize, Deserialize};
use crate::types::{Header, HeaderName, HeaderValue, TypedHeader, TypedHeaderTrait};
use crate::error::{Error, Result};

/// Represents a single parsed contact-param item (address + params)
/// Used by the parser and the updated ContactValue enum.
///
/// A `ContactParamInfo` contains an `Address` which includes the URI, 
/// display name, and address parameters for a contact entry.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a ContactParamInfo with an Address
/// let uri = Uri::from_str("sip:john@example.com").unwrap();
/// let address = Address::new_with_display_name("John Doe", uri);
/// let contact_info = ContactParamInfo { address };
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactParamInfo {
    /// The SIP address contained in this contact
    pub address: Address, // Contains URI, display name, and address parameters
}

/// Represents the value within a Contact header, aligning with parser.
///
/// A `ContactValue` can be either a wildcard "*" (used in registration removals)
/// or a list of `ContactParamInfo` entries.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a Star contact value
/// let star = ContactValue::Star;
///
/// // Create a Params contact value
/// let uri = Uri::from_str("sip:john@example.com").unwrap();
/// let address = Address::new_with_display_name("John Doe", uri);
/// let contact_info = ContactParamInfo { address };
/// let params = ContactValue::Params(vec![contact_info]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactValue {
    /// The wildcard value "*".
    Star,
    /// One or more contact-param values (name-addr/addr-spec with parameters).
    Params(Vec<ContactParamInfo>),
}

/// Typed Contact header.
/// Represents the *entire* header value, which could be STAR or a list of contacts.
///
/// The `Contact` header is a critical component of SIP messages that contains addresses 
/// where a user can be directly contacted. It's used in REGISTER requests to indicate 
/// where a user can be reached, in INVITE requests to specify the caller's address, 
/// and in responses to indicate the callee's location.
///
/// This implementation allows for both regular contacts (with addresses and parameters) 
/// and the special wildcard ("*") contact used for removing registrations.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a Contact with an address
/// let uri = Uri::from_str("sip:john@example.com").unwrap();
/// let address = Address::new_with_display_name("John Doe", uri);
/// let contact_info = ContactParamInfo { address };
/// let contact = Contact::new_params(vec![contact_info]);
///
/// // Create a wildcard Contact
/// let contact = Contact::new_star();
/// assert!(contact.is_star());
///
/// // Access an address
/// if let Some(address) = contact.address() {
///     // Work with the address
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact(pub Vec<ContactValue>);

impl Contact {
    /// Creates a new Contact header from a list of ContactParamInfo.
    ///
    /// This method creates a Contact header with one or more addresses. An empty
    /// list is allowed, which might be used for registration removal in some scenarios,
    /// but a warning will be logged.
    ///
    /// # Parameters
    ///
    /// - `params`: A vector of ContactParamInfo entries
    ///
    /// # Returns
    ///
    /// A new Contact header containing the specified addresses
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a Contact with an address
    /// let uri = Uri::from_str("sip:john@example.com").unwrap();
    /// let address = Address::new_with_display_name("John Doe", uri);
    /// let contact_info = ContactParamInfo { address };
    /// let contact = Contact::new_params(vec![contact_info]);
    ///
    /// // Create a Contact with multiple addresses
    /// let uri1 = Uri::from_str("sip:john@example.com").unwrap();
    /// let uri2 = Uri::from_str("sip:john@mobile.example.com").unwrap();
    /// let address1 = Address::new_with_display_name("John Doe", uri1);
    /// let address2 = Address::new_with_display_name("John Doe Mobile", uri2);
    /// let contact_info1 = ContactParamInfo { address: address1 };
    /// let contact_info2 = ContactParamInfo { address: address2 };
    /// let contact = Contact::new_params(vec![contact_info1, contact_info2]);
    /// ```
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
    ///
    /// The wildcard Contact ("*") is typically used in REGISTER requests
    /// to remove all existing registrations for a user.
    ///
    /// # Returns
    ///
    /// A new Contact header with the wildcard value
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a wildcard Contact
    /// let contact = Contact::new_star();
    /// assert!(contact.is_star());
    /// assert_eq!(contact.to_string(), "*");
    /// ```
    pub fn new_star() -> Self {
        Self(vec![ContactValue::Star])
    }

    /// Gets the first Address from the Params variant, if present.
    /// Useful for single-valued Contact headers.
    /// Returns None for wildcard contacts or empty Params list.
    ///
    /// # Returns
    ///
    /// An Option containing a reference to the first Address if present,
    /// or None if the Contact is a wildcard or has an empty parameter list
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a Contact with an address
    /// let uri = Uri::from_str("sip:john@example.com").unwrap();
    /// let address = Address::new_with_display_name("John Doe", uri);
    /// let contact_info = ContactParamInfo { address };
    /// let contact = Contact::new_params(vec![contact_info]);
    ///
    /// // Access the address
    /// if let Some(addr) = contact.address() {
    ///     assert_eq!(addr.uri.to_string(), "sip:john@example.com");
    /// }
    ///
    /// // Wildcard Contact has no address
    /// let wildcard = Contact::new_star();
    /// assert!(wildcard.address().is_none());
    /// ```
    pub fn address(&self) -> Option<&Address> {
        self.0.first().and_then(|value| match value {
            ContactValue::Params(params) => params.first().map(|cp| &cp.address),
            ContactValue::Star => None,
        })
    }

    /// Gets a mutable reference to the first Address from the Params variant.
    /// Returns None for wildcard contacts or empty Params list.
    ///
    /// # Returns
    ///
    /// An Option containing a mutable reference to the first Address if present,
    /// or None if the Contact is a wildcard or has an empty parameter list
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a Contact with an address
    /// let uri = Uri::from_str("sip:john@example.com").unwrap();
    /// let address = Address::new_with_display_name("John Doe", uri);
    /// let contact_info = ContactParamInfo { address };
    /// let mut contact = Contact::new_params(vec![contact_info]);
    ///
    /// // Modify the address
    /// if let Some(addr) = contact.address_mut() {
    ///     addr.set_param("expires", Some("3600"));
    /// }
    /// ```
    pub fn address_mut(&mut self) -> Option<&mut Address> {
        self.0.first_mut().and_then(|value| match value {
            ContactValue::Params(params) => params.first_mut().map(|cp| &mut cp.address),
            ContactValue::Star => None,
        })
    }

    /// Gets all addresses from the Params variant.
    /// Returns an empty iterator for wildcard contacts or empty lists.
    ///
    /// # Returns
    ///
    /// An iterator over references to all addresses in the Contact header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a Contact with multiple addresses
    /// let uri1 = Uri::from_str("sip:john@example.com").unwrap();
    /// let uri2 = Uri::from_str("sip:john@mobile.example.com").unwrap();
    /// let address1 = Address::new_with_display_name("John Doe", uri1);
    /// let address2 = Address::new_with_display_name("John Doe Mobile", uri2);
    /// let contact_info1 = ContactParamInfo { address: address1 };
    /// let contact_info2 = ContactParamInfo { address: address2 };
    /// let contact = Contact::new_params(vec![contact_info1, contact_info2]);
    ///
    /// // Iterate through all addresses
    /// let addresses: Vec<&Address> = contact.addresses().collect();
    /// assert_eq!(addresses.len(), 2);
    /// ```
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
    ///
    /// # Returns
    ///
    /// An iterator over mutable references to all addresses in the Contact header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a Contact with multiple addresses
    /// let uri1 = Uri::from_str("sip:john@example.com").unwrap();
    /// let uri2 = Uri::from_str("sip:john@mobile.example.com").unwrap();
    /// let address1 = Address::new_with_display_name("John Doe", uri1);
    /// let address2 = Address::new_with_display_name("John Doe Mobile", uri2);
    /// let contact_info1 = ContactParamInfo { address: address1 };
    /// let contact_info2 = ContactParamInfo { address: address2 };
    /// let mut contact = Contact::new_params(vec![contact_info1, contact_info2]);
    ///
    /// // Modify all addresses
    /// for addr in contact.addresses_mut() {
    ///     addr.set_param("transport", Some("tcp"));
    /// }
    /// ```
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
    ///
    /// The expires parameter indicates how long (in seconds) the contact address
    /// is valid. This is particularly important in registration scenarios.
    ///
    /// # Returns
    ///
    /// An Option containing the expires value in seconds, or None if not present
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a Contact with expires parameter
    /// let contact = Contact::from_str("<sip:john@example.com>;expires=3600").unwrap();
    /// assert_eq!(contact.expires(), Some(3600));
    ///
    /// // Contact without expires parameter
    /// let contact = Contact::from_str("<sip:john@example.com>").unwrap();
    /// assert_eq!(contact.expires(), None);
    /// ```
    pub fn expires(&self) -> Option<u32> {
        self.address().and_then(|addr| addr.get_param("expires"))
            .flatten()
            .and_then(|s| s.parse::<u32>().ok())
    }
    
    /// Sets or replaces the expires parameter on the *first* contact.
    /// Adds the first contact if the list is empty.
    /// Panics if called on a Star contact.
    ///
    /// # Parameters
    ///
    /// - `expires`: The expiration time in seconds
    ///
    /// # Panics
    ///
    /// Panics if called on a Star contact or an empty Params list
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a Contact and set expires
    /// let uri = Uri::from_str("sip:john@example.com").unwrap();
    /// let address = Address::new(uri);
    /// let contact_info = ContactParamInfo { address };
    /// let mut contact = Contact::new_params(vec![contact_info]);
    ///
    /// contact.set_expires(3600);
    /// assert_eq!(contact.expires(), Some(3600));
    /// ```
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
    ///
    /// The q parameter indicates a relative preference for this contact
    /// compared to other contacts. It's a floating point value between 0 and 1,
    /// with higher values indicating higher preference.
    ///
    /// # Returns
    ///
    /// An Option containing the q-value, or None if not present
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a Contact with q parameter
    /// let contact = Contact::from_str("<sip:john@example.com>;q=0.8").unwrap();
    /// assert_eq!(contact.q().map(|v| v.into_inner()), Some(0.8));
    /// ```
    pub fn q(&self) -> Option<NotNan<f32>> {
        self.address().and_then(|addr| addr.get_param("q"))
            .flatten()
            .and_then(|s| s.parse::<f32>().ok())
            .and_then(|f| NotNan::new(f).ok())
    }
    
    /// Sets or replaces the q parameter on the *first* contact.
    /// Panics if called on a Star contact or if list is empty.
    ///
    /// The q parameter indicates a relative preference for this contact
    /// compared to other contacts. Values are clamped between 0 and 1,
    /// with higher values indicating higher preference.
    ///
    /// # Parameters
    ///
    /// - `q`: The q-value (clamped between 0.0 and 1.0)
    ///
    /// # Panics
    ///
    /// Panics if called on a Star contact, if the Params list is empty, 
    /// or if the provided value is NaN
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a Contact and set q value
    /// let uri = Uri::from_str("sip:john@example.com").unwrap();
    /// let address = Address::new(uri);
    /// let contact_info = ContactParamInfo { address };
    /// let mut contact = Contact::new_params(vec![contact_info]);
    ///
    /// // Set q value (will be clamped to range 0.0-1.0)
    /// contact.set_q(0.8);
    /// assert_eq!(contact.q().map(|v| v.into_inner()), Some(0.8));
    ///
    /// // Values outside the range are clamped
    /// contact.set_q(1.5);  // Will be clamped to 1.0
    /// assert_eq!(contact.q().map(|v| v.into_inner()), Some(1.0));
    /// ```
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
    ///
    /// # Returns
    ///
    /// An Option containing the tag value as a string slice, or None if not present
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a Contact with tag parameter
    /// let contact = Contact::from_str("<sip:john@example.com>;tag=1234").unwrap();
    /// assert_eq!(contact.tag(), Some("1234"));
    /// ```
    pub fn tag(&self) -> Option<&str> {
        self.address().and_then(|addr| addr.tag())
    }
    
    /// Sets or replaces the tag parameter on the *first* contact.
    /// Panics if called on a Star contact or if list is empty.
    ///
    /// # Parameters
    ///
    /// - `tag`: The tag value to set
    ///
    /// # Panics
    ///
    /// Panics if called on a Star contact or if the Params list is empty
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a Contact and set tag
    /// let uri = Uri::from_str("sip:john@example.com").unwrap();
    /// let address = Address::new(uri);
    /// let contact_info = ContactParamInfo { address };
    /// let mut contact = Contact::new_params(vec![contact_info]);
    ///
    /// contact.set_tag("1234abcd");
    /// assert_eq!(contact.tag(), Some("1234abcd"));
    /// ```
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
    ///
    /// The star Contact is used in REGISTER requests to remove
    /// all existing registrations for a user.
    ///
    /// # Returns
    ///
    /// `true` if this is a wildcard Contact, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Check a wildcard Contact
    /// let star = Contact::new_star();
    /// assert!(star.is_star());
    ///
    /// // Regular Contact is not a star
    /// let contact = Contact::from_str("<sip:john@example.com>").unwrap();
    /// assert!(!contact.is_star());
    /// ```
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

    /// Parse a string into a Contact header.
    ///
    /// This method parses a string representation of a Contact header into a
    /// Contact struct. The string can be either a wildcard "*" or one or more
    /// name-addr or addr-spec entries with parameters.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed Contact, or an error if parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a wildcard Contact
    /// let contact = Contact::from_str("*").unwrap();
    /// assert!(contact.is_star());
    ///
    /// // Parse a regular Contact
    /// let contact = Contact::from_str("\"John Doe\" <sip:john@example.com>;expires=3600").unwrap();
    /// assert_eq!(contact.expires(), Some(3600));
    ///
    /// // Parse a Contact with multiple entries
    /// let contact = Contact::from_str(
    ///     "<sip:john@example.com>;q=0.8, <sip:john@mobile.example.com>;q=0.5"
    /// ).unwrap();
    /// 
    /// // The first contact should have q=0.8
    /// assert_eq!(contact.q().map(|v| v.into_inner()), Some(0.8));
    /// ```
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

// Add TypedHeaderTrait implementation for Contact header
impl TypedHeaderTrait for Contact {
    type Name = HeaderName;

    /// Returns the header name for this header type.
    ///
    /// # Returns
    ///
    /// The `HeaderName::Contact` enum variant
    fn header_name() -> Self::Name {
        HeaderName::Contact
    }

    /// Converts this Contact header into a generic Header.
    ///
    /// Creates a Header instance from this Contact header, which can be used
    /// when constructing SIP messages.
    ///
    /// # Returns
    ///
    /// A Header instance representing this Contact header
    fn to_header(&self) -> Header {
        // If we have a star contact, create a special *
        if self.is_star() {
            return Header::text(Self::header_name(), "*");
        }
        
        // Otherwise use the normal Contact value
        let contact_values: Vec<ContactValue> = self.0.iter().cloned().collect();
        Header::new(Self::header_name(), HeaderValue::Contact(contact_values[0].clone()))
    }

    /// Creates a Contact header from a generic Header.
    ///
    /// Attempts to parse and convert a generic Header into a Contact header.
    /// This will succeed if the header is a valid Contact header.
    ///
    /// # Parameters
    ///
    /// - `header`: The generic Header to convert
    ///
    /// # Returns
    ///
    /// A Result containing the parsed Contact header if successful, or an error otherwise
    fn from_header(header: &Header) -> Result<Self> {
        if header.name != HeaderName::Contact {
            return Err(Error::InvalidHeader(format!(
                "Expected Contact header, got {:?}", header.name
            )));
        }

        // Check for special * value
        if let HeaderValue::Raw(bytes) = &header.value {
            if let Ok(s) = std::str::from_utf8(bytes) {
                if s.trim() == "*" {
                    return Ok(Contact::new_star());
                }
            }
        }

        // Try to use the pre-parsed value if available
        if let HeaderValue::Contact(value) = &header.value {
            return Ok(Contact(vec![value.clone()]));
        }

        // Otherwise parse from raw value
        match &header.value {
            HeaderValue::Raw(bytes) => {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    s.parse::<Contact>()
                } else {
                    Err(Error::ParseError("Invalid UTF-8 in Contact header".to_string()))
                }
            },
            _ => Err(Error::InvalidHeader(format!(
                "Unexpected value type for Contact header: {:?}", header.value
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::uri::Uri;
    use crate::types::address::Address;
    use std::str::FromStr;

    #[test]
    fn test_contact_typed_header_trait() {
        // Create a Contact header with an address
        let uri = Uri::from_str("sip:alice@192.168.1.1:5060").unwrap();
        let address = Address::new_with_display_name("Alice", uri);
        let contact_info = ContactParamInfo { address };
        let contact = Contact::new_params(vec![contact_info]);

        // Test header_name()
        assert_eq!(Contact::header_name(), HeaderName::Contact);

        // Test to_header()
        let header = contact.to_header();
        assert_eq!(header.name, HeaderName::Contact);

        // Test from_header()
        let round_trip = Contact::from_header(&header).unwrap();
        assert_eq!(round_trip.to_string(), contact.to_string());
        
        // Test star contact
        let star_contact = Contact::new_star();
        let star_header = star_contact.to_header();
        let star_round_trip = Contact::from_header(&star_header).unwrap();
        assert!(star_round_trip.is_star());
    }
} 
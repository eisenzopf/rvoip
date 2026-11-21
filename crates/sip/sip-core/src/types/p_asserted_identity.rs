//! # SIP P-Asserted-Identity Header
//!
//! [RFC 3325 §9.1](https://datatracker.ietf.org/doc/html/rfc3325#section-9.1).
//!
//! ```text
//! PAssertedID       = "P-Asserted-Identity" HCOLON PAssertedID-value
//!                       *(COMMA PAssertedID-value)
//! PAssertedID-value = name-addr / addr-spec
//! ```
//!
//! Used by trusted SIP intermediaries (carriers, PBX trunks) to convey the
//! verified identity of the originating user. Typical usage carries one
//! `sip:` URI and optionally one `tel:` URI representing the same asserted
//! identity.

use crate::error::{Error, Result};
use crate::types::address::Address;
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};
use crate::types::uri::Uri;
use nom::combinator::all_consuming;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::Deref;
use std::str::FromStr;

/// `P-Asserted-Identity` header (RFC 3325).
///
/// Holds one or more `Address` entries; carriers usually populate a single
/// `sip:` URI, sometimes paired with a matching `tel:` URI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PAssertedIdentity(pub Vec<Address>);

impl PAssertedIdentity {
    /// Create from an existing list of addresses.
    pub fn new(list: Vec<Address>) -> Self {
        Self(list)
    }

    /// Create an empty header (no entries — invalid per RFC 3325 but useful
    /// when building incrementally).
    pub fn empty() -> Self {
        Self(Vec::new())
    }

    /// Create with a single address entry.
    pub fn with_address(address: Address) -> Self {
        Self(vec![address])
    }

    /// Create with a single URI (no display name).
    pub fn with_uri(uri: Uri) -> Self {
        Self(vec![Address::new(uri)])
    }

    /// Whether this header has no entries.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of asserted-identity entries.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// First entry, if any.
    pub fn first(&self) -> Option<&Address> {
        self.0.first()
    }

    /// Append an address entry.
    pub fn add_address(&mut self, address: Address) -> &mut Self {
        self.0.push(address);
        self
    }

    /// Append a URI as an entry (no display name).
    pub fn add_uri(&mut self, uri: Uri) -> &mut Self {
        self.0.push(Address::new(uri));
        self
    }

    /// Iterate over the entries.
    pub fn iter(&self) -> impl Iterator<Item = &Address> {
        self.0.iter()
    }
}

impl fmt::Display for PAssertedIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for addr in &self.0 {
            if !first {
                write!(f, ", ")?;
            }
            write!(f, "{}", addr)?;
            first = false;
        }
        Ok(())
    }
}

impl FromStr for PAssertedIdentity {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        if s.trim().is_empty() {
            return Ok(PAssertedIdentity::empty());
        }
        let bytes = s.as_bytes();
        match all_consuming(
            crate::parser::headers::p_asserted_identity::parse_p_asserted_identity_value,
        )(bytes)
        {
            Ok((_, list)) => Ok(PAssertedIdentity(list)),
            Err(_) => Err(Error::ParseError(format!(
                "Failed to parse P-Asserted-Identity: {}",
                s
            ))),
        }
    }
}

impl Deref for PAssertedIdentity {
    type Target = Vec<Address>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> IntoIterator for &'a PAssertedIdentity {
    type Item = &'a Address;
    type IntoIter = std::slice::Iter<'a, Address>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl From<Address> for PAssertedIdentity {
    fn from(address: Address) -> Self {
        Self::with_address(address)
    }
}

impl From<Uri> for PAssertedIdentity {
    fn from(uri: Uri) -> Self {
        Self::with_uri(uri)
    }
}

impl TypedHeaderTrait for PAssertedIdentity {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::PAssertedIdentity
    }

    fn to_header(&self) -> Header {
        Header::new(
            Self::header_name(),
            HeaderValue::Raw(self.to_string().into_bytes()),
        )
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(format!(
                "Expected {} header, got {}",
                Self::header_name(),
                header.name
            )));
        }
        match &header.value {
            HeaderValue::Raw(bytes) => match std::str::from_utf8(bytes) {
                Ok(s) => PAssertedIdentity::from_str(s.trim()),
                Err(_) => Err(Error::InvalidHeader(format!(
                    "Invalid UTF-8 in {} header",
                    Self::header_name()
                ))),
            },
            _ => Err(Error::InvalidHeader(format!(
                "Unexpected header value type for {}",
                Self::header_name()
            ))),
        }
    }
}

/// `P-Preferred-Identity` (RFC 3325 §9.2).
///
/// Same wire format as `P-Asserted-Identity`; sent by a UAC towards a
/// trusted intermediary to express the identity it would prefer to assert.
/// The intermediary either honours it (emitting a matching PAI on the
/// outbound leg) or rejects with 403.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PPreferredIdentity(pub Vec<Address>);

impl PPreferredIdentity {
    pub fn new(list: Vec<Address>) -> Self {
        Self(list)
    }

    pub fn empty() -> Self {
        Self(Vec::new())
    }

    pub fn with_address(address: Address) -> Self {
        Self(vec![address])
    }

    pub fn with_uri(uri: Uri) -> Self {
        Self(vec![Address::new(uri)])
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn first(&self) -> Option<&Address> {
        self.0.first()
    }

    pub fn add_address(&mut self, address: Address) -> &mut Self {
        self.0.push(address);
        self
    }

    pub fn add_uri(&mut self, uri: Uri) -> &mut Self {
        self.0.push(Address::new(uri));
        self
    }

    pub fn iter(&self) -> impl Iterator<Item = &Address> {
        self.0.iter()
    }
}

impl fmt::Display for PPreferredIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for addr in &self.0 {
            if !first {
                write!(f, ", ")?;
            }
            write!(f, "{}", addr)?;
            first = false;
        }
        Ok(())
    }
}

impl FromStr for PPreferredIdentity {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        if s.trim().is_empty() {
            return Ok(PPreferredIdentity::empty());
        }
        let bytes = s.as_bytes();
        match all_consuming(
            crate::parser::headers::p_asserted_identity::parse_p_asserted_identity_value,
        )(bytes)
        {
            Ok((_, list)) => Ok(PPreferredIdentity(list)),
            Err(_) => Err(Error::ParseError(format!(
                "Failed to parse P-Preferred-Identity: {}",
                s
            ))),
        }
    }
}

impl Deref for PPreferredIdentity {
    type Target = Vec<Address>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> IntoIterator for &'a PPreferredIdentity {
    type Item = &'a Address;
    type IntoIter = std::slice::Iter<'a, Address>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl From<Address> for PPreferredIdentity {
    fn from(address: Address) -> Self {
        Self::with_address(address)
    }
}

impl From<Uri> for PPreferredIdentity {
    fn from(uri: Uri) -> Self {
        Self::with_uri(uri)
    }
}

impl TypedHeaderTrait for PPreferredIdentity {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::PPreferredIdentity
    }

    fn to_header(&self) -> Header {
        Header::new(
            Self::header_name(),
            HeaderValue::Raw(self.to_string().into_bytes()),
        )
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(format!(
                "Expected {} header, got {}",
                Self::header_name(),
                header.name
            )));
        }
        match &header.value {
            HeaderValue::Raw(bytes) => match std::str::from_utf8(bytes) {
                Ok(s) => PPreferredIdentity::from_str(s.trim()),
                Err(_) => Err(Error::InvalidHeader(format!(
                    "Invalid UTF-8 in {} header",
                    Self::header_name()
                ))),
            },
            _ => Err(Error::InvalidHeader(format!(
                "Unexpected header value type for {}",
                Self::header_name()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pai_empty() {
        let pai = PAssertedIdentity::empty();
        assert!(pai.is_empty());
        assert_eq!(pai.to_string(), "");
    }

    #[test]
    fn pai_with_uri() {
        let uri = Uri::from_str("sip:alice@example.com").unwrap();
        let pai = PAssertedIdentity::with_uri(uri);
        assert_eq!(pai.len(), 1);
        assert_eq!(pai.to_string(), "<sip:alice@example.com>");
    }

    #[test]
    fn pai_with_display_name() {
        let uri = Uri::from_str("sip:alice@example.com").unwrap();
        let addr = Address::new_with_display_name("Alice", uri);
        let pai = PAssertedIdentity::with_address(addr);
        // Single-token display name renders unquoted per Address::Display.
        assert_eq!(pai.to_string(), "Alice <sip:alice@example.com>");
    }

    #[test]
    fn pai_two_uris_sip_and_tel() {
        let sip_uri = Uri::from_str("sip:alice@example.com").unwrap();
        let tel_uri = Uri::from_str("tel:+14155551234").unwrap();
        let mut pai = PAssertedIdentity::with_uri(sip_uri);
        pai.add_uri(tel_uri);
        assert_eq!(pai.len(), 2);
        let s = pai.to_string();
        assert!(s.contains("sip:alice@example.com"));
        assert!(s.contains("tel:+14155551234"));
    }

    #[test]
    fn pai_roundtrip_via_fromstr() {
        let input = "\"Alice\" <sip:alice@example.com>, <tel:+14155551234>";
        let pai = PAssertedIdentity::from_str(input).expect("parse");
        assert_eq!(pai.len(), 2);
        assert_eq!(pai[0].display_name(), Some("Alice"));
        // Round-trip via Display preserves both entries
        let printed = pai.to_string();
        assert!(printed.contains("Alice"));
        assert!(printed.contains("sip:alice@example.com"));
        assert!(printed.contains("tel:+14155551234"));
    }

    #[test]
    fn pai_typed_header_trait_roundtrip() {
        let uri = Uri::from_str("sip:alice@example.com").unwrap();
        let pai = PAssertedIdentity::with_uri(uri);
        let header = pai.to_header();
        assert_eq!(header.name, HeaderName::PAssertedIdentity);
        let back = PAssertedIdentity::from_header(&header).expect("back");
        assert_eq!(back, pai);
    }

    #[test]
    fn ppi_with_uri() {
        let uri = Uri::from_str("sip:bob@example.com").unwrap();
        let ppi = PPreferredIdentity::with_uri(uri);
        assert_eq!(ppi.len(), 1);
        assert_eq!(ppi.to_string(), "<sip:bob@example.com>");
        assert_eq!(ppi.to_header().name, HeaderName::PPreferredIdentity);
    }
}

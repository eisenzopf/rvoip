//! # SIP Replaces Header
//!
//! Implementation of the SIP Replaces header as defined in
//! [RFC 3891](https://datatracker.ietf.org/doc/html/rfc3891).
//!
//! The header names a single existing dialog that the INVITE carrying it is to
//! shut down and logically replace. It is the wire half of attended transfer:
//! the transferor puts the consultation dialog's identity into the `Refer-To`
//! URI of a REFER, and the transferee copies it onto the INVITE it then sends
//! to the transfer target.
//!
//! ## Matching, and the perspective flip
//!
//! RFC 3891 Section 3 is explicit about which side each tag belongs to:
//!
//! > The UAS matches the to-tag and from-tag parameters as if they were tags
//! > present in an incoming request. In other words, the to-tag parameter is
//! > compared to the local tag, and the from-tag parameter is compared to the
//! > remote tag.
//!
//! So the receiver reads `to-tag` as *its own* tag and `from-tag` as the peer's.
//! Getting this backwards is the classic implementation bug: it yields a
//! perfectly well formed lookup key that matches nothing, and the transfer
//! then fails as "no such dialog" rather than as an obvious defect.
//!
//! ## Format
//!
//! ```text
//! Replaces: 12adf2f34456gs5;to-tag=12345;from-tag=54321;early-only
//! Replaces: 87134@171.161.34.23;to-tag=24796;from-tag=0
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::types::replaces::Replaces;
//! use std::str::FromStr;
//!
//! let replaces = Replaces::from_str("call-abc;to-tag=t1;from-tag=f1")?;
//! assert_eq!(replaces.call_id, "call-abc");
//! assert_eq!(replaces.to_tag, "t1");
//! assert!(!replaces.early_only);
//! assert_eq!(replaces.to_string(), "call-abc;to-tag=t1;from-tag=f1");
//! # Ok::<(), rvoip_sip_core::Error>(())
//! ```

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};
use crate::types::param::Param;
use crate::{Error, Result};

/// Represents a parsed `Replaces` header (RFC 3891).
///
/// `to_tag` and `from_tag` are not optional because Section 6.1 requires
/// exactly one of each: "they are required for unique dialog matching". A
/// header missing or repeating either one fails to parse, so a value of this
/// type always identifies exactly one dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replaces {
    /// The Call-ID of the dialog to be replaced.
    pub call_id: String,
    /// The `to-tag` parameter, compared against the *local* tag of the
    /// receiving UA (RFC 3891 Section 3).
    pub to_tag: String,
    /// The `from-tag` parameter, compared against the *remote* tag of the
    /// receiving UA (RFC 3891 Section 3).
    pub from_tag: String,
    /// The `early-only` flag. When set, a match against a confirmed dialog
    /// must be rejected with 486 Busy Here rather than replaced.
    pub early_only: bool,
    /// Any remaining generic parameters, preserved verbatim.
    pub params: Vec<Param>,
}

impl Replaces {
    /// Create a `Replaces` value for the given dialog identity.
    pub fn new(
        call_id: impl Into<String>,
        to_tag: impl Into<String>,
        from_tag: impl Into<String>,
    ) -> Self {
        Self {
            call_id: call_id.into(),
            to_tag: to_tag.into(),
            from_tag: from_tag.into(),
            early_only: false,
            params: Vec::new(),
        }
    }

    /// Set the `early-only` flag.
    pub fn with_early_only(mut self, early_only: bool) -> Self {
        self.early_only = early_only;
        self
    }

    /// The dialog lookup key this header selects, from the perspective of the
    /// UA receiving it.
    ///
    /// The flip described in RFC 3891 Section 3 lives here so that callers
    /// cannot get the argument order wrong: `to-tag` is the receiver's local
    /// tag, `from-tag` is the remote one.
    pub fn as_local_remote_tags(&self) -> (&str, &str) {
        (&self.to_tag, &self.from_tag)
    }
}

impl fmt::Display for Replaces {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{};to-tag={};from-tag={}",
            self.call_id, self.to_tag, self.from_tag
        )?;
        if self.early_only {
            write!(f, ";early-only")?;
        }
        for param in &self.params {
            write!(f, "{}", param)?;
        }
        Ok(())
    }
}

impl FromStr for Replaces {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        crate::parser::headers::replaces::parse_replaces_value(s.trim().as_bytes())
            .map(|(_, replaces)| replaces)
            .map_err(|_| {
                Error::ParseError(format!(
                    "Invalid Replaces header value: {}. RFC 3891 requires a Call-ID with exactly one to-tag and one from-tag.",
                    s
                ))
            })
    }
}

impl TypedHeaderTrait for Replaces {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::Replaces
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(format!(
                "Expected header '{}' but got '{}'",
                Self::header_name().as_str(),
                header.name.as_str()
            )));
        }

        match &header.value {
            HeaderValue::Raw(raw_value) => {
                let text_value = std::str::from_utf8(raw_value).map_err(|e| {
                    Error::ParseError(format!("Invalid UTF-8 in Replaces raw value: {}", e))
                })?;
                Replaces::from_str(text_value.trim())
            }
            hv => Err(Error::Parser(format!(
                "Cannot parse Replaces from HeaderValue variant: {:?}. Expected Raw.",
                hv
            ))),
        }
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::text(self.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_display_and_from_str() {
        let original = Replaces::new("call-abc@host.test", "local-tag", "remote-tag");
        let rendered = original.to_string();
        assert_eq!(
            rendered,
            "call-abc@host.test;to-tag=local-tag;from-tag=remote-tag"
        );
        assert_eq!(Replaces::from_str(&rendered).expect("reparse"), original);
    }

    #[test]
    fn round_trips_with_early_only() {
        let original = Replaces::new("cid", "t1", "f1").with_early_only(true);
        assert_eq!(original.to_string(), "cid;to-tag=t1;from-tag=f1;early-only");
        assert_eq!(
            Replaces::from_str(&original.to_string()).expect("reparse"),
            original
        );
    }

    /// The to-tag belongs to the receiver, not the sender. Pinned because
    /// reversing it produces a well formed key that silently matches nothing.
    #[test]
    fn to_tag_is_the_receivers_local_tag() {
        let replaces = Replaces::new("cid", "charlie-tag", "bob-tag");
        assert_eq!(replaces.as_local_remote_tags(), ("charlie-tag", "bob-tag"));
    }

    #[test]
    fn round_trips_through_an_untyped_header() {
        let original = Replaces::new("cid@1.2.3.4:5060", "t1", "f1");
        let header = original.to_header();
        assert_eq!(header.name, HeaderName::Replaces);
        assert_eq!(
            Replaces::from_header(&header).expect("from_header"),
            original
        );
    }

    #[test]
    fn rejects_a_value_missing_a_tag() {
        assert!(Replaces::from_str("cid;to-tag=t1").is_err());
        assert!(Replaces::from_str("cid").is_err());
    }
}

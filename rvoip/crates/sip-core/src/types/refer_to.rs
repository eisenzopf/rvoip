use crate::types::address::Address; 
use crate::parser::headers::parse_refer_to;
use crate::error::{Error, Result};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use serde::{Deserialize, Serialize};

/// Represents a Refer-To header as defined in RFC 3515
/// 
/// The Refer-To header field is used in a REFER request to provide the
/// URI to reference, and in an INVITE to indicate the replacement target.
/// 
/// Syntax (RFC 3515):
/// Refer-To = "Refer-To" HCOLON (name-addr / addr-spec) *( SEMI refer-param )
/// refer-param = generic-param
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferTo(pub Address);

impl ReferTo {
    /// Creates a new ReferTo header.
    pub fn new(address: Address) -> Self {
        Self(address)
    }

    /// Access the underlying Address
    pub fn address(&self) -> &Address {
        &self.0
    }

    /// Access the URI from the Address
    pub fn uri(&self) -> &crate::types::uri::Uri {
        &self.0.uri
    }

    /// Access parameters from the Address
    pub fn params(&self) -> &[crate::types::param::Param] {
        &self.0.params
    }

    /// Check if a parameter is present (case-insensitive key)
    pub fn has_param(&self, key: &str) -> bool {
        self.0.has_param(key)
    }

    /// Get a parameter value (case-insensitive key)
    pub fn get_param(&self, key: &str) -> Option<Option<&str>> {
        self.0.get_param(key)
    }
}

impl fmt::Display for ReferTo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0) // Delegate to Address display
    }
}

impl FromStr for ReferTo {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Parse using the refer_to_spec parser which handles both name-addr and addr-spec
        // formats as well as any parameters following the address.
        all_consuming(parse_refer_to)(s.as_bytes())
            .map(|(_rem, refer_to)| refer_to)
            .map_err(|e| Error::from(e.to_owned())) // Convert nom::Err to crate::error::Error
    }
} 
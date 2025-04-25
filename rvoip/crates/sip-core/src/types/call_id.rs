use std::fmt;
use std::str::FromStr;
use crate::error::{Result, Error};
use crate::parser::headers::parse_call_id;
use uuid::Uuid;
use std::ops::Deref;
use nom::combinator::all_consuming;
use std::string::FromUtf8Error;
use serde::{Serialize, Deserialize};

/// Represents the Call-ID header field (RFC 3261 Section 8.1.1.6).
/// Uniquely identifies a particular invitation or registration.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CallId(pub String);

impl Deref for CallId {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for CallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl CallId {
    /// Create a new CallId from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    
    /// Get a reference to the inner string value.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for CallId {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Call the parser first
        let parse_result = all_consuming(parse_call_id)(s.as_bytes());

        // Match on the Result
        match parse_result {
            Ok((_, call_id)) => Ok(call_id),
            Err(e) => Err(Error::from(e)), 
        }
    }
}

// TODO: Implement methods (e.g., new_random) 
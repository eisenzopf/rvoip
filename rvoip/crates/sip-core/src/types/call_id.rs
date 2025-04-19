use std::fmt;
use std::str::FromStr;
use crate::error::Result;
use crate::parser::headers::parse_call_id;
use uuid::Uuid;
use std::ops::Deref;

/// Typed Call-ID header value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)] // Add derives as needed
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

impl FromStr for CallId {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        // Parsing is trivial, just wrap the string
        parse_call_id(s)
    }
}

// TODO: Implement methods (e.g., new_random) 
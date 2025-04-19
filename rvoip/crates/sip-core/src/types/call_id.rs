use std::fmt;
use std::str::FromStr;
use crate::error::Result;
use crate::parser::headers::parse_call_id;

/// Typed Call-ID header value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)] // Add derives as needed
pub struct CallId(pub String);

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
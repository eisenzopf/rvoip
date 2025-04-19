use crate::types::Method;
use crate::parser::headers::parse_allow;
use crate::error::Result;
use std::fmt;
use std::str::FromStr;

/// Typed Allow header.
#[derive(Debug, Clone, PartialEq, Eq)] // Add derives as needed
pub struct Allow(pub Vec<Method>);

impl fmt::Display for Allow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let method_strings: Vec<String> = self.0.iter().map(|m| m.to_string()).collect();
        write!(f, "{}", method_strings.join(", "))
    }
}

impl FromStr for Allow {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        parse_allow(s)
    }
}

// TODO: Implement methods (e.g., allows(Method)) 
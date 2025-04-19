use crate::types::media_type::MediaType;
use crate::parser::headers::parse_accept;
use crate::error::Result;
use std::fmt;
use std::str::FromStr;

/// Typed Accept header.
#[derive(Debug, Clone, PartialEq, Eq)] // Add derives as needed
pub struct Accept(pub Vec<MediaType>);

impl fmt::Display for Accept {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let type_strings: Vec<String> = self.0.iter().map(|m| m.to_string()).collect();
        write!(f, "{}", type_strings.join(", "))
    }
}

impl FromStr for Accept {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        parse_accept(s)
    }
}

// TODO: Implement methods (e.g., for checking acceptable types) 
use crate::uri::Uri;
use crate::parser::headers::parse_warning;
use crate::error::Result;
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;

/// Typed Warning header value.
#[derive(Debug, Clone, PartialEq, Eq)] // Add derives as needed
pub struct Warning {
    pub code: u16,   // 3xx
    pub agent: Uri, // Or maybe just Host?
    pub text: String,
}

impl Warning {
    /// Creates a new Warning header.
    pub fn new(code: u16, agent: Uri, text: impl Into<String>) -> Self {
        Self { code, agent, text: text.into() }
    }
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Agent should be host or pseudo-host, URI display might be too much?
        // Using host for now.
        // Text MUST be quoted.
        write!(f, "{} {} \"{}\"", self.code, self.agent.host, self.text)
    }
}

impl FromStr for Warning {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use crate::parser::headers::warning::parse_warning;
        use nom::combinator::all_consuming;

        match all_consuming(parse_warning)(s.as_bytes()) {
            // The parser already returns Vec<WarningValue>
            Ok((_, value)) => Ok(Warning(value)),
            Err(e) => Err(Error::ParsingError{ 
                message: format!("Failed to parse Warning header: {:?}", e), 
                source: None 
            })
        }
    }
}

// TODO: Implement methods if needed 
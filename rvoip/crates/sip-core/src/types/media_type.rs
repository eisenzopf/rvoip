use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use crate::parser::headers::parse_content_type; // Use parser for FromStr
use crate::error::Result;

/// Represents a media type parameter (e.g., charset=utf-8)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaTypeParam {
    pub key: String,
    pub value: String,
}

/// Represents a Media Type (e.g., application/sdp; charset=utf-8)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaType {
    pub type_: String, // e.g., "application"
    pub subtype: String, // e.g., "sdp"
    pub params: HashMap<String, String>, // More efficient lookup than Vec<(K,V)>
}

impl MediaType {
    /// Creates a new MediaType.
    pub fn new(type_: impl Into<String>, subtype: impl Into<String>) -> Self {
        Self {
            type_: type_.into(),
            subtype: subtype.into(),
            params: HashMap::new(),
        }
    }

    /// Builder method to add a parameter.
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.insert(key.into(), value.into());
        self
    }
}

impl fmt::Display for MediaType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.type_)?;
        write!(f, "/")?;
        write!(f, "{}", self.subtype)?;
        for (key, value) in &self.params {
            // TODO: Add quoting logic for parameter values if needed
            write!(f, ";{}={}", key, value)?;
        }
        Ok(())
    }
}

impl FromStr for MediaType {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        // Delegate to the existing parser function
        parse_content_type(s)
    }
}

// TODO: Implement methods, FromStr, Display 
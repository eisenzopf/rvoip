use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use crate::types::param::Param;
use bytes::Bytes;
use std::collections::HashMap;
use crate::parser;
use crate::parser::headers::content_type::ContentTypeValue;
use crate::parser::headers::content_type::parse_content_type_value;
use serde::{Deserialize, Serialize};

/// Represents the Content-Type header field (RFC 3261 Section 7.3.1).
/// Describes the media type of the message body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentType(pub ContentTypeValue);

impl ContentType {
    /// Creates a new Content-Type header.
    pub fn new(value: ContentTypeValue) -> Self {
        Self(value)
    }

    /// Helper to create from basic type/subtype
    pub fn from_type_subtype(m_type: impl Into<String>, m_subtype: impl Into<String>) -> Self {
        Self(ContentTypeValue {
            m_type: m_type.into(),
            m_subtype: m_subtype.into(),
            parameters: HashMap::new(),
        })
    }
}

impl fmt::Display for ContentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0) // Delegate to MediaType display
    }
}

impl FromStr for ContentType {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        all_consuming(parse_content_type_value)(s.as_bytes())
            .map_err(Error::from)
            .map(|(_, value)| ContentType(value))
    }
}

// TODO: Implement methods if needed 
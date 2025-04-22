use crate::types::media_type::MediaType;
use crate::parser::headers::parse_accept;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use std::collections::HashMap;
use ordered_float::NotNan;
use crate::parser::headers::accept::AcceptValue;
use serde::{Deserialize, Serialize};
use crate::types::param::Param;

/// Represents the Accept header field (RFC 3261 Section 20.1).
/// Indicates the media types acceptable for the response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Accept(pub Vec<AcceptValue>);

impl Accept {
    /// Creates an empty Accept header.
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Creates an Accept header with specified capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    /// Creates an Accept header from an iterator of media types.
    pub fn from_media_types<I>(types: I) -> Self
    where
        I: IntoIterator<Item = AcceptValue>
    {
        Self(types.into_iter().collect())
    }

    /// Adds a media type to the list.
    pub fn push(&mut self, media_type: AcceptValue) {
        self.0.push(media_type);
    }

    /// Checks if a specific media type is acceptable (basic check).
    /// TODO: Implement proper matching based on type/subtype/* and parameters (q values).
    pub fn accepts(&self, media_type: &AcceptValue) -> bool {
        self.0.iter().any(|accepted_type| {
            // Simple type/subtype match
            (accepted_type.m_type == "*" || accepted_type.m_type == media_type.m_type) &&
            (accepted_type.m_subtype == "*" || accepted_type.m_subtype == media_type.m_subtype)
            // TODO: Parameter matching (like q values)
        })
    }
}

impl fmt::Display for Accept {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let type_strings: Vec<String> = self.0.iter().map(|m| m.to_string()).collect();
        write!(f, "{}", type_strings.join(", "))
    }
}

impl FromStr for Accept {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        use crate::parser::headers::accept::parse_accept;

        // Convert &str to &[u8] and use all_consuming
        match all_consuming(parse_accept)(s.as_bytes()) {
            Ok((_, accept_header)) => Ok(accept_header),
            Err(e) => Err(Error::ParseError(
                format!("Failed to parse Accept header: {:?}", e)
            ))
        }
    }
}

impl fmt::Display for AcceptValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = format!("{}/{}", self.m_type, self.m_subtype);
        if let Some(q) = self.q {
            s.push_str(&format!(";q={}", q));
        }
        for (k, v) in &self.params {
            s.push_str(&format!(";{}={}", k, v));
        }
        write!(f, "{}", s)
    }
}

// TODO: Implement methods (e.g., for checking acceptable types)
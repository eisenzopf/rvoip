use crate::types::media_type::MediaType;
use crate::parser::headers::parse_accept;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use std::collections::HashMap;
use ordered_float::NotNan;

/// Typed Accept header.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Accept(pub Vec<MediaType>);

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
        I: IntoIterator<Item = MediaType>
    {
        Self(types.into_iter().collect())
    }

    /// Adds a media type to the list.
    pub fn push(&mut self, media_type: MediaType) {
        self.0.push(media_type);
    }

    /// Checks if a specific media type is acceptable (basic check).
    /// TODO: Implement proper matching based on type/subtype/* and parameters (q values).
    pub fn accepts(&self, media_type: &MediaType) -> bool {
        self.0.iter().any(|accepted_type| {
            // Simple type/subtype match for now
            (accepted_type.type_ == "*" || accepted_type.type_ == media_type.type_) &&
            (accepted_type.subtype == "*" || accepted_type.subtype == media_type.subtype)
            // Parameter matching (like q values) is more complex and omitted here
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
            Ok((_, values)) => Ok(Accept(values)),
            Err(e) => Err(Error::ParseError(
                format!("Failed to parse Accept header: {:?}", e)
            ))
        }
    }
}

// TODO: Implement methods (e.g., for checking acceptable types)
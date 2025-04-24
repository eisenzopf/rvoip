use std::fmt;
use serde::{Serialize, Deserialize};
use std::str::FromStr;
use std::ops::Deref;

use crate::error::{Error, Result};
use crate::types::{HeaderName, HeaderValue, Header, TypedHeaderTrait, CallId};
use crate::parser::headers::in_reply_to::parse_in_reply_to;

/// Represents an In-Reply-To header field (RFC 3261 Section 20.22).
/// Contains one or more Call-IDs of previous requests to which this request is a reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InReplyTo(pub Vec<CallId>);

impl InReplyTo {
    /// Creates a new In-Reply-To header with a single Call-ID.
    pub fn new(call_id: impl Into<String>) -> Self {
        Self(vec![CallId(call_id.into())])
    }
    
    /// Creates a new In-Reply-To header with multiple Call-IDs.
    pub fn with_multiple(call_ids: Vec<CallId>) -> Self {
        Self(call_ids)
    }
    
    /// Creates a new In-Reply-To header with multiple Call-IDs from strings.
    pub fn with_multiple_strings(call_ids: Vec<String>) -> Self {
        Self(call_ids.into_iter().map(CallId).collect())
    }
    
    /// Adds a Call-ID to the list.
    pub fn add(&mut self, call_id: impl Into<String>) {
        self.0.push(CallId(call_id.into()));
    }
    
    /// Returns true if the header contains the specified Call-ID.
    pub fn contains(&self, call_id: &str) -> bool {
        self.0.iter().any(|id| id.0 == call_id)
    }
}

// Implement Deref to access Vec methods directly
impl Deref for InReplyTo {
    type Target = Vec<CallId>;
    
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// Implement Display for formatting in SIP messages
impl fmt::Display for InReplyTo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.iter().map(|id| id.0.clone()).collect::<Vec<String>>().join(", "))
    }
}

// Implement FromStr to enable parsing from string
impl FromStr for InReplyTo {
    type Err = Error;
    
    fn from_str(s: &str) -> Result<Self> {
        // We need to parse just the comma-separated CallIds, not the full header with prefix
        let ids: Vec<String> = s.split(',')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect();
        
        if ids.is_empty() {
            return Err(Error::ParseError("Empty In-Reply-To value".to_string()));
        }

        Ok(InReplyTo(ids.into_iter().map(CallId).collect()))
    }
}

// Implement TypedHeaderTrait for header manipulation
impl TypedHeaderTrait for InReplyTo {
    type Name = HeaderName;
    
    fn header_name() -> Self::Name {
        HeaderName::InReplyTo
    }
    
    fn to_header(&self) -> Header {
        Header::new(
            Self::header_name(), 
            HeaderValue::Raw(self.to_string().into_bytes())
        )
    }
    
    fn from_header(header: &Header) -> Result<Self> {
        if header.name != HeaderName::InReplyTo {
            return Err(Error::ParseError(format!(
                "Expected In-Reply-To header, got {}", header.name
            )));
        }
        
        // Get the raw value
        let value_str = match &header.value {
            HeaderValue::Raw(bytes) => String::from_utf8(bytes.clone())?,
            HeaderValue::InReplyTo(in_reply_to) => in_reply_to.to_string(),
            _ => return Err(Error::ParseError(format!(
                "Expected In-Reply-To header value, got {:?}", header.value
            ))),
        };
        
        // Parse the value
        Self::from_str(&value_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_in_reply_to_new() {
        let irt = InReplyTo::new("abc123@example.com");
        assert_eq!(irt.0.len(), 1);
        assert_eq!(irt.0[0].0, "abc123@example.com");
    }
    
    #[test]
    fn test_in_reply_to_with_multiple() {
        let ids = vec![
            CallId("id1@domain.com".to_string()),
            CallId("id2@domain.com".to_string()),
        ];
        let irt = InReplyTo::with_multiple(ids);
        assert_eq!(irt.0.len(), 2);
        assert_eq!(irt.0[0].0, "id1@domain.com");
        assert_eq!(irt.0[1].0, "id2@domain.com");
    }
    
    #[test]
    fn test_in_reply_to_with_multiple_strings() {
        let strings = vec![
            "id1@domain.com".to_string(),
            "id2@domain.com".to_string(),
        ];
        let irt = InReplyTo::with_multiple_strings(strings);
        assert_eq!(irt.0.len(), 2);
        assert_eq!(irt.0[0].0, "id1@domain.com");
        assert_eq!(irt.0[1].0, "id2@domain.com");
    }
    
    #[test]
    fn test_in_reply_to_add() {
        let mut irt = InReplyTo::new("id1@domain.com");
        irt.add("id2@domain.com");
        assert_eq!(irt.0.len(), 2);
        assert_eq!(irt.0[1].0, "id2@domain.com");
    }
    
    #[test]
    fn test_in_reply_to_contains() {
        let irt = InReplyTo::with_multiple(vec![
            CallId("id1@domain.com".to_string()),
            CallId("id2@domain.com".to_string()),
        ]);
        assert!(irt.contains("id1@domain.com"));
        assert!(irt.contains("id2@domain.com"));
        assert!(!irt.contains("id3@domain.com"));
    }
    
    #[test]
    fn test_in_reply_to_display() {
        let irt = InReplyTo::with_multiple(vec![
            CallId("id1@domain.com".to_string()),
            CallId("id2@domain.com".to_string()),
        ]);
        assert_eq!(irt.to_string(), "id1@domain.com, id2@domain.com");
    }
    
    #[test]
    fn test_in_reply_to_from_str() {
        let irt_str = "id1@domain.com, id2@domain.com";
        let irt = InReplyTo::from_str(irt_str).unwrap();
        assert_eq!(irt.0.len(), 2);
        assert_eq!(irt.0[0].0, "id1@domain.com");
        assert_eq!(irt.0[1].0, "id2@domain.com");
    }
    
    #[test]
    fn test_in_reply_to_header_trait() {
        // Test header_name
        assert_eq!(InReplyTo::header_name(), HeaderName::InReplyTo);
        
        // Test to_header
        let irt = InReplyTo::new("id@domain.com");
        let header = irt.to_header();
        assert_eq!(header.name, HeaderName::InReplyTo);
        
        // Test from_header
        let irt2 = InReplyTo::from_header(&header).unwrap();
        assert_eq!(irt, irt2);
    }
} 
//! # JSON Representation and Access Layer for SIP Types
//!
//! This module provides a JSON-based interface for working with SIP types, allowing:
//! - Conversion between SIP types and JSON structures
//! - Path-based access to nested fields
//! - Query-based retrieval of complex data
//! - JSON creation and manipulation of SIP messages
//!
//! ## Usage Examples
//!
//! ```
//! use rvoip_sip_core::prelude::*;
//! use rvoip_sip_core::json::SipJsonExt;
//!
//! // Create a request
//! let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
//!     .from("Alice", "sip:alice@atlanta.com", Some("1928301774"))
//!     .to("Bob", "sip:bob@example.com", None)
//!     .build();
//!
//! // Access fields with path notation
//! let from_tag = request.get_path("headers.from.tag").as_str().unwrap_or("");
//! let to_uri = request.get_path("headers.to.uri").as_str().unwrap_or("");
//!
//! // Use query interface for complex access
//! let display_names = request.query("$.headers.*.display_name").as_array();
//! ```

pub mod value;
pub mod path;
pub mod query;
pub mod ext;

// Re-export main types and traits
pub use value::SipValue;
pub use ext::SipJsonExt;

use serde::Serialize;
use serde::de::DeserializeOwned;
use std::error::Error;
use std::fmt;

/// Error type for JSON operations
#[derive(Debug)]
pub enum SipJsonError {
    /// Error during serialization
    SerializeError(serde_json::Error),
    /// Error during deserialization
    DeserializeError(serde_json::Error),
    /// Invalid path provided
    InvalidPath(String),
    /// Invalid query provided
    InvalidQuery(String),
    /// Type conversion error
    TypeConversionError(String),
    /// Other errors
    Other(String),
}

impl fmt::Display for SipJsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SerializeError(e) => write!(f, "Serialization error: {}", e),
            Self::DeserializeError(e) => write!(f, "Deserialization error: {}", e),
            Self::InvalidPath(e) => write!(f, "Invalid path: {}", e),
            Self::InvalidQuery(e) => write!(f, "Invalid query: {}", e),
            Self::TypeConversionError(e) => write!(f, "Type conversion error: {}", e),
            Self::Other(e) => write!(f, "Other error: {}", e),
        }
    }
}

impl Error for SipJsonError {}

/// Result type for JSON operations
pub type SipJsonResult<T> = Result<T, SipJsonError>;

/// Core trait for converting between SIP types and JSON
pub trait SipJson {
    /// Convert this type to a SipValue
    fn to_sip_value(&self) -> SipJsonResult<SipValue>;
    
    /// Create this type from a SipValue
    fn from_sip_value(value: &SipValue) -> SipJsonResult<Self> where Self: Sized;
} 
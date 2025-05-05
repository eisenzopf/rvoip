//! # JSON Representation and Access Layer for SIP Types
//!
//! This module provides a comprehensive JSON-based interface for working with SIP types,
//! enabling powerful, flexible access to SIP message data.
//!
//! ## Overview
//!
//! The SIP JSON module offers several ways to interact with SIP messages:
//!
//! 1. **Path-based access** - Direct access via dot notation (e.g., `headers.From.display_name`)
//! 2. **Query-based access** - Complex searches using JSONPath-like syntax (e.g., `$..display_name`)
//! 3. **Object conversion** - Convert between SIP types and JSON structures
//! 4. **Convenience traits** - Helper methods for common SIP header access patterns
//!
//! ## Core Components
//!
//! - [`SipValue`](value::SipValue) - A JSON-like value representing any SIP data
//! - [`SipJsonExt`](SipJsonExt) - Extension trait providing JSON operations on SIP types
//! - [`SipMessageJson`](ext::SipMessageJson) - Convenience trait for common SIP headers
//! - [`path`](path) module - Functions for path-based access
//! - [`query`](query) module - Functions for query-based access
//!
//! ## Path Access vs. Query Access
//!
//! * **Path access** is direct and specific - use when you know exactly what you're looking for
//! * **Query access** is flexible and powerful - use when searching for patterns or exploring
//!
//! ## Basic Usage Examples
//!
//! ### Path-based Access
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use rvoip_sip_core::json::SipJsonExt;
//!
//! # fn example() -> Option<()> {
//! let request = RequestBuilder::invite("sip:bob@example.com").unwrap()
//!     .from("Alice", "sip:alice@example.com", Some("tag12345"))
//!     .to("Bob", "sip:bob@example.com", None)
//!     .build();
//!
//! // Simple path access with Option return
//! if let Some(from_display) = request.path("headers.From.display_name") {
//!     println!("From display name: {}", from_display);
//! }
//!
//! // Direct string access with default value
//! let to_display = request.path_str_or("headers.To.display_name", "Unknown");
//! let from_tag = request.path_str_or("headers.From.params[0].Tag", "No tag");
//! # Some(())
//! # }
//! ```
//!
//! ### Query-based Access
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use rvoip_sip_core::json::SipJsonExt;
//!
//! # fn example() -> Option<()> {
//! let request = RequestBuilder::invite("sip:bob@example.com").unwrap()
//!     .from("Alice", "sip:alice@example.com", Some("tag12345"))
//!     .to("Bob", "sip:bob@example.com", Some("tag6789"))
//!     .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
//!     .build();
//!
//! // Find all display names in the message
//! let display_names = request.query("$..display_name");
//! for name in &display_names {
//!     println!("Found display name: {}", name);
//! }
//!
//! // Find all tags anywhere in the message
//! let tags = request.query("$..Tag");
//! println!("Found {} tags", tags.len());
//! # Some(())
//! # }
//! ```
//!
//! ### Using SIP Message Helpers
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use rvoip_sip_core::json::ext::SipMessageJson;
//!
//! # fn example() -> Option<()> {
//! let request = RequestBuilder::invite("sip:bob@example.com").unwrap()
//!     .from("Alice", "sip:alice@example.com", Some("tag12345"))
//!     .to("Bob", "sip:bob@example.com", None)
//!     .build();
//!
//! // Use helper methods for common headers
//! let from_uri = request.from_uri()?;
//! let from_tag = request.from_tag()?;
//! let call_id = request.call_id()?;
//!
//! println!("Call-ID: {}", call_id);
//! println!("From URI: {}", from_uri);
//! println!("From tag: {}", from_tag);
//! # Some(())
//! # }
//! ```
//!
//! ### Converting to/from JSON
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use rvoip_sip_core::json::{SipJsonExt, SipValue};
//!
//! # fn example() -> Option<()> {
//! let request = RequestBuilder::invite("sip:bob@example.com").unwrap()
//!     .from("Alice", "sip:alice@example.com", Some("tag12345"))
//!     .build();
//!
//! // Convert to JSON string
//! let json_str = request.to_json_string().ok()?;
//! println!("JSON representation: {}", json_str);
//!
//! // Convert to a SipValue for manipulation
//! let value = request.to_sip_value().ok()?;
//!
//! // Perform operations on the SipValue...
//! # Some(())
//! # }
//! ```
//!
//! ## When to Use Each Approach
//!
//! - **Typed Headers API**: When type safety is critical (production code)
//! - **Path Accessors**: For direct, simple access to known fields
//! - **Query Interface**: For complex searches or exploring message structure
//! - **SipMessageJson methods**: For common SIP headers with a concise API

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

/// Error type for JSON operations.
///
/// This enum represents the various errors that can occur during JSON operations
/// on SIP messages.
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

/// Result type for JSON operations.
///
/// This is a convenience type alias for `Result<T, SipJsonError>`.
pub type SipJsonResult<T> = Result<T, SipJsonError>;

/// Core trait for converting between SIP types and JSON.
///
/// This trait provides the fundamental conversion methods between SIP types 
/// and JSON representation. It is implemented automatically for any type that
/// implements Serialize and DeserializeOwned.
///
/// # Examples
///
/// ```
/// # use rvoip_sip_core::json::{SipJson, SipValue};
/// # use serde::{Serialize, Deserialize};
/// #[derive(Serialize, Deserialize)]
/// struct Person {
///     name: String,
///     age: u32,
/// }
///
/// # fn example() -> Option<()> {
/// let person = Person { name: "Alice".to_string(), age: 30 };
///
/// // Convert to SipValue
/// let value = person.to_sip_value().ok()?;
///
/// // Convert back
/// let restored: Person = Person::from_sip_value(&value).ok()?;
/// assert_eq!(restored.name, "Alice");
/// assert_eq!(restored.age, 30);
/// # Some(())
/// # }
/// ```
pub trait SipJson {
    /// Convert this type to a SipValue.
    ///
    /// # Returns
    /// - `Ok(SipValue)` on success
    /// - `Err(SipJsonError)` on failure
    fn to_sip_value(&self) -> SipJsonResult<SipValue>;
    
    /// Create this type from a SipValue.
    ///
    /// # Parameters
    /// - `value`: The SipValue to convert from
    ///
    /// # Returns
    /// - `Ok(Self)` on success
    /// - `Err(SipJsonError)` on failure
    fn from_sip_value(value: &SipValue) -> SipJsonResult<Self> where Self: Sized;
} 
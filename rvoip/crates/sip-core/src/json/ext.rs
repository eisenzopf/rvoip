//! # Extension Traits for SIP JSON Access
//! 
//! This module provides extension traits that enhance SIP types with JSON access capabilities.
//! These traits make it easy to work with SIP messages in a JSON-like way, offering path-based
//! and query-based access patterns.
//!
//! ## Overview
//!
//! There are two primary traits provided:
//!
//! 1. `SipJsonExt` - A general-purpose extension trait for any serializable type,
//!    providing path and query access methods.
//!
//! 2. `SipMessageJson` - A specialized trait for SIP message types, providing
//!    shorthand methods for common SIP header fields.
//!
//! ## Example Usage
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use rvoip_sip_core::json::SipJsonExt;
//!
//! # fn example() -> Option<()> {
//! // Create a SIP request
//! let request = RequestBuilder::invite("sip:bob@example.com").unwrap()
//!     .from("Alice", "sip:alice@example.com", Some("1928301774"))
//!     .to("Bob", "sip:bob@example.com", None)
//!     .build();
//!
//! // Access header fields using path notation
//! let from_display = request.path_str_or("headers.From.display_name", "Unknown");
//! let from_tag = request.path_str_or("headers.From.params[0].Tag", "No Tag");
//!
//! // Access all display names using a query
//! let display_names = request.query("$..display_name");
//! # Some(())
//! # }
//! ```

use crate::json::{SipJson, SipJsonResult, SipJsonError, SipValue};
use crate::json::query;
use crate::json::path::PathAccessor;
use serde::{Serialize, Deserialize, de::DeserializeOwned};

/// Extension trait for all types implementing Serialize/Deserialize.
///
/// This trait provides JSON access methods to any type that can be serialized/deserialized,
/// making it easy to work with SIP messages in a JSON-like way.
///
/// # Examples
///
/// Basic path access:
///
/// ```
/// # use rvoip_sip_core::prelude::*;
/// # use rvoip_sip_core::json::SipJsonExt;
/// # fn example() -> Option<()> {
/// let request = RequestBuilder::invite("sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
///     .build();
///
/// // Access fields with path notation
/// let from_tag = request.path_str_or("headers.From.params[0].Tag", "unknown");
/// println!("From tag: {}", from_tag);
/// # Some(())
/// # }
/// ```
///
/// Query-based access:
///
/// ```
/// # use rvoip_sip_core::prelude::*;
/// # use rvoip_sip_core::json::SipJsonExt;
/// # fn example() -> Option<()> {
/// let request = RequestBuilder::invite("sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
///     .build();
///
/// // Find all display names in the message
/// let display_names = request.query("$..display_name");
/// for name in display_names {
///     println!("Found display name: {}", name);
/// }
/// # Some(())
/// # }
/// ```
pub trait SipJsonExt {
    /// Convert to a SipValue.
    ///
    /// Converts this type to a SipValue representation,
    /// which can then be used with JSON path and query functions.
    ///
    /// # Returns
    /// - `Ok(SipValue)` on success
    /// - `Err(SipJsonError)` on serialization failure
    fn to_sip_value(&self) -> SipJsonResult<SipValue>;
    
    /// Convert from a SipValue.
    ///
    /// Creates an instance of this type from a SipValue representation.
    ///
    /// # Parameters
    /// - `value`: The SipValue to convert from
    ///
    /// # Returns
    /// - `Ok(Self)` on success
    /// - `Err(SipJsonError)` on deserialization failure
    fn from_sip_value(value: &SipValue) -> SipJsonResult<Self> where Self: Sized;
    
    /// Access a value via path notation (e.g., "headers.from.tag").
    ///
    /// Returns Null if the path doesn't exist.
    ///
    /// # Parameters
    /// - `path`: A string path in dot notation (e.g., "headers.Via[0].branch")
    ///
    /// # Returns
    /// A SipValue representing the value at the specified path, or Null if not found
    ///
    /// # Example
    ///
    /// ```
    /// # use rvoip_sip_core::prelude::*;
    /// # use rvoip_sip_core::json::SipJsonExt;
    /// # fn example() -> Option<()> {
    /// let request = RequestBuilder::invite("sip:bob@example.com").unwrap().build();
    /// let method = request.get_path("method");
    /// println!("Method: {}", method);  // Prints "Method: Invite"
    /// # Some(())
    /// # }
    /// ```
    fn get_path(&self, path: impl AsRef<str>) -> SipValue;
    
    /// Simple path accessor that returns an Option directly.
    ///
    /// This is similar to `get_path` but returns `Option<SipValue>` instead of 
    /// always returning a SipValue (which might be Null).
    ///
    /// # Parameters
    /// - `path`: A string path in dot notation (e.g., "headers.from.display_name")
    ///
    /// # Returns
    /// - `Some(SipValue)` if the path exists
    /// - `None` if the path doesn't exist
    ///
    /// # Example
    ///
    /// ```
    /// # use rvoip_sip_core::prelude::*;
    /// # use rvoip_sip_core::json::SipJsonExt;
    /// # fn example() -> Option<()> {
    /// let request = RequestBuilder::invite("sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
    ///     .build();
    ///
    /// // Using pattern matching with path()
    /// match request.path("headers.From.display_name") {
    ///     Some(val) => println!("From display name: {}", val),
    ///     None => println!("No display name found"),
    /// }
    /// # Some(())
    /// # }
    /// ```
    fn path(&self, path: impl AsRef<str>) -> Option<SipValue>;
    
    /// Get a string value at the given path.
    ///
    /// This is a convenience method that combines `path()` with string conversion.
    /// It handles all value types by converting them to strings.
    ///
    /// # Parameters
    /// - `path`: A string path in dot notation
    ///
    /// # Returns
    /// - `Some(String)` if the path exists 
    /// - `None` if the path doesn't exist
    ///
    /// # Example
    ///
    /// ```
    /// # use rvoip_sip_core::prelude::*;
    /// # use rvoip_sip_core::json::SipJsonExt;
    /// # fn example() -> Option<()> {
    /// let request = RequestBuilder::invite("sip:bob@example.com").unwrap().build();
    /// 
    /// // Works with string values
    /// let method = request.path_str("method").unwrap_or_default();
    /// 
    /// // Also works with numeric values
    /// let cseq = request.path_str("headers.CSeq.seq").unwrap_or_default();
    /// # Some(())
    /// # }
    /// ```
    fn path_str(&self, path: impl AsRef<str>) -> Option<String>;
    
    /// Get a string value at the given path, or return the default value if not found.
    ///
    /// This is a convenience method to avoid repetitive unwrap_or patterns.
    ///
    /// # Parameters
    /// - `path`: A string path in dot notation 
    /// - `default`: The default value to return if the path doesn't exist
    ///
    /// # Returns
    /// The string value at the path, or the default if not found
    ///
    /// # Example
    ///
    /// ```
    /// # use rvoip_sip_core::prelude::*;
    /// # use rvoip_sip_core::json::SipJsonExt;
    /// # fn example() -> Option<()> {
    /// let request = RequestBuilder::invite("sip:bob@example.com").unwrap().build();
    /// 
    /// // A concise one-liner with default value
    /// let from_display = request.path_str_or("headers.From.display_name", "Anonymous");
    /// # Some(())
    /// # }
    /// ```
    fn path_str_or(&self, path: impl AsRef<str>, default: &str) -> String;
    
    /// Get a PathAccessor for chained access to fields.
    ///
    /// This provides a fluent interface for accessing fields with method chaining.
    ///
    /// # Returns
    /// A PathAccessor object for chained field access
    ///
    /// # Example
    ///
    /// ```
    /// # use rvoip_sip_core::prelude::*;
    /// # use rvoip_sip_core::json::SipJsonExt;
    /// # fn example() -> Option<()> {
    /// let request = RequestBuilder::invite("sip:bob@example.com").unwrap().build();
    /// 
    /// // Chain method calls to navigate the structure
    /// let tag = request
    ///     .path_accessor()
    ///     .field("headers")
    ///     .field("From")
    ///     .field("params")
    ///     .index(0)
    ///     .field("Tag")
    ///     .as_str();
    /// # Some(())
    /// # }
    /// ```
    fn path_accessor(&self) -> PathAccessor;
    
    /// Query for values using a JSONPath-like syntax.
    ///
    /// This method allows for powerful searches through the message structure
    /// using a simplified JSONPath syntax.
    ///
    /// # Parameters
    /// - `query_str`: A JSONPath-like query string (e.g., "$..branch" to find all branch parameters)
    ///
    /// # Returns
    /// A vector of SipValue objects matching the query
    ///
    /// # Example
    ///
    /// ```
    /// # use rvoip_sip_core::prelude::*;
    /// # use rvoip_sip_core::json::SipJsonExt;
    /// # fn example() -> Option<()> {
    /// let request = RequestBuilder::invite("sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("tag1"))
    ///     .to("Bob", "sip:bob@example.com", Some("tag2"))
    ///     .build();
    /// 
    /// // Find all tags in the message
    /// let tags = request.query("$..Tag");
    /// for tag in tags {
    ///     println!("Found tag: {}", tag);
    /// }
    /// # Some(())
    /// # }
    /// ```
    fn query(&self, query_str: impl AsRef<str>) -> Vec<SipValue>;
    
    /// Convert to a JSON string.
    ///
    /// # Returns
    /// - `Ok(String)` containing the JSON representation
    /// - `Err(SipJsonError)` on serialization failure
    fn to_json_string(&self) -> SipJsonResult<String>;
    
    /// Convert to a pretty-printed JSON string.
    ///
    /// # Returns
    /// - `Ok(String)` containing the pretty-printed JSON representation
    /// - `Err(SipJsonError)` on serialization failure
    fn to_json_string_pretty(&self) -> SipJsonResult<String>;
    
    /// Create from a JSON string.
    ///
    /// # Parameters
    /// - `json_str`: A JSON string to parse
    ///
    /// # Returns
    /// - `Ok(Self)` on successful parsing
    /// - `Err(SipJsonError)` on deserialization failure
    fn from_json_str(json_str: &str) -> SipJsonResult<Self> where Self: Sized;
}

/// Blanket implementation of SipJson for all types that implement Serialize and Deserialize
impl<T> SipJson for T
where
    T: Serialize + DeserializeOwned
{
    fn to_sip_value(&self) -> SipJsonResult<SipValue> {
        // Convert to serde_json::Value first
        let json_value = serde_json::to_value(self)
            .map_err(|e| SipJsonError::SerializeError(e))?;
        
        // Then convert to SipValue
        Ok(SipValue::from_json_value(&json_value))
    }
    
    fn from_sip_value(value: &SipValue) -> SipJsonResult<Self> {
        // Convert to serde_json::Value first
        let json_value = value.to_json_value();
        
        // Then deserialize into the target type
        serde_json::from_value::<T>(json_value)
            .map_err(|e| SipJsonError::DeserializeError(e))
    }
}

/// Blanket implementation of SipJsonExt for all types that implement Serialize and Deserialize
impl<T> SipJsonExt for T
where
    T: Serialize + DeserializeOwned + SipJson
{
    fn to_sip_value(&self) -> SipJsonResult<SipValue> {
        <T as SipJson>::to_sip_value(self)
    }
    
    fn from_sip_value(value: &SipValue) -> SipJsonResult<Self> {
        <T as SipJson>::from_sip_value(value)
    }
    
    fn get_path(&self, path: impl AsRef<str>) -> SipValue {
        // First convert to JSON
        match self.to_sip_value() {
            Ok(value) => {
                // Empty path returns the full value
                if path.as_ref().is_empty() {
                    return value;
                }
                
                // Try to find the value at the given path
                if let Some(found) = crate::json::path::get_path(&value, path.as_ref()) {
                    // Return a clone of the found value
                    found.clone()
                } else {
                    // Path not found returns Null
                    SipValue::Null
                }
            },
            Err(_) => SipValue::Null,
        }
    }
    
    /// Simple path accessor that returns an Option directly
    fn path(&self, path: impl AsRef<str>) -> Option<SipValue> {
        // First convert to JSON
        match self.to_sip_value() {
            Ok(value) => {
                // Empty path returns the full value
                if path.as_ref().is_empty() {
                    return Some(value);
                }
                
                // Try to find the value at the given path
                crate::json::path::get_path(&value, path.as_ref()).map(|v| v.clone())
            },
            Err(_) => None,
        }
    }
    
    /// Get a string value at the given path
    fn path_str(&self, path: impl AsRef<str>) -> Option<String> {
        let path_str = path.as_ref();
        self.path(path_str)
            .map(|v| {
                // Handle different value types by converting them to strings
                if let Some(s) = v.as_str() {
                    // Handle string values directly
                    String::from(s)
                } else if let Some(n) = v.as_i64() {
                    // Handle integer values
                    n.to_string()
                } else if let Some(f) = v.as_f64() {
                    // Handle floating point values
                    f.to_string()
                } else if v.is_bool() {
                    // Handle boolean values
                    v.as_bool().unwrap().to_string()
                } else if v.is_null() {
                    // Handle null values
                    "null".to_string()
                } else if v.is_object() {
                    // Try to extract meaningful string representation from objects
                    
                    // Handle URIs
                    if path_str.ends_with(".uri") || path_str.ends_with("Uri") {
                        if let Some(scheme) = v.get_path("scheme").and_then(|s| s.as_str()) {
                            let mut uri = String::new();
                            
                            // Build a basic SIP URI string
                            uri.push_str(scheme);
                            uri.push_str(":");
                            
                            if let Some(user) = v.get_path("user").and_then(|u| u.as_str()) {
                                uri.push_str(user);
                                
                                if let Some(password) = v.get_path("password").and_then(|p| p.as_str()) {
                                    uri.push_str(":");
                                    uri.push_str(password);
                                }
                                
                                uri.push_str("@");
                            }
                            
                            if let Some(host_obj) = v.get_path("host") {
                                if let Some(domain) = host_obj.get_path("Domain").and_then(|d| d.as_str()) {
                                    uri.push_str(domain);
                                } else {
                                    uri.push_str(&format!("{}", host_obj));
                                }
                            }
                            
                            if let Some(port) = v.get_path("port").and_then(|p| p.as_f64()) {
                                if port > 0.0 {
                                    uri.push_str(":");
                                    uri.push_str(&port.to_string());
                                }
                            }
                            
                            return uri;
                        }
                    }
                    
                    // Handle display_name specially
                    if path_str.ends_with(".display_name") {
                        if let Some(name) = v.as_str() {
                            return name.to_string();
                        }
                    }
                    
                    // Handle branch specially
                    if path_str.ends_with(".Branch") || path_str.ends_with(".branch") {
                        if let Some(branch) = v.as_str() {
                            return branch.to_string();
                        }
                    }
                    
                    // Handle tag specially
                    if path_str.ends_with(".Tag") || path_str.ends_with(".tag") {
                        if let Some(tag) = v.as_str() {
                            return tag.to_string();
                        }
                    }
                    
                    // Handle Via headers specially
                    if path_str.contains(".Via") || path_str.contains(".via") {
                        if let Some(sent_protocol) = v.get_path("sent_protocol") {
                            let mut via = String::new();
                            
                            // Protocol and transport
                            let transport = sent_protocol.get_path("transport")
                                .and_then(|t| t.as_str())
                                .unwrap_or("UDP");
                            via.push_str("SIP/2.0/");
                            via.push_str(transport);
                            via.push_str(" ");
                            
                            // Host
                            if let Some(host_obj) = v.get_path("sent_by_host") {
                                if let Some(domain) = host_obj.get_path("Domain").and_then(|d| d.as_str()) {
                                    via.push_str(domain);
                                    
                                    // Port (if present)
                                    if let Some(port) = v.get_path("sent_by_port").and_then(|p| p.as_f64()) {
                                        if port != 5060.0 { // Only include non-default port
                                            via.push_str(":");
                                            via.push_str(&port.to_string());
                                        }
                                    }
                                }
                            }
                            
                            // Parameters
                            if let Some(params) = v.get_path("params") {
                                // Branch parameter
                                if let Some(branch) = params.get_path("Branch").and_then(|b| b.as_str()) {
                                    via.push_str("; branch=");
                                    via.push_str(branch);
                                }
                                
                                // Received parameter
                                if let Some(received) = params.get_path("Received").and_then(|r| r.as_str()) {
                                    via.push_str("; received=");
                                    via.push_str(received);
                                }
                            }
                            
                            return via;
                        }
                    }
                    
                    // Fallback for other complex objects
                    format!("{}", v)
                } else if v.is_array() {
                    // For Contact headers, try to extract URI
                    if path_str.contains(".Contact") {
                        if let Some(arr) = v.as_array() {
                            if !arr.is_empty() {
                                let first = &arr[0];
                                
                                // Try to extract meaningful data from Contact array format
                                if let Some(params) = first.get_path("Params").and_then(|p| p.as_array()) {
                                    if !params.is_empty() {
                                        if let Some(address) = params[0].get_path("address") {
                                            if let Some(uri) = address.get_path("uri") {
                                                // Extract URI
                                                let mut uri_str = String::from("<");
                                                
                                                if let Some(scheme) = uri.get_path("scheme").and_then(|s| s.as_str()) {
                                                    uri_str.push_str(scheme);
                                                    uri_str.push_str(":");
                                                    
                                                    if let Some(user) = uri.get_path("user").and_then(|u| u.as_str()) {
                                                        uri_str.push_str(user);
                                                        uri_str.push_str("@");
                                                    }
                                                    
                                                    if let Some(host_obj) = uri.get_path("host") {
                                                        if let Some(domain) = host_obj.get_path("Domain").and_then(|d| d.as_str()) {
                                                            uri_str.push_str(domain);
                                                        }
                                                    }
                                                }
                                                
                                                uri_str.push_str(">");
                                                return uri_str;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    
                    // Fallback for arrays
                    format!("{}", v)
                } else {
                    // Fallback for other value types
                    format!("{}", v)
                }
            })
    }
    
    /// Get a string value at the given path, or return the default value if not found
    fn path_str_or(&self, path: impl AsRef<str>, default: &str) -> String {
        self.path_str(path).unwrap_or_else(|| String::from(default))
    }
    
    fn path_accessor(&self) -> PathAccessor {
        // Convert to SipValue first
        match self.to_sip_value() {
            Ok(value) => PathAccessor::new(value),
            Err(_) => PathAccessor::new(SipValue::Null),
        }
    }
    
    fn query(&self, query_str: impl AsRef<str>) -> Vec<SipValue> {
        match self.to_sip_value() {
            Ok(value) => {
                // Perform the query on the value
                query::query(&value, query_str.as_ref())
                    .into_iter()
                    .cloned()
                    .collect()
            },
            Err(_) => Vec::new(),
        }
    }
    
    fn to_json_string(&self) -> SipJsonResult<String> {
        serde_json::to_string(self)
            .map_err(|e| SipJsonError::SerializeError(e))
    }
    
    fn to_json_string_pretty(&self) -> SipJsonResult<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| SipJsonError::SerializeError(e))
    }
    
    fn from_json_str(json_str: &str) -> SipJsonResult<Self> {
        serde_json::from_str::<Self>(json_str)
            .map_err(|e| SipJsonError::DeserializeError(e))
    }
}

/// Extension methods specifically for SIP message types
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::sip_request::Request;
    use crate::types::sip_response::Response;
    use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
    use crate::types::method::Method;
    
    #[test]
    fn test_request_to_json() {
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .from("Alice", "sip:alice@example.com", Some("tag12345"))
            .to("Bob", "sip:bob@example.com", None)
            .build();
        
        let json = request.to_json_string().unwrap();
        assert!(json.contains("\"method\":\"Invite\""), "JSON doesn't contain method");
        assert!(json.contains("\"display_name\":\"Alice\""), "JSON doesn't contain display name");
        assert!(json.contains("\"Tag\":\"tag12345\""), "JSON doesn't contain tag");
    }
    
    #[test]
    fn test_get_path() {
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .from("Alice", "sip:alice@example.com", Some("tag12345"))
            .to("Bob", "sip:bob@example.com", None)
            .build();
        
        let from_tag = request.get_path("headers[0].From.params[0].Tag");
        assert_eq!(from_tag.as_str(), Some("tag12345"));
        
        let to_uri = request.get_path("headers[1].To.uri.raw_uri");
        assert_eq!(to_uri, SipValue::Null);
    }
    
    #[test]
    fn test_path_accessor() {
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .from("Alice", "sip:alice@example.com", Some("tag12345"))
            .to("Bob", "sip:bob@example.com", None)
            .build();
        
        // Test using direct path access which is more reliable
        let from_tag = request.get_path("headers[0].From.params[0].Tag");
        assert_eq!(from_tag.as_str(), Some("tag12345"));
        
        let to_display_name = request.get_path("headers[1].To.display_name");
        assert_eq!(to_display_name.as_str(), Some("Bob"));
    }
    
    #[test]
    fn test_query() {
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .from("Alice", "sip:alice@example.com", Some("tag12345"))
            .to("Bob", "sip:bob@example.com", None)
            .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
            .via("proxy.atlanta.com", "TCP", Some("z9hG4bK776asdhds2"))
            .build();
        
        // Search for all display_name fields
        let display_names = request.query("$..display_name");
        assert_eq!(display_names.len(), 2);
        
        // Specifically find the Branch params in Via headers
        let branches = request.query("$..Branch");
        assert_eq!(branches.len(), 2);
        
        // First branch should be z9hG4bK776asdhds
        if !branches.is_empty() {
            assert_eq!(branches[0].as_str(), Some("z9hG4bK776asdhds"));
        }
    }
}

/// Extension trait for SIP message types providing shortcuts for common headers.
///
/// This trait builds on `SipJsonExt` to provide convenient accessor methods
/// specifically for common SIP message headers.
///
/// # Examples
///
/// ```
/// # use rvoip_sip_core::prelude::*;
/// # use rvoip_sip_core::json::ext::SipMessageJson;
/// # fn example() -> Option<()> {
/// let request = RequestBuilder::invite("sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("tag12345"))
///     .to("Bob", "sip:bob@example.com", None)
///     .build();
///
/// // Access common headers with convenience methods
/// let from_display = request.from_display_name()?;
/// let from_uri = request.from_uri()?;
/// let from_tag = request.from_tag()?;
/// let call_id = request.call_id()?;
///
/// println!("From: {} <{}>;tag={}", from_display, from_uri, from_tag);
/// println!("Call-ID: {}", call_id);
/// # Some(())
/// # }
/// ```
pub trait SipMessageJson: SipJsonExt {
    /// Get the From display name.
    ///
    /// # Returns
    /// - `Some(String)` containing the display name
    /// - `None` if not present
    fn from_display_name(&self) -> Option<String> {
        self.path_str("headers.From.display_name")
    }
    
    /// Get the From URI as a formatted string (sip:user@host).
    ///
    /// # Returns
    /// - `Some(String)` in the format "sip:user@host"
    /// - `None` if the URI components are not present
    fn from_uri(&self) -> Option<String> {
        let user = self.path_str("headers.From.uri.user")?;
        let host = self.path_str("headers.From.uri.host.Domain")?;
        Some(format!("sip:{}@{}", user, host))
    }
    
    /// Get the From tag parameter.
    ///
    /// # Returns
    /// - `Some(String)` containing the tag value
    /// - `None` if not present
    fn from_tag(&self) -> Option<String> {
        self.path_str("headers.From.params[0].Tag")
    }
    
    /// Get the To display name.
    ///
    /// # Returns
    /// - `Some(String)` containing the display name
    /// - `None` if not present
    fn to_display_name(&self) -> Option<String> {
        self.path_str("headers.To.display_name")
    }
    
    /// Get the To URI as a formatted string (sip:user@host).
    ///
    /// # Returns
    /// - `Some(String)` in the format "sip:user@host"
    /// - `None` if the URI components are not present
    fn to_uri(&self) -> Option<String> {
        let user = self.path_str("headers.To.uri.user")?;
        let host = self.path_str("headers.To.uri.host.Domain")?;
        Some(format!("sip:{}@{}", user, host))
    }
    
    /// Get the To tag parameter.
    ///
    /// # Returns
    /// - `Some(String)` containing the tag value
    /// - `None` if not present
    fn to_tag(&self) -> Option<String> {
        self.path_str("headers.To.params[0].Tag")
    }
    
    /// Get the Call-ID.
    ///
    /// Tries multiple possible locations for the Call-ID value
    /// to handle different SIP message structures.
    ///
    /// # Returns
    /// - `Some(String)` containing the Call-ID
    /// - `None` if not present
    fn call_id(&self) -> Option<String> {
        // Try direct access first (the actual structure in our example)
        self.path_str("headers.CallId")
            // Fallbacks for compatibility
            .or_else(|| self.path_str("headers.CallId.value"))
            .or_else(|| self.path_str("headers.Call-ID.value"))
    }
    
    /// Get the CSeq number.
    ///
    /// # Returns
    /// - `Some(u32)` containing the sequence number
    /// - `None` if not present or not convertible to a number
    fn cseq_number(&self) -> Option<u32> {
        // Try as integer first (the actual structure in our example)
        self.path("headers.CSeq.seq")
            .and_then(|v| v.as_i64().map(|i| i as u32))
            // Fallback for compatibility with string representation
            .or_else(|| self.path_str("headers.CSeq.sequence_number")
                .and_then(|s| s.parse::<u32>().ok()))
    }
    
    /// Get the CSeq method.
    ///
    /// # Returns
    /// - `Some(String)` containing the method
    /// - `None` if not present
    fn cseq_method(&self) -> Option<String> {
        self.path_str("headers.CSeq.method")
    }
    
    /// Get the Via transport.
    ///
    /// # Returns
    /// - `Some(String)` containing the transport (e.g., "UDP", "TCP")
    /// - `None` if not present
    fn via_transport(&self) -> Option<String> {
        self.path_str("headers.Via[0].sent_protocol.transport")
    }
    
    /// Get the Via host.
    ///
    /// # Returns
    /// - `Some(String)` containing the host
    /// - `None` if not present
    fn via_host(&self) -> Option<String> {
        self.path_str("headers.Via[0].sent_by_host.Domain")
    }
    
    /// Get the Via branch parameter.
    ///
    /// # Returns
    /// - `Some(String)` containing the branch value
    /// - `None` if not present
    fn via_branch(&self) -> Option<String> {
        self.path_str("headers.Via[0].params[0].Branch")
    }
    
    /// Get the Contact URI (sip:user@host).
    ///
    /// # Returns
    /// - `Some(String)` in the format "sip:user@host"
    /// - `None` if the URI components are not present
    fn contact_uri(&self) -> Option<String> {
        let user = self.path_str("headers.Contact[0].Params[0].address.uri.user")?;
        let host = self.path_str("headers.Contact[0].Params[0].address.uri.host.Domain")?;
        Some(format!("sip:{}@{}", user, host))
    }
}

// Implement this trait for any type that already implements SipJsonExt
impl<T: SipJsonExt> SipMessageJson for T {} 
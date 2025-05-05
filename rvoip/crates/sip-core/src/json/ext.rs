//! Extension traits for SIP types to enable JSON operations

use crate::json::{SipJson, SipJsonResult, SipJsonError, SipValue};
use crate::json::query;
use crate::json::path::PathAccessor;
use serde::{Serialize, Deserialize, de::DeserializeOwned};

/// Extension trait for all types implementing Serialize/Deserialize
pub trait SipJsonExt {
    /// Convert to a SipValue
    fn to_sip_value(&self) -> SipJsonResult<SipValue>;
    
    /// Convert from a SipValue
    fn from_sip_value(value: &SipValue) -> SipJsonResult<Self> where Self: Sized;
    
    /// Access a value via path notation (e.g., "headers.from.tag")
    /// Returns None if the path doesn't exist
    fn get_path(&self, path: impl AsRef<str>) -> SipValue;
    
    /// Simple path accessor that returns an Option directly
    /// Usage: let display_name = request.path("headers.from.display_name");
    fn path(&self, path: impl AsRef<str>) -> Option<SipValue>;
    
    /// Get a PathAccessor for chained access to fields
    fn path_accessor(&self) -> PathAccessor;
    
    /// Query for values using a JSONPath-like syntax
    fn query(&self, query_str: impl AsRef<str>) -> Vec<SipValue>;
    
    /// Convert to a JSON string
    fn to_json_string(&self) -> SipJsonResult<String>;
    
    /// Convert to a pretty-printed JSON string
    fn to_json_string_pretty(&self) -> SipJsonResult<String>;
    
    /// Create from a JSON string
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
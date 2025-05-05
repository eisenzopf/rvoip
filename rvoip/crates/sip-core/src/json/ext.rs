//! Extension traits for SIP types to enable JSON operations

use crate::json::{SipJson, SipJsonResult, SipJsonError, SipValue};
use crate::json::query;
use crate::json::path::PathAccessor;
use serde::{Serialize, Deserialize, de::DeserializeOwned};
use std::cell::RefCell;
use std::thread_local;
use std::collections::HashMap;
use std::rc::Rc;

// Thread-local storage for caching SipValues to enable path-based access
thread_local! {
    static VALUE_CACHE: RefCell<HashMap<usize, Rc<SipValue>>> = RefCell::new(HashMap::new());
}

/// Extension trait for all types implementing Serialize/Deserialize
pub trait SipJsonExt {
    /// Convert to a SipValue
    fn to_sip_value(&self) -> SipJsonResult<SipValue>;
    
    /// Convert from a SipValue
    fn from_sip_value(value: &SipValue) -> SipJsonResult<Self> where Self: Sized;
    
    /// Access a value via path notation (e.g., "headers.from.tag")
    /// Returns None if the path doesn't exist
    fn get_path(&self, path: impl AsRef<str>) -> SipValue;
    
    /// Get a PathAccessor for chained access to fields
    fn path(&self) -> PathAccessor;
    
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
        SipJson::to_sip_value(self)
    }
    
    fn from_sip_value(value: &SipValue) -> SipJsonResult<Self> {
        SipJson::from_sip_value(value)
    }
    
    fn get_path(&self, path: impl AsRef<str>) -> SipValue {
        // Convert self to SipValue
        let value = match SipJson::to_sip_value(self) {
            Ok(val) => val,
            Err(_) => return SipValue::Null,
        };
        
        // Get value at path
        let result = if path.as_ref().is_empty() {
            // Empty path returns the full value
            value
        } else if let Some(found) = crate::json::path::get_path(&value, path.as_ref()) {
            // Clone the found value
            found.clone()
        } else {
            // Path not found returns Null
            SipValue::Null
        };
        
        result
    }
    
    fn path(&self) -> PathAccessor {
        // Convert self to SipValue and wrap in a PathAccessor
        let value = SipJson::to_sip_value(self).unwrap_or_default();
        PathAccessor::new(value)
    }
    
    fn query(&self, query_str: impl AsRef<str>) -> Vec<SipValue> {
        let value = SipJson::to_sip_value(self).unwrap_or_default();
        query::query(&value, query_str.as_ref())
            .into_iter()
            .cloned()
            .collect()
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
    use crate::types::SipRequest;
    use crate::types::SipResponse;
    use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
    
    #[test]
    fn test_request_to_json() {
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .from("Alice", "sip:alice@example.com", Some("tag12345"))
            .to("Bob", "sip:bob@example.com", None)
            .build();
        
        let json = request.to_json_string().unwrap();
        assert!(json.contains("INVITE"));
        assert!(json.contains("Alice"));
        assert!(json.contains("tag12345"));
    }
    
    #[test]
    fn test_get_path() {
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .from("Alice", "sip:alice@example.com", Some("tag12345"))
            .to("Bob", "sip:bob@example.com", None)
            .build();
        
        let from_tag = request.get_path("headers.from.tag");
        assert_eq!(from_tag.as_str(), Some("tag12345"));
        
        let to_uri = request.get_path("headers.to.uri");
        assert_eq!(to_uri.as_str(), Some("sip:bob@example.com"));
    }
    
    #[test]
    fn test_path_accessor() {
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .from("Alice", "sip:alice@example.com", Some("tag12345"))
            .to("Bob", "sip:bob@example.com", None)
            .build();
        
        // Test chained path accessor
        let mut path = request.path();
        let from_tag = path.headers().from().tag().as_str();
        assert_eq!(from_tag, Some("tag12345".to_string()));
        
        // Reset and navigate to a different path
        let mut path = request.path();
        let to_uri = path.headers().to().uri().as_str();
        assert_eq!(to_uri, Some("sip:bob@example.com".to_string()));
    }
    
    #[test]
    fn test_query() {
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .from("Alice", "sip:alice@example.com", Some("tag12345"))
            .to("Bob", "sip:bob@example.com", None)
            .via("SIP/2.0/UDP pc33.atlanta.com", Some("z9hG4bK776asdhds"))
            .via("SIP/2.0/TCP proxy.atlanta.com", Some("z9hG4bK776asdhds2"))
            .build();
        
        let branches = request.query("$.headers.via[*].branch");
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0].as_str(), Some("z9hG4bK776asdhds"));
        assert_eq!(branches[1].as_str(), Some("z9hG4bK776asdhds2"));
    }
} 
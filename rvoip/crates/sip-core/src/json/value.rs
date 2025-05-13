use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use serde_json;
use std::fmt;
use crate::json::{SipJsonResult, SipJsonError};

/// # SIP JSON Value Representation
///
/// This module provides a JSON-like value representation for SIP messages and components.
/// `SipValue` is the core type that can represent any SIP data as a JSON-compatible structure.
///
/// ## Overview
///
/// `SipValue` is an enum that can represent:
/// - Primitive values (null, boolean, number, string)
/// - Composite values (arrays, objects)
///
/// This allows SIP messages to be converted to a language-agnostic intermediate representation
/// that can be easily manipulated, transformed, or serialized.
///
/// ## Example
///
/// ```
/// # use rvoip_sip_core::json::SipValue;
/// # use std::collections::HashMap;
/// // Create a SipValue object representing a From header
/// let mut params = HashMap::new();
/// params.insert("Tag".to_string(), SipValue::String("1234".to_string()));
///
/// let mut from_header = HashMap::new();
/// from_header.insert("display_name".to_string(), SipValue::String("Alice".to_string()));
/// from_header.insert("uri".to_string(), SipValue::String("sip:alice@example.com".to_string()));
/// from_header.insert("params".to_string(), SipValue::Array(vec![SipValue::Object(params)]));
///
/// let from_value = SipValue::Object(from_header);
///
/// // Access fields using path notation
/// let tag = from_value.get_path("params[0].Tag").unwrap();
/// assert_eq!(tag.as_str(), Some("1234"));
/// ```

/// A JSON-like representation of SIP values.
///
/// This enum can represent any SIP message component in a JSON-compatible format,
/// providing a flexible intermediate representation for manipulation and access.
///
/// # Examples
///
/// ```
/// # use rvoip_sip_core::json::SipValue;
/// // Creating different value types
/// let null_value = SipValue::Null;
/// let bool_value = SipValue::Bool(true);
/// let num_value = SipValue::Number(42.0);
/// let str_value = SipValue::String("hello".to_string());
/// let arr_value = SipValue::Array(vec![SipValue::Number(1.0), SipValue::Number(2.0)]);
/// ```
///
/// Converting to and from JSON:
///
/// ```
/// # use rvoip_sip_core::json::SipValue;
/// // From a JSON string
/// let json_str = r#"{"name":"Alice","age":30,"active":true}"#;
/// let value = SipValue::from_str(json_str).unwrap();
///
/// // Back to a JSON string
/// let serialized = value.to_string().unwrap();
/// assert!(serialized.contains("Alice"));
/// ```
///
/// Creating complex structures:
///
/// ```
/// # use rvoip_sip_core::json::SipValue;
/// # use std::collections::HashMap;
/// // Create a basic SIP message structure
/// let mut headers = HashMap::new();
///
/// // Add From header
/// let mut from = HashMap::new();
/// from.insert("display_name".to_string(), SipValue::String("Alice".to_string()));
/// from.insert("uri".to_string(), SipValue::String("sip:alice@example.com".to_string()));
/// headers.insert("From".to_string(), SipValue::Object(from));
///
/// // Add To header
/// let mut to = HashMap::new();
/// to.insert("display_name".to_string(), SipValue::String("Bob".to_string()));
/// to.insert("uri".to_string(), SipValue::String("sip:bob@example.com".to_string()));
/// headers.insert("To".to_string(), SipValue::Object(to));
///
/// // Create the message
/// let mut msg = HashMap::new();
/// msg.insert("method".to_string(), SipValue::String("INVITE".to_string()));
/// msg.insert("headers".to_string(), SipValue::Object(headers));
///
/// let message = SipValue::Object(msg);
///
/// // Access using path notation
/// assert_eq!(message.get_path("method").unwrap().as_str(), Some("INVITE"));
/// assert_eq!(message.get_path("headers.From.display_name").unwrap().as_str(), Some("Alice"));
/// ```
#[derive(Clone, Debug, PartialEq)]
pub enum SipValue {
    /// Null or None value
    Null,
    /// Boolean value
    Bool(bool),
    /// Numeric value
    Number(f64),
    /// String value
    String(String),
    /// Array of values
    Array(Vec<SipValue>),
    /// Map of string keys to values
    Object(HashMap<String, SipValue>),
}

impl SipValue {
    /// Convert to a boolean value if possible.
    ///
    /// # Returns
    /// - `Some(bool)` if the value is a boolean
    /// - `None` otherwise
    ///
    /// # Example
    ///
    /// ```
    /// # use rvoip_sip_core::json::SipValue;
    /// let value = SipValue::Bool(true);
    /// assert_eq!(value.as_bool(), Some(true));
    ///
    /// let value = SipValue::String("not a bool".to_string());
    /// assert_eq!(value.as_bool(), None);
    /// ```
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            SipValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Convert to a floating-point value if possible.
    ///
    /// # Returns
    /// - `Some(f64)` if the value is a number
    /// - `None` otherwise
    ///
    /// # Example
    ///
    /// ```
    /// # use rvoip_sip_core::json::SipValue;
    /// let value = SipValue::Number(3.14);
    /// assert_eq!(value.as_f64(), Some(3.14));
    ///
    /// let value = SipValue::String("3.14".to_string());
    /// assert_eq!(value.as_f64(), None);
    /// ```
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            SipValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// Convert to an integer value if possible.
    ///
    /// # Returns
    /// - `Some(i64)` if the value is a number that can be represented as an integer
    /// - `None` otherwise
    ///
    /// # Example
    ///
    /// ```
    /// # use rvoip_sip_core::json::SipValue;
    /// let value = SipValue::Number(42.0);
    /// assert_eq!(value.as_i64(), Some(42));
    ///
    /// // Fractional values return None
    /// let value = SipValue::Number(42.5);
    /// assert_eq!(value.as_i64(), None);
    /// ```
    pub fn as_i64(&self) -> Option<i64> {
        self.as_f64().and_then(|n| {
            if n.fract() == 0.0 && n >= i64::MIN as f64 && n <= i64::MAX as f64 {
                Some(n as i64)
            } else {
                None
            }
        })
    }

    /// Convert to a string reference if possible.
    ///
    /// # Returns
    /// - `Some(&str)` if the value is a string
    /// - `None` otherwise
    ///
    /// # Example
    ///
    /// ```
    /// # use rvoip_sip_core::json::SipValue;
    /// let value = SipValue::String("hello".to_string());
    /// assert_eq!(value.as_str(), Some("hello"));
    ///
    /// let value = SipValue::Number(123.0);
    /// assert_eq!(value.as_str(), None);
    /// ```
    pub fn as_str(&self) -> Option<&str> {
        match self {
            SipValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Convert to an array reference if possible.
    ///
    /// # Returns
    /// - `Some(&Vec<SipValue>)` if the value is an array
    /// - `None` otherwise
    ///
    /// # Example
    ///
    /// ```
    /// # use rvoip_sip_core::json::SipValue;
    /// let value = SipValue::Array(vec![SipValue::Number(1.0), SipValue::Number(2.0)]);
    /// assert_eq!(value.as_array().unwrap().len(), 2);
    ///
    /// let value = SipValue::String("not an array".to_string());
    /// assert_eq!(value.as_array(), None);
    /// ```
    pub fn as_array(&self) -> Option<&Vec<SipValue>> {
        match self {
            SipValue::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Convert to a mutable array reference if possible.
    ///
    /// # Returns
    /// - `Some(&mut Vec<SipValue>)` if the value is an array
    /// - `None` otherwise
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<SipValue>> {
        match self {
            SipValue::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Convert to an object reference if possible.
    ///
    /// # Returns
    /// - `Some(&HashMap<String, SipValue>)` if the value is an object
    /// - `None` otherwise
    ///
    /// # Example
    ///
    /// ```
    /// # use rvoip_sip_core::json::SipValue;
    /// # use std::collections::HashMap;
    /// let mut map = HashMap::new();
    /// map.insert("key".to_string(), SipValue::String("value".to_string()));
    /// let value = SipValue::Object(map);
    ///
    /// let obj = value.as_object().unwrap();
    /// assert_eq!(obj.get("key").unwrap().as_str(), Some("value"));
    /// ```
    pub fn as_object(&self) -> Option<&HashMap<String, SipValue>> {
        match self {
            SipValue::Object(o) => Some(o),
            _ => None,
        }
    }

    /// Convert to a mutable object reference if possible.
    ///
    /// # Returns
    /// - `Some(&mut HashMap<String, SipValue>)` if the value is an object
    /// - `None` otherwise
    pub fn as_object_mut(&mut self) -> Option<&mut HashMap<String, SipValue>> {
        match self {
            SipValue::Object(o) => Some(o),
            _ => None,
        }
    }

    /// Get a value from a path, e.g. "headers.via[0].branch".
    ///
    /// # Arguments
    /// * `path` - A dot-separated path string with optional array indices
    ///
    /// # Returns
    /// - `Some(&SipValue)` if the path exists
    /// - `None` otherwise
    ///
    /// # Example
    ///
    /// ```
    /// # use rvoip_sip_core::json::SipValue;
    /// # use std::collections::HashMap;
    /// // Create a nested structure
    /// let mut via = HashMap::new();
    /// via.insert("branch".to_string(), SipValue::String("z9hG4bK776asdhds".to_string()));
    ///
    /// let mut headers = HashMap::new();
    /// headers.insert("Via".to_string(), SipValue::Array(vec![SipValue::Object(via)]));
    ///
    /// let message = SipValue::Object(headers);
    ///
    /// // Access via path
    /// let branch = message.get_path("Via[0].branch").unwrap();
    /// assert_eq!(branch.as_str(), Some("z9hG4bK776asdhds"));
    /// ```
    pub fn get_path<S: AsRef<str>>(&self, path: S) -> Option<&SipValue> {
        crate::json::path::get_path(self, path.as_ref())
    }

    /// Check if the value is null.
    ///
    /// # Returns
    /// - `true` if the value is Null
    /// - `false` otherwise
    pub fn is_null(&self) -> bool {
        matches!(self, SipValue::Null)
    }

    /// Check if the value is a boolean.
    ///
    /// # Returns
    /// - `true` if the value is a Bool
    /// - `false` otherwise
    pub fn is_bool(&self) -> bool {
        matches!(self, SipValue::Bool(_))
    }

    /// Check if the value is a number.
    ///
    /// # Returns
    /// - `true` if the value is a Number
    /// - `false` otherwise
    pub fn is_number(&self) -> bool {
        matches!(self, SipValue::Number(_))
    }

    /// Check if the value is a string.
    ///
    /// # Returns
    /// - `true` if the value is a String
    /// - `false` otherwise
    pub fn is_string(&self) -> bool {
        matches!(self, SipValue::String(_))
    }

    /// Check if the value is an array.
    ///
    /// # Returns
    /// - `true` if the value is an Array
    /// - `false` otherwise
    pub fn is_array(&self) -> bool {
        matches!(self, SipValue::Array(_))
    }

    /// Check if the value is an object.
    ///
    /// # Returns
    /// - `true` if the value is an Object
    /// - `false` otherwise
    pub fn is_object(&self) -> bool {
        matches!(self, SipValue::Object(_))
    }

    /// Convert to a serde_json::Value.
    ///
    /// This is useful when you need to use standard JSON libraries or
    /// serialize the value to a JSON string.
    ///
    /// # Returns
    /// A serde_json::Value equivalent to this SipValue
    pub fn to_json_value(&self) -> serde_json::Value {
        match self {
            SipValue::Null => serde_json::Value::Null,
            SipValue::Bool(b) => serde_json::Value::Bool(*b),
            SipValue::Number(n) => serde_json::Value::Number(serde_json::Number::from_f64(*n).unwrap_or(serde_json::Number::from(0))),
            SipValue::String(s) => serde_json::Value::String(s.clone()),
            SipValue::Array(a) => {
                serde_json::Value::Array(a.iter().map(|v| v.to_json_value()).collect())
            },
            SipValue::Object(o) => {
                serde_json::Value::Object(
                    o.iter()
                     .map(|(k, v)| (k.clone(), v.to_json_value()))
                     .collect()
                )
            },
        }
    }

    /// Convert from a serde_json::Value.
    ///
    /// This is useful when you need to convert from standard JSON libraries
    /// or when parsing JSON from external sources.
    ///
    /// # Arguments
    /// * `value` - A serde_json::Value to convert
    ///
    /// # Returns
    /// A SipValue equivalent to the input serde_json::Value
    pub fn from_json_value(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => SipValue::Null,
            serde_json::Value::Bool(b) => SipValue::Bool(*b),
            serde_json::Value::Number(n) => {
                SipValue::Number(n.as_f64().unwrap_or(0.0))
            },
            serde_json::Value::String(s) => SipValue::String(s.clone()),
            serde_json::Value::Array(a) => {
                SipValue::Array(a.iter().map(|v| SipValue::from_json_value(v)).collect())
            },
            serde_json::Value::Object(o) => {
                SipValue::Object(
                    o.iter()
                     .map(|(k, v)| (k.clone(), SipValue::from_json_value(v)))
                     .collect()
                )
            },
        }
    }

    /// Convert to JSON string.
    ///
    /// # Returns
    /// A Result containing either the JSON string or an error
    ///
    /// # Example
    ///
    /// ```
    /// # use rvoip_sip_core::json::SipValue;
    /// let value = SipValue::String("hello".to_string());
    /// assert_eq!(value.to_string().unwrap(), "\"hello\"");
    /// ```
    pub fn to_string(&self) -> SipJsonResult<String> {
        let json_value = self.to_json_value();
        serde_json::to_string(&json_value)
            .map_err(|e| SipJsonError::SerializeError(e))
    }

    /// Convert to pretty-printed JSON string.
    ///
    /// # Returns
    /// A Result containing either the pretty-printed JSON string or an error
    pub fn to_string_pretty(&self) -> SipJsonResult<String> {
        let json_value = self.to_json_value();
        serde_json::to_string_pretty(&json_value)
            .map_err(|e| SipJsonError::SerializeError(e))
    }

    /// Parse from JSON string.
    ///
    /// # Arguments
    /// * `s` - A JSON string to parse
    ///
    /// # Returns
    /// A Result containing either the parsed SipValue or an error
    ///
    /// # Example
    ///
    /// ```
    /// # use rvoip_sip_core::json::SipValue;
    /// let json = r#"{"name":"Alice","age":30}"#;
    /// let value = SipValue::from_str(json).unwrap();
    /// assert_eq!(value.get_path("name").unwrap().as_str(), Some("Alice"));
    /// assert_eq!(value.get_path("age").unwrap().as_f64(), Some(30.0));
    /// ```
    pub fn from_str(s: &str) -> SipJsonResult<Self> {
        let json_value = serde_json::from_str(s)
            .map_err(|e| SipJsonError::DeserializeError(e))?;
        Ok(SipValue::from_json_value(&json_value))
    }
}

impl Default for SipValue {
    fn default() -> Self {
        SipValue::Null
    }
}

impl fmt::Display for SipValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.to_string() {
            Ok(s) => write!(f, "{}", s),
            Err(_) => write!(f, "Error converting to string"),
        }
    }
}

// From implementations for common types

impl From<bool> for SipValue {
    fn from(b: bool) -> Self {
        SipValue::Bool(b)
    }
}

impl From<i32> for SipValue {
    fn from(i: i32) -> Self {
        SipValue::Number(i as f64)
    }
}

impl From<i64> for SipValue {
    fn from(i: i64) -> Self {
        SipValue::Number(i as f64)
    }
}

impl From<f64> for SipValue {
    fn from(f: f64) -> Self {
        SipValue::Number(f)
    }
}

impl From<String> for SipValue {
    fn from(s: String) -> Self {
        SipValue::String(s)
    }
}

impl From<&str> for SipValue {
    fn from(s: &str) -> Self {
        SipValue::String(s.to_owned())
    }
}

impl<T> From<Vec<T>> for SipValue 
where 
    T: Into<SipValue>
{
    fn from(v: Vec<T>) -> Self {
        SipValue::Array(v.into_iter().map(|x| x.into()).collect())
    }
}

impl<T> From<HashMap<String, T>> for SipValue 
where 
    T: Into<SipValue>
{
    fn from(m: HashMap<String, T>) -> Self {
        SipValue::Object(m.into_iter().map(|(k, v)| (k, v.into())).collect())
    }
}

impl<T> From<Option<T>> for SipValue
where
    T: Into<SipValue>
{
    fn from(opt: Option<T>) -> Self {
        match opt {
            Some(v) => v.into(),
            None => SipValue::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_primitive_types() {
        // Test boolean
        let bool_value = SipValue::Bool(true);
        assert_eq!(bool_value.as_bool(), Some(true));
        assert!(bool_value.is_bool());
        assert!(!bool_value.is_null());
        assert!(!bool_value.is_number());
        
        // Test number
        let num_value = SipValue::Number(42.0);
        assert_eq!(num_value.as_f64(), Some(42.0));
        assert_eq!(num_value.as_i64(), Some(42));
        assert!(num_value.is_number());
        
        // Test string
        let str_value = SipValue::String("test".to_string());
        assert_eq!(str_value.as_str(), Some("test"));
        assert!(str_value.is_string());
        
        // Test null
        let null_value = SipValue::Null;
        assert!(null_value.is_null());
    }
    
    #[test]
    fn test_composite_types() {
        // Test array
        let array = vec![
            SipValue::Number(1.0),
            SipValue::Number(2.0),
            SipValue::String("three".to_string())
        ];
        let array_value = SipValue::Array(array.clone());
        
        assert!(array_value.is_array());
        assert_eq!(array_value.as_array().unwrap().len(), 3);
        assert_eq!(array_value.as_array().unwrap()[2].as_str(), Some("three"));
        
        // Test object
        let mut obj = HashMap::new();
        obj.insert("name".to_string(), SipValue::String("Alice".to_string()));
        obj.insert("age".to_string(), SipValue::Number(30.0));
        
        let obj_value = SipValue::Object(obj.clone());
        
        assert!(obj_value.is_object());
        assert_eq!(obj_value.as_object().unwrap().len(), 2);
        assert_eq!(obj_value.as_object().unwrap().get("name").unwrap().as_str(), Some("Alice"));
    }
    
    #[test]
    fn test_basic_path_access() {
        // Create a simpler structure to test basic path functionality
        let mut name_obj = HashMap::new();
        name_obj.insert("first".to_string(), SipValue::String("Alice".to_string()));
        name_obj.insert("last".to_string(), SipValue::String("Smith".to_string()));
        
        let mut user_obj = HashMap::new();
        user_obj.insert("name".to_string(), SipValue::Object(name_obj));
        user_obj.insert("age".to_string(), SipValue::Number(30.0));
        
        let mut address_obj = HashMap::new();
        address_obj.insert("city".to_string(), SipValue::String("New York".to_string()));
        address_obj.insert("zip".to_string(), SipValue::String("10001".to_string()));
        user_obj.insert("address".to_string(), SipValue::Object(address_obj));
        
        let value = SipValue::Object(user_obj);
        
        // Test simple path
        assert_eq!(value.get_path("age").unwrap().as_f64(), Some(30.0));
        
        // Test nested path
        assert_eq!(value.get_path("name.first").unwrap().as_str(), Some("Alice"));
        assert_eq!(value.get_path("name.last").unwrap().as_str(), Some("Smith"));
        
        // Test another nested path
        assert_eq!(value.get_path("address.city").unwrap().as_str(), Some("New York"));
        
        // Test non-existent path
        assert!(value.get_path("nonexistent").is_none());
        assert!(value.get_path("name.middle").is_none());
    }
    
    #[test]
    fn test_sip_message_path_access() {
        // Create a SIP message structure
        let mut obj = HashMap::new();
        
        // Add method
        obj.insert("method".to_string(), SipValue::String("INVITE".to_string()));
        
        // Create headers map
        let mut headers = HashMap::new();
        
        // Create From header
        let mut from = HashMap::new();
        from.insert("display_name".to_string(), SipValue::String("Alice".to_string()));
        from.insert("uri".to_string(), SipValue::String("sip:alice@example.com".to_string()));
        
        // Create From params with tag
        let mut from_params = Vec::new();
        let mut tag_param = HashMap::new();
        tag_param.insert("Tag".to_string(), SipValue::String("1234".to_string()));
        from_params.push(SipValue::Object(tag_param));
        from.insert("params".to_string(), SipValue::Array(from_params));
        
        headers.insert("From".to_string(), SipValue::Object(from));
        
        // Add the headers to the root object
        obj.insert("headers".to_string(), SipValue::Object(headers));
        
        let value = SipValue::Object(obj);
        
        // Test method access
        assert_eq!(value.get_path("method").unwrap().as_str(), Some("INVITE"));
        
        // Test From header access
        assert_eq!(value.get_path("headers.From.display_name").unwrap().as_str(), Some("Alice"));
        assert_eq!(value.get_path("headers.From.uri").unwrap().as_str(), Some("sip:alice@example.com"));
        
        // Test From tag access (deeply nested)
        assert_eq!(value.get_path("headers.From.params[0].Tag").unwrap().as_str(), Some("1234"));
    }
    
    #[test]
    fn test_json_conversion() {
        // Create a SipValue
        let mut obj = HashMap::new();
        obj.insert("name".to_string(), SipValue::String("Alice".to_string()));
        obj.insert("age".to_string(), SipValue::Number(30.0));
        obj.insert("active".to_string(), SipValue::Bool(true));
        
        let value = SipValue::Object(obj);
        
        // Convert to JSON string
        let json_str = value.to_string().unwrap();
        
        // Convert back to SipValue
        let parsed = SipValue::from_str(&json_str).unwrap();
        
        // Verify the round-trip conversion
        assert_eq!(parsed.get_path("name").unwrap().as_str(), Some("Alice"));
        assert_eq!(parsed.get_path("age").unwrap().as_f64(), Some(30.0));
        assert_eq!(parsed.get_path("active").unwrap().as_bool(), Some(true));
    }
    
    #[test]
    fn test_conversions() {
        // Test From<bool>
        let bool_value: SipValue = true.into();
        assert_eq!(bool_value, SipValue::Bool(true));
        
        // Test From<i32>
        let int_value: SipValue = 42.into();
        assert_eq!(int_value, SipValue::Number(42.0));
        
        // Test From<f64>
        let float_value: SipValue = 3.14.into();
        assert_eq!(float_value, SipValue::Number(3.14));
        
        // Test From<String>
        let string_value: SipValue = "hello".to_string().into();
        assert_eq!(string_value, SipValue::String("hello".to_string()));
        
        // Test From<&str>
        let str_value: SipValue = "world".into();
        assert_eq!(str_value, SipValue::String("world".to_string()));
        
        // Test From<Vec<T>>
        let vec_value: SipValue = vec![1, 2, 3].into();
        let expected = SipValue::Array(vec![
            SipValue::Number(1.0),
            SipValue::Number(2.0),
            SipValue::Number(3.0),
        ]);
        assert_eq!(vec_value, expected);
        
        // Test From<Option<T>>
        let some_value: SipValue = Some("test").into();
        assert_eq!(some_value, SipValue::String("test".to_string()));
        
        let none_value: SipValue = Option::<String>::None.into();
        assert_eq!(none_value, SipValue::Null);
    }
} 
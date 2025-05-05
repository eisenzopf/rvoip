//! # SIP JSON Value Representation
//!
//! This module provides a JSON-like value representation for SIP messages and components.
//! `SipValue` is the core type that can represent any SIP data as a JSON-compatible structure.
//!
//! ## Overview
//!
//! `SipValue` is an enum that can represent:
//! - Primitive values (null, boolean, number, string)
//! - Composite values (arrays, objects)
//!
//! This allows SIP messages to be converted to a language-agnostic intermediate representation
//! that can be easily manipulated, transformed, or serialized.
//!
//! ## Example
//!
//! ```
//! # use rvoip_sip_core::json::SipValue;
//! # use std::collections::HashMap;
//! // Create a SipValue object representing a From header
//! let mut params = HashMap::new();
//! params.insert("Tag".to_string(), SipValue::String("1234".to_string()));
//!
//! let mut from_header = HashMap::new();
//! from_header.insert("display_name".to_string(), SipValue::String("Alice".to_string()));
//! from_header.insert("uri".to_string(), SipValue::String("sip:alice@example.com".to_string()));
//! from_header.insert("params".to_string(), SipValue::Array(vec![SipValue::Object(params)]));
//!
//! let from_value = SipValue::Object(from_header);
//!
//! // Access fields using path notation
//! let tag = from_value.get_path("params[0].Tag").unwrap();
//! assert_eq!(tag.as_str(), Some("1234"));
//! ```

use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use serde_json;
use std::fmt;
use crate::json::{SipJsonResult, SipJsonError};

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
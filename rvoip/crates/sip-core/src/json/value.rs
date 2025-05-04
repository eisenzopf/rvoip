//! SipValue represents any SIP type as a JSON-like data structure

use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use serde_json;
use std::fmt;
use crate::json::{SipJsonResult, SipJsonError};

/// A JSON-like representation of SIP values
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
    /// Convert to a boolean value if possible
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            SipValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Convert to a numeric value if possible
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            SipValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// Convert to an integer value if possible
    pub fn as_i64(&self) -> Option<i64> {
        self.as_f64().and_then(|n| {
            if n.fract() == 0.0 && n >= i64::MIN as f64 && n <= i64::MAX as f64 {
                Some(n as i64)
            } else {
                None
            }
        })
    }

    /// Convert to a string value if possible
    pub fn as_str(&self) -> Option<&str> {
        match self {
            SipValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Convert to an array if possible
    pub fn as_array(&self) -> Option<&Vec<SipValue>> {
        match self {
            SipValue::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Convert to a mutable array if possible
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<SipValue>> {
        match self {
            SipValue::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Convert to an object if possible
    pub fn as_object(&self) -> Option<&HashMap<String, SipValue>> {
        match self {
            SipValue::Object(o) => Some(o),
            _ => None,
        }
    }

    /// Convert to a mutable object if possible
    pub fn as_object_mut(&mut self) -> Option<&mut HashMap<String, SipValue>> {
        match self {
            SipValue::Object(o) => Some(o),
            _ => None,
        }
    }

    /// Get a value from a path, e.g. "headers.via[0].branch"
    pub fn get_path<S: AsRef<str>>(&self, path: S) -> Option<&SipValue> {
        crate::json::path::get_path(self, path.as_ref())
    }

    /// Check if the value is null
    pub fn is_null(&self) -> bool {
        matches!(self, SipValue::Null)
    }

    /// Check if the value is a boolean
    pub fn is_bool(&self) -> bool {
        matches!(self, SipValue::Bool(_))
    }

    /// Check if the value is a number
    pub fn is_number(&self) -> bool {
        matches!(self, SipValue::Number(_))
    }

    /// Check if the value is a string
    pub fn is_string(&self) -> bool {
        matches!(self, SipValue::String(_))
    }

    /// Check if the value is an array
    pub fn is_array(&self) -> bool {
        matches!(self, SipValue::Array(_))
    }

    /// Check if the value is an object
    pub fn is_object(&self) -> bool {
        matches!(self, SipValue::Object(_))
    }

    /// Convert to a serde_json::Value
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

    /// Convert from a serde_json::Value
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

    /// Convert to JSON string
    pub fn to_string(&self) -> SipJsonResult<String> {
        let json_value = self.to_json_value();
        serde_json::to_string(&json_value)
            .map_err(|e| SipJsonError::SerializeError(e))
    }

    /// Convert to pretty-printed JSON string
    pub fn to_string_pretty(&self) -> SipJsonResult<String> {
        let json_value = self.to_json_value();
        serde_json::to_string_pretty(&json_value)
            .map_err(|e| SipJsonError::SerializeError(e))
    }

    /// Parse from JSON string
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
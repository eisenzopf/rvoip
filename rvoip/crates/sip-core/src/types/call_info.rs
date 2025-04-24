use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};

use crate::error::{Error, Result};
use crate::types::param::Param;
use crate::types::uri::Uri;
use crate::types::header::{Header, HeaderName, TypedHeaderTrait};
use crate::parser::headers::call_info::parse_call_info;

/// Represents the purpose of a Call-Info entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InfoPurpose {
    /// Icon - an image suitable as an iconic representation of the caller or callee
    Icon,
    /// Info - information about the caller or callee
    Info,
    /// Card - information about a business card for the caller or callee
    Card,
    /// Other purpose values (extensible)
    Other(String),
}

impl fmt::Display for InfoPurpose {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InfoPurpose::Icon => write!(f, "icon"),
            InfoPurpose::Info => write!(f, "info"),
            InfoPurpose::Card => write!(f, "card"),
            InfoPurpose::Other(val) => write!(f, "{}", val),
        }
    }
}

/// Represents a single entry in a Call-Info header
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallInfoValue {
    /// The URI for the call info
    pub uri: Uri,
    /// Parameters, including the purpose parameter
    pub params: Vec<Param>,
}

impl CallInfoValue {
    /// Create a new Call-Info value with a URI
    pub fn new(uri: Uri) -> Self {
        CallInfoValue {
            uri,
            params: Vec::new(),
        }
    }

    /// Add a parameter
    pub fn with_param(mut self, param: Param) -> Self {
        self.params.push(param);
        self
    }

    /// Set the purpose parameter
    pub fn with_purpose(self, purpose: InfoPurpose) -> Self {
        let purpose_param = match purpose {
            InfoPurpose::Icon => Param::Other("purpose".to_string(), Some(crate::types::param::GenericValue::Token("icon".to_string()))),
            InfoPurpose::Info => Param::Other("purpose".to_string(), Some(crate::types::param::GenericValue::Token("info".to_string()))),
            InfoPurpose::Card => Param::Other("purpose".to_string(), Some(crate::types::param::GenericValue::Token("card".to_string()))),
            InfoPurpose::Other(val) => Param::Other("purpose".to_string(), Some(crate::types::param::GenericValue::Token(val))),
        };
        self.with_param(purpose_param)
    }

    /// Get the purpose parameter, if present
    pub fn purpose(&self) -> Option<InfoPurpose> {
        for param in &self.params {
            if let Param::Other(name, Some(crate::types::param::GenericValue::Token(value))) = param {
                if name == "purpose" {
                    return match value.as_str() {
                        "icon" => Some(InfoPurpose::Icon),
                        "info" => Some(InfoPurpose::Info),
                        "card" => Some(InfoPurpose::Card),
                        other => Some(InfoPurpose::Other(other.to_string())),
                    };
                }
            }
        }
        None
    }
}

impl fmt::Display for CallInfoValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}>", self.uri)?;
        for param in &self.params {
            write!(f, ";{}", param)?;
        }
        Ok(())
    }
}

/// Represents a Call-Info header as defined in RFC 3261
/// The Call-Info header field provides additional information about the caller or callee,
/// depending on whether it is found in a request or response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallInfo(pub Vec<CallInfoValue>);

impl CallInfo {
    /// Create a new Call-Info header with a list of values
    pub fn new(values: Vec<CallInfoValue>) -> Self {
        CallInfo(values)
    }

    /// Create a new Call-Info header with a single value
    pub fn with_value(value: CallInfoValue) -> Self {
        CallInfo(vec![value])
    }
}

impl TypedHeaderTrait for CallInfo {
    type Name = HeaderName;
    
    fn header_name() -> Self::Name {
        HeaderName::CallInfo
    }
    
    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), crate::types::header::HeaderValue::Raw(self.to_string().into_bytes()))
    }
    
    fn from_header(header: &Header) -> Result<Self> {
        match header.value {
            crate::types::header::HeaderValue::CallInfo(ref values) => {
                Ok(CallInfo(values.clone()))
            },
            crate::types::header::HeaderValue::Raw(ref bytes) => {
                match std::str::from_utf8(bytes) {
                    Ok(s) => s.parse(),
                    Err(_) => Err(Error::ParseError("Invalid UTF-8".to_string())),
                }
            },
            _ => Err(Error::ParseError("Invalid header value type for Call-Info".to_string())),
        }
    }
}

impl fmt::Display for CallInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return Ok(());
        }

        let mut first = true;
        for value in &self.0 {
            if !first {
                write!(f, ", ")?;
            }
            write!(f, "{}", value)?;
            first = false;
        }
        Ok(())
    }
}

impl FromStr for CallInfo {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let parse_result = parse_call_info(s.as_bytes());
        match parse_result {
            Ok((_, values)) => Ok(CallInfo(values)),
            Err(e) => Err(Error::from(e)),
        }
    }
} 
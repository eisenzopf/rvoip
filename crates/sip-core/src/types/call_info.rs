//! # SIP Call-Info Header
//! 
//! This module provides an implementation of the SIP Call-Info header as defined in
//! [RFC 3261 Section 20.9](https://datatracker.ietf.org/doc/html/rfc3261#section-20.9).
//!
//! The Call-Info header provides additional information about the caller or callee,
//! depending on whether it's found in a request or response. This information can include:
//!
//! - Icons representing the caller or callee
//! - Caller/callee information pages
//! - Business card information
//! - Other application-specific data
//!
//! ## Format
//!
//! The Call-Info header consists of a URI and parameters, with the most important
//! parameter being "purpose", which indicates how the information should be interpreted:
//!
//! ```text
//! Call-Info: <http://example.com/alice/photo.jpg>;purpose=icon
//! ```
//!
//! Multiple Call-Info entries can be specified in a single header, separated by commas:
//!
//! ```text
//! Call-Info: <http://example.com/alice/photo.jpg>;purpose=icon,
//!            <http://example.com/alice/info.html>;purpose=info
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::types::call_info::{CallInfo, CallInfoValue, InfoPurpose};
//! use rvoip_sip_core::types::uri::Uri;
//! use std::str::FromStr;
//!
//! // Create a Call-Info header with an icon
//! let uri = Uri::http("example.com"); // Using the new http builder
//! let value = CallInfoValue::new(uri).with_purpose(InfoPurpose::Icon);
//! let call_info = CallInfo::with_value(value);
//!
//! // Parse a Call-Info header from a string
//! let call_info = CallInfo::from_str("<http://example.com/alice/photo.jpg>;purpose=icon").unwrap();
//! ```

use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};

use crate::error::{Error, Result};
use crate::types::param::Param;
use crate::types::uri::Uri;
use crate::types::header::{Header, HeaderName, TypedHeaderTrait};
use crate::parser::headers::call_info::parse_call_info;

/// Represents the purpose of a Call-Info entry
///
/// The purpose parameter indicates how the information referenced by the Call-Info
/// header should be interpreted or presented to the user.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::call_info::InfoPurpose;
///
/// let icon = InfoPurpose::Icon;
/// assert_eq!(icon.to_string(), "icon");
///
/// let info = InfoPurpose::Info;
/// assert_eq!(info.to_string(), "info");
///
/// let card = InfoPurpose::Card;
/// assert_eq!(card.to_string(), "card");
///
/// let other = InfoPurpose::Other("custom".to_string());
/// assert_eq!(other.to_string(), "custom");
/// ```
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
///
/// Each Call-Info entry consists of a URI pointing to the information resource
/// and optional parameters, most notably the "purpose" parameter that indicates
/// how the information should be interpreted.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::call_info::{CallInfoValue, InfoPurpose};
/// use rvoip_sip_core::types::uri::Uri;
/// use std::str::FromStr;
///
/// // Create a Call-Info entry with an icon
/// let uri = Uri::http("example.com/alice/photo.jpg");
/// let value = CallInfoValue::new(uri).with_purpose(InfoPurpose::Icon);
///
/// // Get the purpose
/// assert_eq!(value.purpose().unwrap(), InfoPurpose::Icon);
///
/// // Convert to string
/// assert!(value.to_string().contains("purpose=icon"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallInfoValue {
    /// The URI for the call info
    pub uri: Uri,
    /// Parameters, including the purpose parameter
    pub params: Vec<Param>,
}

impl CallInfoValue {
    /// Create a new Call-Info value with a URI
    ///
    /// Creates a Call-Info entry with the specified URI and no parameters.
    /// Typically, you would chain this with `with_purpose` to specify how
    /// the information should be interpreted.
    ///
    /// # Parameters
    ///
    /// - `uri`: The URI pointing to the information resource
    ///
    /// # Returns
    ///
    /// A new `CallInfoValue` with the specified URI
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::call_info::CallInfoValue;
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::http("example.com/alice/photo.jpg");
    /// let value = CallInfoValue::new(uri);
    /// ```
    pub fn new(uri: Uri) -> Self {
        CallInfoValue {
            uri,
            params: Vec::new(),
        }
    }

    /// Add a parameter
    ///
    /// Adds a generic parameter to the Call-Info entry.
    ///
    /// # Parameters
    ///
    /// - `param`: The parameter to add
    ///
    /// # Returns
    ///
    /// The modified `CallInfoValue` with the added parameter
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::call_info::CallInfoValue;
    /// use rvoip_sip_core::types::uri::Uri;
    /// use rvoip_sip_core::types::param::Param;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::http("example.com/alice/photo.jpg");
    /// // Create a custom parameter instead of using from_str
    /// let param = Param::Other("refresh".to_string(), Some("60".into()));
    /// let value = CallInfoValue::new(uri).with_param(param);
    /// ```
    pub fn with_param(mut self, param: Param) -> Self {
        self.params.push(param);
        self
    }

    /// Set the purpose parameter
    ///
    /// Sets the purpose parameter, which indicates how the information should be
    /// interpreted or presented to the user.
    ///
    /// # Parameters
    ///
    /// - `purpose`: The purpose of the information
    ///
    /// # Returns
    ///
    /// The modified `CallInfoValue` with the purpose parameter set
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::call_info::{CallInfoValue, InfoPurpose};
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::http("example.com/alice/photo.jpg");
    /// 
    /// // Set common purpose values
    /// let icon_value = CallInfoValue::new(uri.clone()).with_purpose(InfoPurpose::Icon);
    /// let info_value = CallInfoValue::new(uri.clone()).with_purpose(InfoPurpose::Info);
    /// let card_value = CallInfoValue::new(uri.clone()).with_purpose(InfoPurpose::Card);
    /// 
    /// // Set custom purpose value
    /// let custom_value = CallInfoValue::new(uri).with_purpose(InfoPurpose::Other("ringtone".to_string()));
    /// ```
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
    ///
    /// Retrieves the purpose parameter that indicates how the information should be
    /// interpreted or presented to the user.
    ///
    /// # Returns
    ///
    /// An `Option<InfoPurpose>` containing the purpose if present, or `None` if not
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::call_info::{CallInfoValue, InfoPurpose};
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::http("example.com/alice/photo.jpg");
    /// let value = CallInfoValue::new(uri).with_purpose(InfoPurpose::Icon);
    ///
    /// assert_eq!(value.purpose().unwrap(), InfoPurpose::Icon);
    /// ```
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
///
/// The Call-Info header can contain multiple values, each representing a different
/// piece of information about the caller or callee. Common uses include providing
/// links to user icons, information pages, or business cards.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::call_info::{CallInfo, CallInfoValue, InfoPurpose};
/// use rvoip_sip_core::types::uri::Uri;
/// use std::str::FromStr;
///
/// // Create a Call-Info header with a single value
/// let uri = Uri::http("example.com/alice/photo.jpg");
/// let value = CallInfoValue::new(uri).with_purpose(InfoPurpose::Icon);
/// let call_info = CallInfo::with_value(value);
///
/// // Create a Call-Info header with multiple values
/// let uri1 = Uri::http("example.com/alice/photo.jpg");
/// let uri2 = Uri::http("example.com/alice/info.html");
/// let value1 = CallInfoValue::new(uri1).with_purpose(InfoPurpose::Icon);
/// let value2 = CallInfoValue::new(uri2).with_purpose(InfoPurpose::Info);
/// let call_info = CallInfo::new(vec![value1, value2]);
///
/// // Parse from a string
/// let call_info = CallInfo::from_str("<http://example.com/alice/photo.jpg>;purpose=icon").unwrap();
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallInfo(pub Vec<CallInfoValue>);

impl CallInfo {
    /// Create a new Call-Info header with a list of values
    ///
    /// Creates a new Call-Info header containing multiple entries.
    ///
    /// # Parameters
    ///
    /// - `values`: A vector of Call-Info values
    ///
    /// # Returns
    ///
    /// A new `CallInfo` header with the specified values
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::call_info::{CallInfo, CallInfoValue, InfoPurpose};
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// let uri1 = Uri::http("example.com/alice/photo.jpg");
    /// let uri2 = Uri::http("example.com/alice/info.html");
    /// let value1 = CallInfoValue::new(uri1).with_purpose(InfoPurpose::Icon);
    /// let value2 = CallInfoValue::new(uri2).with_purpose(InfoPurpose::Info);
    ///
    /// let call_info = CallInfo::new(vec![value1, value2]);
    /// ```
    pub fn new(values: Vec<CallInfoValue>) -> Self {
        CallInfo(values)
    }

    /// Create a new Call-Info header with a single value
    ///
    /// Creates a new Call-Info header containing a single entry.
    /// This is a convenience method for the common case of a header with just one value.
    ///
    /// # Parameters
    ///
    /// - `value`: A single Call-Info value
    ///
    /// # Returns
    ///
    /// A new `CallInfo` header with the specified value
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::call_info::{CallInfo, CallInfoValue, InfoPurpose};
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::http("example.com/alice/photo.jpg");
    /// let value = CallInfoValue::new(uri).with_purpose(InfoPurpose::Icon);
    ///
    /// let call_info = CallInfo::with_value(value);
    /// ```
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

    /// Parse a string into a CallInfo header.
    ///
    /// This method parses a string representation of a Call-Info header into a
    /// `CallInfo` struct following the format specified in RFC 3261.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed `CallInfo`, or an error if parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::call_info::CallInfo;
    /// use std::str::FromStr;
    ///
    /// // Parse a single Call-Info value
    /// let call_info = CallInfo::from_str("<http://example.com/alice/photo.jpg>;purpose=icon").unwrap();
    ///
    /// // Parse multiple Call-Info values
    /// let call_info = CallInfo::from_str(
    ///     "<http://example.com/alice/photo.jpg>;purpose=icon, \
    ///      <http://example.com/alice/info.html>;purpose=info"
    /// ).unwrap();
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        let parse_result = parse_call_info(s.as_bytes());
        match parse_result {
            Ok((_, values)) => Ok(CallInfo(values)),
            Err(e) => Err(Error::from(e)),
        }
    }
} 
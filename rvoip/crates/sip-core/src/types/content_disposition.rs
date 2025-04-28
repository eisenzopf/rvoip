//! # SIP Content-Disposition Header
//!
//! This module provides an implementation of the SIP Content-Disposition header as defined in
//! [RFC 3261 Section 20.11](https://datatracker.ietf.org/doc/html/rfc3261#section-20.11) and
//! [RFC 2183](https://datatracker.ietf.org/doc/html/rfc2183).
//!
//! The Content-Disposition header describes how the message body should be interpreted
//! by the recipient. It consists of a disposition type and optional parameters that
//! provide additional information about how to handle the body.
//!
//! ## Common Disposition Types
//!
//! - **session**: The body describes a session (SDP typically uses this)
//! - **render**: The body should be displayed or rendered to the user
//! - **icon**: The body is an image suitable for rendering as an icon
//! - **alert**: The body is information that should alert the user
//!
//! ## Format
//!
//! ```text
//! Content-Disposition: session
//! Content-Disposition: render;handling=optional
//! Content-Disposition: icon;size=32
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a simple Content-Disposition header
//! let disposition = ContentDisposition::from_str("session").unwrap();
//! assert!(matches!(disposition.disposition_type, DispositionType::Session));
//!
//! // Create with parameters
//! let disposition = ContentDisposition::from_str("render;handling=optional").unwrap();
//! assert!(matches!(disposition.disposition_type, DispositionType::Render));
//! assert_eq!(disposition.params.get("handling"), Some(&"optional".to_string()));
//! ```

use std::collections::HashMap;
use crate::parser::headers::parse_content_disposition;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use crate::types::param::Param;
use serde::{Serialize, Deserialize};
use crate::types::param::GenericValue;

/// Represents the 'handling' parameter values for Content-Disposition
///
/// The 'handling' parameter indicates whether understanding the body
/// part is optional or required.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create different handling values
/// let optional = Handling::Optional;
/// let required = Handling::Required;
/// let custom = Handling::Other("custom-value".to_string());
///
/// // Use in a Content-Disposition parameter
/// let param = DispositionParam::Handling(optional);
/// assert!(matches!(param, DispositionParam::Handling(Handling::Optional)));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Handling {
    /// The UAS may ignore the body part if it doesn't understand it
    Optional,
    /// The UAS must understand the body part or reject the request
    Required,
    /// Custom handling value
    Other(String),
}

/// Represents a parameter in a Content-Disposition header
///
/// Parameters provide additional information about how to handle the body.
/// They can be either specific parameters like 'handling' or generic parameters.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create a handling parameter
/// let handling_param = DispositionParam::Handling(Handling::Optional);
///
/// // Create a generic parameter
/// let generic_param = DispositionParam::Generic(
///     Param::Other("filename".to_string(), Some(GenericValue::Token("document.txt".to_string())))
/// );
///
/// // Match on parameter types
/// match handling_param {
///     DispositionParam::Handling(h) => {
///         assert!(matches!(h, Handling::Optional));
///     },
///     _ => panic!("Expected handling parameter"),
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DispositionParam {
    /// The 'handling' parameter indicates whether understanding 
    /// the body part is optional or required
    Handling(Handling),
    /// Any other parameter represented as key-value pairs
    Generic(Param),
}

/// Content Disposition Type (session, render, icon, alert, etc.)
///
/// This enum represents the disposition type in a Content-Disposition header,
/// indicating how the message body should be interpreted by the recipient.
///
/// # Standard Types (RFC 3261 & RFC 2183)
///
/// - `Session`: Indicates the body is a session description (commonly used with SDP)
/// - `Render`: Indicates the body should be displayed/rendered to the user
/// - `Icon`: Indicates the body is an image suitable for rendering as an icon
/// - `Alert`: Indicates the body contains information that should alert the user
/// - `Other`: Allows for extension to other disposition types
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Parse standard disposition types
/// let session = DispositionType::from_str("session").unwrap();
/// assert!(matches!(session, DispositionType::Session));
///
/// let render = DispositionType::from_str("render").unwrap();
/// assert!(matches!(render, DispositionType::Render));
///
/// // Parse custom disposition type
/// let custom = DispositionType::from_str("attachment").unwrap();
/// assert!(matches!(custom, DispositionType::Other(ref s) if s == "attachment"));
///
/// // Convert to string
/// assert_eq!(session.to_string(), "session");
/// assert_eq!(render.to_string(), "render");
/// assert_eq!(custom.to_string(), "attachment");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DispositionType {
    /// Indicates the body is a session description (typically SDP)
    Session,
    /// Indicates the body should be displayed or otherwise rendered to the user
    Render,
    /// Indicates the body is an image suitable for rendering as an icon
    Icon,
    /// Indicates the body contains information that should alert the user
    Alert,
    /// Custom or extension disposition type
    Other(String),
}

impl fmt::Display for DispositionType {
    /// Formats the disposition type as a string.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let session = DispositionType::Session;
    /// assert_eq!(session.to_string(), "session");
    ///
    /// let custom = DispositionType::Other("attachment".to_string());
    /// assert_eq!(custom.to_string(), "attachment");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DispositionType::Session => write!(f, "session"),
            DispositionType::Render => write!(f, "render"),
            DispositionType::Icon => write!(f, "icon"),
            DispositionType::Alert => write!(f, "alert"),
            DispositionType::Other(s) => write!(f, "{}", s),
        }
    }
}

impl FromStr for DispositionType {
    type Err = Error;

    /// Parses a string into a DispositionType.
    ///
    /// This method is case-insensitive for standard disposition types.
    /// Any unrecognized type becomes `DispositionType::Other`.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed DispositionType, or an error
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse standard disposition types (case-insensitive)
    /// let session = DispositionType::from_str("SeSSioN").unwrap();
    /// assert!(matches!(session, DispositionType::Session));
    ///
    /// // Parse custom disposition type
    /// let custom = DispositionType::from_str("attachment").unwrap();
    /// assert!(matches!(custom, DispositionType::Other(s) if s == "attachment"));
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "session" => Ok(DispositionType::Session),
            "render" => Ok(DispositionType::Render),
            "icon" => Ok(DispositionType::Icon),
            "alert" => Ok(DispositionType::Alert),
            _ => Ok(DispositionType::Other(s.to_string())),
        }
    }
}

/// Typed Content-Disposition header.
///
/// This struct represents a SIP Content-Disposition header, which describes how
/// the message body should be interpreted by the recipient. It consists of a
/// disposition type and optional parameters providing additional information.
///
/// Common parameters include:
/// - `handling`: Indicates whether the body is required or optional
/// - `size`: The size of the content (often used with icon type)
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a Content-Disposition header from a string
/// let disposition = ContentDisposition::from_str("session").unwrap();
/// assert!(matches!(disposition.disposition_type, DispositionType::Session));
/// assert!(disposition.params.is_empty());
///
/// // Parse with parameters
/// let disposition = ContentDisposition::from_str("render;handling=optional").unwrap();
/// assert!(matches!(disposition.disposition_type, DispositionType::Render));
/// assert_eq!(disposition.params.get("handling"), Some(&"optional".to_string()));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentDisposition {
    /// The disposition type
    pub disposition_type: DispositionType,
    /// Optional parameters for the Content-Disposition
    pub params: HashMap<String, String>,
}

impl fmt::Display for ContentDisposition {
    /// Formats the Content-Disposition header as a string.
    ///
    /// This creates the serialized form of the Content-Disposition header
    /// according to the SIP specification. Parameters are appended with
    /// proper quoting for values containing special characters.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    /// use std::collections::HashMap;
    ///
    /// // Simple disposition
    /// let disposition = ContentDisposition {
    ///     disposition_type: DispositionType::Session,
    ///     params: HashMap::new(),
    /// };
    /// assert_eq!(disposition.to_string(), "session");
    ///
    /// // With parameters
    /// let mut params = HashMap::new();
    /// params.insert("handling".to_string(), "optional".to_string());
    /// let disposition = ContentDisposition {
    ///     disposition_type: DispositionType::Render,
    ///     params,
    /// };
    /// assert_eq!(disposition.to_string(), "render;handling=optional");
    ///
    /// // With quoted parameter (contains special characters)
    /// let mut params = HashMap::new();
    /// params.insert("filename".to_string(), "example file.txt".to_string());
    /// let disposition = ContentDisposition {
    ///     disposition_type: DispositionType::Icon,
    ///     params,
    /// };
    /// assert_eq!(disposition.to_string(), "icon;filename=\"example file.txt\"");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.disposition_type)?;
        for (key, value) in &self.params {
            // Remove internal quote escaping for now
            if value.chars().any(|c| !c.is_ascii_alphanumeric() && !matches!(c, '!' | '#' | '$' | '%' | '&' | '\'' | '^' | '_' | '`' | '{' | '}' | '~' | '-')) {
                write!(f, ";{}=\"{}\"", key, value)?;
            } else {
                write!(f, ";{}={}", key, value)?;
            }
        }
        Ok(())
    }
}

impl FromStr for ContentDisposition {
    type Err = Error;

    /// Parses a string into a ContentDisposition.
    ///
    /// This parses a Content-Disposition header string into a structured
    /// ContentDisposition object, extracting the disposition type and
    /// any parameters.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed ContentDisposition, or an error if parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a simple Content-Disposition header
    /// let disposition = ContentDisposition::from_str("session").unwrap();
    /// assert!(matches!(disposition.disposition_type, DispositionType::Session));
    /// assert!(disposition.params.is_empty());
    ///
    /// // Parse with parameters
    /// let disposition = ContentDisposition::from_str("render;handling=optional").unwrap();
    /// assert!(matches!(disposition.disposition_type, DispositionType::Render));
    /// assert_eq!(disposition.params.get("handling"), Some(&"optional".to_string()));
    ///
    /// // Parse with a quoted parameter value
    /// let disposition = ContentDisposition::from_str("icon;filename=\"icon.png\"").unwrap();
    /// assert!(matches!(disposition.disposition_type, DispositionType::Icon));
    /// assert_eq!(disposition.params.get("filename"), Some(&"icon.png".to_string()));
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        use crate::parser::headers::content_disposition::parse_content_disposition;
        use nom::combinator::all_consuming;

        all_consuming(parse_content_disposition)(s.as_bytes())
            .map_err(Error::from)
            .and_then(|(_, (dtype_bytes, params_vec))| {
                // String is already a String type, so we don't need to_vec()
                let disp_type = match dtype_bytes.as_str() {
                    "session" => DispositionType::Session,
                    "render" => DispositionType::Render,
                    "icon" => DispositionType::Icon,
                    "alert" => DispositionType::Alert,
                    _ => DispositionType::Other(dtype_bytes),
                };
                
                // Convert params to HashMap
                let mut params = HashMap::new();
                for param in params_vec {
                    match param {
                        DispositionParam::Handling(handling) => {
                            // Convert Handling enum to string value
                            let value = match handling {
                                Handling::Optional => "optional".to_string(),
                                Handling::Required => "required".to_string(),
                                Handling::Other(s) => s,
                            };
                            params.insert("handling".to_string(), value);
                        },
                        DispositionParam::Generic(generic_param) => {
                            // Extract key and value from generic parameter
                            match generic_param {
                                Param::Other(name, Some(GenericValue::Token(value))) => {
                                    params.insert(name, value);
                                },
                                Param::Other(name, Some(GenericValue::Quoted(value))) => {
                                    params.insert(name, value);
                                },
                                Param::Other(name, None) => {
                                    // Flag parameter without value
                                    params.insert(name, "".to_string());
                                },
                                _ => {
                                    // Skip other parameter types for now
                                }
                            }
                        }
                    }
                }
                
                Ok(ContentDisposition { disposition_type: disp_type, params })
            })
    }
}

// TODO: Implement methods, FromStr, Display 
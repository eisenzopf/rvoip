//! # URI With Parameters
//!
//! This module provides support for SIP/SIPS URIs with associated parameters, which are
//! commonly used in SIP headers such as Route, Record-Route, and Contact.
//!
//! In SIP, many header fields contain URIs that can have parameters associated with them.
//! These parameters influence how the URI is processed. For example, the `lr` parameter
//! in a Route header indicates that the proxy supports loose routing.
//!
//! ## Example
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! 
//! // Create a URI
//! let uri = Uri::new(Scheme::Sip, "proxy.example.com".to_string());
//! 
//! // Create a URI with parameters
//! let uri_with_params = UriWithParams::new(uri)
//!     .with_param(Param::new("lr".to_string(), None));
//! 
//! // Format as a string for a SIP header
//! assert_eq!(uri_with_params.to_string(), "sip:proxy.example.com;lr");
//! ```

use crate::types::uri::Uri;
use crate::types::param::Param;
use std::fmt;
use serde::{Serialize, Deserialize};

/// Represents a URI with associated parameters.
///
/// In SIP, URIs often have parameters associated with them that modify their behavior
/// or provide additional information. This struct combines a SIP URI with a set of
/// parameters, which is a common pattern in headers such as Route, Record-Route,
/// and Contact.
///
/// The parameters in this struct are separate from the URI's own parameters. 
/// For example, in `<sip:user@domain;uri-param>;header-param`, the `uri-param` is 
/// part of the `Uri` itself, while `header-param` would be in the `params` list
/// of this struct.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a URI
/// let uri = Uri::from_str("sip:proxy.example.com").unwrap();
///
/// // Create a URI with parameters
/// let uri_with_params = UriWithParams::new(uri)
///     .with_param(Param::new("lr".to_string(), None));
///
/// // Format the URI with parameters
/// assert_eq!(uri_with_params.to_string(), "sip:proxy.example.com;lr");
///
/// // Create a URI with a parameter that has a value
/// let uri = Uri::from_str("sip:proxy.example.com").unwrap();
/// let uri_with_params = UriWithParams::new(uri)
///     .with_param(Param::new("q".to_string(), Some("0.8".to_string())));
/// 
/// assert_eq!(uri_with_params.to_string(), "sip:proxy.example.com;q=0.8");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UriWithParams {
    pub uri: Uri,
    pub params: Vec<Param>,
}

impl UriWithParams {
    /// Creates a new UriWithParams.
    ///
    /// Initializes a `UriWithParams` with the provided URI and an empty parameter list.
    /// Additional parameters can be added using the `with_param` method.
    ///
    /// # Parameters
    ///
    /// - `uri`: The URI to associate parameters with
    ///
    /// # Returns
    ///
    /// A new `UriWithParams` instance containing the specified URI and no parameters
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:proxy.example.com").unwrap();
    /// let uri_with_params = UriWithParams::new(uri);
    ///
    /// // Initially there are no parameters
    /// assert!(uri_with_params.params.is_empty());
    /// ```
    pub fn new(uri: Uri) -> Self {
        Self { uri, params: Vec::new() }
    }

    /// Builder method to add a parameter.
    ///
    /// Adds a parameter to the URI and returns the modified `UriWithParams` instance,
    /// allowing for method chaining to add multiple parameters.
    ///
    /// # Parameters
    ///
    /// - `param`: The parameter to add to the URI
    ///
    /// # Returns
    ///
    /// The `UriWithParams` instance with the new parameter added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:proxy.example.com").unwrap();
    ///
    /// // Add a simple flag parameter (no value)
    /// let uri_with_params = UriWithParams::new(uri.clone())
    ///     .with_param(Param::new("lr".to_string(), None));
    ///
    /// // Add multiple parameters with method chaining
    /// let uri_with_multiple_params = UriWithParams::new(uri)
    ///     .with_param(Param::new("lr".to_string(), None))
    ///     .with_param(Param::new("q".to_string(), Some("0.8".to_string())));
    ///
    /// // The URI now has two parameters
    /// assert_eq!(uri_with_multiple_params.params.len(), 2);
    /// ```
    pub fn with_param(mut self, param: Param) -> Self {
        self.params.push(param);
        self
    }
}

// Implement Display for UriWithParams
impl fmt::Display for UriWithParams {
    /// Formats the URI with parameters as a string.
    ///
    /// Converts the `UriWithParams` to its string representation suitable for
    /// inclusion in a SIP message. The format includes the URI followed by
    /// its parameters, each prefixed with a semicolon.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    /// use std::fmt::Display;
    ///
    /// let uri = Uri::from_str("sip:proxy.example.com").unwrap();
    ///
    /// // Create a URI with a parameter
    /// let uri_with_params = UriWithParams::new(uri)
    ///     .with_param(Param::new("lr".to_string(), None));
    ///
    /// // Format as a string
    /// assert_eq!(uri_with_params.to_string(), "sip:proxy.example.com;lr");
    ///
    /// // Format a URI with a parameter that has a value
    /// let uri = Uri::from_str("sip:proxy.example.com").unwrap();
    /// let uri_with_params = UriWithParams::new(uri)
    ///     .with_param(Param::new("q".to_string(), Some("0.8".to_string())));
    ///
    /// assert_eq!(uri_with_params.to_string(), "sip:proxy.example.com;q=0.8");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Display the URI part (which includes its own parameters)
        write!(f, "{}", self.uri)?;
        // Display the *header* parameters associated with this URI in the list
        for param in &self.params {
            write!(f, "{}", param)?;
        }
        Ok(())
    }
}

// TODO: Implement helper methods 
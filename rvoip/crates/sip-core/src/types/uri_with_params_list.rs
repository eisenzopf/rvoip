//! # URI With Parameters List
//!
//! This module provides a container type for managing lists of URIs with associated parameters.
//! It is primarily used in SIP headers that need to represent multiple URIs, such as
//! Route, Record-Route, Path, and Service-Route headers.
//!
//! In SIP, many headers contain one or more URIs, each with their own set of parameters.
//! For example, a Record-Route header might contain a list of proxy servers that a
//! request has traversed, with each proxy server represented by a URI with parameters.
//!
//! ## Example
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a list of URIs with parameters
//! let mut uri_list = UriWithParamsList::new();
//! 
//! // Add URIs to the list
//! let uri1 = UriWithParams::from_str("<sip:proxy1.example.com;lr>").unwrap();
//! let uri2 = UriWithParams::from_str("<sip:proxy2.example.com;lr>").unwrap();
//! 
//! uri_list.push(uri1);
//! uri_list.push(uri2);
//! 
//! // Format as a string for a SIP header
//! assert_eq!(uri_list.to_string(), "<sip:proxy1.example.com;lr>, <sip:proxy2.example.com;lr>");
//! ```

use std::fmt;
use std::str::FromStr;
use crate::types::uri_with_params::UriWithParams;
use crate::error::Result;
use serde::{Serialize, Deserialize};

/// Represents a list of URIs with parameters (e.g., for Route, Record-Route).
///
/// This type provides a container for managing collections of `UriWithParams` objects,
/// which is useful for SIP headers that contain lists of URIs with associated parameters.
/// Common examples include:
///
/// - Route headers: List of proxies that a request should traverse
/// - Record-Route headers: List of proxies that a dialog has traversed
/// - Path headers: List of proxies for registration path routing
/// - Service-Route headers: List of services that should handle requests
///
/// The `UriWithParamsList` implements standard collection operations like iteration,
/// and provides methods for managing the URIs within the list.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a new list for a Route header
/// let mut route_list = UriWithParamsList::new();
///
/// // Add some proxy URIs with the loose routing parameter
/// let uri1 = UriWithParams::from_str("<sip:proxy1.example.com;lr>").unwrap();
/// let uri2 = UriWithParams::from_str("<sip:proxy2.example.com;lr>").unwrap();
///
/// route_list.push(uri1);
/// route_list.push(uri2);
///
/// // Access URIs in the list
/// if let Some(first_hop) = route_list.first() {
///     assert_eq!(first_hop.uri().host(), "proxy1.example.com");
/// }
///
/// // Iterate through the list
/// for uri in &route_list {
///     assert!(uri.has_param("lr")); // All proxies should have the 'lr' parameter
/// }
///
/// // Format as a string for a SIP header
/// assert_eq!(route_list.to_string(), "<sip:proxy1.example.com;lr>, <sip:proxy2.example.com;lr>");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)] // Add Default derive
pub struct UriWithParamsList {
    pub uris: Vec<UriWithParams>,
}

impl UriWithParamsList {
    /// Creates an empty list.
    ///
    /// Initializes a new `UriWithParamsList` with no URIs.
    ///
    /// # Returns
    ///
    /// A new, empty `UriWithParamsList` instance.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let list = UriWithParamsList::new();
    /// assert!(list.is_empty());
    /// assert_eq!(list.len(), 0);
    /// ```
    pub fn new() -> Self {
        Self { uris: Vec::new() }
    }

    /// Creates an empty list with the specified capacity.
    ///
    /// Initializes a new `UriWithParamsList` with pre-allocated capacity,
    /// which can improve performance when the expected number of URIs is known.
    ///
    /// # Parameters
    ///
    /// - `capacity`: The number of URIs the list should be able to hold without reallocating
    ///
    /// # Returns
    ///
    /// A new, empty `UriWithParamsList` with the specified capacity.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a list with capacity for 3 URIs
    /// let list = UriWithParamsList::with_capacity(3);
    /// assert!(list.is_empty());
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        Self { uris: Vec::with_capacity(capacity) }
    }

    /// Adds a UriWithParams to the list.
    ///
    /// Appends a URI with parameters to the end of the list.
    ///
    /// # Parameters
    ///
    /// - `uri`: The `UriWithParams` instance to add to the list
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let mut list = UriWithParamsList::new();
    /// let uri = UriWithParams::from_str("<sip:proxy.example.com;lr>").unwrap();
    ///
    /// list.push(uri);
    /// assert_eq!(list.len(), 1);
    ///
    /// // Add another URI
    /// let uri2 = UriWithParams::from_str("<sip:proxy2.example.com;lr>").unwrap();
    /// list.push(uri2);
    /// assert_eq!(list.len(), 2);
    /// ```
    pub fn push(&mut self, uri: UriWithParams) {
        self.uris.push(uri);
    }

    /// Returns an iterator over the URIs.
    ///
    /// Provides an immutable iterator over all `UriWithParams` instances in the list.
    ///
    /// # Returns
    ///
    /// An iterator over references to the URIs in the list
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let mut list = UriWithParamsList::new();
    /// list.push(UriWithParams::from_str("<sip:proxy1.example.com;lr>").unwrap());
    /// list.push(UriWithParams::from_str("<sip:proxy2.example.com;lr>").unwrap());
    ///
    /// for uri in list.iter() {
    ///     assert!(uri.has_param("lr"));
    /// }
    /// ```
    pub fn iter(&self) -> std::slice::Iter<'_, UriWithParams> {
        self.uris.iter()
    }

    /// Returns a mutable iterator over the URIs.
    ///
    /// Provides a mutable iterator over all `UriWithParams` instances in the list,
    /// allowing modification of the URIs.
    ///
    /// # Returns
    ///
    /// A mutable iterator over references to the URIs in the list
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let mut list = UriWithParamsList::new();
    /// list.push(UriWithParams::from_str("<sip:proxy1.example.com>").unwrap());
    /// list.push(UriWithParams::from_str("<sip:proxy2.example.com>").unwrap());
    ///
    /// // Add 'lr' parameter to all URIs
    /// for uri in list.iter_mut() {
    ///     uri.set_param("lr", None);
    /// }
    ///
    /// // Verify all URIs now have the parameter
    /// for uri in &list {
    ///     assert!(uri.has_param("lr"));
    /// }
    /// ```
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, UriWithParams> {
        self.uris.iter_mut()
    }

    /// Checks if the list is empty.
    ///
    /// Determines whether the list contains no URIs.
    ///
    /// # Returns
    ///
    /// `true` if the list is empty, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let list = UriWithParamsList::new();
    /// assert!(list.is_empty());
    ///
    /// let mut list = UriWithParamsList::new();
    /// list.push(UriWithParams::from_str("<sip:proxy.example.com>").unwrap());
    /// assert!(!list.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.uris.is_empty()
    }

    /// Returns the number of URIs in the list.
    ///
    /// Gets the count of URIs with parameters that this list contains.
    ///
    /// # Returns
    ///
    /// The number of URIs in the list
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let list = UriWithParamsList::new();
    /// assert_eq!(list.len(), 0);
    ///
    /// let mut list = UriWithParamsList::new();
    /// list.push(UriWithParams::from_str("<sip:proxy1.example.com>").unwrap());
    /// list.push(UriWithParams::from_str("<sip:proxy2.example.com>").unwrap());
    /// assert_eq!(list.len(), 2);
    /// ```
    pub fn len(&self) -> usize {
        self.uris.len()
    }

    /// Returns the first URI in the list, if any.
    ///
    /// Gets a reference to the first `UriWithParams` in the list,
    /// or `None` if the list is empty.
    ///
    /// # Returns
    ///
    /// `Some(&UriWithParams)` if the list is not empty, `None` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let list = UriWithParamsList::new();
    /// assert!(list.first().is_none());
    ///
    /// let mut list = UriWithParamsList::new();
    /// list.push(UriWithParams::from_str("<sip:proxy1.example.com>").unwrap());
    /// list.push(UriWithParams::from_str("<sip:proxy2.example.com>").unwrap());
    ///
    /// if let Some(first_uri) = list.first() {
    ///     assert_eq!(first_uri.uri().host(), "proxy1.example.com");
    /// }
    /// ```
    pub fn first(&self) -> Option<&UriWithParams> {
        self.uris.first()
    }

    /// Returns the last URI in the list, if any.
    ///
    /// Gets a reference to the last `UriWithParams` in the list,
    /// or `None` if the list is empty.
    ///
    /// # Returns
    ///
    /// `Some(&UriWithParams)` if the list is not empty, `None` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let list = UriWithParamsList::new();
    /// assert!(list.last().is_none());
    ///
    /// let mut list = UriWithParamsList::new();
    /// list.push(UriWithParams::from_str("<sip:proxy1.example.com>").unwrap());
    /// list.push(UriWithParams::from_str("<sip:proxy2.example.com>").unwrap());
    ///
    /// if let Some(last_uri) = list.last() {
    ///     assert_eq!(last_uri.uri().host(), "proxy2.example.com");
    /// }
    /// ```
    pub fn last(&self) -> Option<&UriWithParams> {
        self.uris.last()
    }

    /// Provides a slice containing all the URIs.
    ///
    /// Returns a slice of the URIs in the list, allowing direct access
    /// to the underlying array of `UriWithParams` objects.
    ///
    /// # Returns
    ///
    /// A slice containing all the URIs in the list
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let mut list = UriWithParamsList::new();
    /// list.push(UriWithParams::from_str("<sip:proxy1.example.com>").unwrap());
    /// list.push(UriWithParams::from_str("<sip:proxy2.example.com>").unwrap());
    ///
    /// let slice = list.as_slice();
    /// assert_eq!(slice.len(), 2);
    /// assert_eq!(slice[0].uri().host(), "proxy1.example.com");
    /// assert_eq!(slice[1].uri().host(), "proxy2.example.com");
    /// ```
    pub fn as_slice(&self) -> &[UriWithParams] {
        &self.uris
    }
}

impl IntoIterator for UriWithParamsList {
    type Item = UriWithParams;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    /// Converts the list into an iterator over owned URIs.
    ///
    /// This implementation allows consuming the `UriWithParamsList` and
    /// iterating over its URIs by value, transferring ownership to the caller.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let mut list = UriWithParamsList::new();
    /// list.push(UriWithParams::from_str("<sip:proxy1.example.com>").unwrap());
    /// list.push(UriWithParams::from_str("<sip:proxy2.example.com>").unwrap());
    ///
    /// // Using the list in a for loop consumes it
    /// let mut hosts = Vec::new();
    /// for uri in list {
    ///     hosts.push(uri.uri().host().to_string());
    /// }
    ///
    /// assert_eq!(hosts, vec!["proxy1.example.com", "proxy2.example.com"]);
    /// ```
    fn into_iter(self) -> Self::IntoIter {
        self.uris.into_iter()
    }
}

impl<'a> IntoIterator for &'a UriWithParamsList {
    type Item = &'a UriWithParams;
    type IntoIter = std::slice::Iter<'a, UriWithParams>;

    /// Converts a reference to the list into an iterator over URI references.
    ///
    /// This implementation allows iterating over the URIs without consuming
    /// the `UriWithParamsList`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let mut list = UriWithParamsList::new();
    /// list.push(UriWithParams::from_str("<sip:proxy1.example.com>").unwrap());
    /// list.push(UriWithParams::from_str("<sip:proxy2.example.com>").unwrap());
    ///
    /// // Using &list in a for loop doesn't consume it
    /// let mut hosts = Vec::new();
    /// for uri in &list {
    ///     hosts.push(uri.uri().host().to_string());
    /// }
    ///
    /// assert_eq!(hosts, vec!["proxy1.example.com", "proxy2.example.com"]);
    ///
    /// // Can still use the list after iterating
    /// assert_eq!(list.len(), 2);
    /// ```
    fn into_iter(self) -> Self::IntoIter {
        self.uris.iter()
    }
}

impl fmt::Display for UriWithParamsList {
    /// Formats the list of URIs as a string.
    ///
    /// Converts the `UriWithParamsList` to its string representation suitable for
    /// inclusion in a SIP message. The URIs are separated by commas, following the
    /// standard format for SIP headers that contain multiple URIs.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    /// use std::fmt::Display;
    ///
    /// let mut list = UriWithParamsList::new();
    /// list.push(UriWithParams::from_str("<sip:proxy1.example.com;lr>").unwrap());
    /// list.push(UriWithParams::from_str("<sip:proxy2.example.com;lr>").unwrap());
    ///
    /// assert_eq!(
    ///     list.to_string(),
    ///     "<sip:proxy1.example.com;lr>, <sip:proxy2.example.com;lr>"
    /// );
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let uri_strings: Vec<String> = self.uris.iter().map(|u| u.to_string()).collect();
        write!(f, "{}", uri_strings.join(", "))
    }
}

// TODO: Implement helper methods (e.g., new, push, iter) 
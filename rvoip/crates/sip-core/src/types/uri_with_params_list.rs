use std::fmt;
use std::str::FromStr;
use crate::types::uri_with_params::UriWithParams;
use crate::error::Result;
use serde::{Serialize, Deserialize};

/// Represents a list of URIs with parameters (e.g., for Route, Record-Route).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)] // Add Default derive
pub struct UriWithParamsList {
    pub uris: Vec<UriWithParams>,
}

impl UriWithParamsList {
    /// Creates an empty list.
    pub fn new() -> Self {
        Self { uris: Vec::new() }
    }

    /// Creates an empty list with the specified capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self { uris: Vec::with_capacity(capacity) }
    }

    /// Adds a UriWithParams to the list.
    pub fn push(&mut self, uri: UriWithParams) {
        self.uris.push(uri);
    }

    /// Returns an iterator over the URIs.
    pub fn iter(&self) -> std::slice::Iter<'_, UriWithParams> {
        self.uris.iter()
    }

    /// Returns a mutable iterator over the URIs.
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, UriWithParams> {
        self.uris.iter_mut()
    }

    /// Checks if the list is empty.
    pub fn is_empty(&self) -> bool {
        self.uris.is_empty()
    }

    /// Returns the number of URIs in the list.
    pub fn len(&self) -> usize {
        self.uris.len()
    }

    /// Returns the first URI in the list, if any.
    pub fn first(&self) -> Option<&UriWithParams> {
        self.uris.first()
    }

    /// Returns the last URI in the list, if any.
    pub fn last(&self) -> Option<&UriWithParams> {
        self.uris.last()
    }

    /// Provides a slice containing all the URIs.
    pub fn as_slice(&self) -> &[UriWithParams] {
        &self.uris
    }
}

impl IntoIterator for UriWithParamsList {
    type Item = UriWithParams;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.uris.into_iter()
    }
}

impl<'a> IntoIterator for &'a UriWithParamsList {
    type Item = &'a UriWithParams;
    type IntoIter = std::slice::Iter<'a, UriWithParams>;

    fn into_iter(self) -> Self::IntoIter {
        self.uris.iter()
    }
}

impl fmt::Display for UriWithParamsList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let uri_strings: Vec<String> = self.uris.iter().map(|u| u.to_string()).collect();
        write!(f, "{}", uri_strings.join(", "))
    }
}

// TODO: Implement helper methods (e.g., new, push, iter) 
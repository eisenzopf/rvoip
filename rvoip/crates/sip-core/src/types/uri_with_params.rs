use crate::uri::Uri;
use crate::types::Param;
use std::fmt;

/// Represents a URI with associated parameters.
#[derive(Debug, Clone, PartialEq, Eq)] // Add derives as needed
pub struct UriWithParams {
    pub uri: Uri,
    pub params: Vec<Param>,
}

impl UriWithParams {
    /// Creates a new UriWithParams.
    pub fn new(uri: Uri) -> Self {
        Self { uri, params: Vec::new() }
    }

    /// Builder method to add a parameter.
    pub fn with_param(mut self, param: Param) -> Self {
        self.params.push(param);
        self
    }
}

// TODO: Implement helper methods 
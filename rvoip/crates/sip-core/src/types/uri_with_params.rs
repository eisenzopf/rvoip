use crate::uri::Uri;
use crate::types::param::Param;
use std::fmt;
use serde::{Serialize, Deserialize};

/// Represents a URI with associated parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

// Implement Display for UriWithParams
impl fmt::Display for UriWithParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}", self.uri)?;
        for param in &self.params {
            write!(f, "{}", param)?;
        }
        write!(f, ">")
    }
}

// TODO: Implement helper methods 
use crate::types::uri_with_params_list::UriWithParamsList;
use crate::parser::headers::parse_record_route;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use std::ops::Deref;

/// Typed Record-Route header.
#[derive(Debug, Clone, PartialEq, Eq)] // Add derives as needed
pub struct RecordRoute(pub UriWithParamsList);

impl RecordRoute {
    /// Creates a new RecordRoute header.
    pub fn new(list: UriWithParamsList) -> Self {
        Self(list)
    }
}

impl fmt::Display for RecordRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0) // Delegate to UriWithParamsList
    }
}

impl FromStr for RecordRoute {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        let trimmed_s = s.trim();
        if trimmed_s.is_empty() {
             return Err(Error::InvalidHeader("Empty Record-Route header value".to_string()));
        }
        match parse_record_route(trimmed_s) { // Pass trimmed string
             Ok(route) if route.0.uris.is_empty() => {
                 Err(Error::InvalidHeader("Invalid Record-Route header value".to_string()))
             }
             Ok(route) => Ok(route),
             Err(e) => Err(e)
        }
    }
}

impl Deref for RecordRoute {
    type Target = UriWithParamsList;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// TODO: Implement helper methods (e.g., first(), is_empty()) 
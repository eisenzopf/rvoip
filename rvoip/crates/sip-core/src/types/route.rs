use crate::types::uri_with_params_list::UriWithParamsList;
use crate::parser::headers::parse_route;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use std::ops::Deref;

/// Typed Route header.
#[derive(Debug, Clone, PartialEq, Eq)] // Add derives as needed
pub struct Route(pub UriWithParamsList);

impl Route {
    /// Creates a new Route header.
    pub fn new(list: UriWithParamsList) -> Self {
        Self(list)
    }
}

impl fmt::Display for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0) // Delegate to UriWithParamsList
    }
}

impl FromStr for Route {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        let trimmed_s = s.trim();
        if trimmed_s.is_empty() {
             return Err(Error::InvalidHeader("Empty Route header value".to_string()));
        }
        match parse_route(trimmed_s) {
             Ok(route) if route.0.uris.is_empty() => {
                 Err(Error::InvalidHeader("Invalid Route header value".to_string()))
             }
             Ok(route) => Ok(route),
             Err(e) => Err(e)
        }
    }
}

impl Deref for Route {
    type Target = UriWithParamsList;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// TODO: Implement helper methods (e.g., first(), is_empty()) 
use crate::types::uri_with_params_list::UriWithParamsList;
use crate::parser::headers::parse_record_route;
use crate::error::Result;
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
        parse_record_route(s)
    }
}

impl Deref for RecordRoute {
    type Target = UriWithParamsList;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// TODO: Implement helper methods (e.g., first(), is_empty()) 
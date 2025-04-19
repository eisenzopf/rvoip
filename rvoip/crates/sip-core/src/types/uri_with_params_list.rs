use crate::types::uri_with_params::UriWithParams;
use std::fmt;

/// Represents a list of URIs with parameters (e.g., for Route, Record-Route).
#[derive(Debug, Clone, PartialEq, Eq)] // Add derives as needed
pub struct UriWithParamsList {
    pub uris: Vec<UriWithParams>,
}

impl fmt::Display for UriWithParamsList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let uri_strings: Vec<String> = self.uris.iter().map(|u| u.to_string()).collect();
        write!(f, "{}", uri_strings.join(", "))
    }
}

// TODO: Implement helper methods (e.g., new, push, iter) 
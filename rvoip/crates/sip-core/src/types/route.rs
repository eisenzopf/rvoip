use crate::types::uri_with_params_list::UriWithParamsList;
use crate::parser::headers::parse_route;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use std::ops::Deref;
use nom::combinator::all_consuming;
use crate::types::Address;

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
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        use crate::parser::headers::route::parse_route;

        match all_consuming(parse_route)(s.as_bytes()) {
            Ok((_, entries)) => {
                 // Convert Vec<RouteEntry> -> Vec<Address>
                let addrs = entries.into_iter()
                    .map(|entry| Address::from_parsed(entry.display_name, entry.uri, entry.params))
                    .collect::<Result<Vec<_>>>()?;
                Ok(Route(addrs))
            },
            Err(e) => Err(Error::ParsingError{ 
                message: format!("Failed to parse Route header: {:?}", e), 
                source: None 
            })
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
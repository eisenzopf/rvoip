use crate::types::uri_with_params_list::UriWithParamsList;
use crate::parser::headers::parse_route;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use std::ops::Deref;
use nom::combinator::all_consuming;
use crate::types::Address;
use crate::parser::headers::route::RouteValue as ParserRouteValue;
use serde::{Deserialize, Serialize};
use crate::parser::ParseResult;
use crate::types::param::Param;

/// Represents the Route header field (RFC 3261 Section 8.1.1.1).
/// Contains a list of route entries (typically Addresses).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route(pub Vec<ParserRouteValue>);

impl Route {
    /// Creates a new Route header.
    pub fn new(list: Vec<ParserRouteValue>) -> Self {
        Self(list)
    }
}

impl fmt::Display for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.iter().map(|r| r.to_string()).collect::<Vec<String>>().join(", "))
    }
}

impl FromStr for Route {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        use crate::parser::headers::route::parse_route;

        match all_consuming(parse_route)(s.as_bytes()) {
            Ok((_, route_header)) => Ok(route_header),
            Err(e) => Err(Error::ParseError( 
                format!("Failed to parse Route header: {:?}", e)
            ))
        }
    }
}

impl Deref for Route {
    type Target = Vec<ParserRouteValue>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// TODO: Implement helper methods (e.g., first(), is_empty()) 
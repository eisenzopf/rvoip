use crate::types::uri_with_params_list::UriWithParamsList;
use crate::parser::headers::parse_record_route;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use std::ops::Deref;
use nom::combinator::all_consuming;
use crate::types::Address;
use crate::parser::headers::record_route::RecordRouteValue;
use serde::{Deserialize, Serialize};

/// Typed Record-Route header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordRoute(pub Vec<RecordRouteValue>);

impl RecordRoute {
    /// Creates a new RecordRoute header.
    pub fn new(list: Vec<RecordRouteValue>) -> Self {
        Self(list)
    }
}

impl fmt::Display for RecordRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.iter().map(|rr| rr.to_string()).collect::<Vec<String>>().join(", "))
    }
}

impl FromStr for RecordRoute {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match all_consuming(parse_record_route)(s.as_bytes()) {
            Ok((_, rr_header)) => Ok(rr_header),
            Err(e) => Err(Error::ParseError( 
                format!("Failed to parse Record-Route header: {:?}", e)
            ))
        }
    }
}

impl Deref for RecordRoute {
    type Target = Vec<RecordRouteValue>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// TODO: Implement helper methods (e.g., first(), is_empty()) 
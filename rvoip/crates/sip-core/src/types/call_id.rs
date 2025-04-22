use std::fmt;
use std::str::FromStr;
use crate::error::{Result, Error};
use crate::parser::headers::parse_call_id;
use uuid::Uuid;
use std::ops::Deref;
use nom::combinator::all_consuming;
use std::string::FromUtf8Error;

/// Typed Call-ID header value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)] // Add derives as needed
pub struct CallId(pub String);

impl Deref for CallId {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for CallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for CallId {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Call the parser first
        let parse_result = all_consuming(parse_call_id)(s.as_bytes());

        // Match on the Result
        match parse_result {
            Ok((_, (local, host_opt))) => {
                let local_part = String::from_utf8(local.to_vec())?;
                let host_part = host_opt.map(|h| String::from_utf8(h.to_vec())).transpose()?;
                // Construct the single String for CallId(String)
                let call_id_string = match host_part {
                    Some(host) => format!("{}@{}", local_part, host),
                    None => local_part,
                };
                Ok(CallId(call_id_string))
            },
            Err(e) => Err(Error::from(e)), 
        }
    }
}

// TODO: Implement methods (e.g., new_random) 
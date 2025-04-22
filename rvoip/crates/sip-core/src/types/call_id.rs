use std::fmt;
use std::str::FromStr;
use crate::error::Result;
use crate::parser::headers::parse_call_id;
use uuid::Uuid;
use std::ops::Deref;
use nom::combinator::all_consuming;

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
        use crate::parser::headers::call_id::parse_call_id;

        match all_consuming(parse_call_id)(s.as_bytes()) {
            Ok((_, (local, host_opt))) => {
                // Convert bytes to String - Join parts for now
                let local_part = String::from_utf8(local.to_vec())?;
                let call_id_string = match host_opt {
                    Some(host_bytes) => format!("{}@{}", local_part, String::from_utf8(host_bytes.to_vec())?),
                    None => local_part,
                };
                Ok(CallId(call_id_string))
            },
            Err(e) => Err(Error::ParsingError{ 
                message: format!("Failed to parse Call-ID header: {:?}", e), 
                source: None 
            })
        }
    }
}

// TODO: Implement methods (e.g., new_random) 
use crate::types::address::Address; // Or maybe UriWithParams?
use crate::parser::headers::parse_reply_to; // Use the parser
use crate::error::Result;
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;

/// Typed Reply-To header.
#[derive(Debug, Clone, PartialEq, Eq)] // Add derives as needed
pub struct ReplyTo(pub Address); // Or UriWithParams

impl ReplyTo {
    /// Creates a new ReplyTo header.
    pub fn new(address: Address) -> Self {
        Self(address)
    }
}

impl fmt::Display for ReplyTo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0) // Delegate to Address display
    }
}

impl FromStr for ReplyTo {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        use crate::parser::headers::reply_to::parse_reply_to;

        match all_consuming(parse_reply_to)(s.as_bytes()) {
            Ok((_, reply_to_header)) => Ok(reply_to_header),
            Err(e) => Err(Error::ParseError( 
                format!("Failed to parse Reply-To header: {:?}", e)
            Ok((_, value)) => {
                // Convert ReplyToValue -> Address
                let addr = Address::from_parsed(value.display_name, value.uri, value.params)?;
                Ok(ReplyTo(addr))
            },
            Err(e) => Err(Error::ParsingError{ 
                message: format!("Failed to parse Reply-To header: {:?}", e), 
                source: None 
            })
        }
    }
}

// TODO: Implement methods if needed 
// Parser for the Error-Info header (RFC 3261 Section 20.18)
// Error-Info = "Error-Info" HCOLON error-uri *(COMMA error-uri)
// error-uri = LAQUOT absoluteURI RAQUOT *( SEMI generic-param )

use nom::{
    bytes::complete::{tag_no_case},
    combinator::{map, map_res},
    multi::{many0},
    sequence::{delimited, pair, preceded},
    IResult,
};
use std::str;

// Import from base parser modules
use crate::parser::separators::{hcolon, semi, comma, laquot, raquot};
use crate::parser::common_params::{generic_param, semicolon_separated_params0};
use crate::parser::uri::absolute_uri; // Assuming an absolute_uri parser exists
use crate::parser::common::comma_separated_list1;
use crate::parser::ParseResult;
use crate::parser::uri::parse_uri; // Import the actual URI parser
use nom::combinator::all_consuming; // Import all_consuming

use crate::types::uri::Uri;
use crate::types::error_info::ErrorInfo as ErrorInfoHeader; // Use specific header type
use serde::{Serialize, Deserialize}; // Added serde
use std::str::FromStr; // Import FromStr
use crate::error::Error as CrateError; // Import crate error

// Import shared parsers
use super::uri_with_params::uri_with_generic_params;
use crate::types::param::Param;

// error-uri = LAQUOT absoluteURI RAQUOT
// Returns Uri directly
fn error_uri(input: &[u8]) -> ParseResult<Uri> {
    map_res(
        delimited(laquot, absolute_uri, raquot), // absolute_uri likely returns &[u8]
        |uri_bytes| -> Result<Uri, CrateError> { 
            // Parse the bytes using the dedicated parser
            let (_, uri) = all_consuming(parse_uri)(uri_bytes)
                .map_err(|e: nom::Err<nom::error::Error<&[u8]>>| CrateError::ParseError(format!("URI parse error: {}", e)))?;
            Ok(uri)
        }
    )(input)
}

// error-info-value = error-uri *( SEMI generic-param )
// Assume uri_with_generic_params returns (String, Vec<Param>) erroneously
fn error_info_value(input: &[u8]) -> ParseResult<ErrorInfoValue> {
     map_res(
        uri_with_generic_params, // Assume this returns Result<(String, Vec<Param>), _>
        |(uri_str, params)| -> Result<ErrorInfoValue, CrateError> { 
             // Parse the URI string using the dedicated parser
            let (_, uri) = all_consuming(parse_uri)(uri_str.as_bytes())
                .map_err(|e: nom::Err<nom::error::Error<&[u8]>>| CrateError::ParseError(format!("URI parse error: {}", e)))?;
            Ok(ErrorInfoValue { uri, params })
        }
    )(input)
}

// Define structure for Error-Info value
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)] // Added Serialize, Deserialize
pub struct ErrorInfoValue {
    pub uri: Uri,
    pub params: Vec<Param>,
}

// Error-Info = "Error-Info" HCOLON error-uri *(COMMA error-uri)
/// Parses an Error-Info header value.
pub fn parse_error_info(input: &[u8]) -> ParseResult<Vec<ErrorInfoValue>> {
    comma_separated_list1(error_info_value)(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::param::{GenericValue, Param};

    #[test]
    fn test_parse_error_info() {
        let input = b"<sip:not-in-service@example.com>;reason=Foo";
        let result = parse_error_info(input);
        assert!(result.is_ok());
        let (rem, infos) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].uri, "sip:not-in-service@example.com"); // Our stub parses SIP URIs too
        assert_eq!(infos[0].params.len(), 1);
        assert!(matches!(infos[0].params[0], Param::Other(n, Some(GenericValue::Token(v))) if n == "reason" && v == "Foo"));
    }

    #[test]
    fn test_parse_error_info_multiple() {
         let input = b"<sip:error1@h.com>, <http://error.com/more>;param=1";
         let result = parse_error_info(input);
        assert!(result.is_ok());
        let (rem, infos) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].uri, "sip:error1@h.com");
        assert!(infos[0].params.is_empty());
        assert_eq!(infos[1].uri, "http://error.com/more");
        assert_eq!(infos[1].params.len(), 1);
    }
} 
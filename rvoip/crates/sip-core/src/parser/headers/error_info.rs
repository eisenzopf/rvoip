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

use crate::types::param::Param;
use crate::uri::Uri;

// Import shared parsers
use super::uri_with_params::uri_with_generic_params;
use crate::types::error_info::ErrorInfo as ErrorInfoHeader; // Specific header type
use crate::types::error_info::ErrorInfoValue; // Value type for the header

// error-uri = LAQUOT absoluteURI RAQUOT *( SEMI generic-param )
fn error_uri(input: &[u8]) -> ParseResult<ErrorInfoValue> {
    let (remaining, (uri, params)) = uri_with_generic_params(input)?;
    Ok((remaining, ErrorInfoValue { uri, params }))
}

// Define structure for Error-Info value
#[derive(Debug, PartialEq, Clone)]
pub struct ErrorInfoValue {
    pub uri: Uri,
    pub params: Vec<Param>,
}

// Error-Info = "Error-Info" HCOLON error-uri *(COMMA error-uri)
/// Parses an Error-Info header value.
pub fn parse_error_info(input: &[u8]) -> ParseResult<Vec<ErrorInfoValue>> {
    comma_separated_list1(error_uri)(input)
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
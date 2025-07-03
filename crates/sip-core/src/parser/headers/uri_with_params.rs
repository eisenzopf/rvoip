// Shared parser for <absoluteURI> *(SEMI generic-param)

use nom::{
    combinator::{map, map_res},
    multi::many0,
    sequence::{delimited, pair, preceded},
    IResult,
};
use std::str;

// Import shared parsers
use crate::parser::uri::parse_absolute_uri; // Using stub absolute URI parser
use crate::parser::common_params::generic_param;
use crate::parser::separators::{laquot, raquot, semi};
use crate::parser::ParseResult;

// Import types
use crate::types::param::Param;
// Returns (URI String, Vec<Param>)

pub fn uri_with_generic_params(input: &[u8]) -> ParseResult<(String, Vec<Param>)> {
    map(
        pair(
            // LAQUOT absoluteURI RAQUOT
            map_res( // Use map_res to handle potential UTF-8 error from absoluteURI bytes
                delimited(
                    laquot,
                    take_until_raquot,
                    raquot
                ),
                |bytes| str::from_utf8(bytes).map(String::from)
            ),
            // *( SEMI generic-param )
            many0(preceded(semi, generic_param))
        ),
        |(uri_str, params_vec)| (uri_str, params_vec)
    )(input)
}

// Helper function to extract the URI content between angle brackets
fn take_until_raquot(input: &[u8]) -> ParseResult<&[u8]> {
    if input.is_empty() {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::TakeUntil
        )));
    }
    
    // Look for the closing angle bracket
    let mut i = 0;
    let mut found = false;
    
    while i < input.len() {
        if input[i] == b'>' {
            found = true;
            break;
        }
        i += 1;
    }
    
    if !found {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::TakeUntil
        )));
    }
    
    Ok((&input[i..], &input[0..i]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::param::{GenericValue};

    #[test]
    fn test_uri_with_generic_params() {
        let input = b"<http://example.com/error>;reason=BadValue";
        let result = uri_with_generic_params(input);
        assert!(result.is_ok());
        let (rem, (uri, params)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(uri, "http://example.com/error");
        assert_eq!(params.len(), 1);
        assert!(matches!(&params[0], Param::Other(n, Some(GenericValue::Token(v))) if n == "reason" && v == "BadValue"));
    }
    
    #[test]
    fn test_uri_with_no_params() {
        let input = b"<mailto:help@example.org>";
        let result = uri_with_generic_params(input);
        assert!(result.is_ok());
        let (rem, (uri, params)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(uri, "mailto:help@example.org");
        assert!(params.is_empty());
    }
} 
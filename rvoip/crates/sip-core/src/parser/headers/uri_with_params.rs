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
    println!("uri_with_generic_params input: {:?}", std::str::from_utf8(input));
    
    // Try to find laquot
    if let Some(i) = input.iter().position(|&b| b == b'<') {
        println!("Found laquot at position {}", i);
    } else {
        println!("No laquot found in input");
    }
    
    // Try to find raquot
    if let Some(i) = input.iter().position(|&b| b == b'>') {
        println!("Found raquot at position {}", i);
    } else {
        println!("No raquot found in input");
    }
    
    map(
        pair(
            // LAQUOT absoluteURI RAQUOT
            map_res( // Use map_res to handle potential UTF-8 error from absoluteURI bytes
                delimited(
                    laquot,
                    parse_absolute_uri, 
                    raquot
                ),
                 |bytes| {
                     println!("absoluteURI bytes: {:?}", bytes);
                     let res = str::from_utf8(bytes);
                     if let Err(ref e) = res {
                         println!("UTF-8 conversion error: {:?}", e);
                     }
                     res.map(String::from)
                 }
            ),
            // *( SEMI generic-param )
            many0(preceded(semi, generic_param))
        ),
        |(uri_str, params_vec)| {
            println!("Successful parse: uri_str={}, params={:?}", uri_str, params_vec);
            (uri_str, params_vec)
        }
    )(input)
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
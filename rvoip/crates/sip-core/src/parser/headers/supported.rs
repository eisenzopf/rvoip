// Parser for Supported header (RFC 3261 Section 20.37)
// Supported = ( "Supported" / "k" ) HCOLON [ option-tag *(COMMA option-tag) ]
// option-tag = token

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_while},
    character::complete::char,
    combinator::{map, opt, all_consuming, recognize, value, eof},
    multi::separated_list0,
    sequence::{preceded, terminated, delimited},
    IResult, error::ErrorKind,
};
use std::str;

// Import from other modules
use crate::parser::separators::{hcolon, comma};
use crate::parser::token::token;
use crate::parser::common::comma_separated_list0;
use crate::parser::ParseResult;
use crate::parser::whitespace::{sws, owsp};

/// Parses optional whitespace followed by a comma and more whitespace
fn ws_comma_ws(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(
        delimited(
            owsp,
            char(','),
            owsp
        )
    )(input)
}

/// Parses a list of option-tags (tokens) separated by commas,
/// handling whitespace, empty elements, and trailing commas.
/// Returns a Vec<String> containing the parsed option-tags.
pub fn parse_supported(input: &[u8]) -> ParseResult<Vec<String>> {
    // Handle empty input explicitly
    if input.is_empty() {
        return Ok((input, Vec::new()));
    }
    
    // First trim any leading or trailing whitespace
    let input = match owsp(input) {
        Ok((rest, _)) => rest,
        Err(_) => input,
    };
    
    // If after whitespace trimming the input is empty, return empty Vec
    if input.is_empty() {
        return Ok((input, Vec::new()));
    }
    
    // Parse tokens separated by commas, handling whitespace
    // Convert the tokens to strings, filtering out empty tokens
    let result = separated_list0(
        ws_comma_ws,
        alt((
            token,                           // Normal token
            value(&b""[..], tag(b""))        // Empty token (becomes empty string)
        ))
    )(input);
    
    match result {
        Ok((remaining, tokens)) => {
            // Handle any trailing whitespace or comma
            let remaining = match owsp(remaining) {
                Ok((rest, _)) => rest,
                Err(_) => remaining,
            };
            
            // Handle trailing comma after tokens
            let remaining = match opt(ws_comma_ws)(remaining) {
                Ok((rest, _)) => rest,
                Err(_) => remaining,
            };
            
            // Handle any final whitespace
            let remaining = match owsp(remaining) {
                Ok((rest, _)) => rest,
                Err(_) => remaining,
            };
            
            // Convert tokens to strings, filtering out empty ones
            let strings: Vec<String> = tokens.into_iter()
                .filter(|&t| !t.is_empty())
                .map(|t| str::from_utf8(t).unwrap_or("").to_string())
                .collect();
            
            Ok((remaining, strings))
        },
        Err(e) => Err(e),
    }
}

/// Parses a complete Supported header, including the header name and colon.
/// Handles both the standard "Supported:" form and the compact "k:" form.
pub fn parse_supported_header(input: &[u8]) -> ParseResult<Vec<String>> {
    preceded(
        terminated(
            alt((
                tag_no_case(b"Supported"),
                tag_no_case(b"k")
            )),
            hcolon
        ),
        parse_supported
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nom::combinator::all_consuming;

    // Helper function to test the parser with full input consumption
    fn test_parse_supported(input: &[u8]) -> Result<Vec<String>, nom::Err<nom::error::Error<&[u8]>>> {
        all_consuming(parse_supported)(input).map(|(_, output)| output)
    }

    #[test]
    fn test_parse_supported_normal() {
        // Single option-tag
        let input = b"timer";
        let result = test_parse_supported(input).unwrap();
        assert_eq!(result, vec!["timer"]);
        
        // Multiple option-tags
        let input = b"timer, 100rel";
        let result = test_parse_supported(input).unwrap();
        assert_eq!(result, vec!["timer", "100rel"]);
        
        // Multiple option-tags with common extensions
        let input = b"timer, 100rel, path, outbound";
        let result = test_parse_supported(input).unwrap();
        assert_eq!(result, vec!["timer", "100rel", "path", "outbound"]);
    }

    #[test]
    fn test_parse_supported_empty() {
        // Empty input is valid (no supported extensions)
        let input = b"";
        let result = test_parse_supported(input).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_supported_whitespace() {
        // Whitespace handling in comma-separated list
        let input = b"timer , 100rel, path , outbound";
        let result = test_parse_supported(input).unwrap();
        assert_eq!(result, vec!["timer", "100rel", "path", "outbound"]);
        
        // Trailing whitespace
        let input = b"timer, 100rel ";
        let result = test_parse_supported(input).unwrap();
        assert_eq!(result, vec!["timer", "100rel"]);
        
        // Leading whitespace
        let input = b" timer, 100rel";
        let result = test_parse_supported(input).unwrap();
        assert_eq!(result, vec!["timer", "100rel"]);
    }

    #[test]
    fn test_parse_supported_special_tokens() {
        // Test token parsing with allowed special characters
        let input = b"timer, 100rel-v2, path.basic, x+y";
        let result = test_parse_supported(input).unwrap();
        assert_eq!(result, vec!["timer", "100rel-v2", "path.basic", "x+y"]);
    }

    #[test]
    fn test_parse_supported_header_standard() {
        // Full header with standard form
        let input = b"Supported: timer, 100rel";
        let (_, supported) = parse_supported_header(input).unwrap();
        assert_eq!(supported, vec!["timer", "100rel"]);
    }

    #[test]
    fn test_parse_supported_header_compact() {
        // Full header with compact form
        let input = b"k: timer, 100rel";
        let (_, supported) = parse_supported_header(input).unwrap();
        assert_eq!(supported, vec!["timer", "100rel"]);
    }

    #[test]
    fn test_parse_supported_header_case_insensitive() {
        // Header name should be case-insensitive
        let input = b"SUPPORTED: timer, 100rel";
        let (_, supported) = parse_supported_header(input).unwrap();
        assert_eq!(supported, vec!["timer", "100rel"]);
        
        // Compact form is also case-insensitive
        let input = b"K: timer, 100rel";
        let (_, supported) = parse_supported_header(input).unwrap();
        assert_eq!(supported, vec!["timer", "100rel"]);
    }

    #[test]
    fn test_parse_supported_header_empty() {
        // Empty list after header is valid
        let input = b"Supported: ";
        let (_, supported) = parse_supported_header(input).unwrap();
        assert!(supported.is_empty());
    }

    #[test]
    fn test_rfc3261_examples() {
        // Examples from RFC 3261
        let input = b"Supported: 100rel";
        let (_, supported) = parse_supported_header(input).unwrap();
        assert_eq!(supported, vec!["100rel"]);
        
        // Example with compact form
        let input = b"k: timer, 100rel";
        let (_, supported) = parse_supported_header(input).unwrap();
        assert_eq!(supported, vec!["timer", "100rel"]);
    }

    #[test]
    fn test_single_trailing_comma() {
        // Test with a single trailing comma, which should be handled gracefully
        let input = b"timer,";
        let result = test_parse_supported(input).unwrap();
        assert_eq!(result, vec!["timer"]);
    }

    #[test]
    fn test_empty_elements() {
        // Test with empty elements in list (,, pattern)
        // According to RFC 3261, empty elements should be ignored as they're not valid tokens
        let input = b"timer,,100rel";
        let result = test_parse_supported(input).unwrap();
        assert_eq!(result, vec!["timer", "100rel"]);
        
        // Multiple commas
        let input = b"timer,,,100rel";
        let result = test_parse_supported(input).unwrap();
        assert_eq!(result, vec!["timer", "100rel"]);
    }
} 
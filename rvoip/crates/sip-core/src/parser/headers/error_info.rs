// Parser for the Error-Info header (RFC 3261 Section 20.18)
// Error-Info = "Error-Info" HCOLON error-uri *(COMMA error-uri)
// error-uri = LAQUOT absoluteURI RAQUOT *( SEMI generic-param )

use nom::{
    bytes::complete::{tag, tag_no_case, take_until, is_not},
    character::complete::space0,
    combinator::{map, map_res, opt, verify, fail, recognize},
    multi::{many0, separated_list1},
    sequence::{delimited, pair, preceded, tuple},
    IResult, Err,
};
use std::str;

// Import from base parser modules
use crate::parser::separators::{hcolon, semi, comma, laquot, raquot};
use crate::parser::common_params::{generic_param};
use crate::parser::common::comma_separated_list1;
use crate::parser::ParseResult;
use crate::parser::whitespace::sws;
use nom::combinator::all_consuming;

use crate::types::uri::Uri;
use crate::types::uri_adapter::UriAdapter;
use crate::types::error_info::ErrorInfo as ErrorInfoHeader;
use serde::{Serialize, Deserialize};
use std::str::FromStr;
use crate::error::Error as CrateError;
use crate::types::param::Param;

/// Represents a single error-uri with its parameters in an Error-Info header.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct ErrorInfoValue {
    pub uri: Uri,
    pub uri_str: String,
    pub params: Vec<Param>,
}

/// Parse a URI enclosed in angle brackets with optional whitespace before and after.
fn enclosed_uri(input: &[u8]) -> ParseResult<String> {
    // Verify that there's a closing bracket
    if !input.contains(&b'>') {
        return Err(Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
    }
    
    let (input, _) = space0(input)?;
    
    map_res(
        delimited(
            laquot,
            take_until(">"),  // Take everything until the closing '>'
            raquot
        ),
        |uri_bytes: &[u8]| -> Result<String, CrateError> {
            let uri_str = String::from_utf8_lossy(uri_bytes).into_owned();
            // Ensure the URI is not empty
            if uri_str.is_empty() {
                return Err(CrateError::ParseError("Empty URI".to_string()));
            }
            Ok(uri_str)
        }
    )(input)
}

/// Parses a parameter (;name=value or ;name)
fn param(input: &[u8]) -> ParseResult<Param> {
    preceded(
        semi,
        generic_param
    )(input)
}

/// Verifies that there are no trailing characters after the parameter list
fn verify_no_trailing_chars(input: &[u8]) -> bool {
    // Skip any leading whitespace
    let mut i = 0;
    while i < input.len() && (input[i] == b' ' || input[i] == b'\t') {
        i += 1;
    }
    
    // We need to be at the end of the input or at a comma (which would start the next URI)
    i >= input.len() || input[i] == b','
}

/// Parses an error-info-value, which is an error-uri followed by optional parameters.
/// error-info-value = error-uri *( SEMI generic-param )
fn error_info_value(input: &[u8]) -> ParseResult<ErrorInfoValue> {
    let (input, _) = space0(input)?;
    
    let (remaining, (uri_str, params)) = tuple((
        enclosed_uri,
        many0(param)
    ))(input)?;
    
    // Verify there are no trailing invalid characters
    if !verify_no_trailing_chars(remaining) {
        return Err(Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Verify)));
    }
    
    // Parse the URI using the UriAdapter
    let uri = match UriAdapter::parse_uri(&uri_str) {
        Ok(u) => u,
        Err(e) => return Err(Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag))),
    };
    
    Ok((remaining, ErrorInfoValue { uri, uri_str, params }))
}

/// Handles trailing commas in the list
fn trailing_comma_check(input: &[u8]) -> ParseResult<&[u8]> {
    let (input, _) = space0(input)?;
    
    // If there's nothing left after whitespace, we're done
    if input.is_empty() {
        return Ok((input, input));
    }
    
    // If there's a comma, it should be followed by a non-empty URI
    if input[0] == b',' {
        let (input, _) = tag(b",")(input)?;
        let (input, _) = space0(input)?;
        
        // Now we should have a URI starting with '<'
        if input.is_empty() || input[0] != b'<' {
            return Err(Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
        }
    }
    
    Ok((input, input))
}

/// Parses the value part of an Error-Info header (without the "Error-Info:" prefix).
/// Example: `<sip:error@example.com>;reason=busy, <http://error.org>`
pub fn parse_error_info(input: &[u8]) -> ParseResult<Vec<ErrorInfoValue>> {
    let (input, items) = separated_list1(
        comma,
        error_info_value
    )(input)?;
    
    // Check for trailing comma
    let (input, _) = trailing_comma_check(input)?;
    
    Ok((input, items))
}

/// Parses a complete Error-Info header, including the "Error-Info:" prefix.
/// Example: `Error-Info: <sip:busy@example.com>;reason=busy`
pub fn full_parse_error_info(input: &[u8]) -> ParseResult<Vec<ErrorInfoValue>> {
    preceded(
        pair(tag_no_case(b"Error-Info"), hcolon),
        parse_error_info
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::param::{GenericValue, Param};
    use crate::types::uri::{Uri, Scheme, Host};
    use std::str::FromStr;

    #[test]
    fn test_parse_error_info() {
        let input = "<sip:not-in-service@example.com>;reason=Foo";
        let result = parse_error_info(input.as_bytes());
        assert!(result.is_ok());
        let (rem, infos) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(infos.len(), 1);
        
        // Verify URI and scheme
        assert_eq!(infos[0].uri_str, "sip:not-in-service@example.com");
        assert_eq!(infos[0].uri.scheme.as_str(), "sip");
        
        // Check parameters
        assert_eq!(infos[0].params.len(), 1);
        assert!(matches!(&infos[0].params[0], Param::Other(n, Some(v)) if n == "reason" && v.to_string() == "Foo"));
    }

    #[test]
    fn test_parse_error_info_multiple() {
        let input = "<sip:error1@h.com>, <http://error.com/more>;param=1";
        let result = parse_error_info(input.as_bytes());
        assert!(result.is_ok());
        let (rem, infos) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(infos.len(), 2);
        
        // Check the SIP URI
        assert_eq!(infos[0].uri_str, "sip:error1@h.com");
        assert_eq!(infos[0].uri.scheme.as_str(), "sip");
        assert!(infos[0].params.is_empty());
        
        // Check the HTTP URI - should be stored as custom/raw URI
        assert_eq!(infos[1].uri_str, "http://error.com/more");
        assert!(infos[1].uri.is_custom());
        assert_eq!(infos[1].uri.as_raw_uri().unwrap(), "http://error.com/more");
        assert_eq!(infos[1].params.len(), 1);
        assert!(matches!(&infos[1].params[0], Param::Other(n, Some(v)) if n == "param" && v.to_string() == "1"));
    }
    
    #[test]
    fn test_full_header_parsing() {
        // Test with header name and colon
        let input = "Error-Info: <sip:busy@example.com>;reason=busy";
        let result = full_parse_error_info(input.as_bytes());
        assert!(result.is_ok());
        let (rem, infos) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(infos.len(), 1);
        
        // Check that the URI is parsed correctly
        assert_eq!(infos[0].uri_str, "sip:busy@example.com");
        assert_eq!(infos[0].uri.scheme.as_str(), "sip");
        
        // Check parameters
        assert_eq!(infos[0].params.len(), 1);
        assert!(matches!(&infos[0].params[0], Param::Other(n, Some(v)) 
            if n == "reason" && v.to_string() == "busy"));
        
        // Test case insensitivity of header name
        let input_case = "error-info: <sip:busy@example.com>";
        let result_case = full_parse_error_info(input_case.as_bytes());
        assert!(result_case.is_ok(), "Header name should be case-insensitive");
    }
    
    #[test]
    fn test_uri_schemes() {
        // Test with various URI schemes allowed in absoluteURI
        // The test data includes scheme and expected URI string
        let test_cases = [
            ("<sip:busy@example.com>", "sip:busy@example.com"),
            ("<sips:secure@example.com>", "sips:secure@example.com"),
            ("<http://example.com/errors/busy>", "http://example.com/errors/busy"),
            ("<https://example.com/errors/busy>", "https://example.com/errors/busy"),
            ("<tel:+1-212-555-1234>", "tel:+1-212-555-1234"),
        ];
        
        for (input_str, expected_uri_str) in test_cases {
            let input = input_str.as_bytes();
            let result = parse_error_info(input);
            assert!(result.is_ok(), "Failed to parse URI: {}", input_str);
            let (_, infos) = result.unwrap();
            assert_eq!(infos[0].uri_str, expected_uri_str);
            
            // Special checking for scheme - UriAdapter handles all schemes differently
            // HTTP/HTTPS will be stored as raw_uri
            match &infos[0].uri_str {
                s if s.starts_with("http:") => {
                    assert!(infos[0].uri.is_custom());
                    assert_eq!(infos[0].uri.as_raw_uri().unwrap(), s);
                },
                s if s.starts_with("https:") => {
                    assert!(infos[0].uri.is_custom());
                    assert_eq!(infos[0].uri.as_raw_uri().unwrap(), s);
                },
                s if s.starts_with("sip:") => {
                    assert_eq!(infos[0].uri.scheme.as_str(), "sip");
                },
                s if s.starts_with("sips:") => {
                    assert_eq!(infos[0].uri.scheme.as_str(), "sips");
                },
                s if s.starts_with("tel:") => {
                    assert_eq!(infos[0].uri.scheme.as_str(), "tel");
                },
                _ => panic!("Unexpected scheme in URI: {}", infos[0].uri_str),
            }
        }
    }
    
    #[test]
    fn test_whitespace_handling() {
        // Test with extra whitespace
        let input = "   <sip:busy@example.com>  ;  reason=busy  ,  <sip:error@example.net>";
        let result = parse_error_info(input.as_bytes());
        assert!(result.is_ok(), "Should handle extra whitespace");
        let (rem, infos) = result.unwrap();
        
        // Should parse two URIs
        assert_eq!(infos.len(), 2);
        
        // First URI should have "reason=busy" parameter
        assert_eq!(infos[0].params.len(), 1);
        assert!(matches!(&infos[0].params[0], Param::Other(n, Some(GenericValue::Token(v))) 
            if n == "reason" && v == "busy"));
        
        // Second URI should have no parameters
        assert!(infos[1].params.is_empty());
        
        // Test with whitespace in full header
        let input_full = "Error-Info:  <sip:busy@example.com>";
        let result_full = full_parse_error_info(input_full.as_bytes());
        assert!(result_full.is_ok(), "Should handle whitespace after colon");
    }
    
    #[test]
    fn test_parameter_variations() {
        // Test with various parameter formats
        let input = "<sip:busy@example.com>;reason=busy;id=1;critical;info=\"quoted value\"";
        let result = parse_error_info(input.as_bytes());
        assert!(result.is_ok());
        let (_, infos) = result.unwrap();
        
        // Check that all 4 parameters were parsed
        assert_eq!(infos[0].params.len(), 4);
        
        // Verify each parameter type
        // Token parameter
        assert!(matches!(&infos[0].params[0], Param::Other(n, Some(v)) 
            if n == "reason" && v.to_string() == "busy"));
        
        // Numeric parameter
        assert!(matches!(&infos[0].params[1], Param::Other(n, Some(v)) 
            if n == "id" && v.to_string() == "1"));
        
        // Flag parameter (no value)
        assert!(matches!(&infos[0].params[2], Param::Other(n, None) 
            if n == "critical"));
        
        // Quoted string parameter
        assert!(matches!(&infos[0].params[3], Param::Other(n, Some(v)) 
            if n == "info" && v.to_string().contains("quoted value")));
    }
    
    #[test]
    fn test_error_conditions() {
        // Test with missing angle brackets
        let input1 = "sip:busy@example.com";
        let result1 = parse_error_info(input1.as_bytes());
        assert!(result1.is_err(), "URI must be enclosed in angle brackets");
        
        // Test with unmatched brackets
        let input2 = "<sip:busy@example.com";
        let result2 = parse_error_info(input2.as_bytes());
        assert!(result2.is_err(), "Unmatched angle brackets should fail");
        
        // Test with empty URI
        let input3 = "<>";
        let result3 = parse_error_info(input3.as_bytes());
        assert!(result3.is_err(), "Empty URI should fail");
        
        // Test with invalid parameter format
        let input4 = "<sip:busy@example.com>invalid";
        let result4 = parse_error_info(input4.as_bytes());
        assert!(result4.is_err(), "Invalid parameter format should fail");
        
        // Test with missing URI in a list
        let input5 = "<sip:busy@example.com>, ";
        let result5 = parse_error_info(input5.as_bytes());
        assert!(result5.is_err(), "Missing URI in a list should fail");
    }
    
    #[test]
    fn test_rfc_examples() {
        // Example from RFC 3261 Section 24.2 (INVITE with Error-Info)
        let input = "Error-Info: <sip:not-in-service-recording@atlanta.com>";
        let result = full_parse_error_info(input.as_bytes());
        assert!(result.is_ok());
        let (_, infos) = result.unwrap();
        
        // Verify URI
        assert_eq!(infos[0].uri_str, "sip:not-in-service-recording@atlanta.com");
        assert_eq!(infos[0].uri.scheme.as_str(), "sip");
        
        // Example with a reason parameter (common in practice)
        let input2 = "Error-Info: <sip:busy-message@example.com>;reason=busy";
        let result2 = full_parse_error_info(input2.as_bytes());
        assert!(result2.is_ok());
        let (_, infos2) = result2.unwrap();
        
        // Verify URI and parameter
        assert_eq!(infos2[0].uri_str, "sip:busy-message@example.com");
        assert_eq!(infos2[0].uri.scheme.as_str(), "sip");
        assert_eq!(infos2[0].params.len(), 1);
    }
    
    #[test]
    fn test_abnf_compliance() {
        // Test valid cases according to ABNF
        let valid_cases = [
            "<sip:busy@example.com>",
            "<sip:busy@example.com>;reason=busy",
            "<sip:a@b.c>, <http://d.e/f>",
            "<sip:a@b.c>;p=1, <sip:d@e.f>;q=2",
        ];
        
        for case in &valid_cases {
            let result = parse_error_info(case.as_bytes());
            assert!(result.is_ok(), "Valid ABNF case should parse successfully: {:?}", case);
        }
        
        // Test invalid cases according to ABNF
        let invalid_cases = [
            "", // Empty
            "sip:busy@example.com", // Missing angle brackets
            "<>", // Empty URI
            "<sip:busy@example.com", // Unclosed bracket
            "<sip:busy@example.com>reason=busy", // Missing semicolon (this should now be detected by verify_no_trailing_chars)
            "<sip:busy@example.com>, ", // Trailing comma
        ];
        
        for case in &invalid_cases {
            let result = parse_error_info(case.as_bytes());
            assert!(result.is_err(), "Invalid ABNF case should be rejected: {:?}", case);
        }
    }
} 
// Parser for the Error-Info header (RFC 3261 Section 20.18)
// Error-Info = "Error-Info" HCOLON error-uri *(COMMA error-uri)
// error-uri = LAQUOT absoluteURI RAQUOT *( SEMI generic-param )

use nom::{
    bytes::complete::{tag, tag_no_case, take_until, is_not},
    character::complete::space0,
    combinator::{map, map_res, opt, verify, fail, recognize, all_consuming},
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

use crate::types::uri::Uri;
use crate::types::uri::{Scheme, Host};
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
    pub comment: Option<String>,
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
            
            // Trim any whitespace from the URI
            // RFC 3261 allows linear whitespace within angle brackets
            let trimmed = uri_str.trim();
            if trimmed.is_empty() {
                return Err(CrateError::ParseError("Empty URI after trimming".to_string()));
            }
            
            // Additional basic validation - check for scheme
            let scheme_end = trimmed.find(':');
            if scheme_end.is_none() {
                return Err(CrateError::ParseError("URI missing scheme".to_string()));
            }
            
            // Verify scheme starts with a letter and contains valid characters
            let scheme = &trimmed[0..scheme_end.unwrap()];
            if scheme.is_empty() || !scheme.chars().next().unwrap().is_ascii_alphabetic() {
                return Err(CrateError::ParseError(format!("Invalid scheme: {}", scheme)));
            }
            
            // Check that there's something after the colon
            if scheme_end.unwrap() + 1 >= trimmed.len() {
                return Err(CrateError::ParseError("Missing URI content after scheme".to_string()));
            }
            
            Ok(trimmed.to_string())
        }
    )(input)
}

/// Parses a parameter (;name=value or ;name)
fn param(input: &[u8]) -> ParseResult<Param> {
    preceded(
        pair(semi, space0), // Allow whitespace after semicolon
        generic_param
    )(input)
}

/// Verifies that there are no trailing characters after the parameter list
/// except possibly a properly formatted comment in parentheses
fn verify_no_trailing_chars(input: &[u8]) -> bool {
    // Skip any leading whitespace
    let mut i = 0;
    while i < input.len() && (input[i] == b' ' || input[i] == b'\t') {
        i += 1;
    }
    
    // Empty input is valid
    if i >= input.len() {
        return true;
    }
    
    // A comma (which would start the next URI) is valid
    if input[i] == b',' {
        return true;
    }
    
    // A '(' char starts a comment, which is valid
    if input[i] == b'(' {
        // Find the matching closing parenthesis
        let mut paren_count = 1;
        i += 1;
        
        while i < input.len() && paren_count > 0 {
            if input[i] == b'(' {
                paren_count += 1;
            } else if input[i] == b')' {
                paren_count -= 1;
            }
            i += 1;
        }
        
        // If we found the closing parenthesis
        if paren_count == 0 {
            // Skip any more whitespace
            while i < input.len() && (input[i] == b' ' || input[i] == b'\t') {
                i += 1;
            }
            
            // Now we expect to be at the end of input or at a comma
            return i >= input.len() || input[i] == b',';
        }
    }
    
    // Anything else is invalid
    false
}

/// Parse a comment enclosed in parentheses
fn error_info_comment(input: &[u8]) -> ParseResult<&[u8]> {
    delimited(
        tuple((space0, tag(b"("))), // Allow whitespace before opening parenthesis
        take_until(")"),
        tag(b")")
    )(input)
}

/// Parses an error-info-value, which is an error-uri followed by optional parameters and an optional comment.
/// error-info-value = error-uri *( SEMI generic-param ) [COMMENT]
fn error_info_value(input: &[u8]) -> ParseResult<ErrorInfoValue> {
    let (input, _) = space0(input)?;
    
    // Parse URI
    let (mut remaining, uri_str) = enclosed_uri(input)?;
    
    // Parse parameters
    let (remaining_after_params, params) = many0(param)(remaining)?;
    remaining = remaining_after_params;
    
    // Parse optional comment
    let mut comment_text = None;
    let comment_result = error_info_comment(remaining);
    if let Ok((rest, comment_data)) = comment_result {
        comment_text = Some(String::from_utf8_lossy(comment_data).into_owned());
        remaining = rest;
    }
    
    // Create a URI safely without recursive calls
    let uri = create_safe_uri(&uri_str).unwrap_or_else(|_| {
        // Create a default URI as fallback
        let host = Host::Domain("invalid.example.com".to_string());
        Uri {
            scheme: Scheme::Sip,
            user: None,
            password: None,
            host,
            port: None,
            parameters: Vec::new(),
            headers: std::collections::HashMap::new(),
            raw_uri: Some(uri_str.clone()),
        }
    });
    
    Ok((remaining, ErrorInfoValue { uri, uri_str, params, comment: comment_text }))
}

// Helper function to create an ErrorInfoUri without using FromStr implementation
// which avoids the recursive call path
fn create_safe_uri(uri_str: &str) -> Result<Uri, CrateError> {
    // Use the internal nom parser directly
    match crate::parser::uri::parse_uri(uri_str.as_bytes()) {
        Ok((_remaining, uri)) => {
            // TODO: Check if remaining bytes is an issue? The parser should ideally consume all.
            // Maybe use all_consuming here? But parse_uri might not be designed for that.
            Ok(uri)
        },
        Err(e) => {
            // If nom parsing fails, create a custom URI to preserve the string
            // but log the error. This mimics the previous behavior somewhat.
            eprintln!("Error-Info: Failed to parse URI '{}' with nom parser: {:?}. Storing as raw.", uri_str, e);
            Ok(Uri::custom(uri_str.to_string()))
        }
    }
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
    // Trim leading whitespace 
    let (input, _) = space0(input)?;
    
    // Parse list of error_info_values separated by commas
    let (input, items) = separated_list1(
        tuple((space0, comma, space0)), // Allow whitespace around commas
        error_info_value
    )(input)?;
    
    // Check for trailing comma
    let (input, _) = trailing_comma_check(input)?;
    
    // Verify we've consumed all input (except for trailing whitespace)
    let (input, _) = space0(input)?;
    if !input.is_empty() {
        return Err(Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Eof)));
    }
    
    Ok((input, items))
}

/// Parses a complete Error-Info header, including the "Error-Info:" prefix.
/// Example: `Error-Info: <sip:busy@example.com>;reason=busy`
pub fn full_parse_error_info(input: &[u8]) -> ParseResult<Vec<ErrorInfoValue>> {
    preceded(
        pair(tag_no_case(b"Error-Info"), tuple((hcolon, space0))), // Allow whitespace after colon
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
        
        // Verify URI string and scheme without calling methods that might recurse
        assert_eq!(infos[0].uri_str, "sip:not-in-service@example.com");
        
        // Check scheme directly from the enum variant
        match infos[0].uri.scheme {
            Scheme::Sip => {}, // Expected, test passes
            _ => panic!("Expected SIP scheme"),
        }
        
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
            ("<mailto:user@example.com>", "mailto:user@example.com"), // Additional scheme test
            ("<ftp://ftp.example.com/error.txt>", "ftp://ftp.example.com/error.txt"), // Additional scheme test
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
                _ => {
                    // Custom URI schemes should be stored properly
                    assert!(infos[0].uri.is_custom());
                    assert_eq!(infos[0].uri.as_raw_uri().unwrap(), expected_uri_str);
                },
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
        
        // Test with whitespace inside URI angle brackets (allowed by RFC 3261)
        let input_ws_uri = "<  sip:busy@example.com  >";
        let result_ws_uri = parse_error_info(input_ws_uri.as_bytes());
        assert!(result_ws_uri.is_ok(), "Should handle whitespace inside URI angle brackets");
        let (_, infos_ws) = result_ws_uri.unwrap();
        assert_eq!(infos_ws[0].uri_str, "sip:busy@example.com", "URI should be trimmed");
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
        
        // Test parameter with URI as value
        let input_uri_param = "<sip:busy@example.com>;href=<http://example.com/uri>";
        let result_uri_param = parse_error_info(input_uri_param.as_bytes());
        // This should fail - URIs as parameter values need special handling
        assert!(result_uri_param.is_err(), "URI as parameter value needs special handling");
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
        
        // Test with just whitespace
        let input6 = "   ";
        let result6 = parse_error_info(input6.as_bytes());
        assert!(result6.is_err(), "Just whitespace should fail");
        
        // Test with invalid scheme - This test was failing
        let input7 = "<invalid@example.com>";
        let result7 = parse_error_info(input7.as_bytes());
        assert!(result7.is_err(), "Invalid scheme should fail");
        
        // Test with parameter without semicolon
        let input8 = "<sip:busy@example.com>reason=busy";
        let result8 = parse_error_info(input8.as_bytes());
        assert!(result8.is_err(), "Parameter without semicolon should fail");
    }
    
    #[test]
    fn test_rfc_examples() {
        // Example from RFC 3261 Section 24.2 (INVITE with Error-Info)
        let input = "Error-Info: <sip:not-in-service-recording@atlanta.com>";
        let result = full_parse_error_info(input.as_bytes());
        assert!(result.is_ok(), "RFC example should parse successfully");
        let (_, infos) = result.unwrap();
        
        // Verify URI
        assert_eq!(infos[0].uri_str, "sip:not-in-service-recording@atlanta.com");
        assert_eq!(infos[0].uri.scheme.as_str(), "sip");
        
        // Example with a reason parameter (common in practice)
        let input2 = "Error-Info: <sip:busy-message@example.com>;reason=busy";
        let result2 = full_parse_error_info(input2.as_bytes());
        assert!(result2.is_ok(), "RFC example with parameter should parse successfully");
        let (_, infos2) = result2.unwrap();
        
        // Verify URI and parameter
        assert_eq!(infos2[0].uri_str, "sip:busy-message@example.com");
        assert_eq!(infos2[0].uri.scheme.as_str(), "sip");
        assert_eq!(infos2[0].params.len(), 1);
        
        // Example with multiple items (not from RFC but valid)
        let input3 = "Error-Info: <sip:not-in-service@example.com>, <http://errors.example.com/busy.html>";
        let result3 = full_parse_error_info(input3.as_bytes());
        assert!(result3.is_ok(), "Multiple URIs should parse successfully");
        let (_, infos3) = result3.unwrap();
        assert_eq!(infos3.len(), 2);
    }
    
    #[test]
    fn test_abnf_compliance() {
        // Test valid cases according to ABNF
        let valid_cases = [
            "<sip:busy@example.com>",
            "<sip:busy@example.com>;reason=busy",
            "<sip:a@b.c>, <http://d.e/f>",
            "<sip:a@b.c>;p=1, <sip:d@e.f>;q=2",
            "< sip:busy@example.com >", // With whitespace inside brackets (allowed by RFC)
            "<sip:busy@example.com>  ;  reason=busy", // With whitespace around semicolon
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
            "<sip:busy@example.com>reason=busy", // Missing semicolon
            "<sip:busy@example.com>, ", // Trailing comma
            "<sip:busy@example.com>,", // Trailing comma
            "<:busy@example.com>", // Missing scheme
        ];
        
        for case in &invalid_cases {
            let result = parse_error_info(case.as_bytes());
            assert!(result.is_err(), "Invalid ABNF case should be rejected: {:?}", case);
        }
    }

    #[test]
    fn test_parse_error_info_with_comment() {
        let input = "<sip:not-in-service@example.com>;reason=Foo (Service unavailable)";
        let result = parse_error_info(input.as_bytes());
        assert!(result.is_ok(), "Failed to parse with comment: {:?}", result.err());
        let (rem, infos) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(infos.len(), 1);
        
        // Verify URI
        assert_eq!(infos[0].uri_str, "sip:not-in-service@example.com");
        
        // Check parameters
        assert_eq!(infos[0].params.len(), 1);
        assert!(matches!(&infos[0].params[0], Param::Other(n, Some(v)) 
            if n == "reason" && v.to_string() == "Foo"));
        
        // Check comment
        assert_eq!(infos[0].comment, Some("Service unavailable".to_string()));
    }
} 
// RFC 3261 Section 22.5 Authentication-Info

use super::auth::common::ainfo;
use crate::parser::common::comma_separated_list1;
use crate::parser::ParseResult;
use crate::types::auth::AuthenticationInfoParam;
use nom::{
    IResult,
    multi::separated_list1,
    sequence::{delimited, preceded, terminated},
    bytes::complete::tag,
    combinator::{map, verify, recognize, opt},
    error::{ErrorKind, Error as NomError},
};
use crate::parser::whitespace::{sws, lws, owsp};
use crate::parser::separators::comma;

/// Custom error types for Authentication-Info parser to improve error messages
#[derive(Debug)]
enum AuthInfoError {
    EmptyInput,
    EmptyParameter,
    TrailingComma,
    InvalidParameter,
    MalformedHeaderSyntax,
    UnexpectedTrailingContent,
}

/// Create a NomError with a specific error kind and context
fn auth_info_error<'a>(input: &'a [u8], error: AuthInfoError) -> nom::Err<NomError<&'a [u8]>> {
    let kind = match error {
        AuthInfoError::EmptyInput => ErrorKind::TakeWhile1,
        AuthInfoError::EmptyParameter => ErrorKind::SeparatedList,
        AuthInfoError::TrailingComma => ErrorKind::Tag,
        AuthInfoError::InvalidParameter => ErrorKind::Alt,
        AuthInfoError::MalformedHeaderSyntax => ErrorKind::Verify,
        AuthInfoError::UnexpectedTrailingContent => ErrorKind::Eof,
    };
    
    nom::Err::Error(NomError::new(input, kind))
}

// Authentication-Info = "Authentication-Info" HCOLON auth-info *(COMMA auth-info)
// auth-info = nextnonce / message-qop / response-auth / cnonce / nonce-count
// 
// Per RFC 3261 section 22.5:
// The Authentication-Info header field provides authentication maintenance functions (future
// nonce generation) and message authentication. It typically provides mutual authentication.
pub fn parse_authentication_info(input: &[u8]) -> ParseResult<Vec<AuthenticationInfoParam>> {
    // Handle any leading whitespace, including line folding with lws (which handles CRLF + whitespace)
    let (input, _) = sws(input)?;
    
    // Check for empty input after consuming whitespace
    if input.is_empty() {
        return Err(auth_info_error(input, AuthInfoError::EmptyInput));
    }
    
    // Use separated_list1 which handles line folding through the whitespace handling in the separator
    let (remaining, params) = separated_list1(
        delimited(sws, comma, sws),  // This allows for LWS around the comma
        |input| {
            // Use lws to handle line folding before each parameter
            let (input, _) = opt(lws)(input)?;
            ainfo(input).map_err(|_| auth_info_error(input, AuthInfoError::InvalidParameter))
        }
    )(input)?;
    
    // Make sure there's nothing left after parsing
    let (remaining, _) = sws(remaining)?;
    
    if !remaining.is_empty() {
        return Err(auth_info_error(remaining, AuthInfoError::UnexpectedTrailingContent));
    }
    
    // Verify we got at least one valid parameter
    if params.is_empty() {
        return Err(auth_info_error(input, AuthInfoError::EmptyInput));
    }
    
    Ok((remaining, params))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::auth::{Qop, AuthenticationInfoParam};

    #[test]
    fn test_parse_authentication_info() {
        // Test with all possible parameters
        let input = br#"nextnonce="fedcba98", qop=auth, rspauth="abcdef01", cnonce="abc", nc=00000001"#;
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 5);
        assert!(params.contains(&AuthenticationInfoParam::NextNonce("fedcba98".to_string())));
        assert!(params.contains(&AuthenticationInfoParam::Qop(Qop::Auth)));
        assert!(params.contains(&AuthenticationInfoParam::ResponseAuth("abcdef01".to_string())));
        assert!(params.contains(&AuthenticationInfoParam::Cnonce("abc".to_string())));
        assert!(params.contains(&AuthenticationInfoParam::NonceCount(1)));
    }
    
    #[test]
    fn test_parse_authentication_info_single() {
        let input = br#"nextnonce="12345678""#;
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 1);
        assert!(params.contains(&AuthenticationInfoParam::NextNonce("12345678".to_string())));
    }
    
    #[test]
    fn test_parse_authentication_info_rfc_examples() {
        // Example based on RFC 3261 section 22.5
        // Note: The RFC doesn't provide explicit examples, but based on syntax
        let input = br#"nextnonce="47364c23432d2e131a5fb210812c", qop=auth, rspauth="9fe042d5a51597f51dcd4b41749cdd7f""#;
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 3);
        assert!(params.contains(&AuthenticationInfoParam::NextNonce("47364c23432d2e131a5fb210812c".to_string())));
        assert!(params.contains(&AuthenticationInfoParam::Qop(Qop::Auth)));
        assert!(params.contains(&AuthenticationInfoParam::ResponseAuth("9fe042d5a51597f51dcd4b41749cdd7f".to_string())));
    }
    
    #[test]
    fn test_parse_authentication_info_different_orders() {
        // Test with different orders of parameters - order doesn't matter
        let input = br#"qop=auth, nextnonce="12345678", rspauth="abcdef01""#;
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 3);
        assert!(params.contains(&AuthenticationInfoParam::NextNonce("12345678".to_string())));
        assert!(params.contains(&AuthenticationInfoParam::Qop(Qop::Auth)));
        assert!(params.contains(&AuthenticationInfoParam::ResponseAuth("abcdef01".to_string())));
        
        // Another order variation
        let input = br#"rspauth="abcdef01", cnonce="123", nc=00000001, nextnonce="12345678""#;
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 4);
        assert!(params.contains(&AuthenticationInfoParam::NextNonce("12345678".to_string())));
        assert!(params.contains(&AuthenticationInfoParam::ResponseAuth("abcdef01".to_string())));
        assert!(params.contains(&AuthenticationInfoParam::Cnonce("123".to_string())));
        assert!(params.contains(&AuthenticationInfoParam::NonceCount(1)));
    }
    
    #[test]
    fn test_parse_authentication_info_with_whitespace() {
        // Test with various whitespace patterns
        let input = br#"nextnonce="12345678",    qop=auth,   rspauth="abcdef01""#;
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 3);
        
        // With tabs and spaces mixed
        let input = br#"nextnonce="12345678",	qop=auth,	rspauth="abcdef01""#;
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 3);
        
        // With leading and trailing whitespace
        let input = br#"   nextnonce="12345678", qop=auth, rspauth="abcdef01"   "#;
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 3);
    }
    
    #[test]
    fn test_parse_authentication_info_with_line_folding() {
        // Test with line folding in various places
        // Note: RFC 3261 allows line folding as Linear White Space (LWS)
        
        // Line folding after a comma
        let input = b"nextnonce=\"12345678\",\r\n qop=auth";
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 2);
        
        // Line folding at the beginning (without any leading content)
        let input = b"\r\n nextnonce=\"12345678\"";
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 1);
    }
    
    #[test]
    fn test_parse_authentication_info_qop_variations() {
        // Test with different qop values
        let input = br#"qop=auth, nextnonce="12345678""#;
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(rem.is_empty());
        assert!(params.contains(&AuthenticationInfoParam::Qop(Qop::Auth)));
        
        let input = br#"qop=auth-int, nextnonce="12345678""#;
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(rem.is_empty());
        assert!(params.contains(&AuthenticationInfoParam::Qop(Qop::AuthInt)));
        
        // Test with a custom qop value
        let input = br#"qop=custom-qop, nextnonce="12345678""#;
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(rem.is_empty());
        assert!(params.iter().any(|p| match p {
            AuthenticationInfoParam::Qop(Qop::Other(val)) => val == "custom-qop",
            _ => false
        }));
    }
    
    #[test]
    fn test_parse_authentication_info_hex_values() {
        // Test with various hex values in nc
        let input = br#"nc=00000001, nextnonce="12345678""#;
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(params.contains(&AuthenticationInfoParam::NonceCount(1)));
        
        let input = br#"nc=0000ABCD, nextnonce="12345678""#;
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(params.contains(&AuthenticationInfoParam::NonceCount(0xABCD)));
        
        // Mixed case in hex is valid
        let input = br#"nc=0000aBcD, nextnonce="12345678""#;
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(params.contains(&AuthenticationInfoParam::NonceCount(0xABCD)));
    }
    
    #[test]
    fn test_parse_authentication_info_error_cases() {
        // Test with invalid inputs
        
        // Empty input
        let input = b"";
        assert!(parse_authentication_info(input).is_err());
        
        // Missing parameter value
        let input = b"nextnonce=";
        assert!(parse_authentication_info(input).is_err());
        
        // Missing parameter name
        let input = b"=\"12345678\"";
        assert!(parse_authentication_info(input).is_err());
        
        // Invalid nc value (not hex)
        let input = b"nc=GHIJK, nextnonce=\"12345678\"";
        assert!(parse_authentication_info(input).is_err());
        
        // Malformed quoted string 
        let input = b"nextnonce=\"unclosed";
        assert!(parse_authentication_info(input).is_err());
        
        // Trailing comma
        let input = b"nextnonce=\"12345678\",";
        assert!(parse_authentication_info(input).is_err());
        
        // Empty parameter between commas (will be caught by underlying parser)
        let input = b"nextnonce=\"12345678\",,qop=auth";
        assert!(parse_authentication_info(input).is_err());
    }
    
    #[test]
    fn test_parse_authentication_info_case_sensitivity() {
        // Parameter names are case-insensitive per RFC 3261
        let input = br#"NEXTNONCE="12345678", QOP=auth, RSPAUTH="abcdef01""#;
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 3);
        assert!(params.contains(&AuthenticationInfoParam::NextNonce("12345678".to_string())));
        assert!(params.contains(&AuthenticationInfoParam::Qop(Qop::Auth)));
        assert!(params.contains(&AuthenticationInfoParam::ResponseAuth("abcdef01".to_string())));
        
        // Mixed case should also work
        let input = br#"NextNonce="12345678", Qop=auth, RspAuth="abcdef01""#;
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 3);
        assert!(params.contains(&AuthenticationInfoParam::NextNonce("12345678".to_string())));
        assert!(params.contains(&AuthenticationInfoParam::Qop(Qop::Auth)));
        assert!(params.contains(&AuthenticationInfoParam::ResponseAuth("abcdef01".to_string())));
    }
} 
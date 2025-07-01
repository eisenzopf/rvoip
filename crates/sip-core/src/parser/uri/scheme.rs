// Parser for URI scheme component (RFC 3261/2396)
// scheme = ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )

use nom::{
    branch::alt,
    bytes::complete::{tag, take_while, take_while1},
    character::complete::{alpha1, alphanumeric1},
    combinator::{map, map_res, recognize, verify},
    multi::many0,
    sequence::{pair, preceded, terminated},
    IResult,
    error::{Error as NomError, ErrorKind},
};
use std::str;

use crate::parser::common_chars::{alpha, digit};
use crate::parser::ParseResult;
use crate::error::Error;
use crate::types::uri::Scheme;

// Check if a byte is allowed in a scheme after the first character
// RFC 2396: scheme = ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )
fn is_scheme_char(c: u8) -> bool {
    c.is_ascii_alphabetic() || c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.')
}

// scheme = ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )
// Return the raw bytes of the scheme
pub fn scheme_raw(input: &[u8]) -> ParseResult<&[u8]> {
    let (rem, scheme) = recognize(pair(
        alpha1,
        take_while(is_scheme_char)
    ))(input)?;
    
    // Explicit check for invalid characters
    for c in scheme.iter().skip(1) {
        if !is_scheme_char(*c) {
            return Err(nom::Err::Error(NomError::new(input, ErrorKind::AlphaNumeric)));
        }
    }
    
    // Now look for the colon terminator
    if rem.is_empty() || rem[0] != b':' {
        return Err(nom::Err::Error(NomError::new(input, ErrorKind::Tag)));
    }
    
    Ok((&rem[1..], scheme))
}

// Parse a scheme and convert it into a Scheme enum
pub fn parse_scheme(input: &[u8]) -> ParseResult<Scheme> {
    let (rem, scheme_bytes) = scheme_raw(input)?;
    
    // Convert to lowercase string for case-insensitive matching
    let scheme_str = str::from_utf8(scheme_bytes)
        .map_err(|_| nom::Err::Error(NomError::new(input, ErrorKind::Char)))?
        .to_lowercase();
        
    match scheme_str.as_str() {
        "sip" => Ok((rem, Scheme::Sip)),
        "sips" => Ok((rem, Scheme::Sips)),
        "tel" => Ok((rem, Scheme::Tel)),
        "http" => Ok((rem, Scheme::Http)),
        "https" => Ok((rem, Scheme::Https)),
        // Allow any valid scheme in request-URIs (for RFC 3261 compliance)
        _ => {
            // For unknown schemes, we'll use the first enum variant (Sip)
            // but the raw_uri field in Uri will capture the full URI
            Ok((rem, Scheme::Sip))
        }
    }
}

// Parse a scheme string followed by a colon
// Returns the raw bytes of the scheme without the colon
pub fn parse_scheme_raw(input: &[u8]) -> ParseResult<&[u8]> {
    scheme_raw(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    // === Basic Scheme Tests ===

    #[test]
    fn test_parse_scheme_sip() {
        let (rem, scheme) = parse_scheme(b"sip:").unwrap();
        assert!(rem.is_empty());
        assert_eq!(scheme, Scheme::Sip);
    }

    #[test]
    fn test_parse_scheme_sips() {
        let (rem, scheme) = parse_scheme(b"sips:").unwrap();
        assert!(rem.is_empty());
        assert_eq!(scheme, Scheme::Sips);
    }

    #[test]
    fn test_parse_scheme_tel() {
        let (rem, scheme) = parse_scheme(b"tel:").unwrap();
        assert!(rem.is_empty());
        assert_eq!(scheme, Scheme::Tel);
    }

    #[test]
    fn test_parse_scheme_raw() {
        let (rem, scheme) = parse_scheme_raw(b"sip:user@example.com").unwrap();
        assert_eq!(rem, b"user@example.com");
        assert_eq!(scheme, b"sip");
    }

    // === Case Sensitivity Tests ===

    #[test]
    fn test_scheme_case_insensitivity() {
        // RFC 3986 states schemes are case-insensitive
        let (_, scheme1) = parse_scheme(b"sip:").unwrap();
        let (_, scheme2) = parse_scheme(b"SIP:").unwrap();
        let (_, scheme3) = parse_scheme(b"Sip:").unwrap();
        
        assert_eq!(scheme1, Scheme::Sip);
        assert_eq!(scheme2, Scheme::Sip);
        assert_eq!(scheme3, Scheme::Sip);
    }

    // === Character Set Tests ===

    #[test]
    fn test_scheme_allowed_chars() {
        // Test scheme with all allowed character types
        // ALPHA followed by ALPHA / DIGIT / "+" / "-" / "."
        let (rem, raw) = parse_scheme_raw(b"a0+-.:").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(raw, b"a0+-.");
        
        // More realistic example
        let (rem, raw) = parse_scheme_raw(b"a-custom.scheme+2:test").unwrap();
        assert_eq!(rem, b"test");
        assert_eq!(raw, b"a-custom.scheme+2");
    }

    // === RFC 3261 Examples ===

    #[test]
    fn test_rfc3261_examples() {
        // Examples from RFC 3261
        
        // Section 19.1.1 SIP and SIPS URI Components
        let (_, scheme1) = parse_scheme(b"sip:alice@atlanta.com:5060").unwrap();
        assert_eq!(scheme1, Scheme::Sip);
        
        let (_, scheme2) = parse_scheme(b"sips:alice@atlanta.com").unwrap();
        assert_eq!(scheme2, Scheme::Sips);
        
        // Section 19.1.6 Examples of SIP and SIPS URIs
        let examples = [
            b"sip:alice@atlanta.com".as_ref(),
            b"sip:alice:secretword@atlanta.com;transport=tcp".as_ref(),
            b"sips:alice@atlanta.com?subject=project%20x&priority=urgent".as_ref(),
            b"sip:+1-212-555-1212:1234@gateway.com;user=phone".as_ref(),
            b"sips:1212@gateway.com".as_ref(),
            b"sip:alice@192.0.2.4".as_ref(),
            b"sip:atlanta.com;method=REGISTER?to=alice%40atlanta.com".as_ref(),
            b"sip:alice;day=tuesday@atlanta.com".as_ref()
        ];
        
        for example in examples {
            let (_, scheme) = parse_scheme(example).unwrap();
            // Check that examples starting with "sip:" parse as Sip
            // and those with "sips:" parse as Sips
            if example.starts_with(b"sip:") && !example.starts_with(b"sips:") {
                assert_eq!(scheme, Scheme::Sip);
            } else if example.starts_with(b"sips:") {
                assert_eq!(scheme, Scheme::Sips);
            }
        }
    }

    // === Error Cases ===

    #[test]
    fn test_parse_scheme_other() {
        // Our new parser is more lenient and accepts all valid schemes
        // but returns Scheme::Sip as a default for unknown schemes
        let result = parse_scheme(b"http:");
        assert!(result.is_ok());
        // Check that it returns Scheme::Http for http
        assert_eq!(result.unwrap().1, Scheme::Http);
        
        // Test with another known scheme
        let result = parse_scheme(b"https:");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().1, Scheme::Https);
        
        // Test with unknown scheme - should return Ok with Scheme::Sip
        let result = parse_scheme(b"unknown:");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().1, Scheme::Sip);
    }

    #[test]
    fn test_invalid_scheme_syntax() {
        // Must start with ALPHA
        assert!(parse_scheme(b"1http:").is_err());
        assert!(scheme_raw(b"1http").is_err());
        
        // Invalid characters
        assert!(parse_scheme(b"http@:").is_err());
        assert!(scheme_raw(b"http@").is_err());
        
        // Empty scheme
        assert!(parse_scheme(b":").is_err());
        assert!(scheme_raw(b"").is_err());
    }

    // === Edge Cases ===

    #[test]
    fn test_scheme_with_following_content() {
        // Test scheme followed by various URI components
        let (rem, _) = parse_scheme(b"sip:alice@example.com").unwrap();
        assert_eq!(rem, b"alice@example.com");
        
        let (rem, _) = parse_scheme(b"sip://example.com").unwrap();
        assert_eq!(rem, b"//example.com");
        
        let (rem, _) = parse_scheme(b"tel:+1-212-555-1212").unwrap();
        assert_eq!(rem, b"+1-212-555-1212");
    }

    #[test]
    fn test_long_scheme() {
        // Test with a very long (but valid) scheme
        // RFC doesn't specify length limits
        let long_scheme = b"abcdefghijklmnopqrstuvwxyz:";
        let (rem, raw) = parse_scheme_raw(long_scheme).unwrap();
        assert!(rem.is_empty());
        assert_eq!(raw, &long_scheme[0..long_scheme.len()-1]);
        
        // This is now accepted by parse_scheme because our parser is more lenient
        let result = parse_scheme(long_scheme);
        assert!(result.is_ok());
        // Unknown schemes return Scheme::Sip as default
        assert_eq!(result.unwrap().1, Scheme::Sip);
    }

    #[test]
    fn test_minimal_valid_scheme() {
        // Minimal valid scheme is a single ALPHA char
        let (rem, raw) = parse_scheme_raw(b"a:").unwrap();
        assert!(rem.is_empty());
        assert_eq!(raw, b"a");
        
        // This is now accepted by parse_scheme because our parser is more lenient
        let result = parse_scheme(b"a:");
        assert!(result.is_ok());
        // Unknown schemes return Scheme::Sip as default
        assert_eq!(result.unwrap().1, Scheme::Sip);
    }
} 
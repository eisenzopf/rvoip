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
use crate::Scheme;

// Check if a byte is allowed in a scheme after the first character
fn is_scheme_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.')
}

// scheme = ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )
// Return the raw bytes of the scheme
fn scheme_raw(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(pair(
        alpha,
        take_while(is_scheme_char)
    ))(input)
}

// Parse a scheme and convert it into a Scheme enum
pub fn parse_scheme(input: &[u8]) -> ParseResult<Scheme> {
    map_res(
        terminated(scheme_raw, tag(b":")),
        |scheme_bytes| {
            let scheme_str = str::from_utf8(scheme_bytes)?.to_lowercase();
            match scheme_str.as_str() {
                "sip" => Ok(Scheme::Sip),
                "sips" => Ok(Scheme::Sips),
                "tel" => Ok(Scheme::Tel),
                _ => Err(Error::InvalidUri(format!("Unsupported scheme: {}", scheme_str))),
            }
        }
    )(input)
}

// Parse a scheme string followed by a colon
// Returns the raw bytes of the scheme without the colon
pub fn parse_scheme_raw(input: &[u8]) -> ParseResult<&[u8]> {
    terminated(scheme_raw, tag(b":"))(input)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_parse_scheme_other() {
        let result = parse_scheme(b"http:");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_scheme_raw() {
        let (rem, scheme) = parse_scheme_raw(b"sip:user@example.com").unwrap();
        assert_eq!(rem, b"user@example.com");
        assert_eq!(scheme, b"sip");
    }

    #[test]
    fn test_invalid_scheme() {
        assert!(parse_scheme(b"1http:").is_err()); // Can't start with digit
        assert!(parse_scheme(b"http@:").is_err()); // Invalid character @
    }
} 
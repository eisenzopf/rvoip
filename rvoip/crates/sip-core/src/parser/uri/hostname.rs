use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    character::complete::{alpha1, alphanumeric1},
    combinator::{map_res, recognize},
    multi::many0,
    sequence::pair,
    IResult,
    error::{Error as NomError, ErrorKind, ParseError},
};
use std::str;

use crate::types::uri::Host;
use crate::parser::ParseResult;
use crate::error::{Error, Result};

// hostname = *( domainlabel "." ) toplabel [ "." ]
// domainlabel = alphanum / alphanum *( alphanum / "-" ) alphanum
// toplabel = ALPHA / ALPHA *( alphanum / "-" ) alphanum
// Simplified: Recognizes sequences of alphanumeric/hyphen labels separated by dots.
// Does not enforce toplabel/domainlabel specific content rules, relies on higher-level validation if needed.

// domainlabel or toplabel part
// According to RFC 3261, a hostname should consist of domain labels separated by dots
// This ensures a hostname has at least one dot to distinguish it from a token
fn domain_part(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(pair(
        pair(
            take_while1(|c:u8| c.is_ascii_alphanumeric() || c == b'-'), // domainlabel
            tag(b".".as_slice()) // require at least one dot
        ),
        alt((
            recognize(pair(
                many0(pair(
                    take_while1(|c:u8| c.is_ascii_alphanumeric() || c == b'-'),
                    tag(b".".as_slice())
                )),
                take_while1(|c: u8| c.is_ascii_alphanumeric() || c == b'-') // toplabel
            )),
            tag(b"") // allow trailing dot at the end (FQDN)
        ))
    ))(input)
}

// Helper to identify if a string might be an IPv4 address
// This is a basic check - we assume 1-3 digits followed by a dot, repeated 4 times
fn is_likely_ipv4(input: &[u8]) -> bool {
    if input.len() < 7 || input.len() > 15 { // Min valid IPv4: 1.1.1.1, Max: 255.255.255.255
        return false;
    }
    
    let mut dots = 0;
    let mut digits_since_last_dot = 0;
    
    for &c in input {
        if c == b'.' {
            if digits_since_last_dot == 0 || digits_since_last_dot > 3 {
                return false;
            }
            dots += 1;
            digits_since_last_dot = 0;
        } else if c.is_ascii_digit() {
            digits_since_last_dot += 1;
        } else {
            return false; // Non-digit, non-dot character
        }
    }
    
    // Valid IPv4 has exactly 3 dots (4 number segments)
    dots == 3 && digits_since_last_dot > 0 && digits_since_last_dot <= 3
}

// hostname parser
pub fn hostname(input: &[u8]) -> ParseResult<Host> {
    // First, check if input looks like an IPv4 address
    // If so, fail early to let the IPv4 parser handle it
    if is_likely_ipv4(input) {
        return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
    }

    map_res(
        domain_part,
        |bytes: &[u8]| -> Result<Host> {
            // Basic validation: Ensure not empty and doesn't start/end with hyphen (common basic check)
            if bytes.is_empty() || bytes[0] == b'-' || bytes[bytes.len() - 1] == b'-' {
                Err(Error::ParseError(format!("Invalid hostname format: {:?}", bytes)))
            } else {
                let s = str::from_utf8(bytes)?;
                Ok(Host::Domain(s.to_string()))
            }
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hostname() {
        assert_eq!(hostname(b"example.com"), Ok((&[][..], Host::Domain("example.com".to_string()))));
        assert_eq!(hostname(b"host1.subdomain.example.co.uk"), Ok((&[][..], Host::Domain("host1.subdomain.example.co.uk".to_string()))));
        assert_eq!(hostname(b"a-valid-host.net"), Ok((&[][..], Host::Domain("a-valid-host.net".to_string()))));
        assert_eq!(hostname(b"xn--ls8h.example"), Ok((&[][..], Host::Domain("xn--ls8h.example".to_string())))); // IDN
        
        // Allow trailing dot (RFC 1035, less common in SIP?)
        assert_eq!(hostname(b"example.com."), Ok((&b"."[..], Host::Domain("example.com".to_string())))); // Should consume up to trailing dot? TBC

        // Invalid cases
        assert!(hostname(b"-invalid.start").is_err());
        assert!(hostname(b"invalid.end-").is_err());
        assert!(hostname(b"invalid..dot").is_err()); // Fails domain_part recognition
        assert!(hostname(b".").is_err()); // Fails domain_part recognition
    }
} 
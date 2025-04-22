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

// hostname = *( domainlabel "." ) toplabel [ "." ]
// domainlabel = alphanum / alphanum *( alphanum / "-" ) alphanum
// toplabel = ALPHA / ALPHA *( alphanum / "-" ) alphanum
// Simplified: Recognizes sequences of alphanumeric/hyphen labels separated by dots.
// Does not enforce toplabel/domainlabel specific content rules, relies on higher-level validation if needed.

// domainlabel or toplabel part
// Matches sequences like "example" or "co-uk"
fn domain_part(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(pair(
        many0(pair(take_while1(|c:u8| c.is_ascii_alphanumeric() || c == b'-'), tag(b".".as_slice()))),
        take_while1(|c: u8| c.is_ascii_alphanumeric() || c == b'-')
    ))(input)
}

// hostname parser
pub(crate) fn hostname(input: &[u8]) -> ParseResult<Host> {
    map_res(
        domain_part,
        |bytes| {
            // Basic validation: Ensure not empty and doesn't start/end with hyphen (common basic check)
            if bytes.is_empty() || bytes[0] == b'-' || bytes[bytes.len() - 1] == b'-' {
                Err(NomError::from_error_kind(bytes, ErrorKind::Verify))
            } else {
                str::from_utf8(bytes)
                    .map(|s| Host::Domain(s.to_string()))
                    .map_err(|_| nom::Err::Failure((input, nom::error::ErrorKind::Char)))
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
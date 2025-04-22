// Parser for URI authority component (RFC 3261/2396)
// authority = userinfo@host:port or standalone reg-name

use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    combinator::{map, opt, recognize},
    multi::many1,
    sequence::{pair, preceded, terminated},
    IResult,
};
use std::str;

// Import shared parsers
use crate::parser::common_chars::{escaped, unreserved};
use crate::parser::uri::host::hostport;
use crate::parser::uri::userinfo::userinfo;
use crate::parser::ParseResult;

// reg-name-char = unreserved / escaped / "$" / "," / ";" / ":" / "@" / "&" / "=" / "+"
fn is_reg_name_char(c: u8) -> bool {
    // Check unreserved first (alphanum / mark)
    c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')') ||
    // Check other allowed chars
    matches!(c, b'$' | b',' | b';' | b':' | b'@' | b'&' | b'=' | b'+')
}

// reg-name = 1*( unreserved / escaped / "$" / "," / ";" / ":" / "@" / "&" / "=" / "+" )
fn reg_name(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(many1(alt((escaped, take_while1(is_reg_name_char)))))(input)
}

// userinfo_bytes = use userinfo parser but return matched bytes instead
fn userinfo_bytes(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(terminated(
        pair(crate::parser::uri::userinfo::user, 
             opt(preceded(tag(b":"), crate::parser::uri::userinfo::password))),
        tag(b"@")
    ))(input)
}

// srvr = [ [ userinfo "@" ] hostport ]
fn srvr(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(pair(opt(userinfo_bytes), hostport))(input)
}

// authority = srvr / reg-name
// Returns the matched section of input as raw bytes
pub fn parse_authority(input: &[u8]) -> ParseResult<&[u8]> {
    alt((srvr, reg_name))(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_authority_with_userinfo() {
        let (rem, auth) = parse_authority(b"user:pass@example.com:5060").unwrap();
        assert!(rem.is_empty());
        assert_eq!(auth, b"user:pass@example.com:5060");
    }

    #[test]
    fn test_parse_authority_just_host() {
        let (rem, auth) = parse_authority(b"example.com").unwrap();
        assert!(rem.is_empty());
        assert_eq!(auth, b"example.com");
    }

    #[test]
    fn test_parse_authority_reg_name() {
        let (rem, auth) = parse_authority(b"some-registry.com:8080").unwrap();
        assert!(rem.is_empty());
        assert_eq!(auth, b"some-registry.com:8080");
    }
} 
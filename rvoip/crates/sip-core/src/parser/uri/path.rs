// Parser for URI path component (RFC 3261/2396)
// path consists of segments separated by slashes

use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    combinator::{recognize},
    multi::{many0},
    sequence::{pair, preceded},
};
use std::str;

use crate::parser::common_chars::{escaped};
use crate::parser::ParseResult;

// pchar = unreserved / escaped / ":" / "@" / "&" / "=" / "+" / "$" / ","
fn is_pchar_char(c: u8) -> bool {
    // Check unreserved first (alphanum / mark)
    c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')') ||
    // Check other allowed chars
    matches!(c, b':' | b'@' | b'&' | b'=' | b'+' | b'$' | b',')
}

// pchar parser that matches a single path character
fn pchar(input: &[u8]) -> ParseResult<&[u8]> {
    alt((escaped, take_while1(is_pchar_char)))(input)
}

// param = *pchar
pub fn param(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(many0(pchar))(input)
}

// segment = *pchar *( ";" param )
pub fn segment(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(pair(
        many0(pchar),
        many0(preceded(tag(b";"), param))
    ))(input)
}

// path-segments = segment *( "/" segment )
pub fn path_segments(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(pair(
        segment,
        many0(preceded(tag(b"/"), segment))
    ))(input)
}

// abs-path = "/" path-segments
pub fn abs_path(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(preceded(tag(b"/"), path_segments))(input)
}

// Parse the entire path component of a URI
pub fn parse_path(input: &[u8]) -> ParseResult<&[u8]> {
    abs_path(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_path() {
        let (rem, parsed) = parse_path(b"/simple/path").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/simple/path");
    }

    #[test]
    fn test_parse_path_with_params() {
        let (rem, parsed) = parse_path(b"/path;param1=value1/segment;param2").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/path;param1=value1/segment;param2");
    }

    #[test]
    fn test_parse_complex_path() {
        let (rem, parsed) = parse_path(b"/a/b;c=1/d;e;f=2").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/a/b;c=1/d;e;f=2");
    }

    #[test]
    fn test_parse_path_with_escaped_chars() {
        let (rem, parsed) = parse_path(b"/user%20name/profile%3F").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/user%20name/profile%3F");
    }
} 
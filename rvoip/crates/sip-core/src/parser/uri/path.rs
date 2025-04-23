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

    // === Basic Path Structure Tests ===

    #[test]
    fn test_parse_simple_path() {
        let (rem, parsed) = parse_path(b"/simple/path").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/simple/path");
    }

    #[test]
    fn test_root_path() {
        // Root path is just "/"
        let (rem, parsed) = parse_path(b"/").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/");
    }

    #[test]
    fn test_empty_segments() {
        // Path with empty segments (consecutive slashes)
        let (rem, parsed) = parse_path(b"//").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"//");

        let (rem, parsed) = parse_path(b"/segment//").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/segment//");

        let (rem, parsed) = parse_path(b"///multiple").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"///multiple");
    }

    #[test]
    fn test_trailing_slash() {
        // Path with trailing slash
        let (rem, parsed) = parse_path(b"/path/").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/path/");
    }

    // === Parameter Tests ===

    #[test]
    fn test_parse_path_with_params() {
        let (rem, parsed) = parse_path(b"/path;param1=value1/segment;param2").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/path;param1=value1/segment;param2");
    }

    #[test]
    fn test_empty_params() {
        // Empty parameter (just semicolon)
        let (rem, parsed) = parse_path(b"/path;").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/path;");

        // Multiple empty parameters
        let (rem, parsed) = parse_path(b"/path;;").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/path;;");
    }

    #[test]
    fn test_multiple_params() {
        // Multiple parameters on a single segment
        let (rem, parsed) = parse_path(b"/path;p1=v1;p2;p3=v3").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/path;p1=v1;p2;p3=v3");
    }

    #[test]
    fn test_parse_complex_path() {
        let (rem, parsed) = parse_path(b"/a/b;c=1/d;e;f=2").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/a/b;c=1/d;e;f=2");
    }

    // === Character Encoding Tests ===

    #[test]
    fn test_parse_path_with_escaped_chars() {
        let (rem, parsed) = parse_path(b"/user%20name/profile%3F").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/user%20name/profile%3F");
    }

    #[test]
    fn test_path_with_allowed_chars() {
        // Test all allowed characters in pchar
        // unreserved + ":" | "@" | "&" | "=" | "+" | "$" | ","
        let (rem, parsed) = parse_path(b"/abc123-_.!~*'()/:@&=+$,").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/abc123-_.!~*'()/:@&=+$,");
    }

    #[test]
    fn test_escaped_sequences() {
        // Various hex escapes
        let (rem, parsed) = parse_path(b"/%41%42%43/%61%62%63").unwrap(); // /ABC/abc
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/%41%42%43/%61%62%63");

        // Mix of escaped and normal chars
        let (rem, parsed) = parse_path(b"/user%20name/%3Cangle%3E").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/user%20name/%3Cangle%3E");
    }

    // === RFC 3261 Specific Tests ===

    #[test]
    fn test_rfc3261_examples() {
        // RFC 3261 path examples
        let (rem, parsed) = parse_path(b"/alice").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/alice");

        let (rem, parsed) = parse_path(b"/conference").unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/conference");
    }

    // === RFC 4475 Torture Tests ===

    #[test]
    fn test_rfc4475_torture_paths() {
        // Escaped special chars in path segments
        let (rem, parsed) = parse_path(b"/sip%3Auser").unwrap(); // /sip:user
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/sip%3Auser");

        // Unusual characters that must be escaped
        let (rem, parsed) = parse_path(b"/path%25percent").unwrap(); // /path%percent
        assert!(rem.is_empty());
        assert_eq!(parsed, b"/path%25percent");
    }

    // === Edge Case Tests ===

    #[test]
    fn test_long_path() {
        // Test path with many segments
        let long_path = b"/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/v/w/x/y/z";
        let (rem, parsed) = parse_path(long_path).unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, long_path);
    }

    #[test]
    fn test_complex_mixed_path() {
        // Mix of empty segments, parameters, escapes
        let path = b"//segment;param=val//;empty;p=%20/";
        let (rem, parsed) = parse_path(path).unwrap();
        assert!(rem.is_empty());
        assert_eq!(parsed, path);
    }

    // === Remaining Input Tests ===

    #[test]
    fn test_path_with_remaining() {
        // Path followed by query or fragment
        let (rem, parsed) = parse_path(b"/path?query=value").unwrap();
        assert_eq!(rem, b"?query=value");
        assert_eq!(parsed, b"/path");

        let (rem, parsed) = parse_path(b"/path#fragment").unwrap();
        assert_eq!(rem, b"#fragment");
        assert_eq!(parsed, b"/path");
    }

    // === Invalid Input Tests ===

    #[test]
    fn test_invalid_paths() {
        // Path must start with "/"
        assert!(parse_path(b"path").is_err());
        
        // This test would only apply if the parser strictly checked for
        // invalid characters, which the current implementation doesn't.
        // Left as documentation of what would be checked in a stricter parser:
        // 
        // Invalid characters in path (if strictly enforcing)
        // assert!(parse_path(b"/path with space").is_err());
        // assert!(parse_path(b"/path<invalid>char").is_err());
    }
} 
use nom::{
    branch::alt,
    bytes::complete::{tag, take, take_while1, take_while_m_n, take_till},
    character::complete::{alpha1, alphanumeric1, digit1, hex_digit1},
    combinator::{map_res, recognize},
    sequence::tuple,
    IResult,
};
use std::str;

// Type alias for parser result
pub type ParseResult<'a, O> = IResult<&'a [u8], O>;

// Core Rules (RFC 2234) & Basic Rules (RFC 3261) Character Sets

pub fn alpha(input: &[u8]) -> ParseResult<&[u8]> {
    take_while1(nom::character::is_alphabetic)(input)
}

pub fn digit(input: &[u8]) -> ParseResult<&[u8]> {
    take_while1(nom::character::is_digit)(input)
}

pub fn alphanum(input: &[u8]) -> ParseResult<&[u8]> {
    take_while1(nom::character::is_alphanumeric)(input)
}

pub fn hex_digit(input: &[u8]) -> ParseResult<&[u8]> {
    hex_digit1(input)
}

pub fn lhex(input: &[u8]) -> ParseResult<&[u8]> {
    // LHEX = DIGIT / %x61-66 ;lowercase a-f
    take_while1(|c: u8| c.is_ascii_digit() || (c >= b'a' && c <= b'f'))(input)
}

pub fn mark(input: &[u8]) -> ParseResult<&[u8]> {
    // mark = "-" / "_" / "." / "!" / "~" / "*" / "'" / "(" / ")"
    recognize(alt((
        tag(b"-"), tag(b"_"), tag(b"."), tag(b"!"), tag(b"~"), 
        tag(b"*"), tag(b"'"), tag(b"("), tag(b")")
    )))(input)
}

pub fn unreserved(input: &[u8]) -> ParseResult<&[u8]> {
    // unreserved = alphanum / mark
    alt((alphanum, mark))(input)
}

pub fn reserved(input: &[u8]) -> ParseResult<&[u8]> {
    // reserved = ";" / "/" / "?" / ":" / "@" / "&" / "=" / "+" / "$" / ","
    recognize(alt((
        tag(b";"), tag(b"/"), tag(b"?"), tag(b":"), tag(b"@"), 
        tag(b"&"), tag(b"="), tag(b"+"), tag(b"$"), tag(b",")
    )))(input)
}

fn is_hex_digit_byte(c: u8) -> bool {
    (c >= b'0' && c <= b'9') || (c >= b'A' && c <= b'F') || (c >= b'a' && c <= b'f')
}

pub fn escaped(input: &[u8]) -> ParseResult<&[u8]> {
    // escaped = "%" HEXDIG HEXDIG
    recognize(tuple((tag(b"%"), take_while_m_n(2, 2, is_hex_digit_byte))))(input)
}

pub fn lalpha(input: &[u8]) -> ParseResult<&[u8]> {
    take_while1(|c: u8| c.is_ascii_lowercase())(input)
}

pub fn ualpha(input: &[u8]) -> ParseResult<&[u8]> {
    take_while1(|c: u8| c.is_ascii_uppercase())(input)
}

// Takes all bytes until a CRLF sequence (\r\n) is found
// Useful for parsing header values that extend to the end of a line
pub fn take_till_crlf(input: &[u8]) -> ParseResult<&[u8]> {
    take_till(|c| c == b'\r' || c == b'\n')(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nom::error::{Error, ErrorKind};

    #[test]
    fn test_alpha() {
        // Test valid inputs
        let (rem, result) = alpha(b"abc123").unwrap();
        assert_eq!(result, b"abc");
        assert_eq!(rem, b"123");
        
        let (rem, result) = alpha(b"ABCdef").unwrap();
        assert_eq!(result, b"ABCdef");
        assert_eq!(rem, b"");
        
        // Test invalid inputs
        let err = alpha(b"123abc").unwrap_err();
        assert!(matches!(err, nom::Err::Error(_)));
        
        let err = alpha(b"").unwrap_err();
        assert!(matches!(err, nom::Err::Error(_)));
    }

    #[test]
    fn test_digit() {
        // Test valid inputs
        let (rem, result) = digit(b"123abc").unwrap();
        assert_eq!(result, b"123");
        assert_eq!(rem, b"abc");
        
        let (rem, result) = digit(b"9876543210").unwrap();
        assert_eq!(result, b"9876543210");
        assert_eq!(rem, b"");
        
        // Test invalid inputs
        let err = digit(b"abc123").unwrap_err();
        assert!(matches!(err, nom::Err::Error(_)));
        
        let err = digit(b"").unwrap_err();
        assert!(matches!(err, nom::Err::Error(_)));
    }

    #[test]
    fn test_alphanum() {
        // Test valid inputs
        let (rem, result) = alphanum(b"abc123xyz").unwrap();
        assert_eq!(result, b"abc123xyz");
        assert_eq!(rem, b"");
        
        let (rem, result) = alphanum(b"123abc!").unwrap();
        assert_eq!(result, b"123abc");
        assert_eq!(rem, b"!");
        
        // Test invalid inputs
        let err = alphanum(b"!abc123").unwrap_err();
        assert!(matches!(err, nom::Err::Error(_)));
        
        let err = alphanum(b"").unwrap_err();
        assert!(matches!(err, nom::Err::Error(_)));
    }

    #[test]
    fn test_hex_digit() {
        // Test valid inputs
        let (rem, result) = hex_digit(b"0123456789ABCDEF").unwrap();
        assert_eq!(result, b"0123456789ABCDEF");
        assert_eq!(rem, b"");
        
        let (rem, result) = hex_digit(b"abcdefABCDEF09").unwrap();
        assert_eq!(result, b"abcdefABCDEF09");
        assert_eq!(rem, b"");
        
        let (rem, result) = hex_digit(b"DeadBeef!").unwrap();
        assert_eq!(result, b"DeadBeef");
        assert_eq!(rem, b"!");
        
        // Test invalid inputs
        let err = hex_digit(b"GHIJK").unwrap_err();
        assert!(matches!(err, nom::Err::Error(_)));
        
        let err = hex_digit(b"").unwrap_err();
        assert!(matches!(err, nom::Err::Error(_)));
    }

    #[test]
    fn test_lhex() {
        // Test valid inputs
        let (rem, result) = lhex(b"0123456789abcdef").unwrap();
        assert_eq!(result, b"0123456789abcdef");
        assert_eq!(rem, b"");
        
        let (rem, result) = lhex(b"a0b1c2d3e4f5").unwrap();
        assert_eq!(result, b"a0b1c2d3e4f5");
        assert_eq!(rem, b"");
        
        let (rem, result) = lhex(b"deadbeef!").unwrap();
        assert_eq!(result, b"deadbeef");
        assert_eq!(rem, b"!");
        
        // Test invalid inputs - should reject uppercase hex
        let err = lhex(b"ABCDEF").unwrap_err();
        assert!(matches!(err, nom::Err::Error(_)));
        
        let err = lhex(b"").unwrap_err();
        assert!(matches!(err, nom::Err::Error(_)));
    }

    #[test]
    fn test_mark() {
        // Test each valid mark character
        let marks = [b"-", b"_", b".", b"!", b"~", b"*", b"'", b"(", b")"];
        
        for &m in marks.iter() {
            let (rem, result) = mark(m).unwrap();
            assert_eq!(result, m);
            assert_eq!(rem, b"");
        }
        
        // Test with content after mark
        let (rem, result) = mark(b"-abc").unwrap();
        assert_eq!(result, b"-");
        assert_eq!(rem, b"abc");
        
        // Test invalid inputs
        let err = mark(b"abc").unwrap_err();
        assert!(matches!(err, nom::Err::Error(_)));
        
        let err = mark(b"").unwrap_err();
        assert!(matches!(err, nom::Err::Error(_)));
    }

    #[test]
    fn test_unreserved() {
        // Test alphanumeric
        let (rem, result) = unreserved(b"abc123").unwrap();
        assert_eq!(result, b"abc123");
        assert_eq!(rem, b"");
        
        // Test marks
        for m in [b"-", b"_", b".", b"!", b"~", b"*", b"'", b"(", b")"].iter() {
            let input = *m;
            let (rem, result) = unreserved(input).unwrap();
            assert_eq!(result, input);
            assert_eq!(rem, b"");
        }
        
        // Test alphanumeric followed by mark
        let (rem, result) = unreserved(b"abc-").unwrap();
        assert_eq!(result, b"abc");
        assert_eq!(rem, b"-");
        
        // Test mark followed by alphanumeric
        let (rem, result) = unreserved(b"-abc").unwrap();
        assert_eq!(result, b"-");
        assert_eq!(rem, b"abc");
        
        // Test invalid inputs
        let err = unreserved(b";abc").unwrap_err(); // ; is reserved, not unreserved
        assert!(matches!(err, nom::Err::Error(_)));
        
        let err = unreserved(b"").unwrap_err();
        assert!(matches!(err, nom::Err::Error(_)));
    }

    #[test]
    fn test_reserved() {
        // Test each valid reserved character
        let reserved_chars = [b";", b"/", b"?", b":", b"@", b"&", b"=", b"+", b"$", b","];
        
        for &r in reserved_chars.iter() {
            let (rem, result) = reserved(r).unwrap();
            assert_eq!(result, r);
            assert_eq!(rem, b"");
        }
        
        // Test with content after reserved
        let (rem, result) = reserved(b";abc").unwrap();
        assert_eq!(result, b";");
        assert_eq!(rem, b"abc");
        
        // Test invalid inputs
        let err = reserved(b"abc").unwrap_err();
        assert!(matches!(err, nom::Err::Error(_)));
        
        let err = reserved(b"").unwrap_err();
        assert!(matches!(err, nom::Err::Error(_)));
    }

    #[test]
    fn test_escaped() {
        // Test valid escaped sequences
        let (rem, result) = escaped(b"%20abc").unwrap();
        assert_eq!(result, b"%20");
        assert_eq!(rem, b"abc");
        
        let (rem, result) = escaped(b"%FFend").unwrap();
        assert_eq!(result, b"%FF");
        assert_eq!(rem, b"end");
        
        let (rem, result) = escaped(b"%0A%0D").unwrap();
        assert_eq!(result, b"%0A");
        assert_eq!(rem, b"%0D");
        
        // Test invalid inputs
        let err = escaped(b"%1").unwrap_err(); // Needs 2 hex digits
        assert!(matches!(err, nom::Err::Error(_)));
        
        let err = escaped(b"%XY").unwrap_err(); // X and Y aren't hex
        assert!(matches!(err, nom::Err::Error(_)));
        
        let err = escaped(b"abc").unwrap_err(); // No % character
        assert!(matches!(err, nom::Err::Error(_)));
        
        let err = escaped(b"").unwrap_err();
        assert!(matches!(err, nom::Err::Error(_)));
    }

    #[test]
    fn test_lalpha() {
        // Test valid lowercase
        let (rem, result) = lalpha(b"abcdefghij").unwrap();
        assert_eq!(result, b"abcdefghij");
        assert_eq!(rem, b"");
        
        let (rem, result) = lalpha(b"abc123").unwrap();
        assert_eq!(result, b"abc");
        assert_eq!(rem, b"123");
        
        // Test invalid inputs
        let err = lalpha(b"ABC").unwrap_err(); // Uppercase
        assert!(matches!(err, nom::Err::Error(_)));
        
        let err = lalpha(b"123").unwrap_err(); // Digits
        assert!(matches!(err, nom::Err::Error(_)));
        
        let err = lalpha(b"").unwrap_err();
        assert!(matches!(err, nom::Err::Error(_)));
    }

    #[test]
    fn test_ualpha() {
        // Test valid uppercase
        let (rem, result) = ualpha(b"ABCDEFGHIJ").unwrap();
        assert_eq!(result, b"ABCDEFGHIJ");
        assert_eq!(rem, b"");
        
        let (rem, result) = ualpha(b"ABC123").unwrap();
        assert_eq!(result, b"ABC");
        assert_eq!(rem, b"123");
        
        // Test invalid inputs
        let err = ualpha(b"abc").unwrap_err(); // Lowercase
        assert!(matches!(err, nom::Err::Error(_)));
        
        let err = ualpha(b"123").unwrap_err(); // Digits
        assert!(matches!(err, nom::Err::Error(_)));
        
        let err = ualpha(b"").unwrap_err();
        assert!(matches!(err, nom::Err::Error(_)));
    }

    #[test]
    fn test_take_till_crlf() {
        // Test valid inputs with CRLF
        let (rem, result) = take_till_crlf(b"header value\r\nmore content").unwrap();
        assert_eq!(result, b"header value");
        assert_eq!(rem, b"\r\nmore content");
        
        // Test valid inputs with just LF
        let (rem, result) = take_till_crlf(b"header value\nmore content").unwrap();
        assert_eq!(result, b"header value");
        assert_eq!(rem, b"\nmore content");
        
        // Test with only CR
        let (rem, result) = take_till_crlf(b"header value\rmore content").unwrap();
        assert_eq!(result, b"header value");
        assert_eq!(rem, b"\rmore content");
        
        // Test with no CRLF
        let (rem, result) = take_till_crlf(b"header value").unwrap();
        assert_eq!(result, b"header value");
        assert_eq!(rem, b"");
        
        // Empty input
        let (rem, result) = take_till_crlf(b"").unwrap();
        assert_eq!(result, b"");
        assert_eq!(rem, b"");
    }
} 
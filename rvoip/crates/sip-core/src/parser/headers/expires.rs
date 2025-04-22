// Parser for Expires header (RFC 3261 Section 20.19)
// Expires = "Expires" HCOLON delta-seconds

use nom::IResult;

// Import delta_seconds parser
use crate::parser::values::delta_seconds;
use crate::parser::ParseResult;

// Expires = "Expires" HCOLON delta-seconds
// Note: HCOLON handled elsewhere
pub fn parse_expires(input: &[u8]) -> ParseResult<u32> {
    delta_seconds(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_expires() {
        let (rem, val) = parse_expires(b"3600").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 3600);

        let (rem_zero, val_zero) = parse_expires(b"0").unwrap();
        assert!(rem_zero.is_empty());
        assert_eq!(val_zero, 0);
    }

    #[test]
    fn test_invalid_expires() {
        assert!(parse_expires(b"").is_err());
        assert!(parse_expires(b"abc").is_err());
        assert!(parse_expires(b"-10").is_err());
        assert!(parse_expires(b"10.5").is_err()); // delta-seconds is integer
    }
} 
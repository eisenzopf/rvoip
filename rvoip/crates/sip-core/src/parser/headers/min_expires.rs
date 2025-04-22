// Parser for Min-Expires header (RFC 3261 Section 20.27)
// Min-Expires = "Min-Expires" HCOLON delta-seconds

use nom::IResult;

// Import delta_seconds parser
use crate::parser::values::delta_seconds;
use crate::parser::ParseResult;

// Min-Expires = "Min-Expires" HCOLON delta-seconds
// Note: HCOLON handled elsewhere
pub(crate) fn parse_min_expires(input: &[u8]) -> ParseResult<u32> {
    delta_seconds(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_min_expires() {
        let (rem, val) = parse_min_expires(b"60").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 60);
    }
} 
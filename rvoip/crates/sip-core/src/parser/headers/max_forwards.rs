// Parser for Max-Forwards header (RFC 3261 Section 20.24)
// Max-Forwards = "Max-Forwards" HCOLON 1*DIGIT

use nom::{
    character::complete::digit1,
    combinator::map_res,
    IResult,
};
use std::str;

use crate::parser::ParseResult;

// Max-Forwards = "Max-Forwards" HCOLON 1*DIGIT
// Note: HCOLON handled elsewhere
pub(crate) fn parse_max_forwards(input: &[u8]) -> ParseResult<u32> {
    map_res(
        digit1, 
        |bytes| {
            let s = str::from_utf8(bytes).map_err(|_| "Invalid UTF8")?;
            s.parse::<u32>().map_err(|_| "Invalid u32")
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_max_forwards() {
        let (rem, val) = parse_max_forwards(b"70").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 70);

        let (rem_zero, val_zero) = parse_max_forwards(b"0").unwrap();
        assert!(rem_zero.is_empty());
        assert_eq!(val_zero, 0);
    }

    #[test]
    fn test_invalid_max_forwards() {
        assert!(parse_max_forwards(b"").is_err());
        assert!(parse_max_forwards(b"abc").is_err());
        assert!(parse_max_forwards(b"-10").is_err()); // digit1 doesn't allow minus
    }
} 
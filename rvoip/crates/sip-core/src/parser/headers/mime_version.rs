// Parser for MIME-Version header (RFC 3261 Section 20.26)
// MIME-Version = "MIME-Version" HCOLON 1*DIGIT "." 1*DIGIT

use nom::{
    bytes::complete::tag,
    character::complete::digit1,
    combinator::map_res,
    sequence::{pair, separated_pair},
    IResult,
};
use std::str;

// Import from new modules
use crate::parser::separators::hcolon;
use crate::parser::ParseResult;

// mime-version-val = 1*DIGIT "." 1*DIGIT
fn mime_version_val(input: &[u8]) -> ParseResult<(u32, u32)> {
    map_res(
        separated_pair(digit1, tag("."), digit1),
        |(major_bytes, minor_bytes)| {
            let major = str::from_utf8(major_bytes)?.parse::<u32>()?;
            let minor = str::from_utf8(minor_bytes)?.parse::<u32>()?;
            Ok((major, minor))
        }
    )(input)
}

// MIME-Version = "MIME-Version" HCOLON 1*DIGIT "." 1*DIGIT
// Note: HCOLON handled elsewhere
pub(crate) fn parse_mime_version(input: &[u8]) -> ParseResult<(u32, u32)> {
    mime_version_val(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mime_version() {
        let (rem, val) = parse_mime_version(b"1.0").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, (1, 0));

        let (rem_multi, val_multi) = parse_mime_version(b"10.25").unwrap();
        assert!(rem_multi.is_empty());
        assert_eq!(val_multi, (10, 25));
    }
    
    #[test]
    fn test_invalid_mime_version() {
        assert!(parse_mime_version(b"1.").is_err());
        assert!(parse_mime_version(b".0").is_err());
        assert!(parse_mime_version(b"1").is_err());
        assert!(parse_mime_version(b"a.b").is_err());
    }
} 
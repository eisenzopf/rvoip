// Parser for MIME-Version header (RFC 3261 Section 20.26)
// MIME-Version = "MIME-Version" HCOLON 1*DIGIT "." 1*DIGIT

use nom::{
    bytes::complete as bytes,
    character::complete::{digit1},
    combinator::{map_res},
    sequence::{separated_pair, pair},
    IResult,
    error::{ErrorKind, NomError},
};
use std::str;

// Import from new modules
use crate::parser::separators::hcolon;
use crate::parser::ParseResult;
use crate::types::version::Version as MimeVersion; // Alias to avoid confusion
use crate::error::{Error, Result};

// mime-version-val = 1*DIGIT "." 1*DIGIT
fn mime_version_val(input: &[u8]) -> ParseResult<MimeVersion> {
    map_res(
        separated_pair(digit1, bytes::tag(b"."), digit1),
        |(major_bytes, minor_bytes)| {
            let major_str = str::from_utf8(major_bytes).map_err(|_| nom::Err::Failure(NomError::from_error_kind(major_bytes, ErrorKind::Char)))?;
            let minor_str = str::from_utf8(minor_bytes).map_err(|_| nom::Err::Failure(NomError::from_error_kind(minor_bytes, ErrorKind::Char)))?;
            let major = major_str.parse::<u32>().map_err(|_| nom::Err::Failure(NomError::from_error_kind(major_bytes, ErrorKind::Digit)))?;
            let minor = minor_str.parse::<u32>().map_err(|_| nom::Err::Failure(NomError::from_error_kind(minor_bytes, ErrorKind::Digit)))?;
            Ok(MimeVersion::new(major, minor))
        }
    )(input)
}

// MIME-Version = "MIME-Version" HCOLON 1*DIGIT "." 1*DIGIT
// Note: HCOLON handled elsewhere
pub(crate) fn parse_mime_version(input: &[u8]) -> ParseResult<MimeVersion> {
    map_res(
        separated_pair(digit1, bytes::tag(b"."), digit1),
        |(major_bytes, minor_bytes)| {
            let major_str = str::from_utf8(major_bytes)?;
            let minor_str = str::from_utf8(minor_bytes)?;
            let major = major_str.parse::<u8>()?;
            let minor = minor_str.parse::<u8>()?;
            Ok(MimeVersion::new(major, minor))
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mime_version() {
        let (rem, val) = parse_mime_version(b"1.0").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, MimeVersion::new(1, 0));

        let (rem_multi, val_multi) = parse_mime_version(b"10.25").unwrap();
        assert!(rem_multi.is_empty());
        assert_eq!(val_multi, MimeVersion::new(10, 25));
    }
    
    #[test]
    fn test_invalid_mime_version() {
        assert!(parse_mime_version(b"1.").is_err());
        assert!(parse_mime_version(b".0").is_err());
        assert!(parse_mime_version(b"1").is_err());
        assert!(parse_mime_version(b"a.b").is_err());
    }
} 
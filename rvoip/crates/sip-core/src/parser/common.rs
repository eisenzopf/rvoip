// Placeholder for common parsing utilities
use nom::{
    multi::separated_list0,
    sequence::preceded,
    IResult,
};
use std::str;
use super::separators::comma;

use nom::{
    bytes::complete::tag,
    character::complete::{digit1},
    combinator::{map_res, recognize},
    sequence::tuple,
};
use crate::types::Version;

// Type alias for parser result
pub(crate) type ParseResult<'a, O> = IResult<&'a [u8], O>;

/// Parses a comma-separated list of items using a provided item parser.
/// Handles optional whitespace around the commas.
/// Returns a Vec of the parsed items.
pub(crate) fn comma_separated_list0<'a, O, F>(item_parser: F) -> impl FnMut(&'a [u8]) -> ParseResult<Vec<O>> 
where
    F: FnMut(&'a [u8]) -> ParseResult<O> + Copy,
{
    separated_list0(
        comma, // Uses the comma parser which handles surrounding SWS
        item_parser,
    )
}

/// Parses a comma-separated list of items that must have at least one item.
/// Handles optional whitespace around the commas.
/// Returns a Vec of the parsed items.
pub(crate) fn comma_separated_list1<'a, O, F>(item_parser: F) -> impl FnMut(&'a [u8]) -> ParseResult<Vec<O>> 
where
    F: FnMut(&'a [u8]) -> ParseResult<O> + Copy,
{
    nom::multi::separated_list1(
        comma, // Uses the comma parser which handles surrounding SWS
        item_parser,
    )
}

// SIP-Version = "SIP" "/" 1*DIGIT "." 1*DIGIT
pub(crate) fn sip_version(input: &[u8]) -> ParseResult<Version> {
    map_res(
        recognize(
            tuple((
                tag("SIP"),
                tag("/"),
                digit1,
                tag("."),
                digit1,
            ))
        ),
        |bytes| {
            // Expect format like "SIP/2.0"
            let s = str::from_utf8(bytes)?;
            if let Some(parts) = s.strip_prefix("SIP/").and_then(|v| v.split_once('.')) {
                let major = parts.0.parse::<u8>()?;
                let minor = parts.1.parse::<u8>()?;
                Ok(Version::new(major, minor))
            } else {
                // This indicates a logic error in the parser if reached
                Err("Invalid SIP version format parsed") 
            }
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sip_version() {
        assert_eq!(sip_version(b"SIP/2.0"), Ok((&[][..], Version::new(2, 0))));
        assert_eq!(sip_version(b"SIP/1.10"), Ok((&[][..], Version::new(1, 10))));
        assert_eq!(sip_version(b"SIP/2.0 MoreData"), Ok((&b" MoreData"[..], Version::new(2, 0))));
        assert!(sip_version(b"SIP/2.").is_err());
        assert!(sip_version(b"SIP/A.0").is_err());
        assert!(sip_version(b"HTTP/1.1").is_err());
        assert!(sip_version(b"SIP/2/0").is_err());
    }
}

// Example usage (not actual code to be added here):
// fn parse_some_item(input: &[u8]) -> ParseResult<&[u8]> { token(input) }
// let mut parser = comma_separated_list1(parse_some_item);
// let result = parser(b"item1, item2 ,item3"); 
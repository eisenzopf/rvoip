// Parser for Organization header (RFC 3261 Section 20.27)
// Organization = "Organization" HCOLON [TEXT-UTF8-TRIM]

use nom::{
    bytes::complete::tag_no_case,
    combinator::{map, opt, map_res},
    sequence::{pair, preceded},
    IResult,
};

// Import from new modules
use crate::parser::separators::hcolon;
use crate::parser::values::text_utf8_trim;
use crate::parser::ParseResult;
use std::str;

// Returns Option<&[u8]> representing the trimmed text
pub(crate) fn parse_organization(input: &[u8]) -> ParseResult<String> {
    // text_utf8_trim expects at least one char if not empty.
    // If input is empty, this will correctly fail (as it should for optional field)
    // If header exists but value is empty (e.g., "Organization: \r\n"), input will be empty.
    map_res(text_utf8_trim, |bytes| str::from_utf8(bytes).map(String::from))(input)
    // Note: text_utf8_trim doesn't actually trim leading/trailing LWS currently.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_organization() {
        let input = b"Example Org";
        let (rem, val) = parse_organization(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "Example Org");

        // With internal LWS
        let input_lws = b"Some \t Company, Inc.";
        let (rem_lws, val_lws) = parse_organization(input_lws).unwrap();
        assert!(rem_lws.is_empty());
        assert_eq!(val_lws, "Some \t Company, Inc.");
    }

    #[test]
    fn test_parse_organization_empty() {
        // If the header value is truly empty after the colon, parse should fail
        // as text_utf8_trim requires 1*char
        let input = b""; 
        assert!(parse_organization(input).is_err());
    }
} 
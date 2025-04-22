// Parser for In-Reply-To header (RFC 3261 Section 20.22)
// In-Reply-To = "In-Reply-To" HCOLON callid *(COMMA callid)

use nom::{
    bytes::complete::tag_no_case,
    combinator::map,
    sequence::{pair, preceded},
    IResult,
};

// Import from new modules
use crate::parser::separators::{hcolon, comma};
use super::call_id::callid_parser; // Reuse callid parser logic
use crate::parser::common::comma_separated_list1;
use crate::parser::ParseResult;


// Return Vec<(local_part_bytes, Option<host_bytes>)> 
pub fn parse_in_reply_to(input: &[u8]) -> ParseResult<Vec<(&[u8], Option<&[u8]>)>> {
    preceded(
        pair(tag_no_case(b"In-Reply-To"), hcolon),
        comma_separated_list1(callid_parser) // Use the callid parser
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_in_reply_to() {
        let input = b"70710@saturn.bell-tel.com, 17320@saturn.bell-tel.com";
        let result = parse_in_reply_to(input);
        assert!(result.is_ok());
        let (rem, ids) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], "70710@saturn.bell-tel.com");
        assert_eq!(ids[1], "17320@saturn.bell-tel.com");
    }

    #[test]
    fn test_parse_in_reply_to_single() {
        let input = b"local-id";
        let result = parse_in_reply_to(input);
        assert!(result.is_ok());
        let (rem, ids) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ids, vec!["local-id"]);
    }

     #[test]
    fn test_parse_in_reply_to_empty_fail() {
        assert!(parse_in_reply_to(b"").is_err());
    }
} 
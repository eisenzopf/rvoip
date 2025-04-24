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
use super::call_id::callid; // Reuse callid parser logic
use crate::parser::common::comma_separated_list1;
use crate::parser::ParseResult;


// Return Vec<String> - each string is a call-id
pub fn parse_in_reply_to(input: &[u8]) -> ParseResult<Vec<String>> {
    preceded(
        pair(tag_no_case(b"In-Reply-To"), hcolon),
        comma_separated_list1(callid) // Use the callid parser
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_in_reply_to() {
        let input = b"In-Reply-To: 70710@saturn.bell-tel.com, 17320@saturn.bell-tel.com";
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
        let input = b"In-Reply-To: local-id";
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
    
    // Additional RFC compliance tests
    
    #[test]
    fn test_parse_in_reply_to_case_insensitive() {
        // Test for case insensitivity in header name (RFC 3261 Section 7.3.1)
        let input = b"in-reply-to: abc123@example.com";
        let result = parse_in_reply_to(input);
        assert!(result.is_ok());
        let (rem, ids) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "abc123@example.com");
    }
    
    #[test]
    fn test_parse_in_reply_to_whitespace() {
        // Test for whitespace handling around commas (RFC 3261 Section 7.3.1)
        let input = b"In-Reply-To: id1@domain.com   ,    id2@domain.com,id3@domain.com";
        let result = parse_in_reply_to(input);
        assert!(result.is_ok());
        let (rem, ids) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0], "id1@domain.com");
        assert_eq!(ids[1], "id2@domain.com");
        assert_eq!(ids[2], "id3@domain.com");
    }
    
    #[test]
    fn test_parse_in_reply_to_special_chars() {
        // Test for Call-ID with special characters (RFC 3261 Section 25)
        let input = b"In-Reply-To: abc123.!%*+-_`'~@example.com";
        let result = parse_in_reply_to(input);
        assert!(result.is_ok());
        let (rem, ids) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "abc123.!%*+-_`'~@example.com");
    }
    
    #[test]
    fn test_parse_in_reply_to_uuid_style() {
        // Test with UUID-style Call-IDs commonly used in SIP
        let input = b"In-Reply-To: f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com";
        let result = parse_in_reply_to(input);
        assert!(result.is_ok());
        let (rem, ids) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
    }
    
    #[test]
    fn test_parse_in_reply_to_multiple_complex() {
        // Test with multiple Call-IDs of varying complexity
        let input = b"In-Reply-To: simple-id, complex.id-with_special!chars@domain.com, f81d4fae-7dec-11d0-a765-00a0c91e6bf6";
        let result = parse_in_reply_to(input);
        assert!(result.is_ok());
        let (rem, ids) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0], "simple-id");
        assert_eq!(ids[1], "complex.id-with_special!chars@domain.com");
        assert_eq!(ids[2], "f81d4fae-7dec-11d0-a765-00a0c91e6bf6");
    }
    
    #[test]
    fn test_parse_in_reply_to_malformed() {
        // Test with malformed input - should fail
        assert!(parse_in_reply_to(b"In-Reply-To:").is_err()); // No Call-ID
        assert!(parse_in_reply_to(b"In-Reply-To: ,").is_err()); // Empty Call-ID between commas
        assert!(parse_in_reply_to(b"In-Reply-To").is_err()); // No colon
        assert!(parse_in_reply_to(b"Wrong-Header: id@domain").is_err()); // Wrong header name
    }
    
    #[test]
    fn test_parse_in_reply_to_trailing_content() {
        // Test with trailing content
        let input = b"In-Reply-To: id@domain.com;param=value";
        let result = parse_in_reply_to(input);
        assert!(result.is_ok());
        let (rem, ids) = result.unwrap();
        assert_eq!(rem, b";param=value");
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "id@domain.com");
    }
} 
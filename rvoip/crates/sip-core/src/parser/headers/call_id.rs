// Parser for Call-ID header (RFC 3261 Section 20.8)
// Call-ID = ( "Call-ID" / "i" ) HCOLON callid
// callid = word [ "@" word ]

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, tag},
    character::complete::char,
    combinator::{map, opt, map_res},
    sequence::{pair, preceded},
    IResult,
};

// Import from new modules
use crate::parser::separators::hcolon;
use crate::parser::token::word;
use crate::parser::ParseResult;
use std::str;
use crate::types::call_id::CallId; // Import the specific type

// callid = word [ "@" word ]
// Returns String representation
pub fn callid(input: &[u8]) -> ParseResult<String> {
    map_res(
        pair(word, opt(preceded(tag(b"@"), word))),
        |(word1, opt_word2)| {
            let s1 = str::from_utf8(word1)?;
            if let Some(word2) = opt_word2 {
                let s2 = str::from_utf8(word2)?;
                Ok::<String, std::str::Utf8Error>(format!("{}@{}", s1, s2))
            } else {
                Ok::<String, std::str::Utf8Error>(s1.to_string())
            }
        }
    )(input)
}

// Call-ID = ( "Call-ID" / "i" ) HCOLON callid
pub fn parse_call_id(input: &[u8]) -> ParseResult<CallId> { // Return CallId
    preceded(
        pair(
            alt((
                tag_no_case(b"Call-ID"),
                tag_no_case(b"i")
            )),
            hcolon
        ),
        map(
            callid,
            |call_id_str| CallId(call_id_str)
        )
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nom::error::ErrorKind;
    use nom::Err;

    // Test the callid function directly (without header name)
    #[test]
    fn test_callid_parser() {
        // Simple word
        let (rem, val) = callid(b"abcdef").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "abcdef");

        // Word with special characters
        let (rem, val) = callid(b"a.b-c_d!e%f*g+h`i'j~k").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "a.b-c_d!e%f*g+h`i'j~k");

        // Word with @ part
        let (rem, val) = callid(b"local@domain").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "local@domain");

        // Complex example with special chars on both sides of @
        let (rem, val) = callid(b"abc.123_!%*+-`'~@example.com").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "abc.123_!%*+-`'~@example.com");

        // Word includes all allowed special characters according to RFC 3261
        let (rem, val) = callid(b"a()<>:\"\\[]{}/?.~@b()<>:\"\\[]{}/?.~").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "a()<>:\"\\[]{}/?.~@b()<>:\"\\[]{}/?.~");
    }

    // Standard test cases for the complete Call-ID header parser
    #[test]
    fn test_parse_call_id_standard() {
        // UUID style Call-ID with domain
        let input = b"Call-ID: f81d4fae-7dec-11d0-a765-00a0c91e6bf6@foo.bar.com";
        let result = parse_call_id(input);
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(val.0, "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@foo.bar.com");

        // Short form header name
        let input = b"i: local-id-123";
        let result = parse_call_id(input);
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(val.0, "local-id-123");

        // Case insensitivity in header name
        let input = b"call-id: test456@example.com";
        let result = parse_call_id(input);
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(val.0, "test456@example.com");

        // Mixed case in header name
        let input = b"Call-Id: ABC123";
        let result = parse_call_id(input);
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(val.0, "ABC123");
    }

    #[test]
    fn test_parse_call_id_rfc_examples() {
        // Example from RFC 3261 Section 8.1.1.4
        let input = b"Call-ID: f81d4fae-7dec-11d0-a765-00a0c91e6bf6@biloxi.com";
        let result = parse_call_id(input);
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(val.0, "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@biloxi.com");

        // Similar to example from RFC 3261 Section 24.2
        let input = b"i: 70710@saturn.bell-tel.com";
        let result = parse_call_id(input);
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(val.0, "70710@saturn.bell-tel.com");
    }

    #[test]
    fn test_parse_call_id_with_complex_characters() {
        // Test with special characters defined in the word grammar
        let input = b"Call-ID: asd<.(!%*_+`'~)-:>\"/[]?{}=asd@example.com";
        let result = parse_call_id(input);
        assert!(result.is_ok(), "Failed to parse input with special characters");
        let (rem, val) = result.unwrap();
        
        // Debug output to see what's left in the remainder
        if !rem.is_empty() {
            println!("Remainder: {:?}", std::str::from_utf8(rem).unwrap_or("Invalid UTF-8"));
            println!("Parsed value: {}", val.0);
        }
        
        assert!(rem.is_empty(), "Parser didn't consume all input");
        assert_eq!(val.0, "asd<.(!%*_+`'~)-:>\"/[]?{}=asd@example.com");

        // Test with a simpler example that should definitely pass
        let input = b"Call-ID: simple-id@domain-with-special.chars";
        let result = parse_call_id(input);
        assert!(result.is_ok(), "Failed to parse simple-id@domain-with-special.chars");
        let (rem, val) = result.unwrap();
        
        // Debug output
        if !rem.is_empty() {
            println!("Simple example remainder: {:?}", std::str::from_utf8(rem).unwrap_or("Invalid UTF-8"));
        }
        
        assert!(rem.is_empty(), "Parser didn't consume all input for simple example");
        assert_eq!(val.0, "simple-id@domain-with-special.chars");
        
        // Try one more example with minimal special characters
        let input = b"Call-ID: test@domain";
        let result = parse_call_id(input);
        assert!(result.is_ok(), "Failed to parse test@domain");
        let (rem, val) = result.unwrap();
        
        // Debug output
        if !rem.is_empty() {
            println!("Basic example remainder: {:?}", std::str::from_utf8(rem).unwrap_or("Invalid UTF-8"));
        }
        
        assert!(rem.is_empty(), "Parser didn't consume all input for basic example");
        assert_eq!(val.0, "test@domain");
    }

    #[test]
    fn test_parse_call_id_whitespace_handling() {
        // Test with whitespace after header name
        let input = b"Call-ID:    f81d4fae-7dec-11d0-a765-00a0c91e6bf6@foo.bar.com";
        let result = parse_call_id(input);
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(val.0, "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@foo.bar.com");

        // Test with trailing whitespace
        let input = b"Call-ID: simple-id   ";
        let result = parse_call_id(input);
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert_eq!(rem, b"   ");
        assert_eq!(val.0, "simple-id");
    }

    #[test]
    fn test_parse_call_id_error_cases() {
        // Missing header name
        let input = b": f81d4fae-7dec-11d0-a765-00a0c91e6bf6@foo.bar.com";
        let result = parse_call_id(input);
        assert!(result.is_err());

        // Missing colon
        let input = b"Call-ID f81d4fae-7dec-11d0-a765-00a0c91e6bf6@foo.bar.com";
        let result = parse_call_id(input);
        assert!(result.is_err());

        // Empty call-id value
        let input = b"Call-ID: ";
        let result = parse_call_id(input);
        assert!(result.is_err());

        // Invalid characters in call-id (spaces not allowed in word)
        let input = b"Call-ID: invalid call id with spaces";
        let result = parse_call_id(input);
        assert!(result.is_err() || result.unwrap().0 != b"");

        // Double @ symbols not allowed
        let input = b"Call-ID: local@domain@another";
        let (rem, val) = parse_call_id(input).unwrap();
        assert_eq!(rem, b"@another");
        assert_eq!(val.0, "local@domain");
    }
} 
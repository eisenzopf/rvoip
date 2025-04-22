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
        pair(word, opt(preceded(tag("@"), word))),
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
        map_res(
            pair(word, opt(preceded(tag(b"@"), word))),
            |(word1, opt_word2)| -> Result<CallId, std::str::Utf8Error> {
                let s1 = str::from_utf8(word1)?;
                if let Some(word2) = opt_word2 {
                    let s2 = str::from_utf8(word2)?;
                    Ok(CallId(format!("{}@{}", s1, s2)))
                } else {
                    Ok(CallId(s1.to_string()))
                }
            }
        )
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_call_id_simple() {
        let input = b"Call-ID: f81d4fae-7dec-11d0-a765-00a0c91e6bf6@foo.bar.com";
        let result = parse_call_id(input);
        assert!(result.is_ok());
        let (rem, val) = result.unwrap(); // Returns CallId
        assert!(rem.is_empty());
        assert_eq!(val.0, "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@foo.bar.com"); // Access inner String
    }

    #[test]
    fn test_parse_call_id_short_form() {
        let input = b"i: local-id-123";
        let result = parse_call_id(input);
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(val.0, "local-id-123");
    }

    #[test]
    fn test_parse_call_id_complex_word() {
        // Example from RFC 3261
        let input = b"Call-ID: asd<.(!%*_+`'~)-:>\"/[]?{}=asd@example.com";
        let result = parse_call_id(input);
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(val.0, "asd<.(!%*_+`'~)-:>\"/[]?{}=asd@example.com");
    }
} 
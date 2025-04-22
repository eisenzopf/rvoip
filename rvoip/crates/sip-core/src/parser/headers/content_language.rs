// Parser for Content-Language header (RFC 3261 Section 20.14)
// Content-Language = "Content-Language" HCOLON language-tag *(COMMA language-tag)
// language-tag = primary-tag *( "-" subtag )
// primary-tag = 1*8ALPHA
// subtag = 1*8ALPHA

use nom::{
    bytes::complete::{tag, take_while_m_n},
    character::complete::alpha1,
    combinator::{map_res, recognize},
    multi::{many0, separated_list1},
    sequence::{pair, preceded},
    IResult,
};
use std::str;

// Import from new modules
use crate::parser::separators::{hcolon, comma};
use crate::parser::common_chars::alpha;
use crate::parser::common::comma_separated_list1;
use crate::parser::ParseResult;

// primary-tag = 1*8ALPHA
// subtag = 1*8ALPHA
fn lang_tag_part(input: &[u8]) -> ParseResult<&[u8]> {
    take_while_m_n(1, 8, |c: u8| c.is_ascii_alphabetic())(input)
}

// language-tag = primary-tag *( "-" subtag )
fn language_tag(input: &[u8]) -> ParseResult<String> {
    map_res(
        recognize(
            pair(
                lang_tag_part,
                many0(preceded(tag("-"), lang_tag_part))
            )
        ),
        |bytes| str::from_utf8(bytes).map(String::from)
    )(input)
}

// Parse the comma-separated list of language-tags
fn language_tag_list(input: &[u8]) -> ParseResult<Vec<String>> {
    comma_separated_list1(language_tag)(input)
}

pub fn parse_content_language(input: &[u8]) -> ParseResult<Vec<String>> {
    preceded(
        pair(tag(b"Content-Language"), hcolon),
        language_tag_list // Requires at least one
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_tag() {
        let (rem, tag_val) = language_tag(b"en").unwrap();
        assert!(rem.is_empty());
        assert_eq!(tag_val, "en");

        let (rem_sub, tag_sub) = language_tag(b"fr-ca-quebec").unwrap();
        assert!(rem_sub.is_empty());
        assert_eq!(tag_sub, "fr-ca-quebec");
        
        assert!(language_tag(b"toolongprimary").is_err());
        assert!(language_tag(b"en-toolongsubtag").is_err());
        assert!(language_tag(b"en-").is_err()); // Dangling hyphen
        assert!(language_tag(b"-en").is_err()); // Leading hyphen
    }

    #[test]
    fn test_parse_content_language() {
        let input = b"en, fr-CA";
        let result = parse_content_language(input);
        assert!(result.is_ok());
        let (rem, tags) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(tags, vec!["en".to_string(), "fr-CA".to_string()]);
    }
    
    #[test]
    fn test_parse_content_language_single() {
        let input = b"es";
        let result = parse_content_language(input);
        assert!(result.is_ok());
        let (rem, tags) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(tags, vec!["es".to_string()]);
    }

    #[test]
    fn test_parse_content_language_empty_fail() {
        // Header value cannot be empty according to ABNF (1*language-tag)
        let input = b"";
        let result = parse_content_language(input);
        assert!(result.is_err());
    }
} 
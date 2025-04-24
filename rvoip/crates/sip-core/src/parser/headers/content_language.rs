// Parser for Content-Language header (RFC 3261 Section 20.14)
// Content-Language = "Content-Language" HCOLON language-tag *(COMMA language-tag)
// language-tag = primary-tag *( "-" subtag )
// primary-tag = 1*8ALPHA
// subtag = 1*8ALPHA
//
// Note: The language-tag syntax follows RFC 3066/5646
// - Language tags are case-insensitive, but lowercase is preferred
// - The parser should normalize to lowercase
// - Underscores are NOT allowed per RFC 5646

use nom::{
    bytes::complete::{tag, tag_no_case, take_while1},
    character::complete::{alpha1, space0},
    combinator::{map, fail, recognize, verify},
    multi::{many0, separated_list1},
    sequence::{pair, preceded, delimited, tuple},
    IResult, error::ErrorKind, Err,
};
use std::str;

// Import from parser modules
use crate::parser::separators::{hcolon, comma};
use crate::parser::common::{comma_separated_list1};
use crate::parser::ParseResult;

// Define the LanguageTag struct
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct LanguageTag(pub String);

// Helper function to check if primary tag is valid (1-8 alpha characters)
fn is_valid_primary_tag(s: &[u8]) -> bool {
    s.len() >= 1 && s.len() <= 8 && s.iter().all(|&c| c.is_ascii_alphabetic())
}

// Helper function to check if subtag is valid (1-8 alphanumeric characters)
fn is_valid_subtag(subtag: &str) -> bool {
    if subtag.is_empty() {
        return false;
    }
    
    // RFC 5646 allows subtags to be alphanumeric
    // Check if all characters are alphanumeric and length is 1-8 characters
    subtag.len() <= 8 && subtag.chars().all(|c| c.is_ascii_alphanumeric())
}

// Parse a language tag, which consists of a primary tag and optional subtags
fn parse_language_tag(input: &[u8]) -> IResult<&[u8], LanguageTag> {
    // Check for empty input
    if input.is_empty() {
        return Err(Err::Error(nom::error::Error::new(input, ErrorKind::Alpha)));
    }
    
    // Check for leading or trailing hyphens
    if input[0] == b'-' || input[input.len() - 1] == b'-' {
        return Err(Err::Error(nom::error::Error::new(input, ErrorKind::Alpha)));
    }
    
    // Check for underscores in the entire input first
    if input.contains(&b'_') {
        return Err(Err::Error(nom::error::Error::new(input, ErrorKind::Alpha)));
    }
    
    // We need to handle byte input correctly for nom
    let (input, parts) = separated_list1(tag(b"-"), take_while1(|c: u8| c.is_ascii_alphanumeric()))(input)?;
    
    if parts.is_empty() {
        return Err(Err::Error(nom::error::Error::new(input, ErrorKind::Alpha)));
    }
    
    // Validate primary tag (1-8 alpha chars)
    let primary = parts[0];
    if primary.len() < 1 || primary.len() > 8 || !primary.iter().all(|&c| c.is_ascii_alphabetic()) {
        return Err(Err::Error(nom::error::Error::new(input, ErrorKind::Alpha)));
    }
    
    // Validate subtags (if any)
    for part in parts.iter().skip(1) {
        if part.len() < 1 || part.len() > 8 || !part.iter().all(|&c| c.is_ascii_alphanumeric()) {
            return Err(Err::Error(nom::error::Error::new(input, ErrorKind::Alpha)));
        }
    }
    
    let language_tag = parts
        .iter()
        .map(|part| std::str::from_utf8(part).unwrap().to_lowercase())
        .collect::<Vec<_>>()
        .join("-");
    
    Ok((input, LanguageTag(language_tag)))
}

// Add the wrapper function that is called in the tests
pub fn language_tag(input: &[u8]) -> IResult<&[u8], String> {
    let (rem, tag) = parse_language_tag(input)?;
    Ok((rem, tag.0))
}

// Parse a comma-separated list of language tags with proper whitespace handling
fn language_tag_list(input: &[u8]) -> ParseResult<Vec<String>> {
    // Reject empty input
    if input.is_empty() || input.iter().all(|&c| c.is_ascii_whitespace()) {
        return Err(Err::Error(nom::error::Error::new(input, ErrorKind::Tag)));
    }
    
    // Manually parse comma-separated language tags
    let mut result = Vec::new();
    let mut remaining = input;
    
    // Skip initial whitespace
    let mut idx = 0;
    while idx < remaining.len() && remaining[idx].is_ascii_whitespace() {
        idx += 1;
    }
    remaining = &remaining[idx..];
    
    // Parse tags
    while !remaining.is_empty() {
        // Parse a language tag
        let tag_end = remaining.iter()
            .position(|&c| c == b',' || c.is_ascii_whitespace())
            .unwrap_or(remaining.len());
        
        if tag_end == 0 {
            // Empty element - reject
            return Err(Err::Error(nom::error::Error::new(input, ErrorKind::Tag)));
        }
        
        let tag_input = &remaining[..tag_end];
        match parse_language_tag(tag_input) {
            Ok((_, tag)) => result.push(tag.0),
            Err(e) => return Err(e),
        }
        
        // Move past the tag
        remaining = &remaining[tag_end..];
        
        // Skip whitespace
        idx = 0;
        while idx < remaining.len() && remaining[idx].is_ascii_whitespace() {
            idx += 1;
        }
        remaining = &remaining[idx..];
        
        // If we hit a comma, move past it
        if !remaining.is_empty() && remaining[0] == b',' {
            remaining = &remaining[1..];
            
            // Skip whitespace after comma
            idx = 0;
            while idx < remaining.len() && remaining[idx].is_ascii_whitespace() {
                idx += 1;
            }
            remaining = &remaining[idx..];
            
            // Check for empty element
            if remaining.is_empty() || remaining[0] == b',' {
                return Err(Err::Error(nom::error::Error::new(input, ErrorKind::Tag)));
            }
        }
    }
    
    if result.is_empty() {
        return Err(Err::Error(nom::error::Error::new(input, ErrorKind::Tag)));
    }
    
    Ok((&b""[..], result))
}

// Parse Content-Language header
pub fn parse_content_language(input: &[u8]) -> ParseResult<Vec<String>> {
    // Skip header name and colon
    if let Some(idx) = input.windows(2).position(|w| w == b": " || w == b":") {
        let (header_part, value_part) = input.split_at(idx);
        
        // Verify header name
        if !header_part.eq_ignore_ascii_case(b"Content-Language") {
            return Err(Err::Error(nom::error::Error::new(input, ErrorKind::Tag)));
        }
        
        // Skip colon and space
        let value_start = if value_part.starts_with(b": ") { 2 } else { 1 };
        let value = &value_part[value_start..];
        
        // Parse language tags
        language_tag_list(value)
    } else {
        Err(Err::Error(nom::error::Error::new(input, ErrorKind::Tag)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_tag() {
        // Basic case
        let (rem, tag_val) = language_tag(b"en").unwrap();
        assert!(rem.is_empty());
        assert_eq!(tag_val, "en");

        // Multiple subtags
        let (rem_sub, tag_sub) = language_tag(b"fr-ca-quebec").unwrap();
        assert!(rem_sub.is_empty());
        assert_eq!(tag_sub, "fr-ca-quebec");
        
        // Case insensitivity (should be normalized to lowercase)
        let (_, tag_case) = language_tag(b"EN-US").unwrap();
        assert_eq!(tag_case, "en-us", "Language tags should be normalized to lowercase");
        
        // Edge of allowed length
        let (_, tag_max) = language_tag(b"abcdefgh-abcdefgh").unwrap();
        assert_eq!(tag_max, "abcdefgh-abcdefgh", "Should handle maximum allowed length");
        
        // Error cases
        assert!(language_tag(b"toolongprimarytag").is_err(), "Primary tag too long (>8 chars)");
        assert!(language_tag(b"en-toolongsubtag12").is_err(), "Subtag too long (>8 chars)");
        assert!(language_tag(b"en-").is_err(), "Dangling hyphen not allowed");
        assert!(language_tag(b"-en").is_err(), "Leading hyphen not allowed");
        assert!(language_tag(b"en_us").is_err(), "Underscores not allowed per RFC 5646");
        assert!(language_tag(b"1en").is_err(), "Must start with alpha characters");
    }

    #[test]
    fn test_parse_content_language() {
        // Standard case with multiple languages
        let input = b"Content-Language: en, fr-ca";
        let result = parse_content_language(input);
        assert!(result.is_ok());
        let (rem, tags) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(tags, vec!["en".to_string(), "fr-ca".to_string()]);
        
        // Case insensitivity in header name
        let input_case = b"content-language: en, fr-ca";
        let result_case = parse_content_language(input_case);
        assert!(result_case.is_ok(), "Header name should be case-insensitive");
        
        // Whitespace handling
        let input_ws = b"Content-Language:  en , fr-ca ";
        let result_ws = parse_content_language(input_ws);
        assert!(result_ws.is_ok(), "Should handle extra whitespace");
        let (_, tags_ws) = result_ws.unwrap();
        assert_eq!(tags_ws, vec!["en".to_string(), "fr-ca".to_string()]);
    }
    
    #[test]
    fn test_parse_content_language_single() {
        let input = b"Content-Language: es";
        let result = parse_content_language(input);
        assert!(result.is_ok());
        let (rem, tags) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(tags, vec!["es".to_string()]);
    }

    #[test]
    fn test_parse_content_language_empty_fail() {
        // Header value cannot be empty according to ABNF (1*language-tag)
        let input = b"Content-Language: ";
        let result = parse_content_language(input);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_rfc_examples() {
        // Examples from RFC 3066 and RFC 5646
        let rfc_examples = [
            "de",                // German
            "en-US",             // American English
            "zh-Hant",           // Chinese written using Traditional script
            "zh-Hans-CN",        // Simplified Chinese for mainland China
            "sr-Latn-RS",        // Serbian written using Latin script for Serbia
            "sl-IT-nedis",       // Slovenian, Nediš dialect, spoken in Italy
            "hy-Latn-IT-arevela" // Armenian written in Latin script, Western dialect, as used in Italy
        ];
        
        for example in rfc_examples {
            let result = language_tag(example.as_bytes());
            assert!(result.is_ok(), "RFC example '{}' should parse successfully", example);
            let (_, tag) = result.unwrap();
            assert_eq!(tag, example.to_lowercase(), "Should normalize to lowercase");
        }
    }
    
    #[test]
    fn test_complex_comma_separated() {
        // Test with multiple language tags in a header
        let input = b"Content-Language: en-US, fr-CA, zh-Hans-CN, de";
        let result = parse_content_language(input);
        assert!(result.is_ok());
        let (_, tags) = result.unwrap();
        assert_eq!(tags.len(), 4);
        assert_eq!(tags[0], "en-us");
        assert_eq!(tags[1], "fr-ca");
        assert_eq!(tags[2], "zh-hans-cn");
        assert_eq!(tags[3], "de");
    }
    
    #[test]
    fn test_abnf_compliance() {
        // Valid cases according to ABNF
        let valid_cases = [
            "en",                // Simple tag
            "en-US",             // With region
            "zh-Hant-HK",        // Multiple subtags
            "de-CH-1996",        // With variant
        ];
        
        for case in valid_cases {
            let result = language_tag(case.as_bytes());
            assert!(result.is_ok(), "Valid ABNF case '{}' should parse successfully", case);
        }
        
        // Invalid cases according to ABNF
        let invalid_cases = [
            "_invalid",           // Invalid character
            "en_US",              // Invalid separator (underscore instead of hyphen)
            "123",                // Numeric primary tag (should be alpha)
            "abcdefghi",          // Primary tag too long (>8 chars)
            "en-abcdefghi",       // Subtag too long (>8 chars)
            "-en",                // Leading hyphen
            "en-",                // Trailing hyphen
        ];
        
        for case in invalid_cases {
            let result = language_tag(case.as_bytes());
            assert!(result.is_err(), "Invalid ABNF case '{}' should be rejected", case);
        }
    }
    
    #[test]
    fn test_malformed_inputs() {
        // Test with malformed inputs that should be rejected
        assert!(parse_content_language(b"Content-Language:").is_err(), 
                "Empty value should be rejected");
                
        // Invalid comma usage
        assert!(parse_content_language(b"Content-Language: ,").is_err(), 
                "Empty elements should be rejected");
        assert!(parse_content_language(b"Content-Language: en,,fr").is_err(), 
                "Empty elements should be rejected");
        assert!(parse_content_language(b"Content-Language: en, ,fr").is_err(), 
                "Empty elements should be rejected");
        assert!(parse_content_language(b"Content-Language: en,").is_err(), 
                "Trailing comma should be rejected");
        
        // Incorrect header
        assert!(parse_content_language(b"Content-Languages: en").is_err(), 
                "Incorrect header name");
                
        // Missing colon
        assert!(parse_content_language(b"Content-Language en").is_err(), 
                "Missing colon");
    }
} 
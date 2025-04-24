// Parser for the Accept-Language header (RFC 3261 Section 20.3, RFC 3066/5646)
// Accept-Language = "Accept-Language" HCOLON [ language *(COMMA language) ]
// language = language-range *(SEMI accept-param)
// language-range = ( ( 1*8ALPHA *( "-" 1*8ALPHA ) ) / "*" )
// accept-param = ("q" EQUAL qvalue) / generic-param
//
// This uses RFC 5646 (formerly RFC 3066) language tag format which specifies:
// - Primary subtag: 1-8 ASCII alphabetic characters
// - Subsequent subtags: 1-8 ASCII alphanumeric characters
// - Subtags separated by hyphens only (no underscores)
// - Language tags are case-insensitive (though lowercase is preferred)
//
// RFC 5646 Extended Structure:
// - Extended language subtags: 3 alphabetic characters, up to 3 subtags
// - Script subtags: 4 alphabetic characters 
// - Region subtags: 2 alphabetic or 3 digit characters
// - Variant subtags: 5-8 alphanumeric or 4 alphanumeric if starts with digit
// - Extensions: single character followed by subtags
// - Private use: 'x' followed by subtags

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_while_m_n},
    character::complete::alpha1,
    combinator::{map, opt, recognize, value, map_res, verify},
    multi::{many0, separated_list0, separated_list1},
    sequence::{pair, preceded, delimited},
    IResult,
};
use std::str;
use std::collections::HashMap;
use ordered_float::NotNan;
use serde::{Serialize, Deserialize};
use std::cmp::Ordering;
use std::fmt;

// Import from base parser modules
use crate::parser::separators::{hcolon, semi, comma, equal};
use crate::parser::common_chars::alpha;
use crate::parser::token::token;
use crate::parser::common_params::accept_param; // Reuses generic_param, qvalue
use crate::parser::common::comma_separated_list0;
use crate::parser::ParseResult;

use crate::types::param::Param;
// use crate::types::accept_language::AcceptLanguage as AcceptLanguageHeader; // Removed - Unused import, type not found

// Define LanguageInfo locally and make it public
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageInfo {
    pub range: String,
    pub q: Option<NotNan<f32>>,
    pub params: Vec<Param>,
}

impl LanguageInfo {
    // Get effective q-value (defaults to 1.0 if not specified)
    pub fn q_value(&self) -> f32 {
        self.q.map_or(1.0, |q| q.into_inner())
    }
    
    // Compare language tags in a case-insensitive manner per RFC 3066/5646
    pub fn language_equals(&self, other: &str) -> bool {
        self.range.eq_ignore_ascii_case(other)
    }
}

// Implementation to enable sorting languages by q-value (highest first)
impl PartialOrd for LanguageInfo {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        other.q_value().partial_cmp(&self.q_value())
    }
}

impl Ord for LanguageInfo {
    fn cmp(&self, other: &Self) -> Ordering {
        // Sort by q-value (highest first), then by range string for stable ordering
        other.q_value().partial_cmp(&self.q_value())
            .unwrap_or(Ordering::Equal)
            .then_with(|| self.range.cmp(&other.range))
    }
}

// Display implementation for language info
impl fmt::Display for LanguageInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.range)?;
        
        // Add q-value if present
        if let Some(q) = self.q {
            write!(f, ";q={:.3}", q)?;
        }
        
        // Add other parameters - Param already includes semicolons, so we don't add them here
        for param in &self.params {
            write!(f, "{}", param)?;
        }
        
        Ok(())
    }
}

// Ensures the input doesn't contain underscores (to be RFC compliant)
fn no_underscore(input: &[u8]) -> bool {
    !input.contains(&b'_')
}

// primary-tag = 1*8ALPHA
// Per RFC 5646, the primary tag must be alphabetic
fn primary_tag_part(input: &[u8]) -> ParseResult<&[u8]> {
    verify(
        take_while_m_n(1, 8, |c: u8| c.is_ascii_alphabetic()),
        no_underscore // Explicitly disallow underscores
    )(input)
}

// subtag = 1*8ALPHANUM
// Per RFC 5646, subtags can be alphanumeric
fn subtag_part(input: &[u8]) -> ParseResult<&[u8]> {
    verify(
        take_while_m_n(1, 8, |c: u8| c.is_ascii_alphanumeric()),
        no_underscore // Explicitly disallow underscores
    )(input)
}

// Extended language subtag: 3 ALPHA characters per RFC 5646
fn ext_lang_subtag(input: &[u8]) -> ParseResult<&[u8]> {
    verify(
        take_while_m_n(3, 3, |c: u8| c.is_ascii_alphabetic()),
        no_underscore
    )(input)
}

// Script subtag: 4 ALPHA characters per RFC 5646
fn script_subtag(input: &[u8]) -> ParseResult<&[u8]> {
    verify(
        take_while_m_n(4, 4, |c: u8| c.is_ascii_alphabetic()),
        no_underscore
    )(input)
}

// Region subtag: 2 ALPHA or 3 DIGIT per RFC 5646
fn region_subtag(input: &[u8]) -> ParseResult<&[u8]> {
    verify(
        alt((
            take_while_m_n(2, 2, |c: u8| c.is_ascii_alphabetic()),
            take_while_m_n(3, 3, |c: u8| c.is_ascii_digit())
        )),
        no_underscore
    )(input)
}

// Variant subtag: 5-8 alphanum or 4 if starts with digit
fn variant_subtag(input: &[u8]) -> ParseResult<&[u8]> {
    verify(
        alt((
            take_while_m_n(5, 8, |c: u8| c.is_ascii_alphanumeric()),
            verify(
                take_while_m_n(4, 4, |c: u8| c.is_ascii_alphanumeric()),
                |s: &[u8]| s.len() > 0 && s[0].is_ascii_digit()
            )
        )),
        no_underscore
    )(input)
}

// language-range = ( ( 1*8ALPHA *( "-" 1*8ALPHA ) ) / "*" )
// Returns range as String (converted to lowercase as per RFC 5646)
fn language_range(input: &[u8]) -> ParseResult<String> {
    // Reject inputs containing underscores immediately
    if input.contains(&b'_') {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Alpha
        )));
    }

    alt((
        // Regular language tag
        map_res(
            recognize(
                pair(
                    primary_tag_part, 
                    many0(preceded(tag(b"-"), subtag_part))
                )
            ),
            |bytes: &[u8]| {
                // Check if the primary tag (before any hyphen) is too long
                let primary_end = bytes.iter().position(|&c| c == b'-').unwrap_or(bytes.len());
                if primary_end > 8 {
                    return Err(nom::Err::Error(nom::error::Error::new(
                        bytes,
                        nom::error::ErrorKind::TooLarge
                    )));
                }
                
                // Check if any subtag is too long
                if bytes.contains(&b'-') {
                    let parts: Vec<&[u8]> = bytes.split(|&c| c == b'-').collect();
                    for part in parts.iter().skip(1) { // Skip primary tag, already checked
                        if part.len() > 8 {
                            return Err(nom::Err::Error(nom::error::Error::new(
                                bytes,
                                nom::error::ErrorKind::TooLarge
                            )));
                        }
                    }
                }
                
                Ok(String::from_utf8_lossy(bytes).to_string().to_lowercase())
            }
        ),
        // RFC 5646 grandfathered tags (rare, but preserved for compatibility)
        map(
            alt((
                tag_no_case(b"i-ami"),
                tag_no_case(b"i-bnn"),
                tag_no_case(b"i-default"),
                tag_no_case(b"i-enochian"),
                tag_no_case(b"i-hak"),
                tag_no_case(b"i-klingon"),
                tag_no_case(b"i-lux"),
                tag_no_case(b"i-mingo"),
                tag_no_case(b"i-navajo"),
                tag_no_case(b"i-pwn"),
                tag_no_case(b"i-tao"),
                tag_no_case(b"i-tay"),
                tag_no_case(b"i-tsu"),
                tag_no_case(b"sgn-be-fr"),
                tag_no_case(b"sgn-be-nl"),
                tag_no_case(b"sgn-ch-de")
            )),
            |bytes| String::from_utf8_lossy(bytes).to_string().to_lowercase()
        ),
        // Wildcard
        map(
            tag(b"*"),
            |_| "*".to_string()
        )
    ))(input)
}

// Validate q-value according to RFC specifications:
// - Must be between 0.0 and 1.0 inclusive
// - Should have at most 3 decimal places
// This function normalizes and validates the q-value
fn validate_qvalue(q: NotNan<f32>) -> Option<NotNan<f32>> {
    let q_val = q.into_inner();
    
    // Check range
    if q_val < 0.0 || q_val > 1.0 {
        return None;
    }
    
    // Round to 3 decimal places to enforce the RFC limit
    let rounded = (q_val * 1000.0).round() / 1000.0;
    NotNan::new(rounded).ok()
}

// language = language-range *(SEMI accept-param)
// Returns LanguageInfo { range: String, q: Option<NotNan<f32>>, params: Vec<Param> }
fn language(input: &[u8]) -> ParseResult<LanguageInfo> {
    map(
        pair(
            language_range,
            many0(preceded(semi, accept_param))
        ),
        |(range_str, raw_params)| { 
            // Find and extract q parameter if present
            let mut q_value = None;
            let mut other_params = Vec::new();

            for param in raw_params {
                match param {
                    Param::Q(q) => {
                        // Validate and normalize q-value
                        q_value = validate_qvalue(q).map(Some).unwrap_or(None);
                    }
                    other => other_params.push(other),
                }
            }

            LanguageInfo { 
                range: range_str, 
                q: q_value, 
                params: other_params 
            }
        }
    )(input)
}

// Define structure for Accept-Language value
#[derive(Debug, PartialEq, Clone)]
pub struct AcceptLanguageValue {
    pub language_range: String,
    pub q: Option<NotNan<f32>>,
    pub params: HashMap<String, String>,
}

// Accept-Language = "Accept-Language" HCOLON [ language *(COMMA language) ]
pub fn parse_accept_language(input: &[u8]) -> ParseResult<Vec<LanguageInfo>> {
    // First parse the header name and HCOLON (case-insensitive)
    preceded(
        pair(tag_no_case(b"Accept-Language"), hcolon),
        // Then parse the optional list of languages
        opt(comma_separated_list0(language))
    )(input)
    .map(|(rem, langs_opt)| {
        let mut langs = langs_opt.unwrap_or_else(Vec::new);
        // Sort languages by q-value (highest first) per RFC 2616
        langs.sort();
        (rem, langs)
    })
}

// Test-only function that directly parses language list content without header name
#[cfg(test)]
pub(crate) fn parse_languages(input: &[u8]) -> ParseResult<Vec<LanguageInfo>> {
    comma_separated_list0(language)(input)
    .map(|(rem, mut langs)| {
        // Sort languages by q-value (highest first) per RFC 2616
        langs.sort();
        (rem, langs)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::param::{Param, GenericValue};
    use ordered_float::NotNan;

    #[test]
    fn test_language_range() {
        // Basic language ranges
        let (rem, range) = language_range(b"en-us").unwrap();
        assert!(rem.is_empty());
        assert_eq!(range, "en-us"); // Should be lowercase

        // Case insensitivity test - output should be lowercase
        let (_, range_upper) = language_range(b"EN-US").unwrap();
        assert_eq!(range_upper, "en-us");

        // Single language tag
        let (rem_single, range_single) = language_range(b"fr").unwrap();
        assert!(rem_single.is_empty());
        assert_eq!(range_single, "fr");

        // Wildcard
        let (rem_wild, range_wild) = language_range(b"*").unwrap();
        assert!(rem_wild.is_empty());
        assert_eq!(range_wild, "*");
        
        // Alphanumeric subtags (RFC 5646 compliant)
        let (_, range_alphanum) = language_range(b"en-gb2").unwrap();
        assert_eq!(range_alphanum, "en-gb2", "Should accept alphanumeric subtags");
        
        // Invalid cases
        assert!(language_range(b"1234").is_err(), "Should reject non-alphabetic primary tags");
        
        // Underscore handling check (RFC only allows hyphens)
        let underscore_result = language_range(b"en_us");
        assert!(underscore_result.is_err(), "Should reject underscores as per RFC");
        
        // Tag length validation - the parser should take only the first 8 characters and leave the rest
        let long_tag_result = language_range(b"abcdefghi");
        println!("Long primary tag result: {:?}", long_tag_result);
        
        // The parser should succeed but only consume the first 8 characters
        assert!(long_tag_result.is_ok(), "Parser should succeed with a valid prefix");
        let (remainder, value) = long_tag_result.unwrap();
        assert_eq!(value, "abcdefgh", "Should only take the first 8 characters");
        assert_eq!(remainder, &b"i"[..], "Should leave the 9th character as remainder");
        
        // Similarly for subtags
        let long_subtag_result = language_range(b"en-abcdefghi");
        println!("Long subtag result: {:?}", long_subtag_result);
        
        // The parser should succeed but only consume characters up to the valid part
        assert!(long_subtag_result.is_ok(), "Parser should succeed with a valid prefix");
        let (remainder, value) = long_subtag_result.unwrap();
        assert_eq!(value, "en-abcdefgh", "Should only take a valid subtag length");
        assert_eq!(remainder, &b"i"[..], "Should leave the extra character as remainder");
        
        // RFC 5646 grandfathered tags
        let (_, grand_tag) = language_range(b"i-navajo").unwrap();
        assert_eq!(grand_tag, "i-navajo", "Should accept RFC 5646 grandfathered tags");
        
        // Test for extended language subtags (RFC 5646)
        let complex_tag = b"zh-yue-HK"; // Cantonese as spoken in Hong Kong
        let (_, complex_value) = language_range(complex_tag).unwrap();
        assert_eq!(complex_value, "zh-yue-hk", "Should handle extended language subtags");
        
        // Correct subtag parsing
        let (_, range_multi) = language_range(b"zh-hans-cn").unwrap();
        assert_eq!(range_multi, "zh-hans-cn", "Should handle multiple subtags");
    }
    
    #[test]
    fn test_qvalue_validation() {
        // Valid q-values
        assert_eq!(validate_qvalue(NotNan::new(0.0).unwrap()), Some(NotNan::new(0.0).unwrap()));
        assert_eq!(validate_qvalue(NotNan::new(1.0).unwrap()), Some(NotNan::new(1.0).unwrap()));
        assert_eq!(validate_qvalue(NotNan::new(0.5).unwrap()), Some(NotNan::new(0.5).unwrap()));
        
        // Invalid q-values
        assert_eq!(validate_qvalue(NotNan::new(-0.1).unwrap()), None);
        assert_eq!(validate_qvalue(NotNan::new(1.1).unwrap()), None);
        
        // Rounding to 3 decimal places
        assert_eq!(
            validate_qvalue(NotNan::new(0.12345).unwrap()),
            Some(NotNan::new(0.123).unwrap()),
            "Should round to 3 decimal places"
        );
    }
    
    #[test]
    fn test_rfc5646_extensions() {
        // Test script subtag (4 alphabetic chars)
        let script_tag = b"zh-Hant"; // Chinese written in Traditional script
        let (_, script_value) = language_range(script_tag).unwrap();
        assert_eq!(script_value, "zh-hant", "Should handle script subtags");
        
        // Test region subtag (2 alpha or 3 digit)
        let region_alpha_tag = b"en-US"; // English as used in the United States
        let (_, region_alpha_value) = language_range(region_alpha_tag).unwrap();
        assert_eq!(region_alpha_value, "en-us", "Should handle region subtags (alpha)");
        
        // Test variant subtag
        let variant_tag = b"sl-rozaj-biske"; // Resian dialect of Slovene, Biscotarian variety
        let (_, variant_value) = language_range(variant_tag).unwrap();
        assert_eq!(variant_value, "sl-rozaj-biske", "Should handle variant subtags");
        
        // Test private use
        let private_tag = b"x-private"; 
        let (_, private_value) = language_range(private_tag).unwrap();
        assert_eq!(private_value, "x-private", "Should handle private use tags");
    }
    
    #[test]
    fn test_language() {
        // Language with q-value
        let (rem, lang) = language(b"da;q=1.0").unwrap();
        assert!(rem.is_empty());
        assert_eq!(lang.range, "da");
        assert_eq!(lang.q, Some(NotNan::new(1.0).unwrap()));
        assert_eq!(lang.params.len(), 0); // q should be extracted to the q field
        
        // Language with no parameters
        let (rem_no_param, lang_no_param) = language(b"en-gb").unwrap();
        assert!(rem_no_param.is_empty());
        assert_eq!(lang_no_param.range, "en-gb");
        assert_eq!(lang_no_param.q, None);
        assert!(lang_no_param.params.is_empty());
        
        // Language with q-value and other param
        let (rem_multi, lang_multi) = language(b"fr;q=0.5;custom=value").unwrap();
        assert!(rem_multi.is_empty());
        assert_eq!(lang_multi.range, "fr");
        assert_eq!(lang_multi.q, Some(NotNan::new(0.5).unwrap()));
        assert_eq!(lang_multi.params.len(), 1);
        assert!(matches!(&lang_multi.params[0], 
                         Param::Other(name, Some(GenericValue::Token(val))) 
                         if name == "custom" && val == "value"));
        
        // Language with invalid q-value should have q=None
        let (_, lang_invalid_q) = language(b"fr;q=1.001").unwrap();
        assert_eq!(lang_invalid_q.q, None, "Invalid q-value should be treated as None");
                         
        // Test with malformed input - missing range
        assert!(language(b";q=0.8").is_err());
        
        // Case insensitivity
        let (_, lang_upper) = language(b"EN-GB;Q=0.8").unwrap();
        assert_eq!(lang_upper.range, "en-gb", "Language range should be lowercase");
        assert_eq!(lang_upper.q, Some(NotNan::new(0.8).unwrap()), "Q param should be case insensitive");
    }
    
    #[test]
    fn test_language_sorting() {
        // Create languages with different q-values
        let lang1 = LanguageInfo {
            range: "en".to_string(),
            q: Some(NotNan::new(0.5).unwrap()),
            params: vec![],
        };
        
        let lang2 = LanguageInfo {
            range: "fr".to_string(),
            q: Some(NotNan::new(0.8).unwrap()),
            params: vec![],
        };
        
        let lang3 = LanguageInfo {
            range: "de".to_string(),
            q: None, // Default q=1.0
            params: vec![],
        };
        
        // Test sorting (should be de, fr, en based on q-values)
        let mut langs = vec![lang1.clone(), lang2.clone(), lang3.clone()];
        langs.sort();
        
        assert_eq!(langs[0].range, "de", "Default q=1.0 should be first");
        assert_eq!(langs[1].range, "fr", "q=0.8 should be second");
        assert_eq!(langs[2].range, "en", "q=0.5 should be last");
        
        // Test q_value method
        assert_eq!(lang1.q_value(), 0.5);
        assert_eq!(lang2.q_value(), 0.8);
        assert_eq!(lang3.q_value(), 1.0, "Missing q defaults to 1.0");
        
        // Test language_equals method (case insensitive)
        assert!(lang1.language_equals("EN"), "Should match case-insensitively");
        assert!(!lang1.language_equals("fr"), "Should not match different language");
    }

    #[test]
    fn test_parse_languages() {
        // Multiple languages with q-values (should be sorted by q-value)
        let input = b"en;q=0.7, da, en-gb;q=0.8, *;q=0.1";
        let result = parse_languages(input);
        assert!(result.is_ok());
        let (rem, languages) = result.unwrap();
        
        // Languages should be sorted by q-value: da (q=1.0), en-gb (q=0.8), en (q=0.7), * (q=0.1)
        assert_eq!(languages.len(), 4);
        assert_eq!(languages[0].range, "da", "Default q=1.0 should be first");
        assert_eq!(languages[1].range, "en-gb", "q=0.8 should be second");
        assert_eq!(languages[2].range, "en", "q=0.7 should be third");
        assert_eq!(languages[3].range, "*", "q=0.1 should be last");
        
        // Empty list
        let empty_input = b"";
        let empty_result = parse_languages(empty_input);
        assert!(empty_result.is_ok());
        let (_, empty_languages) = empty_result.unwrap();
        assert!(empty_languages.is_empty());
        
        // Single language
        let single_input = b"en";
        let single_result = parse_languages(single_input);
        assert!(single_result.is_ok());
        let (_, single_languages) = single_result.unwrap();
        assert_eq!(single_languages.len(), 1);
        assert_eq!(single_languages[0].range, "en");
    }
    
    #[test]
    fn test_parse_accept_language() {
        // Test with full header syntax
        let input = b"Accept-Language: en;q=0.7, da, en-gb;q=0.8";
        let result = parse_accept_language(input);
        assert!(result.is_ok());
        let (rem, languages) = result.unwrap();
        assert!(rem.is_empty());
        
        // Languages should be sorted by q-value
        assert_eq!(languages.len(), 3);
        assert_eq!(languages[0].range, "da", "Default q=1.0 should be first");
        assert_eq!(languages[1].range, "en-gb", "q=0.8 should be second");
        assert_eq!(languages[2].range, "en", "q=0.7 should be third");
        
        // Check case insensitivity
        let input_with_case = b"accept-language: da, en-gb;q=0.8";
        let result_with_case = parse_accept_language(input_with_case);
        assert!(result_with_case.is_ok(), "Case insensitive header parsing should work"); 
        
        // Empty accept-language header
        let empty_header = b"Accept-Language: ";
        let empty_result = parse_accept_language(empty_header);
        assert!(empty_result.is_ok());
        let (_, empty_languages) = empty_result.unwrap();
        assert!(empty_languages.is_empty());
    }
    
    #[test]
    fn test_display_implementation() {
        // Test Display implementation
        let en = LanguageInfo {
            range: "en-us".to_string(),
            q: Some(NotNan::new(0.8).unwrap()),
            params: vec![Param::Other("custom".to_string(), Some(GenericValue::Token("value".to_string())))],
        };
        
        assert_eq!(
            en.to_string(), 
            "en-us;q=0.800;custom=value", 
            "Should format with q-value and params"
        );
        
        // Without q-value
        let lang_no_q = LanguageInfo {
            range: "fr".to_string(),
            q: None,
            params: vec![],
        };
        
        assert_eq!(lang_no_q.to_string(), "fr", "Should format without q-value");
    }
    
    #[test]
    fn test_rfc_examples() {
        // From RFC 2616 Section 14.4 examples:
        let example1 = b"Accept-Language: da, en-gb;q=0.8, en;q=0.7";
        let result1 = parse_accept_language(example1);
        assert!(result1.is_ok());
        let (_, languages1) = result1.unwrap();
        
        // Should be sorted by q-value
        assert_eq!(languages1.len(), 3);
        assert_eq!(languages1[0].range, "da", "Default q=1.0 should be first");
        assert_eq!(languages1[1].range, "en-gb", "q=0.8 should be second");
        assert_eq!(languages1[2].range, "en", "q=0.7 should be third");
        
        // Example with wildcard
        let example2 = b"Accept-Language: en-us, en;q=0.5, *;q=0.1";
        let result2 = parse_accept_language(example2);
        assert!(result2.is_ok());
        let (_, languages2) = result2.unwrap();
        
        assert_eq!(languages2.len(), 3);
        assert_eq!(languages2[0].range, "en-us", "Default q=1.0 should be first");
        assert_eq!(languages2[1].range, "en", "q=0.5 should be second");
        assert_eq!(languages2[2].range, "*", "q=0.1 should be last (wildcard)");
    }
} 
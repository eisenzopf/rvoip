// Parser for the Accept-Language header (RFC 3261 Section 20.3)
// Accept-Language = "Accept-Language" HCOLON [ language *(COMMA language) ]
// language = language-range *(SEMI accept-param)
// language-range = ( ( 1*8ALPHA *( "-" 1*8ALPHA ) ) / "*" )
// accept-param = ("q" EQUAL qvalue) / generic-param

use nom::{
    branch::alt,
    bytes::complete::{tag, take_while_m_n},
    character::complete::alpha1,
    combinator::{map, opt, recognize, value},
    multi::{many0, separated_list0, separated_list1},
    sequence::{pair, preceded},
    IResult,
};
use std::str;
use std::collections::HashMap;
use ordered_float::NotNan;

// Import from base parser modules
use crate::parser::separators::{hcolon, semi, comma, equal};
use crate::parser::common_chars::alpha;
use crate::parser::token::token;
use crate::parser::common_params::accept_param; // Reuses generic_param, qvalue
use crate::parser::common::comma_separated_list0;
use crate::parser::ParseResult;

use crate::types::param::Param;
use crate::types::accept::LanguageInfo; // Assuming struct { range: String, params: Vec<Param> }

// primary-tag = 1*8ALPHA
// subtag = 1*8ALPHA
fn language_tag_part(input: &[u8]) -> ParseResult<&[u8]> {
    take_while_m_n(1, 8, |c: u8| c.is_ascii_alphabetic())(input)
}

// language-range = ( ( 1*8ALPHA *( "-" 1*8ALPHA ) ) / "*" )
// Returns range as String
fn language_range(input: &[u8]) -> ParseResult<String> {
    map(
        alt((
            recognize(
                pair(
                    language_tag_part, 
                    many0(preceded(tag("-"), language_tag_part))
                )
            ),
            tag("*")
        )),
        |bytes| String::from_utf8_lossy(bytes).to_string()
    )(input)
}

// language = language-range *(SEMI accept-param)
// Returns LanguageInfo { range: String, params: Vec<Param> }
fn language(input: &[u8]) -> ParseResult<LanguageInfo> {
    map(
        pair(
            language_range,
            many0(preceded(semi, accept_param))
        ),
        |(range_str, params_vec)| LanguageInfo { range: range_str, params: params_vec }
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
pub(crate) fn parse_accept_language(input: &[u8]) -> ParseResult<Vec<LanguageInfo>> {
    comma_separated_list0(language)(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::param::{Param, GenericValue};
    use ordered_float::NotNan;

    #[test]
    fn test_language_range() {
        let (rem, range) = language_range(b"en-us").unwrap();
        assert!(rem.is_empty());
        assert_eq!(range, "en-us");

        let (rem_single, range_single) = language_range(b"fr").unwrap();
        assert!(rem_single.is_empty());
        assert_eq!(range_single, "fr");

        let (rem_wild, range_wild) = language_range(b"*").unwrap();
        assert!(rem_wild.is_empty());
        assert_eq!(range_wild, "*");
        
        assert!(language_range(b"en-us-long").is_err()); // subtag too long
        assert!(language_range(b"toolongprimary").is_err()); // primary too long
    }
    
     #[test]
    fn test_language() {
        let (rem, lang) = language(b"da;q=1.0").unwrap();
        assert!(rem.is_empty());
        assert_eq!(lang.range, "da");
        assert_eq!(lang.params.len(), 1);
        assert!(lang.params.contains(&Param::Q(NotNan::new(1.0).unwrap())));

        let (rem_no_param, lang_no_param) = language(b"en-gb").unwrap();
        assert!(rem_no_param.is_empty());
        assert_eq!(lang_no_param.range, "en-gb");
        assert!(lang_no_param.params.is_empty());
    }

    #[test]
    fn test_parse_accept_language() {
        let input = b"da, en-gb;q=0.8, en;q=0.7, *;q=0.1";
        let result = parse_accept_language(input);
        assert!(result.is_ok());
        let (rem, languages) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(languages.len(), 4);
        
        assert_eq!(languages[0].range, "da");
        assert!(languages[0].params.is_empty());

        assert_eq!(languages[1].range, "en-gb");
        assert!(matches!(languages[1].params[0], Param::Q(q) if q == NotNan::new(0.8).unwrap()));

        assert_eq!(languages[2].range, "en");
         assert!(matches!(languages[2].params[0], Param::Q(q) if q == NotNan::new(0.7).unwrap()));

        assert_eq!(languages[3].range, "*");
        assert!(matches!(languages[3].params[0], Param::Q(q) if q == NotNan::new(0.1).unwrap()));
    }
    
     #[test]
    fn test_parse_accept_language_empty() {
        let input = b""; // Empty value allowed
        let result = parse_accept_language(input);
        assert!(result.is_ok());
        let (rem, languages) = result.unwrap();
        assert!(rem.is_empty());
        assert!(languages.is_empty());
    }
} 
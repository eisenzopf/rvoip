// Parser for the Content-Type header (RFC 3261 Section 20.15)
// Content-Type = ( "Content-Type" / "c" ) HCOLON media-type
// media-type = m-type SLASH m-subtype *(SEMI m-parameter)
// m-type = discrete-type / composite-type
// discrete-type = "text" / "image" / "audio" / "video" / "application" / extension-token
// composite-type = "message" / "multipart" / extension-token
// extension-token = ietf-token / x-token
// ietf-token = token
// x-token = "x-" token
// m-subtype = extension-token / iana-token
// iana-token = token
// m-parameter = m-attribute EQUAL m-value
// m-attribute = token
// m-value = token / quoted-string

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case},
    combinator::{map, map_res},
    sequence::{pair, preceded, separated_pair},
    IResult,
};
use std::str;
use std::collections::HashMap;

// Import from base parser modules
use crate::parser::separators::{hcolon, equal, slash};
use crate::parser::ParseResult;
use crate::parser::token::token;
use crate::parser::quoted::quoted_string;
use crate::parser::common_params::{semicolon_separated_params0, generic_param};

// Import from sibling header modules
use super::media_type::{parse_media_type, media_params_to_hashmap}; // Use the specific media_type parser

// Import the shared media_type parser
use super::media_type::media_type;
use crate::types::media_type::MediaType;
use crate::types::content_type::ContentType as ContentTypeHeader; // Specific header type
use crate::types::param::Param;

// m-type, m-subtype are just tokens
// Note: These seem to be defined in media_type.rs now, potentially remove if unused locally
// fn m_token(input: &[u8]) -> ParseResult<&[u8]> {
//     token(input)
// }
// Access m_type and m_subtype through imported media_type module functions if needed.

// m-value = token / quoted-string
fn m_value(input: &[u8]) -> ParseResult<&[u8]> {
    alt((token, quoted_string))(input)
}

// m-parameter = m-attribute EQUAL m-value
// Returns (attribute_bytes, value_bytes)
fn m_parameter(input: &[u8]) -> ParseResult<(&[u8], &[u8])> {
    separated_pair(token, equal, m_value)(input)
}

// media-type = m-type SLASH m-subtype *(SEMI m-parameter)
// Returns (type, subtype, Vec<(attr, val)>)
fn media_type(input: &[u8]) -> ParseResult<MediaType> {
    map_res(
        pair(
            pair(m_type, preceded(slash, m_subtype)),
            semicolon_separated_params0(generic_param)
        ),
        |(((type_bytes, subtype_bytes), params_vec))| {
            let media_type = str::from_utf8(type_bytes)?.to_string();
            let sub_type = str::from_utf8(subtype_bytes)?.to_string();
            Ok(MediaType::new(media_type, sub_type, params_vec))
        }
    )(input)
}

// Define structure for Content-Type value
#[derive(Debug, PartialEq, Clone)]
pub struct ContentTypeValue {
    pub m_type: String,
    pub m_subtype: String,
    // TODO: Unescape quoted-string values in params
    pub parameters: HashMap<String, String>,
}

// Content-Type = "Content-Type" HCOLON media-type
// Note: HCOLON and compact form handled elsewhere.
pub(crate) fn parse_content_type(input: &[u8]) -> ParseResult<ContentTypeHeader> {
    map(media_type, ContentTypeHeader)(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::content_type::ContentType;
    use crate::types::media_type::MediaType

    #[test]
    fn test_parse_content_type_simple() {
        let input = b"application/sdp";
        let result = parse_content_type(input);
        assert!(result.is_ok());
        let (rem, ct) = result.unwrap(); // Returns ContentType
        let mt = ct.0; // Access inner MediaType
        assert!(rem.is_empty());
        assert_eq!(mt.m_type, "application");
        assert_eq!(mt.m_subtype, "sdp");
        assert!(mt.parameters.is_empty());
    }

    #[test]
    fn test_parse_content_type_params() {
        let input = b"text/html; charset=utf-8";
        let result = parse_content_type(input);
        assert!(result.is_ok());
        let (rem, ct) = result.unwrap();
        let mt = ct.0;
        assert!(rem.is_empty());
        assert_eq!(mt.m_type, "text");
        assert_eq!(mt.m_subtype, "html");
        assert_eq!(mt.parameters.len(), 1);
        assert_eq!(mt.parameters.get("charset"), Some(&"utf-8".to_string()));
    }
} 
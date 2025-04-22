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
    sequence::{pair, preceded},
    IResult,
};
use std::str;
use std::collections::HashMap;

// Import from base parser modules
use crate::parser::separators::hcolon;
use crate::parser::ParseResult;

// Import from sibling header modules
use super::media_type::{parse_media_type, media_params_to_hashmap}; // Use the specific media_type parser

// Import the shared media_type parser
use super::media_type::media_type;
use crate::types::media_type::MediaType;
use crate::types::content_type::ContentType; // Import specific type

// m-type, m-subtype are just tokens
fn m_token(input: &[u8]) -> ParseResult<&[u8]> {
    token(input)
}

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
fn media_type(input: &[u8]) -> ParseResult<(&[u8], &[u8], Vec<(&[u8], &[u8])>)> {
    map(
        pair(
            separated_pair(m_token, slash, m_token),
            many0(preceded(semi, m_parameter))
        ),
        |((mtype, msubtype), params)| (mtype, msubtype, params)
    )(input)
}

// Define a struct to represent the Content-Type value
#[derive(Debug, PartialEq, Clone)]
pub struct ContentTypeValue {
    pub m_type: String,
    pub m_subtype: String,
    // TODO: Unescape quoted-string values in params
    pub parameters: HashMap<String, String>,
}

// Content-Type = "Content-Type" HCOLON media-type
// Note: HCOLON and compact form handled elsewhere.
pub(crate) fn parse_content_type(input: &[u8]) -> ParseResult<ContentType> { // Return ContentType
    // Map the MediaType result into the ContentType newtype
    map(media_type, ContentType)(input)
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
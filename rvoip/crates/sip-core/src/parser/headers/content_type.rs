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
    error::ParseError,
};
use std::str;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use std::fmt;

// Import from base parser modules
use crate::parser::separators::{hcolon, equal, slash};
use crate::parser::ParseResult;
use crate::parser::token::token;
use crate::parser::quoted::quoted_string;
use crate::parser::common_params::{semicolon_separated_params0, generic_param};

// Import from sibling header modules
// use super::media_type::{parse_media_type, media_params_to_hashmap}; // Use the specific media_type parser - REMOVED

// Import the shared media_type parser - REMOVED
// use super::media_type::media_type;
// use crate::types::media_type::MediaType;
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
// fn m_parameter(input: &[u8]) -> ParseResult<(&[u8], &[u8])> {
//     separated_pair(token, equal, m_value)(input)
// }

// Comment out unused media_type function
/*
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
*/

// Define structure for Content-Type value
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct ContentTypeValue {
    pub m_type: String,
    pub m_subtype: String,
    // TODO: Unescape quoted-string values in params
    pub parameters: HashMap<String, String>,
}

// Implement Display for ContentTypeValue
impl fmt::Display for ContentTypeValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.m_type, self.m_subtype)?;
        for (key, value) in &self.parameters {
            // Basic parameter formatting, might need escaping logic later
             if value.chars().any(|c| !c.is_ascii_alphanumeric()) || value.is_empty() {
                 // Quote if value is not a simple token or is empty
                 write!(f, ";{}=\"{}\"", key, value.replace('\\', "\\\\").replace('"', "\\\""))?;
             } else {
                 write!(f, ";{}={}", key, value)?;
            }
        }
        Ok(())
    }
}

// Content-Type = "Content-Type" HCOLON media-type
// Note: HCOLON and compact form handled elsewhere.
// This parser needs to return ContentTypeValue, not ContentTypeHeader
// Make this function public
pub fn parse_content_type_value(input: &[u8]) -> ParseResult<ContentTypeValue> {
    // We need a parser that directly returns ContentTypeValue based on media_type logic
    // Let's reuse the media_type parser logic from parser/headers/media_type.rs here
    // Reimplementing m_type, m_subtype, m_parameter, m_value for simplicity here
    // m-type, m-subtype are just tokens
    fn m_token(input: &[u8]) -> ParseResult<&[u8]> {
        token(input)
    }

    // m-value = token / quoted-string (returning owned String)
    fn m_value_str(input: &[u8]) -> ParseResult<String> {
        alt((
            map_res(quoted_string, |bytes| crate::parser::common_params::unquote_string(bytes)),
            map_res(token, |bytes| str::from_utf8(bytes).map(String::from)),
        ))(input)
    }

    // m-parameter = m-attribute EQUAL m-value (returning String, String)
    fn m_parameter_str(input: &[u8]) -> ParseResult<(String, String)> {
        map_res(
            separated_pair(m_token, equal, m_value_str),
            |(attr_bytes, value_string)| {
                str::from_utf8(attr_bytes)
                    .map(|attr_str| (attr_str.to_lowercase(), value_string))
            },
        )(input)
    }

    // media-type parser logic adapted to return ContentTypeValue
    map_res(
        pair(
            pair(m_token, preceded(slash, m_token)), // Use m_token for type/subtype
            semicolon_separated_params0(m_parameter_str) // Use param parser returning String, String
        ),
        |(((type_bytes, subtype_bytes), params_vec))| { 
            str::from_utf8(type_bytes)
                .map_err(|_| nom::Err::Failure(nom::error::Error::from_error_kind(type_bytes, nom::error::ErrorKind::Char)))
                .and_then(|m_type_str| {
                    str::from_utf8(subtype_bytes)
                        .map_err(|_| nom::Err::Failure(nom::error::Error::from_error_kind(subtype_bytes, nom::error::ErrorKind::Char)))
                        .map(|m_subtype_str| {
                             let parameters = params_vec.into_iter().collect::<HashMap<_,_>>();
                             ContentTypeValue {
                                m_type: m_type_str.to_lowercase(),
                                m_subtype: m_subtype_str.to_lowercase(),
                                parameters
                            }
                        })
                })
        }
    )(input)
}

// Old parser function - keep for reference or remove later
// pub(crate) fn parse_content_type(input: &[u8]) -> ParseResult<ContentTypeHeader> {
//     map(media_type, ContentTypeHeader)(input)
// }

#[cfg(test)]
mod tests {
    use super::*;
    // Remove unused imports
    // use crate::types::content_type::ContentType;
    // use crate::types::media_type::MediaType;

    #[test]
    fn test_parse_content_type_value_simple() { // Rename test function
        let input = b"application/sdp";
        let result = parse_content_type_value(input); // Use new parser
        assert!(result.is_ok());
        let (rem, ctv) = result.unwrap(); // Returns ContentTypeValue
        assert!(rem.is_empty());
        assert_eq!(ctv.m_type, "application");
        assert_eq!(ctv.m_subtype, "sdp");
        assert!(ctv.parameters.is_empty());
    }

    #[test]
    fn test_parse_content_type_value_params() { // Rename test function
        let input = b"text/html; charset=utf-8";
        let result = parse_content_type_value(input); // Use new parser
        assert!(result.is_ok());
        let (rem, ctv) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ctv.m_type, "text");
        assert_eq!(ctv.m_subtype, "html");
        assert_eq!(ctv.parameters.len(), 1);
        assert_eq!(ctv.parameters.get("charset"), Some(&"utf-8".to_string()));
    }

    #[test]
    fn test_content_type_value_display() {
        let ctv = ContentTypeValue {
            m_type: "multipart".to_string(),
            m_subtype: "mixed".to_string(),
            parameters: HashMap::from([("boundary".to_string(), "boundary42".to_string())]),
        };
        assert_eq!(ctv.to_string(), "multipart/mixed;boundary=boundary42");

        let ctv_quoted = ContentTypeValue {
            m_type: "text".to_string(),
            m_subtype: "plain".to_string(),
            parameters: HashMap::from([("charset".to_string(), "us-ascii (quoted)".to_string())]),
        };
        assert_eq!(ctv_quoted.to_string(), "text/plain;charset=\"us-ascii (quoted)\"");
    }
} 
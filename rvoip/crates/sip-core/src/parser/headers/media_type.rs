// Parsers for Media Type and Media Range (RFC 2045, RFC 3261)

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case},
    character::complete::{alphanumeric1, char},
    combinator::{map, map_res, opt, recognize, value},
    multi::{many0, separated_list0},
    sequence::{delimited, preceded, terminated, pair, tuple, separated_pair},
    IResult,
};
use std::str;
use std::collections::HashMap;

use crate::parser::token::token;
use crate::parser::separators::{slash, semi, equal};
use crate::parser::quoted::quoted_string;
use crate::parser::ParseResult;
use crate::parser::common_params::unquote_string; // Re-use unquoting logic
use crate::types::media_type::MediaType; // Import the actual type

// m-attribute = token
fn m_attribute(input: &[u8]) -> ParseResult<&[u8]> {
    token(input)
}

// m-value = token / quoted-string
// Returns unquoted string
fn m_value(input: &[u8]) -> ParseResult<String> {
    alt((
        map_res(quoted_string, |bytes| unquote_string(bytes)),
        map_res(token, |bytes| str::from_utf8(bytes).map(String::from)),
    ))(input)
}

// m-parameter = m-attribute EQUAL m-value
// Returns (String, String)
fn m_parameter(input: &[u8]) -> ParseResult<(String, String)> {
    map_res(
        separated_pair(m_attribute, equal, m_value),
        |(attr_bytes, value_string)| {
            str::from_utf8(attr_bytes)
                .map(|attr_str| (attr_str.to_string(), value_string))
        },
    )(input)
}

// ietf-token = token
// extension-token = ietf-token / x-token
// x-token = "x-" token
fn extension_token(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(
       alt((
           token,
           preceded(tag_no_case(b"x-"), token)
       ))
    )(input)
}

// m-subtype = extension-token / iana-token
// iana-token = token
fn m_subtype(input: &[u8]) -> ParseResult<&[u8]> {
    // Both alternatives resolve to token or x-token
    extension_token(input)
}

// discrete-type = "text" / "image" / "audio" / "video"
//                 / "application" / extension-token
// composite-type = "message" / "multipart" / extension-token
// m-type = discrete-type / composite-type
fn m_type(input: &[u8]) -> ParseResult<&[u8]> {
    // Using tag_no_case for known types, fallback to extension_token
    alt((
        // discrete
        tag_no_case("text"),
        tag_no_case("image"),
        tag_no_case("audio"),
        tag_no_case("video"),
        tag_no_case("application"),
        // composite
        tag_no_case("message"),
        tag_no_case("multipart"),
        // extension
        extension_token,
    ))(input)
}

// media-type = m-type SLASH m-subtype *(SEMI m-parameter)
// Returns MediaType struct
pub fn media_type(input: &[u8]) -> ParseResult<MediaType> {
    map_res(
        tuple((
            m_type,
            preceded(slash, m_subtype),
            many0(preceded(semi, m_parameter)) // Returns Vec<(String, String)>
        )),
        |(type_bytes, subtype_bytes, params_vec)| {
            let type_str = str::from_utf8(type_bytes)?.to_string();
            let subtype_str = str::from_utf8(subtype_bytes)?.to_string();
            let params_map = params_vec.into_iter().collect::<HashMap<_,_>>();
            // Construct MediaType struct
            Ok(MediaType { 
                m_type: type_str, 
                m_subtype: subtype_str, 
                parameters: params_map 
            })
        }
    )(input)
}

/// Helper to convert parsed media parameters (Vec<(&[u8], &[u8])>) into a HashMap.
/// Lowers keys, leaves values as Strings.
/// TODO: Handle unescaping of quoted values.
pub fn media_params_to_hashmap(params_b: Vec<(&[u8], &[u8])>) -> Result<HashMap<String, String>, str::Utf8Error> {
    params_b.into_iter().map(|(attr_b, val_b)| {
        let attr = str::from_utf8(attr_b)?.to_lowercase();
        let val = str::from_utf8(val_b)?.to_string();
        Ok((attr, val))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_m_parameter() {
        let (rem, (name, value)) = m_parameter(b"charset=utf-8").unwrap();
        assert!(rem.is_empty());
        assert_eq!(name, "charset");
        assert_eq!(value, "utf-8");

        let (rem_qs, (name_qs, value_qs)) = m_parameter(b"boundary=\"simple boundary\"").unwrap();
        assert!(rem_qs.is_empty());
        assert_eq!(name_qs, "boundary");
        assert_eq!(value_qs, "simple boundary");
        
        let (rem_esc, (name_esc, value_esc)) = m_parameter(b"desc=\"\\\"\\\\\"").unwrap(); // desc="\"\"
        assert!(rem_esc.is_empty());
        assert_eq!(name_esc, "desc");
        assert_eq!(value_esc, "\"\\");
    }
    
    #[test]
    fn test_media_type_simple() {
        let input = b"application/sdp";
        let (rem, mt) = media_type(input).unwrap(); // Returns MediaType now
        assert!(rem.is_empty());
        assert_eq!(mt.m_type, "application");
        assert_eq!(mt.m_subtype, "sdp");
        assert!(mt.parameters.is_empty());
    }

    #[test]
    fn test_media_type_with_params() {
        let input = b"text/html; charset=ISO-8859-4";
        let (rem, mt) = media_type(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(mt.m_type, "text");
        assert_eq!(mt.m_subtype, "html");
        assert_eq!(mt.parameters.len(), 1);
        assert_eq!(mt.parameters.get("charset"), Some(&"ISO-8859-4".to_string()));
    }
    
    #[test]
    fn test_media_type_x_token() {
        let input = b"application/x-custom-app; version=1";
        let (rem, mt) = media_type(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(mt.m_type, "application");
        assert_eq!(mt.m_subtype, "x-custom-app");
        assert_eq!(mt.parameters.get("version"), Some(&"1".to_string()));
    }
    
    #[test]
    fn test_media_type_complex_params() {
        let input = b"multipart/form-data; boundary=\"----WebKitFormBoundary7MA4YWxkTrZu0gW\" ; name=upload";
        let (rem, mt) = media_type(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(mt.m_type, "multipart");
        assert_eq!(mt.m_subtype, "form-data");
        assert_eq!(mt.parameters.len(), 2);
        assert_eq!(mt.parameters.get("boundary"), Some(&"----WebKitFormBoundary7MA4YWxkTrZu0gW".to_string()));
        assert_eq!(mt.parameters.get("name"), Some(&"upload".to_string()));
    }
} 
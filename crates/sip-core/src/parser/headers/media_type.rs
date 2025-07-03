// Parsers for Media Type and Media Range (RFC 2045, RFC 3261)

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_while1},
    character::complete::{char},
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
use crate::parser::common_params::unquote_string;
use crate::types::media_type::MediaType;

// m-attribute = token
fn m_attribute(input: &[u8]) -> ParseResult<&[u8]> {
    token(input)
}

// m-value = token / quoted-string
// Returns unquoted string
fn m_value(input: &[u8]) -> ParseResult<String> {
    alt((
        map_res(quoted_string, unquote_string),
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
                .map(|attr_str| (attr_str.to_lowercase(), value_string))
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
        tag_no_case(b"text"),
        tag_no_case(b"image"),
        tag_no_case(b"audio"),
        tag_no_case(b"video"),
        tag_no_case(b"application"),
        // composite
        tag_no_case(b"message"),
        tag_no_case(b"multipart"),
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
        |(type_bytes, subtype_bytes, params_vec)| -> Result<MediaType, std::str::Utf8Error> {
            let type_str = str::from_utf8(type_bytes)?.to_lowercase();
            let subtype_str = str::from_utf8(subtype_bytes)?.to_lowercase();
            let params_map = params_vec.into_iter().collect::<HashMap<_,_>>();
            // Construct MediaType struct with the correct field names
            Ok(MediaType { 
                typ: type_str, 
                subtype: subtype_str, 
                parameters: params_map 
            })
        }
    )(input)
}

/// Helper to convert parsed media parameters (Vec<(&[u8], &[u8])>) into a HashMap.
/// Lowers keys and properly handles unescaping of quoted values.
pub fn media_params_to_hashmap(params_b: Vec<(&[u8], &[u8])>) -> Result<HashMap<String, String>, std::str::Utf8Error> {
    params_b.into_iter().map(|(attr_b, val_b)| {
        let attr = std::str::from_utf8(attr_b)?.to_lowercase();
        let val = if val_b.len() >= 2 && val_b[0] == b'"' && val_b[val_b.len() - 1] == b'"' {
            // Handle quoted string unescaping
            let inner = &val_b[1..val_b.len() - 1];
            match unquote_string(inner) {
                Ok(unescaped) => unescaped,
                Err(_) => return Err(std::str::from_utf8(&[0]).unwrap_err())
            }
        } else {
            std::str::from_utf8(val_b)?.to_string()
        };
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
        assert_eq!(mt.typ, "application");
        assert_eq!(mt.subtype, "sdp");
        assert!(mt.parameters.is_empty());
    }

    #[test]
    fn test_media_type_with_params() {
        let input = b"text/html; charset=ISO-8859-4";
        let (rem, mt) = media_type(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(mt.typ, "text");
        assert_eq!(mt.subtype, "html");
        assert_eq!(mt.parameters.len(), 1);
        assert_eq!(mt.parameters.get("charset"), Some(&"ISO-8859-4".to_string()));
    }
    
    #[test]
    fn test_media_type_x_token() {
        let input = b"application/x-custom-app; version=1";
        let (rem, mt) = media_type(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(mt.typ, "application");
        assert_eq!(mt.subtype, "x-custom-app");
        assert_eq!(mt.parameters.get("version"), Some(&"1".to_string()));
    }
    
    #[test]
    fn test_media_type_complex_params() {
        let input = b"multipart/form-data; boundary=\"----WebKitFormBoundary7MA4YWxkTrZu0gW\" ; name=upload";
        let (rem, mt) = media_type(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(mt.typ, "multipart");
        assert_eq!(mt.subtype, "form-data");
        assert_eq!(mt.parameters.len(), 2);
        assert_eq!(mt.parameters.get("boundary"), Some(&"----WebKitFormBoundary7MA4YWxkTrZu0gW".to_string()));
        assert_eq!(mt.parameters.get("name"), Some(&"upload".to_string()));
    }
    
    #[test]
    fn test_case_insensitivity() {
        // RFC 2045 states that the type and subtype are case-insensitive
        let input1 = b"Application/SDP";
        let input2 = b"application/sdp";
        
        let (_, mt1) = media_type(input1).unwrap();
        let (_, mt2) = media_type(input2).unwrap();
        
        assert_eq!(mt1.typ, "application");
        assert_eq!(mt1.subtype, "sdp");
        assert_eq!(mt2.typ, "application");
        assert_eq!(mt2.subtype, "sdp");
        
        // Parameter names should also be case-insensitive
        let input3 = b"text/html; CHARSET=utf-8";
        let (_, mt3) = media_type(input3).unwrap();
        assert_eq!(mt3.parameters.get("charset"), Some(&"utf-8".to_string()));
    }
    
    #[test]
    fn test_rfc_defined_types() {
        // Test various standard media types from RFC 2045, RFC 3261
        let test_types = [
            (b"text/plain".as_ref(), "text", "plain"),
            (b"text/html".as_ref(), "text", "html"),
            (b"text/xml".as_ref(), "text", "xml"),
            (b"application/sdp".as_ref(), "application", "sdp"),
            (b"application/sip".as_ref(), "application", "sip"),
            (b"application/cpl+xml".as_ref(), "application", "cpl+xml"),
            (b"message/sipfrag".as_ref(), "message", "sipfrag"),
            (b"multipart/mixed".as_ref(), "multipart", "mixed"),
            (b"multipart/related".as_ref(), "multipart", "related"),
            (b"multipart/alternative".as_ref(), "multipart", "alternative"),
            (b"image/jpeg".as_ref(), "image", "jpeg"),
            (b"audio/basic".as_ref(), "audio", "basic"),
            (b"video/mp4".as_ref(), "video", "mp4"),
        ];
        
        for (input, expected_type, expected_subtype) in test_types {
            let (_, mt) = media_type(input).unwrap();
            assert_eq!(mt.typ, expected_type);
            assert_eq!(mt.subtype, expected_subtype);
        }
    }
    
    #[test]
    fn test_token_boundary_chars() {
        // RFC 2045 defines token as: token := 1*<any CHAR except CTLs or separators>
        // Test boundary cases for valid token characters
        let valid_token_inputs = [
            b"application/sdp!#$%&'*+-.^_`|~0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ".as_ref(),
            b"application/sdp+xml".as_ref(),
            b"application/sdp.v2".as_ref(),
            b"application/vnd.3gpp.sms".as_ref(),
        ];
        
        for input in valid_token_inputs {
            assert!(media_type(input).is_ok());
        }
    }
    
    #[test]
    fn test_error_handling() {
        // Test invalid media types
        let invalid_inputs = [
            b"".as_ref(), // Empty input
            b"application".as_ref(), // Missing subtype
            b"/sdp".as_ref(), // Missing type
            b"application/".as_ref(), // Missing subtype after slash
            b"app lication/sdp".as_ref(), // Space in type
            b"application/s dp".as_ref(), // Space in subtype
            b"application/sdp; ".as_ref(), // Empty parameter
            b"application/sdp; charset".as_ref(), // Parameter without value
            b"application/sdp; =utf-8".as_ref(), // Parameter without name
            b"application/sdp; ch@rset=utf-8".as_ref(), // Invalid character in parameter name
            b"application/sdp; charset=utf@8".as_ref(), // Still valid - parameter values can contain @ as they can be quoted-strings
        ];
        
        for input in invalid_inputs {
            if input.is_empty() {
                assert!(media_type(input).is_err());
            } else {
                let result = media_type(input);
                if let Ok((rem, _)) = result {
                    // If parsing succeeded, it shouldn't have consumed all input for invalid cases
                    // (except for the special case of parameter values which can contain @ even in unquoted form)
                    if input != b"application/sdp; charset=utf@8" {
                        assert!(!rem.is_empty());
                    }
                }
            }
        }
    }
    
    #[test]
    fn test_quoted_string_handling() {
        // Test handling of quoted strings in parameter values
        let input = b"application/sdp; key=\"quoted \\\"value\\\" with \\\\backslashes\"";
        let (_, mt) = media_type(input).unwrap();
        assert_eq!(mt.parameters.get("key"), Some(&"quoted \"value\" with \\backslashes".to_string()));
        
        // Test escaped quotes and backslashes
        let input2 = b"application/sdp; complex=\"\\\"\\\\\"";
        let (_, mt2) = media_type(input2).unwrap();
        assert_eq!(mt2.parameters.get("complex"), Some(&"\"\\".to_string()));
    }
    
    #[test]
    fn test_media_params_to_hashmap() {
        // Test the helper function
        let params = vec![
            (b"charset" as &[u8], b"utf-8" as &[u8]),
            (b"BOUNDARY" as &[u8], b"\"simple boundary\"" as &[u8]),
            (b"Complex" as &[u8], b"\"quoted \\\"value\\\"\"" as &[u8]),
        ];
        
        let result = media_params_to_hashmap(params).unwrap();
        
        assert_eq!(result.get("charset"), Some(&"utf-8".to_string()));
        assert_eq!(result.get("boundary"), Some(&"simple boundary".to_string()));
        assert_eq!(result.get("complex"), Some(&"quoted \"value\"".to_string()));
    }
} 
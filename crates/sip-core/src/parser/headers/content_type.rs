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
    bytes::complete::{tag, tag_no_case},
    combinator::{map, map_res, opt, verify},
    sequence::{pair, preceded, separated_pair, terminated, tuple},
    multi::many0,
    IResult,
    error::{Error as NomError, ErrorKind, ParseError},
};
use std::str;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use std::fmt;

// Import from base parser modules
use crate::parser::separators::{hcolon, equal, slash, semi};
use crate::parser::ParseResult;
use crate::parser::token::token;
use crate::parser::quoted::quoted_string;
use crate::parser::common_params::{semicolon_separated_params0, generic_param};
use crate::parser::whitespace::{lws, owsp, sws};

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
    // First check for empty input
    if input.is_empty() {
        return Err(nom::Err::Error(NomError::new(input, ErrorKind::TakeWhile1)));
    }
    
    // Handle leading whitespace, including line folding
    let (input, _) = opt(lws)(input)?;
    
    // Parse the media type and subtype with proper whitespace handling
    let (input, (m_type, m_subtype)) = parse_media_type_with_whitespace(input)?;
    
    // Parse the parameters with proper whitespace handling
    let (input, params) = parse_media_parameters(input)?;
    
    // Handle any trailing whitespace
    let (input, _) = sws(input)?;
    
    // Check that there's nothing left to parse
    if !input.is_empty() {
        return Err(nom::Err::Error(NomError::new(input, ErrorKind::Eof)));
    }
    
    // Create ContentTypeValue with normalized type/subtype (lowercase)
    let content_type = ContentTypeValue {
        m_type: m_type.to_lowercase(),
        m_subtype: m_subtype.to_lowercase(),
        parameters: params,
    };
    
    Ok((input, content_type))
}

// Parse the media type and subtype part (m-type SLASH m-subtype) with proper whitespace handling
fn parse_media_type_with_whitespace(input: &[u8]) -> ParseResult<(String, String)> {
    // Parse type, slash, and subtype with proper whitespace handling
    let (input, (m_type_bytes, _, m_subtype_bytes)) = tuple((
        terminated(token, opt(lws)),                    // type token followed by optional whitespace
        terminated(slash, opt(lws)),                    // slash followed by optional whitespace
        token                                           // subtype token
    ))(input)?;
    
    // Convert to strings
    let m_type = str::from_utf8(m_type_bytes)
        .map_err(|_| nom::Err::Error(NomError::new(input, ErrorKind::AlphaNumeric)))?
        .to_string();
        
    let m_subtype = str::from_utf8(m_subtype_bytes)
        .map_err(|_| nom::Err::Error(NomError::new(input, ErrorKind::AlphaNumeric)))?
        .to_string();
    
    Ok((input, (m_type, m_subtype)))
}

// Parse the media parameters (*(SEMI m-parameter)) with proper whitespace handling
fn parse_media_parameters(input: &[u8]) -> ParseResult<HashMap<String, String>> {
    let (input, params) = many0(parse_media_parameter)(input)?;
    
    // Create a HashMap from the parameter pairs
    let parameters = params.into_iter().collect::<HashMap<_, _>>();
    
    Ok((input, parameters))
}

// Parse a single media parameter (SEMI m-parameter) with proper whitespace handling
fn parse_media_parameter(input: &[u8]) -> ParseResult<(String, String)> {
    // Parse a semicolon followed by a parameter
    let (input, _) = terminated(semi, opt(lws))(input)?;
    
    // Parse the parameter name and value
    let (input, (name_bytes, value)) = separated_pair(
        token,                                          // Parameter name is a token
        terminated(equal, opt(lws)),                    // Equals sign followed by optional whitespace
        alt((
            // Parse quoted string value
            map_res(quoted_string, |bytes| {
                crate::parser::common_params::unquote_string(bytes)
                    .map_err(|_| nom::Err::Error(NomError::new(bytes, ErrorKind::AlphaNumeric)))
            }),
            
            // Parse token value
            map_res(token, |bytes| {
                str::from_utf8(bytes).map(String::from)
                    .map_err(|_| nom::Err::Error(NomError::new(bytes, ErrorKind::AlphaNumeric)))
            })
        ))
    )(input)?;
    
    // Convert parameter name to lowercase string (parameters are case-insensitive)
    let name = str::from_utf8(name_bytes)
        .map_err(|_| nom::Err::Error(NomError::new(input, ErrorKind::AlphaNumeric)))?
        .to_lowercase();
    
    Ok((input, (name, value)))
}

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
    
    #[test]
    fn test_parse_content_type_with_whitespace() {
        // Test with various whitespace patterns
        let input = b"  application/json  ;  charset=utf-8  ;  boundary=1234  ";
        let result = parse_content_type_value(input);
        assert!(result.is_ok());
        let (rem, ctv) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ctv.m_type, "application");
        assert_eq!(ctv.m_subtype, "json");
        assert_eq!(ctv.parameters.len(), 2);
        assert_eq!(ctv.parameters.get("charset"), Some(&"utf-8".to_string()));
        assert_eq!(ctv.parameters.get("boundary"), Some(&"1234".to_string()));
        
        // Test with tabs
        let input = b"text/plain	;	charset=iso-8859-1";
        let result = parse_content_type_value(input);
        assert!(result.is_ok());
        let (rem, ctv) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ctv.m_type, "text");
        assert_eq!(ctv.m_subtype, "plain");
        assert_eq!(ctv.parameters.len(), 1);
    }
    
    #[test]
    fn test_parse_content_type_with_line_folding() {
        // Test with line folding after type
        let input = b"application\r\n /json";
        let result = parse_content_type_value(input);
        assert!(result.is_ok());
        
        // Test with line folding after slash
        let input = b"application/\r\n json";
        let result = parse_content_type_value(input);
        assert!(result.is_ok());
        
        // Test with line folding in parameters
        let input = b"application/json;\r\n charset=utf-8;\r\n boundary=1234";
        let result = parse_content_type_value(input);
        assert!(result.is_ok());
        let (rem, ctv) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ctv.m_type, "application");
        assert_eq!(ctv.m_subtype, "json");
        assert_eq!(ctv.parameters.len(), 2);
        
        // Test with complex line folding
        let input = b"application\r\n /\r\n json\r\n ;\r\n charset\r\n =\r\n utf-8";
        let result = parse_content_type_value(input);
        assert!(result.is_ok());
        let (rem, ctv) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ctv.m_type, "application");
        assert_eq!(ctv.m_subtype, "json");
        assert_eq!(ctv.parameters.len(), 1);
    }
    
    #[test]
    fn test_parse_content_type_case_sensitivity() {
        // Test case insensitivity for type and subtype
        let input = b"APPLICATION/JSON";
        let result = parse_content_type_value(input);
        assert!(result.is_ok());
        let (rem, ctv) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ctv.m_type, "application");
        assert_eq!(ctv.m_subtype, "json");
        
        // Test case insensitivity for parameter names
        let input = b"text/plain; CHARSET=utf-8";
        let result = parse_content_type_value(input);
        assert!(result.is_ok());
        let (rem, ctv) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ctv.parameters.get("charset"), Some(&"utf-8".to_string()));
        
        // Test mixed case
        let input = b"Text/Plain; Charset=UTF-8";
        let result = parse_content_type_value(input);
        assert!(result.is_ok());
        let (rem, ctv) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ctv.m_type, "text");
        assert_eq!(ctv.m_subtype, "plain");
        assert_eq!(ctv.parameters.get("charset"), Some(&"UTF-8".to_string())); // Parameter values should preserve case
    }
    
    #[test]
    fn test_parse_content_type_error_handling() {
        // Test empty input
        let input = b"";
        assert!(parse_content_type_value(input).is_err());
        
        // Test invalid format (missing slash)
        let input = b"application";
        assert!(parse_content_type_value(input).is_err());
        
        // Test missing subtype
        let input = b"application/";
        assert!(parse_content_type_value(input).is_err());
        
        // Test invalid parameter format (missing value)
        let input = b"text/plain; charset=";
        assert!(parse_content_type_value(input).is_err());
        
        // Test invalid parameter format (missing equals)
        let input = b"text/plain; charset";
        assert!(parse_content_type_value(input).is_err());
        
        // Test invalid characters in type
        let input = b"text@/plain";
        assert!(parse_content_type_value(input).is_err());
        
        // Test invalid characters in subtype
        let input = b"text/plain@";
        assert!(parse_content_type_value(input).is_err());
        
        // Test unclosed quoted string
        let input = b"text/plain; charset=\"unclosed";
        assert!(parse_content_type_value(input).is_err());
    }
    
    #[test]
    fn test_parse_content_type_rfc_examples() {
        // Examples from RFC 3261
        let input = b"application/sdp";
        let result = parse_content_type_value(input);
        assert!(result.is_ok());
        
        let input = b"application/sdp;version=2";
        let result = parse_content_type_value(input);
        assert!(result.is_ok());
        
        // Common examples from HTTP
        let input = b"text/plain";
        let result = parse_content_type_value(input);
        assert!(result.is_ok());
        
        let input = b"text/html; charset=utf-8";
        let result = parse_content_type_value(input);
        assert!(result.is_ok());
        
        // Multipart example
        let input = b"multipart/mixed; boundary=gc0p4Jq0M2Yt08jU534c0p";
        let result = parse_content_type_value(input);
        assert!(result.is_ok());
        let (rem, ctv) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ctv.m_type, "multipart");
        assert_eq!(ctv.m_subtype, "mixed");
        assert_eq!(ctv.parameters.get("boundary"), Some(&"gc0p4Jq0M2Yt08jU534c0p".to_string()));
    }
    
    #[test]
    fn test_parse_content_type_multiple_parameters() {
        // Test with multiple parameters
        let input = b"application/json; charset=utf-8; boundary=1234; modified=true";
        let result = parse_content_type_value(input);
        assert!(result.is_ok());
        let (rem, ctv) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ctv.m_type, "application");
        assert_eq!(ctv.m_subtype, "json");
        assert_eq!(ctv.parameters.len(), 3);
        assert_eq!(ctv.parameters.get("charset"), Some(&"utf-8".to_string()));
        assert_eq!(ctv.parameters.get("boundary"), Some(&"1234".to_string()));
        assert_eq!(ctv.parameters.get("modified"), Some(&"true".to_string()));
    }
    
    #[test]
    fn test_parse_content_type_quoted_string_parameters() {
        // Test with quoted string parameters
        let input = b"text/plain; charset=\"utf-8\"; description=\"Some text with spaces\"";
        let result = parse_content_type_value(input);
        assert!(result.is_ok());
        let (rem, ctv) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ctv.m_type, "text");
        assert_eq!(ctv.m_subtype, "plain");
        assert_eq!(ctv.parameters.len(), 2);
        assert_eq!(ctv.parameters.get("charset"), Some(&"utf-8".to_string()));
        assert_eq!(ctv.parameters.get("description"), Some(&"Some text with spaces".to_string()));
        
        // Test with escaped quotes in quoted string
        let input = b"text/plain; desc=\"This is a \\\"quoted\\\" word\"";
        let result = parse_content_type_value(input);
        assert!(result.is_ok());
        let (rem, ctv) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ctv.parameters.get("desc"), Some(&"This is a \"quoted\" word".to_string()));
    }
    
    #[test]
    fn test_parse_content_type_abnf_compliance() {
        // Test compliance with RFC 3261 ABNF grammar
        
        // Media type
        for m_type in &["text", "image", "audio", "video", "application", "message", "multipart", "custom-type", "x-custom"] {
            for m_subtype in &["plain", "html", "json", "custom-subtype", "x-custom"] {
                let input = format!("{}/{}", m_type, m_subtype).into_bytes();
                let result = parse_content_type_value(&input);
                assert!(result.is_ok(), "Failed to parse valid media type: {}/{}", m_type, m_subtype);
            }
        }
        
        // Parameters
        let valid_params = [
            "charset=utf-8",
            "boundary=\"simple boundary\"",
            "custom-param=value",
            "q=0.7",
            "level=1"
        ];
        
        let input = b"text/plain";
        let result = parse_content_type_value(input);
        assert!(result.is_ok());
        
        for param in valid_params.iter() {
            let full_input = format!("text/plain; {}", param).into_bytes();
            let result = parse_content_type_value(&full_input);
            assert!(result.is_ok(), "Failed to parse valid parameter: {}", param);
        }
    }
} 
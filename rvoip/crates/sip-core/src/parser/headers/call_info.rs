// Parser for the Call-Info header (RFC 3261 Section 20.9)
// Call-Info = "Call-Info" HCOLON info *(COMMA info)
// info = LAQUOT absoluteURI RAQUOT *( SEMI info-param)
// info-param = ( "purpose" EQUAL ( "icon" / "info" / "card" / token ) ) / generic-param

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_while1},
    combinator::{map, map_res},
    multi::{many0, separated_list1},
    sequence::{pair, preceded, delimited, tuple},
    IResult,
    error::{Error as NomError, ErrorKind, ParseError},
};
use std::str;

// Import from base parser modules
use crate::parser::separators::{hcolon, semi, comma, equal, laquot, raquot};
use crate::parser::common_params::generic_param;
use crate::parser::uri::parse_absolute_uri; // Using the correct function name
use crate::parser::token::token;
use crate::parser::common::comma_separated_list1;
use crate::parser::ParseResult;

use crate::types::param::Param;
use crate::types::uri::{Uri, Scheme, Host};
// use crate::types::call_info::{CallInfo as CallInfoHeader, CallInfoValue, InfoPurpose}; // Removed unused import
use serde::{Serialize, Deserialize};
use std::str::FromStr;

// Make these types public
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfoPurpose {
    Icon,
    Info,
    Card,
    Other(String),
}
#[derive(Debug, Clone, PartialEq)]
pub enum InfoParam {
    Purpose(InfoPurpose),
    Generic(Param),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallInfoValue {
    pub uri: Uri,
    pub params: Vec<Param>,
}

// info-param = ( "purpose" EQUAL ( "icon" / "info" / "card" / token ) ) / generic-param
fn info_param(input: &[u8]) -> ParseResult<InfoParam> {
    alt((
        map(
            preceded(
                pair(tag_no_case(b"purpose"), equal),
                alt((
                    map_res(tag_no_case("icon"), |_| Ok::<InfoPurpose, nom::error::Error<&[u8]>>(InfoPurpose::Icon)),
                    map_res(tag_no_case("info"), |_| Ok::<InfoPurpose, nom::error::Error<&[u8]>>(InfoPurpose::Info)),
                    map_res(tag_no_case("card"), |_| Ok::<InfoPurpose, nom::error::Error<&[u8]>>(InfoPurpose::Card)),
                    map_res(token, |bytes| {
                        match str::from_utf8(bytes) {
                            Ok(purpose_str) => Ok(InfoPurpose::Other(purpose_str.to_string())),
                            Err(_) => Err(nom::error::Error::from_error_kind(bytes, nom::error::ErrorKind::Char))
                        }
                    })
                ))
            ),
            InfoParam::Purpose
        ),
        map(generic_param, InfoParam::Generic)
    ))(input)
}

// Convert InfoParam to Param
fn convert_info_param_to_param(info_param: InfoParam) -> Param {
    match info_param {
        InfoParam::Generic(param) => param,
        InfoParam::Purpose(purpose) => {
            let value_str = match purpose {
                InfoPurpose::Icon => "icon".to_string(),
                InfoPurpose::Info => "info".to_string(),
                InfoPurpose::Card => "card".to_string(),
                InfoPurpose::Other(s) => s,
            };
            Param::Other("purpose".to_string(), Some(crate::types::param::GenericValue::Token(value_str)))
        }
    }
}

// info = LAQUOT absoluteURI RAQUOT *( SEMI info-param)
// Returns (Uri, Vec<Param>)
fn info(input: &[u8]) -> ParseResult<CallInfoValue> {
    map_res(
        pair(
            delimited(
                laquot,
                // We're using take_while1 instead of parse_absolute_uri because
                // we just need the raw URI string for now
                take_while1(|c| c != b'>'),
                raquot
            ),
            many0(preceded(semi, info_param))
        ),
        |(uri_bytes, params_vec)| -> Result<CallInfoValue, nom::error::Error<&[u8]>> {
            // Extract URI string
            let uri_str = match str::from_utf8(uri_bytes) {
                Ok(s) => s,
                Err(_) => return Err(nom::error::Error::new(uri_bytes, nom::error::ErrorKind::Verify)),
            };

            // Create a simple URI representation for now
            // We'll extract the scheme to determine what kind of URI to create
            let uri = if uri_str.starts_with("sip:") {
                // SIP URI
                match crate::types::uri::Uri::from_str(uri_str) {
                    Ok(uri) => uri,
                    Err(_) => {
                        // Fallback to a simple representation
                        let host = uri_str.strip_prefix("sip:").unwrap_or(uri_str);
                        Uri::sip(host)
                    }
                }
            } else if uri_str.starts_with("sips:") {
                // SIPS URI
                match crate::types::uri::Uri::from_str(uri_str) {
                    Ok(uri) => uri,
                    Err(_) => {
                        // Fallback to a simple representation
                        let host = uri_str.strip_prefix("sips:").unwrap_or(uri_str);
                        Uri::sips(host)
                    }
                }
            } else if uri_str.starts_with("tel:") {
                // TEL URI
                let number = uri_str.strip_prefix("tel:").unwrap_or(uri_str);
                Uri::tel(number)
            } else {
                // Default handling for HTTP, HTTPS, and other URI schemes
                // Extract host or create a simple representation
                // We'll use SIP as the default scheme since that's all that's supported
                let host_part = if uri_str.contains("://") {
                    // Extract host from URL like http://example.com/path
                    let after_scheme = uri_str.split("://").nth(1).unwrap_or(uri_str);
                    let host = after_scheme.split('/').next().unwrap_or(after_scheme);
                    host
                } else if uri_str.contains(':') {
                    // Extract host after scheme like http:example.com
                    uri_str.split(':').nth(1).unwrap_or(uri_str)
                } else {
                    // Use the whole string as host
                    uri_str
                };
                
                // Create a simple URI representation
                Uri::new(
                    Scheme::Sip, // Default to SIP since other schemes aren't supported
                    Host::domain(host_part)
                )
            };
            
            // Convert InfoParam to Param
            let params = params_vec.into_iter()
                .map(convert_info_param_to_param)
                .collect();
                
            Ok(CallInfoValue { uri, params })
        }
    )(input)
}

// Call-Info = "Call-Info" HCOLON info *(COMMA info)
/// Parses a Call-Info header value.
pub fn parse_call_info(input: &[u8]) -> ParseResult<Vec<CallInfoValue>> {
    separated_list1(comma, info)(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::param::{GenericValue, Param};
    use crate::types::uri::{Scheme, Host};

    #[test]
    fn test_info_param() {
        // Test predefined purpose values
        let (rem_p, param_p) = info_param(b"purpose=icon").unwrap();
        assert!(rem_p.is_empty());
        assert!(matches!(param_p, InfoParam::Purpose(InfoPurpose::Icon)));

        let (rem_p, param_p) = info_param(b"purpose=info").unwrap();
        assert!(rem_p.is_empty());
        assert!(matches!(param_p, InfoParam::Purpose(InfoPurpose::Info)));

        let (rem_p, param_p) = info_param(b"purpose=card").unwrap();
        assert!(rem_p.is_empty());
        assert!(matches!(param_p, InfoParam::Purpose(InfoPurpose::Card)));

        // Test custom purpose token
        let (rem_p, param_p) = info_param(b"purpose=custom-token").unwrap();
        assert!(rem_p.is_empty());
        if let InfoParam::Purpose(InfoPurpose::Other(token)) = param_p {
            assert_eq!(token, "custom-token");
        } else {
            panic!("Expected custom purpose token");
        }

        // Test case insensitivity of purpose values
        let (rem_p, param_p) = info_param(b"purpose=ICON").unwrap();
        assert!(rem_p.is_empty());
        assert!(matches!(param_p, InfoParam::Purpose(InfoPurpose::Icon)));

        // Test generic parameter
        let (rem_g, param_g) = info_param(b"random=xyz").unwrap();
        assert!(rem_g.is_empty());
        assert!(matches!(param_g, InfoParam::Generic(Param::Other(n, Some(GenericValue::Token(v)))) if n=="random" && v=="xyz"));

        // Test parameter with no value
        let (rem_g, param_g) = info_param(b"no-value").unwrap();
        assert!(rem_g.is_empty());
        assert!(matches!(param_g, InfoParam::Generic(Param::Other(n, None)) if n=="no-value"));
    }
    
    #[test]
    fn test_info_param_conversion() {
        // Test converting Icon purpose to Param
        let info_param = InfoParam::Purpose(InfoPurpose::Icon);
        let param = convert_info_param_to_param(info_param);
        assert!(matches!(param, Param::Other(n, Some(GenericValue::Token(v))) if n=="purpose" && v=="icon"));

        // Test converting custom purpose to Param
        let info_param = InfoParam::Purpose(InfoPurpose::Other("custom".to_string()));
        let param = convert_info_param_to_param(info_param);
        assert!(matches!(param, Param::Other(n, Some(GenericValue::Token(v))) if n=="purpose" && v=="custom"));

        // Test passing through Generic param
        let original = Param::Other("test".to_string(), None);
        let info_param = InfoParam::Generic(original.clone());
        let param = convert_info_param_to_param(info_param);
        assert_eq!(param, original);
    }
    
    #[test]
    fn test_parse_info() {
        // Test basic info with no parameters
        let input = b"<http://www.example.com/alice/photo.jpg>";
        let result = info(input);
        assert!(result.is_ok());
        let (rem, info_val) = result.unwrap();
        assert!(rem.is_empty());
        assert!(info_val.params.is_empty());
        
        // Test info with purpose parameter
        let input = b"<http://www.example.com/alice/photo.jpg>;purpose=icon";
        let result = info(input);
        assert!(result.is_ok());
        let (rem, info_val) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(info_val.params.len(), 1);
        assert!(matches!(&info_val.params[0], 
            Param::Other(n, Some(GenericValue::Token(v))) if n=="purpose" && v=="icon"));
        
        // Test info with multiple parameters
        let input = b"<http://www.example.com/alice/photo.jpg>;purpose=icon;size=large;color=true";
        let result = info(input);
        assert!(result.is_ok());
        let (rem, info_val) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(info_val.params.len(), 3);
        
        // Test info with parameter having no value
        let input = b"<http://www.example.com/alice/photo.jpg>;purpose=icon;no-value";
        let result = info(input);
        assert!(result.is_ok());
        let (rem, info_val) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(info_val.params.len(), 2);
        assert!(matches!(&info_val.params[1], Param::Other(n, None) if n=="no-value"));
    }

    #[test]
    fn test_parse_call_info_basic() {
        // Basic example from RFC 3261
        let input = b"<http://www.example.com/alice/photo.jpg>;purpose=icon";
        let result = parse_call_info(input);
        assert!(result.is_ok());
        let (rem, infos) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(infos.len(), 1);
        
        // Check parameters
        assert_eq!(infos[0].params.len(), 1);
        if let Param::Other(name, Some(GenericValue::Token(value))) = &infos[0].params[0] {
            assert_eq!(name, "purpose");
            assert_eq!(value, "icon");
        } else {
            panic!("Expected purpose parameter with icon value");
        }
    }
    
    #[test]
    fn test_parse_call_info_multiple() {
        // Multiple info values
        let input = b"<http://www.example.com/alice/photo.jpg>;purpose=icon, <http://www.example.com/alice/>;purpose=info";
        let result = parse_call_info(input);
        assert!(result.is_ok());
        let (rem, infos) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(infos.len(), 2);
        
        // Check the first info
        assert_eq!(infos[0].params.len(), 1);
        if let Param::Other(name, Some(GenericValue::Token(value))) = &infos[0].params[0] {
            assert_eq!(name, "purpose");
            assert_eq!(value, "icon");
        } else {
            panic!("Expected purpose parameter with icon value");
        }
        
        // Check the second info
        assert_eq!(infos[1].params.len(), 1);
        if let Param::Other(name, Some(GenericValue::Token(value))) = &infos[1].params[0] {
            assert_eq!(name, "purpose");
            assert_eq!(value, "info");
        } else {
            panic!("Expected purpose parameter with info value");
        }
    }
    
    #[test]
    fn test_parse_call_info_different_schemes() {
        // Test different URI schemes
        let inputs = [
            b"<http://www.example.com/alice/photo.jpg>;purpose=icon".as_slice(),
            b"<https://secure.example.com/alice/photo.jpg>;purpose=icon".as_slice(),
            b"<sip:alice@example.com>;purpose=card".as_slice(),
            b"<sips:alice@secure.example.com>;purpose=card".as_slice(),
            b"<tel:+1-212-555-1234>;purpose=info".as_slice()
        ];
        
        for input in inputs.iter() {
            let result = parse_call_info(input);
            assert!(result.is_ok(), "Failed to parse: {}", String::from_utf8_lossy(input));
            let (rem, infos) = result.unwrap();
            assert!(rem.is_empty());
            assert_eq!(infos.len(), 1);
        }
    }
    
    #[test]
    fn test_parse_call_info_complex() {
        // Complex case with multiple info values and many parameters
        let input = b"<http://www.example.com/alice/photo.jpg>;purpose=icon;size=large, \
                     <http://www.example.com/alice/>;purpose=info;index=1;active, \
                     <sip:alice@example.com>;purpose=card;priority=high";
        let result = parse_call_info(input);
        assert!(result.is_ok());
        let (rem, infos) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(infos.len(), 3);
        
        // Check parameter counts for each info
        assert_eq!(infos[0].params.len(), 2);
        assert_eq!(infos[1].params.len(), 3);
        assert_eq!(infos[2].params.len(), 2);
    }
    
    #[test]
    fn test_parse_call_info_error_cases() {
        // Missing angle brackets
        let input = b"http://www.example.com/alice/photo.jpg;purpose=icon";
        let result = parse_call_info(input);
        assert!(result.is_err());
        
        // Missing closing angle bracket
        let input = b"<http://www.example.com/alice/photo.jpg;purpose=icon";
        let result = parse_call_info(input);
        assert!(result.is_err());
        
        // Empty URI
        let input = b"<>;purpose=icon";
        let result = parse_call_info(input);
        assert!(result.is_err());
        
        // No comma between info values
        let input = b"<http://example.com> <http://example.org>";
        let result = parse_call_info(input);
        assert!(result.is_err() || result.unwrap().0.len() > 0); // Should either fail or not consume all input
    }
} 
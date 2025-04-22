// Parser for the Call-Info header (RFC 3261 Section 20.9)
// Call-Info = "Call-Info" HCOLON info *(COMMA info)
// info = LAQUOT absoluteURI RAQUOT *( SEMI info-param)
// info-param = ( "purpose" EQUAL ( "icon" / "info" / "card" / token ) ) / generic-param

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case},
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
use crate::types::uri::Uri;
// use crate::types::call_info::{CallInfo as CallInfoHeader, CallInfoValue, InfoPurpose}; // Removed unused import
use serde::{Serialize, Deserialize};

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
             map_res( // Use map_res to handle potential UTF-8 error from absoluteURI bytes
                delimited(
                    crate::parser::separators::laquot,
                    crate::parser::uri::parse_absolute_uri, 
                    crate::parser::separators::raquot
                ),
                 |bytes| str::from_utf8(bytes).map(String::from)
            ),
            many0(preceded(semi, info_param))
        ),
        |(uri_str, params_vec)| -> Result<CallInfoValue, nom::error::Error<&str>> {
            // Parse URI string to extract necessary components
            // For example, http://www.example.com/path
            if let Some(scheme_end) = uri_str.find("://") {
                let scheme_str = &uri_str[0..scheme_end];
                let rest = &uri_str[scheme_end + 3..];
                
                // Split host and path
                let host_str = if let Some(path_start) = rest.find('/') {
                    &rest[0..path_start]
                } else {
                    rest
                };
                
                // Create appropriate Uri based on scheme
                let uri = match scheme_str {
                    "http" | "https" => {
                        // For http URLs, create a basic Uri with the host
                        Uri::new(
                            crate::types::uri::Scheme::Sip, // Just use SIP scheme for simplicity
                            crate::types::uri::Host::domain(host_str)
                        )
                    },
                    "sip" => Uri::sip(host_str),
                    "sips" => Uri::sips(host_str),
                    "tel" => {
                        // For tel URLs, the rest is the number
                        Uri::tel(rest)
                    },
                    _ => {
                        // Default to SIP
                        Uri::sip(host_str)
                    }
                };
                
                // Convert Vec<InfoParam> to Vec<Param>
                let params = params_vec.into_iter()
                    .map(convert_info_param_to_param)
                    .collect();
                    
                Ok(CallInfoValue { uri, params })
            } else {
                // If URI format is unexpected, create a default SIP URI
                Err(nom::error::Error::from_error_kind("invalid uri format", nom::error::ErrorKind::Verify))
            }
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

    #[test]
    fn test_info_param() {
        let (rem_p, param_p) = info_param(b"purpose=icon").unwrap();
        assert!(rem_p.is_empty());
        assert!(matches!(param_p, InfoParam::Purpose(InfoPurpose::Icon)));

        let (rem_g, param_g) = info_param(b"random=xyz").unwrap();
        assert!(rem_g.is_empty());
        assert!(matches!(param_g, InfoParam::Generic(Param::Other(n, Some(GenericValue::Token(v)))) if n=="random" && v=="xyz"));
    }
    
    #[test]
    fn test_parse_call_info() {
        let input = b"<http://www.example.com/alice/photo.jpg> ;purpose=icon, <http://www.example.com/alice/> ;purpose=info";
        let result = parse_call_info(input);
        assert!(result.is_ok());
        let (rem, infos) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(infos.len(), 2);
        
        // Updated assertions for URI type
        assert_eq!(infos[0].uri.scheme.as_str(), "http");
        assert_eq!(infos[0].uri.host.to_string(), "www.example.com");
        
        // Check the purpose parameter
        assert_eq!(infos[0].params.len(), 1);
        if let Param::Other(name, Some(GenericValue::Token(value))) = &infos[0].params[0] {
            assert_eq!(name, "purpose");
            assert_eq!(value, "icon");
        } else {
            panic!("Expected purpose parameter with icon value");
        }
        
        // Check the second info
        assert_eq!(infos[1].uri.scheme.as_str(), "http");
        assert_eq!(infos[1].uri.host.to_string(), "www.example.com");
        
        // Check the purpose parameter
        if let Param::Other(name, Some(GenericValue::Token(value))) = &infos[1].params[0] {
            assert_eq!(name, "purpose");
            assert_eq!(value, "info");
        } else {
            panic!("Expected purpose parameter with info value");
        }
    }
} 
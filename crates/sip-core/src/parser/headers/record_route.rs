// Parser for the Record-Route header (RFC 3261 Section 20.31)
// Record-Route = "Record-Route" HCOLON rec-route *(COMMA rec-route)
// rec-route = name-addr *( SEMI rr-param )
// rr-param = generic-param

use nom::{
    branch::alt,
    combinator::{map, map_res, opt},
    multi::{many0, separated_list1},
    sequence::{pair, preceded, tuple},
    IResult,
    error::{Error as NomError, ErrorKind, ParseError},
    bytes::complete::{tag, take_until, take_while},
    character::complete::char as nom_char,
};

// Import from base parser modules
use crate::parser::separators::{hcolon, comma};
use crate::parser::address::name_addr; // For reference
use crate::parser::common_params::{generic_param, semicolon_separated_params0};
use crate::parser::common::comma_separated_list1;
use crate::parser::ParseResult;
use crate::parser::quoted::quoted_string;
use crate::parser::token::token;
use crate::parser::uri::parse_uri;
use crate::parser::uri::params::uri_parameters;

use crate::types::param::Param;
use crate::types::uri::Uri;
use crate::types::address::Address;
use crate::types::record_route::{RecordRoute as RecordRouteHeader, RecordRouteEntry};
use serde::{Serialize, Deserialize};
use std::str::{self, FromStr};

// Helper to parse an optional display name
fn parse_display_name(input: &[u8]) -> ParseResult<Option<String>> {
    let (input, display_name_opt) = opt(
        alt((
            // Quoted string path
            map_res(quoted_string, |bytes| {
                str::from_utf8(bytes).map(|s| s.to_string())
            }),
            // Token path
            map_res(token, |bytes| {
                str::from_utf8(bytes).map(|s| s.to_string())
            })
        ))
    )(input)?;
    
    // If we found a display name, look for whitespace followed by '<'
    if let Some(name) = &display_name_opt {
        // Check if this is actually a display name by looking for a following '<'
        // First skip any whitespace
        let (input_after_ws, _) = take_while(|c: u8| c == b' ' || c == b'\t')(input)?;
        
        // If there's no '<' after possible whitespace, this might not be a display name
        if input_after_ws.is_empty() || input_after_ws[0] != b'<' {
            return Ok((input, None));
        }
    }
    
    Ok((input, display_name_opt))
}

// Parse a URI with parameters inside angle brackets
fn parse_uri_with_params(input: &[u8]) -> ParseResult<(Uri, Vec<Param>)> {
    // Check for opening angle bracket
    let (input, _) = tag(b"<")(input)?;
    
    // Find the position of the closing angle bracket
    let closing_bracket_pos = input.iter()
        .position(|&c| c == b'>')
        .ok_or_else(|| nom::Err::Error(NomError::new(input, ErrorKind::Tag)))?;
    
    // Extract content inside angle brackets
    let uri_part = &input[..closing_bracket_pos];
    let input_after_uri = &input[closing_bracket_pos..];
    
    // Find position of semicolon (if any) - this separates URI from its parameters
    let semicolon_pos = uri_part.iter().position(|&c| c == b';');
    
    // Parse the URI
    let uri;
    let params;
    
    if let Some(pos) = semicolon_pos {
        // URI has parameters
        let (_, parsed_uri) = parse_uri(&uri_part[..pos])?;
        let (_, parsed_params) = uri_parameters(&uri_part[pos..])?;
        
        uri = parsed_uri;
        params = parsed_params;
    } else {
        // URI without parameters
        let (_, parsed_uri) = parse_uri(uri_part)?;
        uri = parsed_uri;
        params = Vec::new();
    }
    
    // Consume the closing bracket
    let (input, _) = tag(b">")(input_after_uri)?;
    
    Ok((input, (uri, params)))
}

// Parse a single record-route entry
fn parse_record_route_address(input: &[u8]) -> ParseResult<Address> {
    // Try to parse a display name
    let (input, display_name) = parse_display_name(input)?;
    
    // Skip whitespace after display name
    let (input, _) = take_while(|c: u8| c == b' ' || c == b'\t')(input)?;
    
    // Parse the URI with parameters inside angle brackets
    let (input, (uri, params)) = parse_uri_with_params(input)?;
    
    Ok((input, Address {
        display_name,
        uri,
        params,
    }))
}

/// Parse a Record-Route header value as defined in RFC 3261 Section 20.31
/// Record-Route = "Record-Route" HCOLON rec-route *(COMMA rec-route)
pub fn parse_record_route(input: &[u8]) -> ParseResult<RecordRouteHeader> {
    map(
        comma_separated_list1(parse_record_route_address),
        |addresses: Vec<Address>| {
            // Convert each Address to a RecordRouteEntry
            let entries = addresses.into_iter()
                .map(RecordRouteEntry)
                .collect();
            
            RecordRouteHeader(entries)
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::param::Param;
    use crate::types::uri::{Uri, Scheme, Host};

    #[test]
    fn test_parse_record_route_single() {
        let input = b"<sip:ss1.example.com;lr>";
        let result = parse_record_route(input);
        assert!(result.is_ok());
        let (rem, rr_header) = result.unwrap();
        let routes = rr_header.0;
        assert!(rem.is_empty());
        assert_eq!(routes.len(), 1);
        assert!(routes[0].0.display_name.is_none());
        assert_eq!(routes[0].0.uri.scheme, Scheme::Sip);
        assert_eq!(routes[0].0.params.len(), 1);
        assert!(routes[0].0.params.contains(&Param::Lr));
    }
    
    #[test]
    fn test_parse_record_route_multiple() {
        let input = b"<sip:ss1.example.com;lr>, <sip:p2.example.com;lr>";
        let result = parse_record_route(input);
        assert!(result.is_ok());
        let (rem, rr_header) = result.unwrap();
        let routes = rr_header.0;
        assert!(rem.is_empty());
        assert_eq!(routes.len(), 2);
        assert!(routes[0].0.params.contains(&Param::Lr));
        assert!(routes[1].0.params.contains(&Param::Lr));
    }

    #[test]
    fn test_parse_record_route_with_display_name() {
        let input = b"\"Service Server\" <sip:ss1.example.com;lr>";
        let result = parse_record_route(input);
        assert!(result.is_ok());
        let (rem, rr_header) = result.unwrap();
        let routes = rr_header.0;
        assert!(rem.is_empty());
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].0.display_name, Some("Service Server".to_string()));
        assert_eq!(routes[0].0.uri.scheme, Scheme::Sip);
    }

    #[test]
    fn test_parse_record_route_with_multiple_params() {
        let input = b"<sip:ss1.example.com;lr;transport=tcp>";
        let result = parse_record_route(input);
        assert!(result.is_ok());
        let (rem, rr_header) = result.unwrap();
        let routes = rr_header.0;
        assert!(rem.is_empty());
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].0.params.len(), 2);
        assert!(routes[0].0.params.contains(&Param::Lr));
        assert!(routes[0].0.params.contains(&Param::Transport("tcp".to_string())));
    }

    #[test]
    fn test_parse_record_route_with_sips_uri() {
        let input = b"<sips:secure.example.com;lr>";
        let result = parse_record_route(input);
        assert!(result.is_ok());
        let (rem, rr_header) = result.unwrap();
        let routes = rr_header.0;
        assert!(rem.is_empty());
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].0.uri.scheme, Scheme::Sips);
    }

    #[test]
    fn test_parse_record_route_complex() {
        let input = b"\"Gateway\" <sip:gw.example.com:5061;lr;transport=tcp>, \
                      <sip:proxy.example.org;maddr=10.0.1.1>";
        let result = parse_record_route(input);
        assert!(result.is_ok());
        let (rem, rr_header) = result.unwrap();
        let routes = rr_header.0;
        assert!(rem.is_empty());
        assert_eq!(routes.len(), 2);
        
        // First entry
        assert_eq!(routes[0].0.display_name, Some("Gateway".to_string()));
        assert_eq!(routes[0].0.uri.scheme, Scheme::Sip);
        assert_eq!(routes[0].0.uri.port, Some(5061));
        assert!(routes[0].0.params.contains(&Param::Lr));
        assert!(routes[0].0.params.contains(&Param::Transport("tcp".to_string())));
        
        // Second entry
        assert!(routes[1].0.display_name.is_none());
        assert_eq!(routes[1].0.uri.scheme, Scheme::Sip);
        assert_eq!(routes[1].0.uri.host, Host::Domain("proxy.example.org".to_string()));
        assert!(routes[1].0.params.contains(&Param::Maddr("10.0.1.1".to_string())));
    }

    #[test]
    fn test_parse_record_route_empty_should_fail() {
        let input = b"";
        let result = parse_record_route(input);
        assert!(result.is_err());
    }
} 
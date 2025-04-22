// Parser for the Route header (RFC 3261 Section 20.34)

use nom::{
    branch::alt,
    combinator::map,
    multi::{many0, separated_list1},
    sequence::{pair, preceded},
    IResult,
};

// Import from base parser modules
use crate::parser::separators::{hcolon, comma};
use crate::parser::address::name_addr; // Route uses name-addr strictly
use crate::parser::common_params::{generic_param, semicolon_separated_params0};
use crate::parser::common::comma_separated_list1; // Route requires at least one
use crate::parser::ParseResult;

use crate::types::param::Param;
use crate::types::uri::Uri;

// Import types (assuming)
use crate::types::address::Address;
use crate::types::route::Route as RouteHeader; // Use the specific header type
use crate::types::uri_with_params_list::UriWithParamsList;
use crate::types::uri_with_params::UriWithParams;
use crate::parser::parse_address;
use serde::{Serialize, Deserialize}; // Added serde
use crate::parser::address::name_addr_or_addr_spec;

/// Represents a single route entry (typically name-addr or addr-spec)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] // Added Serialize, Deserialize
pub struct RouteEntry(pub Address);

// route-param = name-addr *( SEMI rr-param )
// rr-param = generic-param
fn route_param(input: &[u8]) -> ParseResult<Address> {
    map(
        pair(
            name_addr, // Requires name-addr format (must have < >)
            semicolon_separated_params0(generic_param) // rr-param = generic-param
        ),
        |(mut addr, params_vec)| {
            addr.params = params_vec;
            addr
        }
    )(input)
}

// Define a simple function that just calls parse_address, so it implements Copy
fn parse_route_address(input: &[u8]) -> ParseResult<Address> {
    parse_address(input)
}

// route = 1#("<" addr-spec ">" *( SEMI route-param ))
pub fn parse_route(input: &[u8]) -> ParseResult<RouteHeader> {
    map(
        comma_separated_list1(parse_route_address),
        RouteHeader // Use the imported alias
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::address::{Address};
    use crate::types::param::{Param, GenericValue};
    use crate::types::uri::Uri;

    #[test]
    fn test_parse_route_single() {
        let input = b"<sip:ss1.example.com;lr>";
        let result = parse_route(input);
        assert!(result.is_ok());
        let (rem, route_header) = result.unwrap(); // Returns RouteHeader
        let routes = route_header.0; // Access inner Vec
        assert!(rem.is_empty());
        assert_eq!(routes.len(), 1);
        assert!(routes[0].address.display_name.is_none());
        assert_eq!(routes[0].address.uri.scheme, "sip");
        assert_eq!(routes[0].address.params.len(), 1);
        assert!(matches!(routes[0].address.params[0], Param::Other(ref n, None) if n == "lr"));
    }
    
    #[test]
    fn test_parse_route_multiple() {
        let input = b"<sip:ss1.example.com;lr>, <sip:ss2.example.com;lr>";
        let result = parse_route(input);
        assert!(result.is_ok());
        let (rem, route_header) = result.unwrap(); // Returns RouteHeader
        let routes = route_header.0; // Access inner Vec
        assert!(rem.is_empty());
        assert_eq!(routes.len(), 2);
        assert!(routes[0].address.params.contains(&Param::Other("lr".to_string(), None)));
        assert!(routes[1].address.params.contains(&Param::Other("lr".to_string(), None)));
    }

    #[test]
    fn test_parse_route_with_display_name() {
        // Although unusual for Route, technically allowed by name-addr
        let input = b"\"Proxy 1\" <sip:p1.example.com;lr>";
        let result = parse_route(input);
        assert!(result.is_ok());
        let (rem, route_header) = result.unwrap(); // Returns RouteHeader
        let routes = route_header.0; // Access inner Vec
        assert!(rem.is_empty());
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].address.display_name, Some("Proxy 1".to_string()));
        assert!(routes[0].address.params.contains(&Param::Other("lr".to_string(), None)));
    }

     #[test]
    fn test_parse_route_addr_spec_fail() {
        // Should fail because Route requires name-addr (with <>)
        let input = b"sip:ss1.example.com;lr";
        let result = parse_route(input);
        assert!(result.is_err());
    }
} 
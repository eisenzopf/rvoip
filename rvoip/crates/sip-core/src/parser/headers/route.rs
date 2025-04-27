// Parser for the Route header (RFC 3261 Section 20.34)
// Route        =  "Route" HCOLON route-param *(COMMA route-param)
// route-param  =  name-addr *( SEMI rr-param )
// rr-param     =  generic-param

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
use crate::parser::parse_address;
use serde::{Serialize, Deserialize}; // Added serde
use crate::parser::address::name_addr_or_addr_spec;

/// Represents a single route entry (typically name-addr or addr-spec)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] // Added Serialize, Deserialize
pub struct RouteEntry(pub Address);

impl std::fmt::Display for RouteEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

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

// Route = "Route" HCOLON route-param *(COMMA route-param)
pub fn parse_route(input: &[u8]) -> ParseResult<RouteHeader> {
    map(
        comma_separated_list1(route_param), // Use route_param which strictly enforces name-addr format
        |addresses: Vec<Address>| {
            // Convert Vec<Address> to Vec<RouteEntry>
            let entries = addresses.into_iter()
                .map(RouteEntry)
                .collect();
            
            RouteHeader(entries)
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::param::Param;
    use crate::types::uri::{Uri, Scheme};

    #[test]
    fn test_parse_route_single() {
        let input = b"<sip:ss1.example.com;lr>";
        let result = parse_route(input);
        assert!(result.is_ok());
        let (rem, route_header) = result.unwrap(); // Returns RouteHeader
        let routes = route_header.0; // Access inner Vec
        assert!(rem.is_empty());
        assert_eq!(routes.len(), 1);
        assert!(routes[0].0.display_name.is_none());
        assert_eq!(routes[0].0.uri.scheme, Scheme::Sip);
        assert!(routes[0].0.params.is_empty());
        
        // The 'lr' parameter is now stored in the URI parameters, not in the address params
        assert!(routes[0].0.uri.parameters.iter().any(|p| matches!(p, Param::Lr)));
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
        
        // The 'lr' parameters are now stored in the URI parameters, not in the address params
        assert!(routes[0].0.uri.parameters.iter().any(|p| matches!(p, Param::Lr)));
        assert!(routes[1].0.uri.parameters.iter().any(|p| matches!(p, Param::Lr)));
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
        assert_eq!(routes[0].0.display_name, Some("Proxy 1".to_string()));
        
        // The 'lr' parameter is now stored in the URI parameters, not in the address params
        assert!(routes[0].0.uri.parameters.iter().any(|p| matches!(p, Param::Lr)));
    }

    #[test]
    fn test_parse_route_addr_spec_fail() {
        // Should fail because Route requires name-addr (with <>)
        let input = b"sip:ss1.example.com;lr";
        let result = parse_route(input);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_parse_route_with_sips_uri() {
        let input = b"<sips:secure.example.com;lr>";
        let (_, route_header) = parse_route(input).unwrap();
        assert_eq!(route_header.0[0].0.uri.scheme, Scheme::Sips);
    }

    #[test]
    fn test_parse_route_with_multiple_params() {
        let input = b"<sip:proxy.example.com;lr;transport=tcp;ttl=15>";
        let (_, route_header) = parse_route(input).unwrap();
        let params = &route_header.0[0].0.uri.parameters;
        assert!(params.iter().any(|p| matches!(p, Param::Lr)));
        assert!(params.iter().any(|p| matches!(p, Param::Transport(s) if s == "tcp")));
        assert!(params.iter().any(|p| matches!(p, Param::Ttl(n) if *n == 15)));
    }

    #[test]
    fn test_parse_route_with_escaped_display_name() {
        let input = b"\"John \\\"The Proxy\\\" Smith\" <sip:proxy.example.com;lr>";
        let (_, route_header) = parse_route(input).unwrap();
        assert_eq!(route_header.0[0].0.display_name, Some("John \"The Proxy\" Smith".to_string()));
    }

    #[test]
    fn test_parse_route_malformed_inputs() {
        // Missing closing angle bracket
        assert!(parse_route(b"<sip:proxy.example.com").is_err());
        
        // Empty route
        assert!(parse_route(b"").is_err());
        
        // Invalid URI
        assert!(parse_route(b"<invalid:uri>").is_err());
    }
    
    #[test]
    fn test_parse_route_with_route_params() {
        // Parameters after the URI (route parameters) should be attached to the Address, not the URI
        let input = b"<sip:proxy.example.com>;priority=high;ttl=16";
        let (_, route_header) = parse_route(input).unwrap();
        
        // Check that URI has no parameters
        assert!(route_header.0[0].0.uri.parameters.is_empty());
        
        // Check that Address params contain the route parameters
        let params = &route_header.0[0].0.params;
        assert_eq!(params.len(), 2);
        assert!(params.iter().any(|p| matches!(p, Param::Other(k, Some(v)) if k == "priority" && v.as_str() == Some("high"))));
        // Ttl parameter may be stored as either a typed Ttl parameter or as a generic parameter
        assert!(params.iter().any(|p| matches!(p, Param::Ttl(16)) || 
                                    matches!(p, Param::Other(k, Some(v)) if k == "ttl" && v.as_str() == Some("16"))));
    }
} 
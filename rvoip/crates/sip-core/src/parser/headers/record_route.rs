// Parser for the Record-Route header (RFC 3261 Section 20.31)
// Record-Route = "Record-Route" HCOLON rec-route *(COMMA rec-route)
// rec-route = name-addr *( SEMI rr-param )
// rr-param = generic-param

use nom::{
    branch::alt,
    combinator::{map, map_res},
    multi::{many0, separated_list1},
    sequence::{pair, preceded},
    IResult,
    error::{Error as NomError, ErrorKind, ParseError},
};

// Import from base parser modules
use crate::parser::separators::{hcolon, comma};
use crate::parser::address::name_addr; // Record-Route uses name-addr strictly
use crate::parser::common_params::{generic_param, semicolon_separated_params0};
use crate::parser::common::comma_separated_list1; // Changed from list0
use crate::parser::ParseResult;

use crate::types::param::Param;
use crate::types::uri::Uri;
use crate::types::address::Address; // Use Address directly
// use crate::types::record_route::RecordRouteInfo; // Removed, seems unused
use crate::types::record_route::RecordRoute as RecordRouteHeader; // Import specific type
use crate::types::uri_with_params::UriWithParams; // Added
use crate::types::uri_with_params_list::UriWithParamsList; // Added
use serde::{Serialize, Deserialize}; // Added serde
use crate::parser::parse_address;

/// Represents a single record-route entry (typically name-addr)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] // Added Serialize, Deserialize
pub struct RecordRouteEntry(pub Address);

// Define a simple function that just calls parse_address, so it implements Copy
fn parse_record_route_address(input: &[u8]) -> ParseResult<Address> {
    parse_address(input)
}

// Record-Route = "Record-Route" HCOLON rec-route *(COMMA rec-route)
pub fn parse_record_route(input: &[u8]) -> ParseResult<RecordRouteHeader> {
    map(
        comma_separated_list1(parse_record_route_address), // Returns Vec<Address>
        |addresses: Vec<Address>| {
            // Convert each Address to a RecordRouteEntry
            let entries = addresses.into_iter()
                .map(|addr| RecordRouteEntry(addr))
                .collect();
            
            // Return the RecordRouteHeader with the Vec<RecordRouteEntry>
            RecordRouteHeader(entries)
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::address::Address;
    use crate::types::param::{Param, GenericValue};
    use crate::types::uri::Uri;

    #[test]
    fn test_parse_record_route_single() {
        let input = b"<sip:ss1.example.com;lr>";
        let result = parse_record_route(input);
        assert!(result.is_ok());
        let (rem, rr_header) = result.unwrap(); // Returns RecordRouteHeader
        let routes = rr_header.0; // Access inner Vec
        assert!(rem.is_empty());
        assert_eq!(routes.len(), 1);
        assert!(routes[0].address.display_name.is_none());
        assert_eq!(routes[0].address.uri.scheme, "sip");
        assert_eq!(routes[0].address.params.len(), 1);
        assert!(matches!(routes[0].address.params[0], Param::Other(ref n, None) if n == "lr"));
    }
    
    #[test]
    fn test_parse_record_route_multiple() {
        let input = b"<sip:ss1.example.com;lr>, <sip:p2.example.com;lr>";
        let result = parse_record_route(input);
        assert!(result.is_ok());
        let (rem, routes) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(routes.len(), 2);
        assert!(routes[1].params.contains(&Param::Other("lr".to_string(), None)));
    }
} 
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
use crate::uri::Uri;
use crate::types::address::Address; // Use Address directly
// use crate::types::record_route::RecordRouteInfo; // Removed, seems unused
use crate::types::record_route::RecordRoute as RecordRouteHeader; // Import specific type
use crate::types::uri_with_params::UriWithParams; // Added
use crate::types::uri_with_params_list::UriWithParamsList; // Added
use serde::{Serialize, Deserialize}; // Added serde

/// Represents a single record-route entry (typically name-addr)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] // Added Serialize, Deserialize
pub struct RecordRouteEntry(pub Address);

// rec-route = name-addr *( SEMI rr-param )
// rr-param = generic-param
// Changed return type to ParseResult<Address>
fn record_route_entry(input: &[u8]) -> ParseResult<Address> {
     map_res(
        pair(
            name_addr, // name_addr returns (Option<display_name_bytes>, Uri)
            semicolon_separated_params0(generic_param) // rr-params are generic params
        ),
        |((dn_bytes_opt, uri), params)| {
            // Convert display name bytes, mapping Utf8Error to nom::Err::Failure
             let display_name = dn_bytes_opt
                .map(|b| std::str::from_utf8(b).map(|s| s.to_string()))
                .transpose()
                .map_err(|_| nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Char)))?; // Correct error mapping

            // Return Address directly, ensuring the Ok variant matches the map_res expectation
            // The error type E in Result<O, E> for map_res needs to be convertible From<Utf8Error>
            // We handle Utf8Error above, so we just need to return Ok(Address)
            Ok(Address { display_name, uri, params })
        }
    )(input)
}

// Record-Route = "Record-Route" HCOLON rec-route *(COMMA rec-route)
pub fn parse_record_route(input: &[u8]) -> ParseResult<RecordRouteHeader> {
    map(
        comma_separated_list1(record_route_entry), // Now returns Vec<Address>
        |entries: Vec<Address>| { // Changed input type to Vec<Address>
            let uris: Vec<UriWithParams> = entries
                .into_iter()
                .map(|addr| UriWithParams { uri: addr.uri, params: addr.params }) // Convert each Address
                .collect();
            RecordRouteHeader(UriWithParamsList { uris }) // Construct the final type
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
// Parser for the Record-Route header (RFC 3261 Section 20.31)
// Record-Route = "Record-Route" HCOLON rec-route *(COMMA rec-route)
// rec-route = name-addr *( SEMI rr-param )
// rr-param = generic-param

use nom::{
    branch::alt,
    combinator::map,
    multi::{many0, separated_list1},
    sequence::{pair, preceded},
    IResult,
};

// Import from base parser modules
use crate::parser::separators::{hcolon, comma};
use crate::parser::address::name_addr; // Record-Route uses name-addr strictly
use crate::parser::common_params::{generic_param, semicolon_separated_params0};
use crate::parser::common::comma_separated_list0; // Record-Route can be empty
use crate::parser::ParseResult;

use crate::types::param::Param;
use crate::uri::Uri;
use crate::types::address::Address;
use crate::types::record_route::RecordRouteInfo;
use crate::types::record_route::RecordRoute as RecordRouteHeader; // Import specific type

// Define a struct to represent a single Record-Route entry (same as RouteEntry)
#[derive(Debug, PartialEq, Clone)]
pub struct RecordRouteEntry {
    pub display_name: Option<String>,
    pub uri: Uri,
    pub params: Vec<Param>,
}

// rec-route = name-addr *( SEMI rr-param )
// rr-param = generic-param
fn record_route_entry(input: &[u8]) -> ParseResult<RecordRouteEntry> {
     map_res(
        pair(
            name_addr, // name_addr returns (Option<display_name_bytes>, Uri)
            semicolon_separated_params0(generic_param) // rr-params are generic params
        ),
        |((dn_bytes_opt, uri), params)| {
            // Convert display name bytes
            // TODO: Ensure address parser handles potential quoting/unescaping
             let display_name = dn_bytes_opt
                .map(|b| std::str::from_utf8(b).map(|s| s.to_string()))
                .transpose()?;

            Ok(RecordRouteEntry { display_name, uri, params })
        }
    )(input)
}

// Record-Route = "Record-Route" HCOLON rec-route *(COMMA rec-route)
pub(crate) fn parse_record_route(input: &[u8]) -> ParseResult<RecordRouteHeader> {
    map(comma_separated_list1(rec_route), RecordRouteHeader)(input)
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
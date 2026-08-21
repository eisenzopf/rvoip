// Parser for the Record-Route header (RFC 3261 Section 20.31)
// Record-Route = "Record-Route" HCOLON rec-route *(COMMA rec-route)
// rec-route = name-addr *( SEMI rr-param )
// rr-param = generic-param

use nom::{combinator::map, sequence::pair};

// Import from base parser modules
// For reference
use crate::parser::address::name_addr;
use crate::parser::common::comma_separated_list1;
use crate::parser::common_params::{generic_param, semicolon_separated_params0};
use crate::parser::ParseResult;

use crate::types::address::Address;
use crate::types::record_route::{RecordRoute as RecordRouteHeader, RecordRouteEntry};

// Parse a single record-route entry
fn parse_record_route_address(input: &[u8]) -> ParseResult<'_, Address> {
    map(
        pair(name_addr, semicolon_separated_params0(generic_param)),
        |(mut address, params)| {
            address.params = params;
            address
        },
    )(input)
}

/// Parse a Record-Route header value as defined in RFC 3261 Section 20.31
/// Record-Route = "Record-Route" HCOLON rec-route *(COMMA rec-route)
pub fn parse_record_route(input: &[u8]) -> ParseResult<'_, RecordRouteHeader> {
    map(
        comma_separated_list1(parse_record_route_address),
        |addresses: Vec<Address>| {
            // Convert each Address to a RecordRouteEntry
            let entries = addresses.into_iter().map(RecordRouteEntry).collect();

            RecordRouteHeader(entries)
        },
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::param::Param;
    use crate::types::uri::{Host, Scheme};

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
        assert!(routes[0].0.params.is_empty());
        assert!(routes[0].0.uri.parameters.contains(&Param::Lr));
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
        assert!(routes[0].0.uri.parameters.contains(&Param::Lr));
        assert!(routes[1].0.uri.parameters.contains(&Param::Lr));
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
        assert!(routes[0].0.params.is_empty());
        assert!(routes[0].0.uri.parameters.contains(&Param::Lr));
        assert!(routes[0]
            .0
            .uri
            .parameters
            .contains(&Param::Transport("tcp".to_string())));
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
        assert!(routes[0].0.uri.parameters.contains(&Param::Lr));
        assert!(routes[0]
            .0
            .uri
            .parameters
            .contains(&Param::Transport("tcp".to_string())));

        // Second entry
        assert!(routes[1].0.display_name.is_none());
        assert_eq!(routes[1].0.uri.scheme, Scheme::Sip);
        assert_eq!(
            routes[1].0.uri.host,
            Host::Domain("proxy.example.org".to_string())
        );
        assert!(routes[1]
            .0
            .uri
            .parameters
            .contains(&Param::Maddr("10.0.1.1".to_string())));
    }

    #[test]
    fn record_route_uri_parameters_survive_round_trip() {
        let raw = "<sip:proxy.example.com:5060;transport=tcp;lr;esp=abc>";
        let record_route: RecordRouteHeader = raw.parse().unwrap();

        assert_eq!(record_route.to_string(), raw);
        assert!(record_route[0].is_loose_routing());
    }

    #[test]
    fn record_route_separates_uri_and_header_parameters() {
        let raw = "<sip:proxy.example.com;lr;esp=abc>;ftag=xyz";
        let record_route: RecordRouteHeader = raw.parse().unwrap();
        let entry = &record_route[0];

        assert!(entry.uri().parameters.contains(&Param::Lr));
        assert!(entry
            .uri()
            .parameters
            .iter()
            .any(|param| matches!(param, Param::Other(name, _) if name == "esp")));
        assert_eq!(entry.address().params.len(), 1);
        assert!(matches!(
            &entry.address().params[0],
            Param::Other(name, _) if name == "ftag"
        ));
        assert_eq!(record_route.to_string(), raw);
    }

    #[test]
    fn test_parse_record_route_empty_should_fail() {
        let input = b"";
        let result = parse_record_route(input);
        assert!(result.is_err());
    }
}

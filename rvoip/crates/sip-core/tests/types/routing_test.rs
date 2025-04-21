// Tests for Routing related types (Route, RecordRoute, ReplyTo)
use crate::common::{uri, addr, param_lr, param_transport, assert_parses_ok, assert_parse_fails, assert_display_parses_back};
use rvoip_sip_core::types::{Address, Param};
use rvoip_sip_core::types::route::Route;
use rvoip_sip_core::types::record_route::RecordRoute;
use rvoip_sip_core::types::reply_to::ReplyTo;
use rvoip_sip_core::types::uri_with_params::{UriWithParams};
use rvoip_sip_core::types::uri_with_params_list::UriWithParamsList;
use rvoip_sip_core::uri::{Uri, Scheme, Host};
use std::str::FromStr;


#[test]
fn test_routing_display_parse_roundtrip() {
    // Test Route
    let uri1 = uri("sip:p1.example.com").with_parameter(param_lr());
    let uri2 = uri("sip:p2.example.com").with_parameter(param_transport("tcp"));
    let route1 = Route(UriWithParamsList {
        uris: vec![
            UriWithParams { uri: uri1, params: vec![] }, // Params belong to URI
            UriWithParams { uri: uri2, params: vec![] }, // Params belong to URI
        ]
    });
    assert_display_parses_back(&route1);

    // Test RecordRoute (similar structure)
    let rr_uri1 = uri("sip:proxy1.example.com").with_parameter(param_lr());
    let rr_uri2 = uri("sip:proxy2.example.com");
    let record_route1 = RecordRoute(UriWithParamsList {
        uris: vec![
            UriWithParams { uri: rr_uri1, params: vec![] },
            UriWithParams { uri: rr_uri2, params: vec![] },
        ]
    });
     assert_display_parses_back(&record_route1);

    let reply_to = ReplyTo(addr(Some("Reply Person"), "sip:reply@host.com", vec![]));
    // assert_eq!(reply_to.to_string(), "\"Reply Person\" <sip:reply@host.com>");
    assert_display_parses_back(&reply_to);
}

#[test]
fn test_routing_from_str() {
     // Test Route parsing
     let route_str = "<sip:p1.example.com;lr>, sip:p2.example.com;transport=tcp";
     let expected_uri1 = uri("sip:p1.example.com").with_parameter(param_lr());
     let expected_uri2 = uri("sip:p2.example.com").with_parameter(param_transport("tcp"));
     let expected_route = Route(UriWithParamsList {
         uris: vec![
             UriWithParams { uri: expected_uri1, params: vec![] },
             UriWithParams { uri: expected_uri2, params: vec![] },
         ]
     });
    assert_parses_ok(route_str, expected_route);

    // Test RecordRoute parsing
    let rr_str = "<sip:proxy1.com;lr>";
    let expected_rr_uri = uri("sip:proxy1.com").with_parameter(param_lr());
    let expected_rr = RecordRoute(UriWithParamsList {
        uris: vec![UriWithParams { uri: expected_rr_uri, params: vec![] }]
    });
    assert_parses_ok(rr_str, expected_rr);
    
    assert_parse_fails::<Route>("<");
    assert_parse_fails::<RecordRoute>(",");
    assert_parse_fails::<ReplyTo>("Name Only");
} 
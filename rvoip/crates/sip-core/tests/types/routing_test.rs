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
    let uri1 = UriWithParams { uri: uri("sip:p1.example.com"), params: vec![param_lr()] };
    let uri2 = UriWithParams { uri: uri("sip:p2.example.com"), params: vec![param_transport("tcp")] };
    let list = UriWithParamsList { uris: vec![uri1.clone(), uri2.clone()] };

    let route = Route(list.clone());
    // assert_eq!(route.to_string(), "<sip:p1@example.com>;lr, <sip:p2@example.com>;transport=tcp"); // Uri Display needs <>?
    assert_display_parses_back(&route);

    let record_route = RecordRoute(list);
    // assert_eq!(record_route.to_string(), "<sip:p1@example.com>;lr, <sip:p2@example.com>;transport=tcp");
    assert_display_parses_back(&record_route);

    let reply_to = ReplyTo(addr(Some("Reply Person"), "sip:reply@host.com", vec![]));
    // assert_eq!(reply_to.to_string(), "\"Reply Person\" <sip:reply@host.com>");
    assert_display_parses_back(&reply_to);
}

#[test]
fn test_routing_from_str() {
    let route_str = "<sip:p1.example.com;lr>, sip:p2.example.com;transport=tcp";
    let uri1 = UriWithParams { uri: uri("sip:p1.example.com"), params: vec![param_lr()] };
    let uri2 = UriWithParams { uri: uri("sip:p2.example.com"), params: vec![param_transport("tcp")] };
    assert_parses_ok(route_str, Route(UriWithParamsList { uris: vec![uri1.clone(), uri2.clone()] }));

    let rr_str = "<sip:rec1@host.net;lr>";
    let rr_uri = UriWithParams { uri: uri("sip:rec1@host.net"), params: vec![param_lr()] };
    assert_parses_ok(rr_str, RecordRoute(UriWithParamsList { uris: vec![rr_uri] }));
    
    let reply_to_str = "\"Bob\" <sip:bob@biloxi.com>";
    assert_parses_ok(reply_to_str, ReplyTo(addr(Some("Bob"), "sip:bob@biloxi.com", vec![])));

    assert_parse_fails::<Route>(",");
    assert_parse_fails::<RecordRoute>("<");
    assert_parse_fails::<ReplyTo>("Name Only");
} 
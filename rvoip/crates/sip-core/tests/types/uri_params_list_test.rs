// Tests for UriWithParamsList type and its use in Route/RecordRoute

use rvoip_sip_core::types::{Param, Address};
use rvoip_sip_core::types::uri_with_params::{UriWithParams};
use rvoip_sip_core::types::uri_with_params_list::{UriWithParamsList};
use rvoip_sip_core::types::route::Route;
use rvoip_sip_core::types::record_route::RecordRoute;
use rvoip_sip_core::uri::{Uri, Host};
use crate::common::{uri, param_lr, param_transport};
use std::str::FromStr;

#[test]
fn test_uri_with_params_list_helpers() {
    let uri1_parsed = Uri::from_str("sip:p1@example.com")
                         .expect("Failed to parse uri1");
    let uri1 = UriWithParams { uri: uri1_parsed, params: vec![param_lr()] };
    let uri2 = UriWithParams { uri: uri("sip:p2@example.com"), params: vec![param_transport("tcp")] };
    
    let mut list = UriWithParamsList::new();
    assert!(list.is_empty());
    assert_eq!(list.len(), 0);
    assert!(list.first().is_none());
    
    list.push(uri1.clone());
    assert!(!list.is_empty());
    assert_eq!(list.len(), 1);
    assert_eq!(list.first(), Some(&uri1));
    assert_eq!(list.last(), Some(&uri1));
    
    list.push(uri2.clone());
     assert_eq!(list.len(), 2);
     assert_eq!(list.first(), Some(&uri1));
     assert_eq!(list.last(), Some(&uri2));
     
     let mut count = 0;
     for item in list.iter() {
         count += 1;
     }
     assert_eq!(count, 2);
     
     // Test Route/RecordRoute delegation via Deref
     let route = Route(list.clone());
     assert_eq!(route.len(), 2);
     println!("Debugging route.first(): {:?}", route.first()); 
     let expected_host1 = Host::Domain("example.com".to_string());
     assert_eq!(route.first().unwrap().uri.host, expected_host1);
     
     let rr = RecordRoute(list);
      assert_eq!(rr.len(), 2);
      let expected_host2 = Host::Domain("example.com".to_string());
      assert_eq!(rr.last().unwrap().uri.host, expected_host2);
} 

#[test]
fn debug_host_parsing() {
    // Use common::uri which calls Uri::from_str
    let uri_str = "sip:p1@example.com";
    let parsed_uri = uri(uri_str); 
    println!("Parsed URI for {}: {:?}", uri_str, parsed_uri);
    // Correct the expected host
    assert_eq!(parsed_uri.host, Host::Domain("example.com".to_string()));

    let uri_str_2 = "sip:p2@example.com";
    let parsed_uri_2 = uri(uri_str_2);
    println!("Parsed URI for {}: {:?}", uri_str_2, parsed_uri_2);
    // Correct the expected host
     assert_eq!(parsed_uri_2.host, Host::Domain("example.com".to_string()));

     // Test without user part
     let uri_str_3 = "sip:example.com";
     let parsed_uri_3 = uri(uri_str_3);
     println!("Parsed URI for {}: {:?}", uri_str_3, parsed_uri_3);
     assert_eq!(parsed_uri_3.host, Host::Domain("example.com".to_string()));
} 
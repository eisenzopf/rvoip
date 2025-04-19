// Tests for UriWithParamsList and related types (Route, RecordRoute)

use crate::common::{uri, param_lr, param_transport};
use rvoip_sip_core::types::{UriWithParams, UriWithParamsList, Route, RecordRoute};

#[test]
fn test_uri_with_params_list_helpers() {
    let uri1 = UriWithParams { uri: uri("sip:p1@example.com"), params: vec![param_lr()] };
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
     assert_eq!(route.first().unwrap().uri.host.to_string(), "p1.example.com");
     
     let rr = RecordRoute(list);
      assert_eq!(rr.len(), 2);
      assert_eq!(rr.last().unwrap().uri.host.to_string(), "p2.example.com");
} 
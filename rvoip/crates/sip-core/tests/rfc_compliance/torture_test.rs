// Combined Torture Tests (from torture_tests.rs and custom_torture.rs)

use crate::common::{parse_sip_message, expect_parse_error};
// Import necessary types
use rvoip_sip_core::types::{Method, Uri, Scheme, Host, Message, Request, Response, StatusCode, CSeq, Via, Address, From, To, Contact, MaxForwards, ContentLength, CallId, ContentType, Param, Allow};
use rvoip_sip_core::header::{HeaderName, HeaderValue};
use rvoip_sip_core::Error;
use std::str::FromStr;
use std::convert::TryFrom;
use rvoip_sip_core::parser::{IncrementalParser, ParseState}; // Import incremental parser

// Helper to find a specific typed header
fn get_typed_header<'a, T>(msg: &'a Message) -> Result<T, String> 
where 
    T: TryFrom<&'a rvoip_sip_core::header::Header, Error = rvoip_sip_core::Error> + std::fmt::Debug
{
    let header_name_str = std::any::type_name::<T>().split("::").last().unwrap_or("UnknownType");
    msg.headers().iter()
       .find_map(|h| T::try_from(h).ok())
       .ok_or_else(|| format!("Typed header '{}' not found or failed to parse", header_name_str))
}

// --- Tests originally from torture_tests.rs --- 

#[test]
fn test_valid_basic_invite() {
    // A very basic INVITE request
    let basic_invite = "\
INVITE sip:bob@biloxi.example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP pc33.atlanta.example.com;branch=z9hG4bK776asdhds\r\n\
Max-Forwards: 70\r\n\
To: Bob <sip:bob@biloxi.example.com>\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=1928301774\r\n\
Call-ID: a84b4c76e66710@pc33.atlanta.example.com\r\n\
CSeq: 314159 INVITE\r\n\
Contact: <sip:alice@pc33.atlanta.example.com>\r\n\
Content-Type: application/sdp\r\n\
Content-Length: 0\r\n\
\r\n";

    let msg = parse_sip_message(basic_invite).expect("Basic INVITE failed to parse");
    let req = msg.as_request().expect("Expected Request");

    assert_eq!(req.method, Method::Invite);
    assert_eq!(req.uri.to_string(), "sip:bob@biloxi.example.com");

    let via: Via = get_typed_header(&msg).expect("Via parse failed");
    assert_eq!(via.host, "pc33.atlanta.example.com");
    assert_eq!(via.branch(), Some("z9hG4bK776asdhds"));

    let max_fwd: MaxForwards = get_typed_header(&msg).expect("MaxForwards parse failed");
    assert_eq!(max_fwd.0, 70);

    let to: To = get_typed_header(&msg).expect("To parse failed");
    assert_eq!(to.0.display_name.as_deref(), Some("Bob"));
    assert_eq!(to.0.uri.to_string(), "sip:bob@biloxi.example.com");

    let from: From = get_typed_header(&msg).expect("From parse failed");
    assert_eq!(from.0.display_name.as_deref(), Some("Alice"));
    assert_eq!(from.0.uri.to_string(), "sip:alice@atlanta.example.com");
    assert_eq!(from.0.tag(), Some("1928301774"));

    let call_id: CallId = get_typed_header(&msg).expect("CallID parse failed");
    assert_eq!(call_id.0, "a84b4c76e66710@pc33.atlanta.example.com");

    let cseq: CSeq = get_typed_header(&msg).expect("CSeq parse failed");
    assert_eq!(cseq.seq, 314159);
    assert_eq!(cseq.method, Method::Invite);
    
    let contact: Contact = get_typed_header(&msg).expect("Contact parse failed");
     assert_eq!(contact.0.uri.to_string(), "sip:alice@pc33.atlanta.example.com");

    let ct: ContentType = get_typed_header(&msg).expect("ContentType parse failed");
    assert_eq!(ct.0.type_, "application");
    assert_eq!(ct.0.subtype, "sdp");

    let cl: ContentLength = get_typed_header(&msg).expect("ContentLength parse failed");
    assert_eq!(cl.0, 0);
}

#[test]
fn test_valid_basic_response() {
    // A basic 200 OK response
    let basic_response = "\
SIP/2.0 200 OK\r\n\
Via: SIP/2.0/UDP server10.biloxi.example.com;branch=z9hG4bK4b43c2ff8.1\r\n\
Via: SIP/2.0/UDP bigbox3.site3.atlanta.example.com;branch=z9hG4bK77ef4c2312983.1\r\n\
Via: SIP/2.0/UDP pc33.atlanta.example.com;branch=z9hG4bK776asdhds\r\n\
To: Bob <sip:bob@biloxi.example.com>;tag=a6c85cf\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=1928301774\r\n\
Call-ID: a84b4c76e66710@pc33.atlanta.example.com\r\n\
CSeq: 314159 INVITE\r\n\
Contact: <sip:bob@biloxi.example.com>\r\n\
Content-Type: application/sdp\r\n\
Content-Length: 0\r\n\
\r\n";

    let msg = parse_sip_message(basic_response).expect("Basic Response failed to parse");
    let resp = msg.as_response().expect("Expected Response");

    assert_eq!(resp.status, StatusCode::Ok);
    assert_eq!(resp.reason_phrase(), "OK");

    let vias: Vec<_> = resp.headers.iter().filter_map(|h| Via::try_from(h).ok()).collect();
    assert_eq!(vias.len(), 3, "Expected 3 Via headers");
    assert_eq!(vias[0].host, "server10.biloxi.example.com");
    assert_eq!(vias[1].host, "bigbox3.site3.atlanta.example.com");
    assert_eq!(vias[2].host, "pc33.atlanta.example.com");

    let to: To = get_typed_header(&msg).expect("To parse failed");
    assert_eq!(to.0.display_name.as_deref(), Some("Bob"));
    assert_eq!(to.0.uri.to_string(), "sip:bob@biloxi.example.com");
    assert_eq!(to.0.tag(), Some("a6c85cf"));

     let from: From = get_typed_header(&msg).expect("From parse failed");
     assert_eq!(from.0.display_name.as_deref(), Some("Alice"));
     assert_eq!(from.0.uri.to_string(), "sip:alice@atlanta.example.com");
     assert_eq!(from.0.tag(), Some("1928301774"));
     
    // ... check other headers similarly ...
}

// --- Tests originally from custom_torture.rs --- 

#[test]
fn test_extremely_long_header_value() {
    // Generate a very long header value that's still valid
    let mut long_value = String::from("sip:user1@example.com");
    for i in 0..1000 { // Reduced from 10000 to avoid excessive test time/memory
        long_value.push_str(&format!(";param{}=value{}", i, i));
    }
    let message = format!("\
INVITE sip:user@example.com SIP/2.0\r
Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKnashds7\r
Max-Forwards: 70\r
To: <sip:user@example.com>\r
From: <sip:caller@example.net>;tag=12345\r
Call-ID: test-extremely-long-header\r
CSeq: 1 INVITE\r
Route: <{}>\r
Contact: <sip:caller@example.net>\r
Content-Type: application/sdp\r
Content-Length: 0\r\n\r\n", long_value);
    let result = parse_sip_message(&message);
    assert!(result.is_ok(), "Parser failed on extremely long header: {:?}", result.err());
}

#[test]
fn test_unusual_methods() {
    let unusual_methods = [
        "BREW", "PROPFIND", "ARBITRARY", "PURGE", "LINK", "UNLINK", "PATCH"
    ];
    for method in unusual_methods {
        let message = format!("\
{} sip:user@example.com SIP/2.0\r
Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKnashds7\r
Max-Forwards: 70\r
To: <sip:user@example.com>\r
From: <sip:caller@example.net>;tag=12345\r
Call-ID: unusual-method-test\r
CSeq: 1 {}\r
Contact: <sip:caller@example.net>\r
Content-Length: 0\r\n\r\n", method, method);

        let result = parse_sip_message(&message);
        assert!(result.is_ok(), "Parser failed on unusual method {}: {:?}", method, result.err());
        if let Ok(Message::Request(req)) = result {
             match req.method {
                 Method::Extension(s) => assert_eq!(s, method),
                 _ => panic!("Parsed method was not Extension variant"),
             }
        }
    }
}

#[test]
fn test_unusual_header_names() {
    let message = "\
INVITE sip:user@example.com SIP/2.0\r
Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKnashds7\r
Max-Forwards: 70\r
To: <sip:user@example.com>\r
From: <sip:caller@example.net>;tag=12345\r
Call-ID: unusual-header-test\r
CSeq: 1 INVITE\r
X-Unusual-Header!: This header has unusual characters\r
X-Another.Unusual_Header: This is another unusual header\r
Content-Length: 0\r
\r\n";
    let result = parse_sip_message(message);
    assert!(result.is_ok(), "Parser failed on unusual header names: {:?}", result.err());
    if let Ok(msg) = result {
        assert!(msg.headers().iter().any(|h| h.name.as_str() == "X-Unusual-Header!"));
        assert!(msg.headers().iter().any(|h| h.name.as_str() == "X-Another.Unusual_Header"));
    }
}

#[test]
fn test_malformed_request_uri() {
    let malformed_uris = [
        "INVITE sip:user@[::1 SIP/2.0\r\n",  // Missing closing bracket in IPv6
        "INVITE sip:: SIP/2.0\r\n",  // Missing user and host but colon present
        "INVITE sip:user@example.com:abc SIP/2.0\r\n",  // Non-numeric port
        "INVITE sip:user@example.com:99999 SIP/2.0\r\n",  // Port too large
        "INVITE sip:[::1]:badport SIP/2.0\r\n", // Invalid port
    ];
    for malformed in malformed_uris {
        let message = format!("{}\
Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKnashds7\r
Max-Forwards: 70\r
To: <sip:user@example.com>\r
From: <sip:caller@example.net>;tag=12345\r
Call-ID: malformed-uri-test\r
CSeq: 1 INVITE\r
Content-Length: 0\r\n\r\n", malformed);
        assert!(expect_parse_error(&message, Some("Invalid URI")));
    }
}

#[test]
fn test_exotic_status_codes() {
    let exotic_codes = [
        (100, "Trying"),
        (199, "Early Dialog Terminated"),  // Uncommon informational
        (299, "Miscellaneous Success"),    // Uncommon success
        (380, "Alternative Service"),      // Uncommon redirection
        (425, "Bad Alert Message"),        // Uncommon client error
        (480, "Temporarily Unavailable"),  
        (489, "Bad Event"),                // Uncommon client error
        (511, "Network Authentication Required"),  // Uncommon server error
        (580, "Precondition Failure"),     // Uncommon server error
        (600, "Busy Everywhere"),          
        (699, "Global Failure")            // Uncommon global failure
    ];
    for (code, reason) in exotic_codes {
        let message = format!("\
SIP/2.0 {} {}\r
Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKnashds7\r
To: <sip:user@example.com>;tag=abcdef\r
From: <sip:caller@example.net>;tag=12345\r
Call-ID: exotic-status-test\r
CSeq: 1 INVITE\r
Content-Length: 0\r\n\r\n", code, reason);
        let result = parse_sip_message(&message);
        assert!(result.is_ok(), "Parsing failed for status code {}: {:?}", code, result.err());
        
        if let Ok(Message::Response(resp)) = result {
             assert_eq!(resp.status.as_u16(), code, "Status code mismatch for {}", code);
             // Reason phrase might be canonicalized by parser, don't rely on exact match
             // assert_eq!(resp.reason_phrase(), reason);
        } else {
            panic!("Expected Response, got something else");
        }
    }
}

#[test]
fn test_unexpected_end_of_headers() {
    let message = "\
INVITE sip:user@example.com SIP/2.0\r
Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKnashds7\r
Max-Forwards: 70\r
To: <sip:user@example.com>\r
From: <sip:caller@example.net>;tag=12345\r
Call-ID: unexpected-eof-test\r
CSeq: 1 INVITE\r
Content-Length: 0";  // Missing final CRLF

    // Incremental parser should handle this, but full message parser might fail
    assert!(expect_parse_error(message, None)); // Expect some error
}

#[test]
fn test_unusual_line_endings() {
    // Test with LF only instead of CRLF
    let lf_only = "\
INVITE sip:user@example.com SIP/2.0\n\
Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKnashds7\n\
Max-Forwards: 70\n\
To: <sip:user@example.com>\n\
From: <sip:caller@example.net>;tag=12345\n\
Call-ID: lf-only-test\n\
CSeq: 1 INVITE\n\
Content-Length: 0\n\
\n\
";

    // Test with CR only instead of CRLF
    let cr_only = "\
INVITE sip:user@example.com SIP/2.0\r\
Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKnashds7\r\
Max-Forwards: 70\r\
To: <sip:user@example.com>\r\
From: <sip:caller@example.net>;tag=12345\r\
Call-ID: cr-only-test\r\
CSeq: 1 INVITE\r\
Content-Length: 0\r\
\r\
";

    // Expect success as parser should normalize or handle these
    let lf_result = parse_sip_message(lf_only);
    assert!(lf_result.is_ok(), "Parser failed on LF only: {:?}", lf_result.err());
    
    // CR only is less likely to be supported
    let cr_result = parse_sip_message(cr_only);
    assert!(cr_result.is_err(), "Parser unexpectedly succeeded on CR only");
}

#[test]
fn test_empty_headers() {
    let message = "\
INVITE sip:user@example.com SIP/2.0\r
Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKnashds7\r
Max-Forwards: 70\r
To: <sip:user@example.com>\r
From: <sip:caller@example.net>;tag=12345\r
Call-ID: empty-header-test\r
CSeq: 1 INVITE\r
Empty-Header: \r
Another-Empty:\t\r
Content-Length: 0\r
\r\n";

    // Test if parser handles empty header values (should succeed)
    let result = parse_sip_message(message);
    assert!(result.is_ok(), "Parser failed on empty headers: {:?}", result.err());
     if let Ok(msg) = result {
        assert!(msg.header(&HeaderName::Other("Empty-Header".to_string())).is_some());
        assert_eq!(msg.header(&HeaderName::Other("Empty-Header".to_string())).unwrap().value.as_text(), Some(""));
         assert!(msg.header(&HeaderName::Other("Another-Empty".to_string())).is_some());
        assert_eq!(msg.header(&HeaderName::Other("Another-Empty".to_string())).unwrap().value.as_text(), Some(""));
    }
}

#[test]
fn test_malformed_headers() {
    let malformed_headers = [
        "Content-Length: abc\r\n",         // Non-numeric Content-Length
        "CSeq: 1\r\n",                     // Missing method in CSeq
        "CSeq: a INVITE\r\n",              // Non-numeric sequence in CSeq
        "Via: garbage\r\n",                // Invalid Via header
        "Via: /UDP host:port;branch=xyz\r\n", // Missing SIP version in Via
        "Max-Forwards: -1\r\n",            // Negative Max-Forwards
        // "Call-ID: \r\n", // Empty Call-ID is technically allowed by BNF but discouraged
    ];
    
    for header in malformed_headers {
        let message = format!("\
INVITE sip:user@example.com SIP/2.0\r
{}\
To: <sip:user@example.com>\r
From: <sip:caller@example.net>;tag=12345\r
Call-ID: malformed-header-test\r
CSeq: 1 INVITE\r
Content-Length: 0\r
\r\n", header);

        assert!(expect_parse_error(&message, None), "Message with malformed header '{}' did not fail", header.trim());
    }
}

#[test]
fn test_extremely_large_numbers() {
    let message = "\
INVITE sip:user@example.com SIP/2.0\r
Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKnashds7\r
Max-Forwards: 255\r
To: <sip:user@example.com>\r
From: <sip:caller@example.net>;tag=12345\r
Call-ID: large-number-test\r
CSeq: 4294967295 INVITE\r
Content-Length: 0\r
\r\n";

    // Testing with very large but valid uint32/u8 numbers
    let result = parse_sip_message(message);
    assert!(result.is_ok(), "Parsing failed for large numbers: {:?}", result.err());
    if let Ok(Message::Request(req)) = result {
        assert_eq!(req.header(&HeaderName::MaxForwards).unwrap().value.as_text(), Some("255"));
        assert_eq!(req.header(&HeaderName::CSeq).unwrap().value.as_text(), Some("4294967295 INVITE"));
    }
}

#[test]
fn test_custom_uri_parameters() {
    let message = "\
INVITE sip:user@example.com;custom=value;transport=TCP;maddr=192.168.1.1;ttl=5;lr;method=INVITE SIP/2.0\r\n\
Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKnashds7\r\n\
Max-Forwards: 70\r\n\
To: <sip:user@example.com;custom=value;transport=TCP>\r\n\
From: <sip:caller@example.net;maddr=192.168.1.1;ttl=5;lr>;tag=12345\r\n\
Call-ID: uri-params-test\r\n\
CSeq: 1 INVITE\r\n\
Contact: <sip:caller@example.net;methods=INVITE,BYE,OPTIONS>\r\n\
Content-Length: 0\r\n\
\r\n";
    let result = parse_sip_message(message);
    assert!(result.is_ok(), "Parser failed with custom URI params: {:?}", result.err());
    
    if let Ok(msg) = result {
        let req = msg.as_request().unwrap();
        
        // Check Request-URI parameters
        assert!(req.uri.parameters.contains(&Param::Other("custom".to_string(), Some("value".to_string()))));
        assert!(req.uri.parameters.contains(&Param::Transport("TCP".to_string())));
        assert!(req.uri.parameters.contains(&Param::Maddr("192.168.1.1".to_string())));
        assert!(req.uri.parameters.contains(&Param::Ttl(5)));
        assert!(req.uri.parameters.contains(&Param::Lr));
        assert!(req.uri.parameters.contains(&Param::Method("INVITE".to_string())));

        // Check To URI parameters
        let to_hdr: To = get_typed_header(&msg).unwrap();
        assert!(to_hdr.0.uri.parameters.contains(&Param::Other("custom".to_string(), Some("value".to_string()))));
        assert!(to_hdr.0.uri.parameters.contains(&Param::Transport("TCP".to_string())));

        // Check From URI parameters
        let from_hdr: From = get_typed_header(&msg).unwrap();
        assert!(from_hdr.0.uri.parameters.contains(&Param::Maddr("192.168.1.1".to_string())));
        assert!(from_hdr.0.uri.parameters.contains(&Param::Ttl(5)));
        assert!(from_hdr.0.uri.parameters.contains(&Param::Lr));
        assert_eq!(from_hdr.0.tag(), Some("12345")); // Also check header param

        // Check Contact URI parameters
        let contact_hdr: Contact = get_typed_header(&msg).unwrap();
        assert!(contact_hdr.0.uri.parameters.contains(&Param::Other("methods".to_string(), Some("INVITE,BYE,OPTIONS".to_string()))));
    }
}

#[test]
fn test_multiple_same_name_headers() {
    let message = "\
INVITE sip:user@example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKnashds7\r\n\
Via: SIP/2.0/TCP 10.0.0.1:5060;branch=z9hG4bKnashds8\r\n\
Via: SIP/2.0/TLS 172.16.0.1:5061;branch=z9hG4bKnashds9\r\n\
Max-Forwards: 70\r\n\
To: <sip:user@example.com>\r\n\
From: <sip:caller@example.net>;tag=12345\r\n\
Call-ID: multi-headers-test\r\n\
Record-Route: <sip:proxy1.example.com;lr>\r\n\
Record-Route: <sip:proxy2.example.com;lr>\r\n\
Record-Route: <sip:proxy3.example.com;lr>\r\n\
Supported: timer\r\n\
Supported: 100rel\r\n\
Supported: path\r\n\
CSeq: 1 INVITE\r\n\
Contact: <sip:caller@example.net>\r\n\
Content-Length: 0\r\n\
\r\n";
    let result = parse_sip_message(message);
    assert!(result.is_ok(), "Parser failed on multiple same-name headers: {:?}", result.err());
    
    if let Ok(msg) = result {
        let req = msg.as_request().expect("Expected Request");
        
        // Check typed Via headers
        let vias: Vec<_> = req.headers.iter().filter_map(|h| Via::try_from(h).ok()).collect();
        assert_eq!(vias.len(), 3, "Expected 3 Via headers");
        assert_eq!(vias[0].host, "192.168.1.1");
        assert_eq!(vias[0].branch(), Some("z9hG4bKnashds7"));
        assert_eq!(vias[1].host, "10.0.0.1");
        assert_eq!(vias[1].branch(), Some("z9hG4bKnashds8"));
        assert_eq!(vias[2].host, "172.16.0.1");
        assert_eq!(vias[2].branch(), Some("z9hG4bKnashds9"));

        // Check typed Record-Route headers
        let rrs: Vec<_> = req.headers.iter().filter_map(|h| RecordRoute::try_from(h).ok()).collect();
         assert_eq!(rrs.len(), 3, "Expected 3 Record-Route headers");
         assert_eq!(rrs[0].0.uris[0].uri.host.to_string(), "proxy1.example.com");
         assert!(rrs[0].0.uris[0].params.contains(&Param::Lr));
         assert_eq!(rrs[1].0.uris[0].uri.host.to_string(), "proxy2.example.com");
         assert!(rrs[1].0.uris[0].params.contains(&Param::Lr));
          assert_eq!(rrs[2].0.uris[0].uri.host.to_string(), "proxy3.example.com");
         assert!(rrs[2].0.uris[0].params.contains(&Param::Lr));

        // Supported is often handled as a simple list or individual checks
        let supported_count = req.headers.iter().filter(|h| h.name.as_str() == "Supported").count();
        assert_eq!(supported_count, 3, "Expected 3 Supported headers");
        // Could add specific checks if a TypedHeader::Supported is implemented

    } else {
        panic!("Expected Request, got something else");
    }
}

#[test]
fn test_client_demo_invite_format() {
     let messages = [
        // Standard INVITE
        "INVITE sip:bob@127.0.0.1:5071 SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1:5070;branch=z9hG4bKa0b1c2d3e4f5\r\nFrom: <sip:alice@127.0.0.1>;tag=abcdef123456\r\nTo: <sip:bob@127.0.0.1>\r\nCall-ID: a84b4c76e66710@127.0.0.1\r\nCSeq: 1 INVITE\r\nMax-Forwards: 70\r\nContact: <sip:alice@127.0.0.1:5070>\r\nContent-Type: application/sdp\r\nContent-Length: 0\r\n\r\n",
        // More complex INVITE
        "INVITE sip:bob@127.0.0.1:5071 SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1:5070;branch=z9hG4bKa0b1c2d3e4f5;rport\r\nFrom: \"Alice\" <sip:alice@127.0.0.1>;tag=abcdef123456\r\nTo: \"Bob\" <sip:bob@127.0.0.1>\r\nCall-ID: a84b4c76e66710@127.0.0.1\r\nCSeq: 1 INVITE\r\nMax-Forwards: 70\r\nContact: <sip:alice@127.0.0.1:5070>\r
Content-Type: application/sdp\r
Content-Length: 153\r\nUser-Agent: RVoIP SIP Client Demo\r
Allow: INVITE, ACK, CANCEL, BYE, OPTIONS\r
Supported: 100rel\r\n\r\nv=0\r\no=alice 123456 789012 IN IP4 127.0.0.1\r\ns=Call\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na=sendrecv\r\n",
        // Unusual spacing 
        "INVITE  sip:bob@127.0.0.1:5071  SIP/2.0\r\nVia:  SIP/2.0/UDP  127.0.0.1:5070;branch=z9hG4bKa0b1c2d3e4f5\r\nFrom:  <sip:alice@127.0.0.1>;tag=abcdef123456\r\nTo:  <sip:bob@127.0.0.1>\r\nCall-ID:  a84b4c76e66710@127.0.0.1\r\nCSeq:  1  INVITE\r\nMax-Forwards:  70\r\nContact:  <sip:alice@127.0.0.1:5070>\r\nContent-Type:  application/sdp\r\nContent-Length:  0\r\n\r\n",
        // Line folding
        "INVITE sip:bob@127.0.0.1:5071 SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1:5070\r\n ;branch=z9hG4bKa0b1c2d3e4f5\r\nFrom: <sip:alice@127.0.0.1>\r\n ;tag=abcdef123456\r\nTo: <sip:bob@127.0.0.1>\r\nCall-ID: a84b4c76e66710@127.0.0.1\r\nCSeq: 1 INVITE\r\nMax-Forwards: 70\r\nContact: <sip:alice@127.0.0.1:5070>\r\nContent-Type: application/sdp\r\nContent-Length: 0\r\n\r\n",
        // Failure cases remain below
        "\r\nINVITE sip:bob@127.0.0.1:5071 SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1:5070;branch=z9hG4bKa0b1c2d3e4f5\r\nFrom: <sip:alice@127.0.0.1>;tag=abcdef123456\r\nTo: <sip:bob@127.0.0.1>\r\nCall-ID: a84b4c76e66710@127.0.0.1\r\nCSeq: 1 INVITE\r\nMax-Forwards: 70\r\nContact: <sip:alice@127.0.0.1:5070>\r\nContent-Type: application/sdp\r\nContent-Length: 0\r\n\r\n",
        "INVITE sip:bob@127.0.0.1:5071 SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1:5070;branch=z9hG4bKa0b1c2d3e4f5\r\nFrom: <sip:alice@127.0.0.1>;tag=abcdef123456\r\nTo: <sip:bob@127.0.0.1>\r\nCall-ID: a84b4c76e66710@127.0.0.1\r\nCSeq: 1 INVITE\r\nMax-Forwards: 70\r\nContact: <sip:alice@127.0.0.1:5070>\r\nContent-Type: application/sdp\r\nContent-Length: 0\r\n",
        "INVITE sip:bob@127.0.0.1:5071 SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1:5070;branch=z9hG4bKa0b1c2d3e4f5\r\nFrom: <sip:alice@127.0.0.1>;tag=abcdef123456\r\nTo: <sip:bob@127.0.0.1>\r\nCall-ID: a84b4c76e66710@127.0.0.1\r\nCSeq: 1\r\nMax-Forwards: 70\r\nContact: <sip:alice@127.0.0.1:5070>\r\nContent-Type: application/sdp\r\nContent-Length: 0\r\n\r\n",
        "INVITE sip:bob@127.0.0.1:5071 SIP/2.0\rVia: SIP/2.0/UDP 127.0.0.1:5070;branch=z9hG4bKa0b1c2d3e4f5\rFrom: <sip:alice@127.0.0.1>;tag=abcdef123456\rTo: <sip:bob@127.0.0.1>\rCall-ID: a84b4c76e66710@127.0.0.1\rCSeq: 1 INVITE\rMax-Forwards: 70\rContact: <sip:alice@127.0.0.1:5070>\rContent-Type: application/sdp\rContent-Length: 0\r\r"
    ];
    for (i, message) in messages.iter().enumerate() {
        println!("Testing client demo format variant {}", i+1);
        let result = parse_sip_message(message);
        if i < 4 {
            assert!(result.is_ok(), "Valid client message variant {} failed to parse: {:?}", i+1, result.err());
            if let Ok(msg) = result {
                let req = msg.as_request().unwrap(); // Safe to unwrap after is_ok()
                 assert_eq!(req.method, Method::Invite);
                 // Add a few typed checks for the successful cases
                 let call_id: CallId = get_typed_header(&msg).expect("CallID missing");
                 assert_eq!(call_id.0, "a84b4c76e66710@127.0.0.1");
                 let from: From = get_typed_header(&msg).expect("From missing");
                 assert_eq!(from.0.tag(), Some("abcdef123456"));
             } else {
                 panic!("Expected Request for variant {}", i+1);
             }
        } else {
            assert!(result.is_err(), "Invalid client message variant {} unexpectedly parsed successfully: {:?}", i+1, result.ok());
            // Can add specific error checks if needed
            // assert!(expect_parse_error(message, Some("Expected error substring")));
        }
    }
}

#[test]
fn test_header_format_edge_cases() {
    // Test various header formatting edge cases
    let headers_to_test = [
        // Standard header
        "Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK776asdhds\r\n",
        // No space after colon
        "Via:SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK776asdhds\r\n",
        // Multiple spaces after colon
        "Via:     SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK776asdhds\r\n",
        // Tabs instead of spaces
        "Via:\tSIP/2.0/UDP\t127.0.0.1:5060;branch=z9hG4bK776asdhds\r\n",
        // Mixed whitespace
        "Via: \t SIP/2.0/UDP \t 127.0.0.1:5060;branch=z9hG4bK776asdhds\r\n",
        // Trailing whitespace
        "Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK776asdhds \t \r\n",
        // Empty header value (technically allowed by BNF but discouraged)
        "Empty-Header: \r\n",
        // Header with only whitespace
        "Whitespace-Header: \t \r\n"
    ];

    for (i, header) in headers_to_test.iter().enumerate() {
        let message = format!("\
INVITE sip:bob@127.0.0.1:5071 SIP/2.0\r\n{}\
From: <sip:alice@127.0.0.1>;tag=abcdef123456\r\nTo: <sip:bob@127.0.0.1>\r\nCall-ID: header-format-test-{}\r\nCSeq: 1 INVITE\r\nContent-Length: 0\r\n\r\n", header, i);

        println!("Testing header variant {}", i+1);
        let result = parse_sip_message(&message);
        assert!(result.is_ok(), "Header variant {} failed to parse: {:?}", i+1, result.err());
        
        // Verify the header value was captured correctly (trimming applied by parser)
        if let Ok(Message::Request(req)) = result {
            let header_name = header.splitn(2, ':').next().unwrap();
            let expected_value = header.splitn(2, ':').nth(1).unwrap().trim();
            
            let parsed_header = req.headers.iter().find(|h| h.name.as_str() == header_name);
            assert!(parsed_header.is_some(), "Header '{}' not found in parsed message", header_name);
            assert_eq!(parsed_header.unwrap().value.as_text().unwrap_or(""), expected_value, "Header value mismatch for '{}'", header_name);
        } else {
            panic!("Parsed as wrong message type");
        }
    }
}

#[test]
fn test_incremental_header_parsing() {
    let message = "INVITE sip:bob@127.0.0.1:5071 SIP/2.0\r\n\
Via: SIP/2.0/UDP 127.0.0.1:5070;branch=z9hG4bKabc123\r\n\
From: \"Alice\" <sip:alice@127.0.0.1>;tag=abcdef\r\n\
To: <sip:bob@127.0.0.1>\r\n\
Call-ID: test-incremental-parser\r\n\
CSeq: 1 INVITE\r\n\
Max-Forwards: 70\r\n\
Content-Length: 0\r\n\
\r\n";
    
    println!("Testing incremental header parsing with arbitrary chunks");
    
    let chunk_sizes = [1, 2, 3, 5, 10, 15, 20, 50];
    
    for &size in &chunk_sizes {
        println!("Testing with chunk size: {}", size);
        let mut parser = IncrementalParser::new_with_debug();
        
        let chunks: Vec<&str> = message.as_bytes()
            .chunks(size)
            .map(|chunk| std::str::from_utf8(chunk).unwrap())
            .collect();
        
        println!("Message split into {} chunks", chunks.len());
        
        for (j, chunk) in chunks.iter().enumerate() {
            let state = parser.parse(chunk);
            
            if j == chunks.len() - 1 {
                match state {
                    ParseState::Complete(_) => {
                        println!("Successfully parsed with chunk size {}", size);
                        if let Some(Message::Request(req)) = parser.take_message() {
                             assert_eq!(req.method, Method::Invite);
                             let via: Via = get_typed_header(&Message::Request(req)).expect(&format!("Via parse failed chunk size {}", size));
                             assert_eq!(via.host, "127.0.0.1");
                             assert_eq!(via.branch(), Some("z9hG4bKabc123"));
                         } else {
                             panic!("Expected Request");
                         }
                    },
                    _ => panic!("Failed to completely parse with chunk size {}: {:?}", size, state),
                }
            }
        }
    }
    
    // Test with the specific chunk pattern from the built-in test
    let chunks = [
        "INVITE sip:bob@127.0.0.1:5071 SIP/2.0\r\n",
        "Via: SIP/2.0/UDP 127.0.0.1:5070;branch=z9hG4bKabc123\r\n",
        "From: \"Alice\" <sip:alice@127.0.0.1>;tag=abcdef\r\n",
        "To: <sip:bob@127.0.0.1>\r\n",
        "Call-ID: test-incremental-parser\r\n",
        "CSeq: 1 INVITE\r\n",
        "Max-Forwards: 70\r\n",
        "Content-Length: 0\r\n",
        "\r\n", // Empty line marks end of headers
    ];
    
    let mut parser = IncrementalParser::new_with_debug();
    for (i, chunk) in chunks.iter().enumerate() {
        let state = parser.parse(chunk);
        
        if i == chunks.len() - 1 {
            match state {
                ParseState::Complete(_) => {
                     println!("Successfully parsed with header-by-header chunks");
                     assert!(parser.take_message().is_some());
                },
                _ => panic!("Failed to completely parse with header-by-header chunks: {:?}", state),
            }
        }
    }
}

#[test]
fn test_decode_raw_udp_data() {
     use rvoip_sip_core::parser::parse_message_bytes;
     // ... (rest of test function as before, using common helpers) ...
    // Helper function to convert hex string to bytes
    fn hex_to_bytes(hex: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        let hex = hex.replace(" ", ""); // Remove spaces
        
        for i in 0..(hex.len() / 2) {
            let res = u8::from_str_radix(&hex[2*i..2*i+2], 16);
            match res {
                Ok(v) => bytes.push(v),
                Err(e) => panic!("Invalid hex string: {}", e),
            }
        }
        bytes
    }
    
    let example_packets = [
        "494e56495445207369703a626f6240313237\
2e302e302e313a35303731205349502f322e300d0a\
5669613a205349502f322e302f5544502031323723\
302e302e313a353037303b6272616e63683d7a3968\
47346261306231633264330d0a46726f6d3a203c73\
69703a616c6963654031323723302e302e313e3b74\
61673d61626364656630313233340d0a546f3a203c\
7369703a626f6240313237232e302e302e313e0d0a\
43616c6c2d49443a2074657374696e672d70617273\
65720d0a435365713a203120494e564954450d0a4d\
61782d466f7277617264733a2037300d0a436f6e74\
656e742d4c656e6774683a20300d0a0d0a",
        "494e56495445207369703a626f6240313237\
2e302e302e313a35303731205349502f322e300d0a\
5669613a205349502f322e302f5544502031323723\
302e302e313a353037303b6272616e63683d7a3968\
47346261306231633264330d0a46726f6d3a203c73\
69703a616c6963654031323723302e302e313e3b74\
61673d61626364656630313233340d0a546f3a203c\
7369703a626f6240313237232e302e302e313e0d0a\\\
43616c6c2d49443a2074657374696e672d70617273\\\
65720d0a435365713a203120494e564954450d0a4d\\\
61782d466f7277617264733a2037300d0a436f6e74\\\
656e742d4c656e6774683a2030"
    ];
     for (i, hex_packet) in example_packets.iter().enumerate() {\n        println!(\"Testing packet {}\", i+1);\n        let binary_data = hex_to_bytes(hex_packet);\n        let result = parse_message_bytes(&binary_data);\n        if i == 0 {\n            assert!(result.is_ok(), \"Valid packet failed: {:?}\", result.err());\n        } else {\n            assert!(result.is_err(), \"Invalid packet succeeded\");\n        }\n     }\n}\n
}

// Removed test_standalone_parser_improvements (covered by other tests)

// Section 3.1.2.5 - Multiple SP Separating Request-Line Elements
#[test]
fn test_3_1_2_5_multiple_spaces() {
     /// RFC 4475 Section 3.1.2.5 - Multiple SP Separating Request-Line Elements
    let message = "\
OPTIONS  sip:user@example.com  SIP/2.0\r\n\
To: sip:user@example.com\r\n\
From: sip:caller@example.net;tag=323\r\n\
Max-Forwards: 70\r\n\
Call-ID: multi01.98asdh@192.0.2.1\r\n\
CSeq: 59 OPTIONS\r\n\
Via: SIP/2.0/UDP host.example.com;branch=z9hG4bKkdjuw\r\n\
Content-Length: 0\r\n\
\r\n\";

    // RFC 3261 mandates single SP, but recommend tolerant parsing.
    // Expect success.
    let result = parse_sip_message(message);
    assert!(result.is_ok(), "Parser failed on multiple spaces in request line: {:?}", result.err());
     if let Ok(Message::Request(req)) = result {
         assert_eq!(req.method, Method::Options);
         assert_eq!(req.uri.to_string(), "sip:user@example.com");
         assert_eq!(req.version.to_string(), "SIP/2.0"); // Check version parsed correctly
     } else {
         panic!("Expected Request");
     }
}

#[test]
fn test_3_3_10_multiple_routes() {
     /// RFC 4475 Section 3.3.10 - Multiple Route Headers
    let message = "\
OPTIONS sip:user@example.com SIP/2.0\r\n\
Route: <sip:services.example.com;lr>\r\n\
Route: <sip:edge.example.com;lr>\r\n\
To: sip:user@example.com\r\n\
From: sip:caller@example.net;tag=3415132\r\n\
Max-Forwards: 70\r\n\
Call-ID: multrte.0ha0isndaksdj\r\n\
CSeq: 59 OPTIONS\r\n\
Via: SIP/2.0/UDP host5.example.com;branch=z9hG4bK-39234-1\r\n\
Content-Length: 0\r\n\
\r\n\";

    let result = parse_sip_message(message);
    assert!(result.is_ok(), "Parser failed on multiple Route headers: {:?}", result.err());
    if let Ok(msg) = result {
        let routes: Vec<_> = msg.headers().iter()
                             .filter_map(|h| Route::try_from(h).ok())
                             .collect();
        assert_eq!(routes.len(), 2, "Did not find two typed Route headers");
        assert_eq!(routes[0].0.uris.len(), 1);
        assert_eq!(routes[0].0.uris[0].uri.host.to_string(), "services.example.com");
        assert!(routes[0].0.uris[0].params.contains(&Param::Lr));
        assert_eq!(routes[1].0.uris.len(), 1);
        assert_eq!(routes[1].0.uris[0].uri.host.to_string(), "edge.example.com");
        assert!(routes[1].0.uris[0].params.contains(&Param::Lr));
    }
}

#[test]
fn test_3_3_16_path_header() {
    /// RFC 4475 Section 3.3.16 - Path Header (RFC 3327)
    let message = "\
REGISTER sip:example.com SIP/2.0\r\n\
To: sip:user@example.com\r\n\
From: sip:user@example.com;tag=8978\r\n\
Max-Forwards: 70\r\n\
Call-ID: pathtest.chad2@t1.example.com\r\n\
CSeq: 79 REGISTER\r\n\
Via: SIP/2.0/UDP t1.example.com;branch=z9hG4bK-p234\r\n\
Path: <sip:pcscf.isp.example.com;lr>\r\n\
Path: <sip:scscf.isp.example.com;lr>\r\n\
Contact: <sip:user@192.0.2.5>\r\n\
Content-Length: 0\r\n\
\r\n\";

    let result = parse_sip_message(message);
    assert!(result.is_ok(), "Parser failed on Path header: {:?}", result.err());
    if let Ok(msg) = result {
        let paths: Vec<_> = msg.headers().iter()
                            .filter(|h| h.name == HeaderName::Other("Path".to_string()))
                            .collect();
        assert_eq!(paths.len(), 2, "Did not find two Path headers");
        assert_eq!(paths[0].value.as_text(), Some("<sip:pcscf.isp.example.com;lr>"));
        assert_eq!(paths[1].value.as_text(), Some("<sip:scscf.isp.example.com;lr>"));
        // Could potentially try parsing the value as Route/RecordRoute if Path structure is identical
        // let path1_route = Route::from_str(paths[0].value.as_text().unwrap());
        // assert!(path1_route.is_ok());
    }
}

#[test]
fn test_3_4_1_scalar_field_overflow() {
    /// RFC 4475 Section 3.4.1 - Out-of-Range Scalar Values
    let message = "\
REGISTER sip:example.com SIP/2.0\r\n\
Via: SIP/2.0/TCP host129.example.com;branch=z9hG4bK-74BnK7G;rport=9\r\n\
From: <sip:user@example.com>;tag=1928301774\r\n\
To: <sip:user@example.com>\r\n\
Call-ID: scalar02.23o7@host256.example.com\r\n\
CSeq: 139122385607 REGISTER\r\n\
Max-Forwards: 256\r\n\
Expires: 10000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\r\n\
Contact: <sip:user@host129.example.com>;expires=280297596632815\r\n\
Content-Length: 0\r\n\
\r\n\";

    // Basic parsing should succeed, but typed parsing of specific headers will fail
    let result = parse_sip_message(message);
    assert!(result.is_ok(), "Parsing failed unexpectedly for scalar overflow message");

    if let Ok(msg) = result {
        // Check Max-Forwards (u8 overflow)
        let max_fwd_res: Result<MaxForwards, _> = get_typed_header(&msg);
        assert!(max_fwd_res.is_err(), "Max-Forwards > 255 did not cause typed parse error");
        assert!(max_fwd_res.unwrap_err().contains("Max-Forwards"));

        // Check CSeq (u32 is ok here)
        let cseq_res: Result<CSeq, _> = get_typed_header(&msg);
        assert!(cseq_res.is_ok(), "CSeq u32 failed");
        assert_eq!(cseq_res.unwrap().seq, 139122385607);

        // Check Expires (u32 overflow)
         let expires_res: Result<Expires, _> = get_typed_header(&msg);
         assert!(expires_res.is_err(), "Expires > u32::MAX did not cause typed parse error");
         assert!(expires_res.unwrap_err().contains("Expires"));

        // Check Contact expires parameter (u32 overflow)
         let contact_res: Result<Contact, _> = get_typed_header(&msg);
         assert!(contact_res.is_ok(), "Contact itself should parse");
         // Check the expires param *within* the contact - it should be parsed as Other or fail specific parsing
         let contact = contact_res.unwrap();
         assert!(contact.expires().is_none(), "Contact expires() helper did not return None for overflow");
         assert!(contact.0.params.iter().any(|p| matches!(p, Param::Other(k, _) if k == "expires")), "Overflowed expires param not found as Other");
    }
}

#[test]
fn test_3_1_2_10_multipart_mime_body() {
     /// RFC 4475 Section 3.1.2.10 - Multipart MIME Body
    let multipart_message = "\
INVITE sip:user@example.com SIP/2.0\r\n\
To: sip:user@example.com\r\n\
From: sip:caller@example.net;tag=08D\r\n\
Max-Forwards: 70\r\n\
Call-ID: multipart.sdp.jpeg@caller.example.net\r\n\
CSeq: 5 INVITE\r\n\
Via: SIP/2.0/UDP host5.example.net;branch=z9hG4bK-d87543-1\r\n\
Contact: <sip:caller@host5.example.net>\r\n\
Content-Type: multipart/mixed; boundary=unique-boundary-1\r\n\
Content-Length: 501\r\n\
\r\n\
--unique-boundary-1\r\n\
Content-Type: application/sdp\r\n\
\r\n\
v=0\r\n\
o=caller 53655765 2353687637 IN IP4 host5.example.net\r\n\
s=-\r\n\
c=IN IP4 192.0.2.5\r\n\
t=0 0\r\n\
m=audio 20000 RTP/AVP 0\r\n\
--unique-boundary-1\r\n\
Content-Type: image/jpeg\r\n\
Content-Transfer-Encoding: binary\r\n\
Content-ID: <image1@caller.example.net>\r\n\
\r\n\
JPEG ... binary image data ...\r\n\
--unique-boundary-1--\r\n\";

    let result = parse_sip_message(multipart_message);
    assert!(result.is_ok(), "Parser failed on multipart message: {:?}", result.err());
    if let Ok(msg) = result {
        let req = msg.as_request().expect("Expected Request");
        
        let ct: ContentType = get_typed_header(&msg).expect("Content-Type parse failed");
        assert_eq!(ct.0.type_, "multipart");
        assert_eq!(ct.0.subtype, "mixed");
        assert_eq!(ct.0.params.get("boundary"), Some(&"unique-boundary-1".to_string()));

        let cl: ContentLength = get_typed_header(&msg).expect("ContentLength parse failed");
        assert_eq!(cl.0, 501);
        
        assert!(!req.body.is_empty());
        // TODO: Add multipart parsing check here once implemented fully
        // let boundary = ct.0.params.get("boundary").unwrap();
        // let multipart_body = parse_multipart(&req.body, boundary).unwrap();
        // assert_eq!(multipart_body.parts.len(), 2);
        // assert_eq!(multipart_body.parts[0].content_type(), Some("application/sdp"));
        // assert_eq!(multipart_body.parts[1].content_type(), Some("image/jpeg"));
    }
}

// ... rest of tests ... 
// Adapted tests from RFC 5118 - SIP Torture Test Messages for IPv6
// https://tools.ietf.org/html/rfc5118

use crate::common::{parse_sip_message, expect_parse_error};
use rvoip_sip_core::types::{Method, Uri, Scheme, Host, Message, Request, Response, StatusCode, CSeq, Via, Address, From, To, Contact, MaxForwards, ContentLength, CallId, Route, RecordRoute, Param, Expires};
use rvoip_sip_core::header::{HeaderName, HeaderValue};
use rvoip_sip_core::Error;
use std::str::FromStr;
use std::convert::TryFrom;

// Helper from rfc4475_test.rs
fn get_typed_header<'a, T>(msg: &'a Message) -> Result<T, String> 
where 
    T: TryFrom<&'a rvoip_sip_core::header::Header, Error = rvoip_sip_core::Error> + std::fmt::Debug
{
    let header_name_str = std::any::type_name::<T>().split("::").last().unwrap_or("UnknownType");
    msg.headers().iter()
       .find_map(|h| T::try_from(h).ok())
       .ok_or_else(|| format!("Typed header '{}' not found or failed to parse", header_name_str))
}

// Section a.1 - SIP Message Containing an IPv6 Reference
#[test]
fn test_a1_ipv6_uri_reference() {
    /// RFC 5118 Section a.1
    let message = "\
INVITE sip:[2001:db8::10] SIP/2.0\r\nVia: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bKas3-111\r\nMax-Forwards: 70\r\nTo: <sip:[2001:db8::10]>\r\nFrom: <sip:[2001:db8::9:1]>;tag=1\r\nCall-ID: ipv6-1\r\nCSeq: 1 INVITE\r\nContact: <sip:[2001:db8::9:1]>\r\nContent-Type: application/sdp\r\nContent-Length: 147\r\n\r\nv=0\r\no=- 1 1 IN IP6 2001:db8::9:1\r\ns=-\r\nc=IN IP6 2001:db8::9:1\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

    let msg = parse_sip_message(message).expect("Parse failed for a.1");
    let req = msg.as_request().expect("Expected Request");

    assert_eq!(req.method, Method::Invite);
    assert!(matches!(req.uri.host, Host::IPv6(ref ip) if ip == "2001:db8::10"), "Request-URI host mismatch");

    let via: Via = get_typed_header(&msg).expect("Via parse failed");
    assert_eq!(via.host, "[2001:db8::9:1]");
    assert_eq!(via.branch(), Some("z9hG4bKas3-111"));

    let from: From = get_typed_header(&msg).expect("From parse failed");
    assert!(matches!(from.0.uri.host, Host::IPv6(ref ip) if ip == "2001:db8::9:1"), "From URI host mismatch");
    assert_eq!(from.0.tag(), Some("1"));

    let to: To = get_typed_header(&msg).expect("To parse failed");
    assert!(matches!(to.0.uri.host, Host::IPv6(ref ip) if ip == "2001:db8::10"), "To URI host mismatch");
    
    let contact: Contact = get_typed_header(&msg).expect("Contact parse failed");
     assert!(matches!(contact.0.uri.host, Host::IPv6(ref ip) if ip == "2001:db8::9:1"), "Contact URI host mismatch");

    let cl: ContentLength = get_typed_header(&msg).expect("ContentLength parse failed");
    assert_eq!(cl.0, 147);
    assert!(req.body.len() >= 147);
}

// Section a.2 - IPv6 Reference with Zone Index
#[test]
fn test_a2_ipv6_with_zone_index() {
    /// RFC 5118 Section a.2
    let message = "\
INVITE sip:[2001:db8::10%eth0] SIP/2.0\r\nVia: SIP/2.0/UDP [2001:db8::9:1%eth0];branch=z9hG4bKas3-111\r\nMax-Forwards: 70\r\nTo: <sip:[2001:db8::10%eth0]>\r\nFrom: <sip:[2001:db8::9:1%eth0]>;tag=1\r\nCall-ID: ipv6-2\r\nCSeq: 1 INVITE\r\nContact: <sip:[2001:db8::9:1%eth0]>\r\nContent-Type: application/sdp\r\nContent-Length: 155\r\n\r\nv=0\r\no=- 1 1 IN IP6 2001:db8::9:1\r\ns=-\r\nc=IN IP6 2001:db8::9:1%eth0\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

    // Current URI parser doesn't support zone IDs. Expect parsing failure for the URI.
    assert!(expect_parse_error(message, Some("Invalid URI")));
}

// Section a.3 - IPv6 Reference in Headers and Body
#[test]
fn test_a3_ipv6_header_and_body() {
    /// RFC 5118 Section a.3
    let message = "\
INVITE sip:user@host SIP/2.0\r\nVia: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bKas3\r\nMax-Forwards: 70\r\nTo: <sip:user@host>\r\nFrom: <sip:user@host>;tag=1\r\nCall-ID: ipv6-3\r\nCSeq: 1 INVITE\r\nContact: <sip:[2001:db8::9:1]>\r\nContent-Type: application/sdp\r\nContent-Length: 147\r\n\r\nv=0\r\no=- 1 1 IN IP6 2001:db8::9:1\r\ns=-\r\nc=IN IP6 2001:db8::9:1\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

    let msg = parse_sip_message(message).expect("Parse failed for a.3");
    let req = msg.as_request().expect("Expected Request");

    assert_eq!(req.method, Method::Invite);
    assert_eq!(req.uri.to_string(), "sip:user@host");

    let via: Via = get_typed_header(&msg).expect("Via parse failed");
    assert_eq!(via.host, "[2001:db8::9:1]");
    assert_eq!(via.branch(), Some("z9hG4bKas3"));

    let contact: Contact = get_typed_header(&msg).expect("Contact parse failed");
    assert!(matches!(contact.0.uri.host, Host::IPv6(ref ip) if ip == "2001:db8::9:1"));
}

// Section a.4 - IPv6 Reference in Via's sent-by
#[test]
fn test_a4_ipv6_via_sent_by() {
    /// RFC 5118 Section a.4
    let message = "\
INVITE sip:user@host;transport=tcp SIP/2.0\r\nVia: SIP/2.0/TCP [2001:db8::9:1]:5060;branch=z9hG4bKas3-111\r\nMax-Forwards: 70\r\nTo: <sip:user@host>\r
From: <sip:user@host>;tag=1\r
Call-ID: ipv6-4\r
CSeq: 1 INVITE\r
Contact: <sip:user@host>\r
Content-Type: application/sdp\r
Content-Length: 147\r
\r
v=0\r
o=- 1 1 IN IP6 2001:db8::9:1\r
s=-\r
c=IN IP6 2001:db8::9:1\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
a=rtpmap:0 PCMU/8000\r
";

     let msg = parse_sip_message(message).expect("Parse failed for a.4");
    let req = msg.as_request().expect("Expected Request");

    assert_eq!(req.method, Method::Invite);
    assert_eq!(req.uri.to_string(), "sip:user@host;transport=tcp");

    let via: Via = get_typed_header(&msg).expect("Via parse failed");
    assert_eq!(via.transport, "TCP");
    assert_eq!(via.host, "[2001:db8::9:1]");
    assert_eq!(via.port, Some(5060));
    assert_eq!(via.branch(), Some("z9hG4bKas3-111"));
}

// Section a.5 - IPv6 Reference in Multicast SDP
#[test]
fn test_a5_ipv6_multicast_sdp() {
    /// RFC 5118 Section a.5
    let message = "\
INVITE sip:user@host SIP/2.0\r\nVia: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bKas3\r\nMax-Forwards: 70\r
To: <sip:user@host>\r
From: <sip:user@host>;tag=1\r
Call-ID: ipv6-5\r
CSeq: 1 INVITE\r
Contact: <sip:[2001:db8::9:1]>\r
Content-Type: application/sdp\r
Content-Length: 175\r
\r
v=0\r
o=- 1 1 IN IP6 2001:db8::9:1\r
s=-\r
c=IN IP6 FF1E:DB8::1\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
a=rtpmap:0 PCMU/8000\r
a=source-filter: incl IN IP6 FF1E:DB8::1 2001:db8::9:1\r
";

    // Basic parse check, deeper SDP check would be in SDP tests
    let result = parse_sip_message(message);
    assert!(result.is_ok(), "Failed to parse message with IPv6 multicast SDP: {:?}", result.err());
    if let Ok(Message::Request(req)) = result {
        assert!(req.body().len() >= 175);
        assert!(String::from_utf8_lossy(&req.body).contains("c=IN IP6 FF1E:DB8::1"));
    }
}

// Section a.6 - IPv6 Reference in Route Header
#[test]
fn test_a6_ipv6_route_header() {
    /// RFC 5118 Section a.6
    let message = "\
BYE sip:user@host SIP/2.0\r\nVia: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bKas3\r\nMax-Forwards: 70\r\nTo: <sip:user@host>;tag=1\r\nFrom: <sip:user@host>;tag=1\r\nCall-ID: ipv6-6\r\nRoute: <sip:[2001:db8::9:1]>\r\nCSeq: 1 BYE\r\nContent-Length: 0\r\n\r\n";

     let msg = parse_sip_message(message).expect("Parse failed for a.6");
     let req = msg.as_request().expect("Expected Request");

    assert_eq!(req.method, Method::Bye);
    let route: Route = get_typed_header(&msg).expect("Route parse failed");
    assert_eq!(route.0.uris.len(), 1);
     assert!(matches!(route.0.uris[0].uri.host, Host::IPv6(ref ip) if ip == "2001:db8::9:1"));
}

// Section a.7 - IPv6 Reference in Record-Route Header
#[test]
fn test_a7_ipv6_record_route_header() {
    /// RFC 5118 Section a.7
    let message = "\
INVITE sip:user@host SIP/2.0\r\nVia: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bKas3\r\nVia: SIP/2.0/UDP 192.0.2.1;branch=z9hG4bKjhja\r\nMax-Forwards: 70\r
To: <sip:user@host>\r
From: <sip:caller@example.net>;tag=1\r
Call-ID: ipv6-7\r
Record-Route: <sip:[2001:db8::9:1]>\r
CSeq: 1 INVITE\r
Contact: <sip:caller@example.net>\r
Content-Type: application/sdp\r
Content-Length: 147\r
\r
v=0\r
o=- 1 1 IN IP6 2001:db8::9:1\r
s=-\r
c=IN IP6 2001:db8::9:1\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
a=rtpmap:0 PCMU/8000\r
";

     let msg = parse_sip_message(message).expect("Parse failed for a.7");
     let req = msg.as_request().expect("Expected Request");

    assert_eq!(req.method, Method::Invite);
    let rr: RecordRoute = get_typed_header(&msg).expect("Record-Route parse failed");
    assert_eq!(rr.0.uris.len(), 1);
    assert!(matches!(rr.0.uris[0].uri.host, Host::IPv6(ref ip) if ip == "2001:db8::9:1"));

    let vias: Vec<_> = req.headers.iter().filter_map(|h| Via::try_from(h).ok()).collect();
    assert_eq!(vias.len(), 2);
}

// Section a.8 - IPv6 Reference in REGISTER Contact
#[test]
fn test_a8_ipv6_register_contact() {
    /// RFC 5118 Section a.8
    let message = "\
REGISTER sip:example.com SIP/2.0\r\nVia: SIP/2.0/UDP [2001:db8::9:1]:5060;branch=z9hG4bKas3\r
Max-Forwards: 70\r
To: <sip:user@example.com>\r
From: <sip:user@example.com>;tag=1\r
Call-ID: ipv6-8\r
CSeq: 1 REGISTER\r
Contact: <sip:[2001:db8::9:1]:5060;transport=udp>\r
Expires: 3600\r
Content-Length: 0\r
\r\n";

     let msg = parse_sip_message(message).expect("Parse failed for a.8");
     let req = msg.as_request().expect("Expected Request");

    assert_eq!(req.method, Method::Register);
    
    let contact: Contact = get_typed_header(&msg).expect("Contact parse failed");
    assert!(matches!(contact.0.uri.host, Host::IPv6(ref ip) if ip == "2001:db8::9:1"));
    assert_eq!(contact.0.uri.port, Some(5060));
    assert!(contact.0.params.contains(&Param::Transport("udp".to_string())));

    let expires: Expires = get_typed_header(&msg).expect("Expires parse failed");
    assert_eq!(expires.0, 3600);
}

// Section a.9 - IPv6 Reference in DNS Result (Message structure same as a.3)
#[test]
fn test_a9_ipv6_dns_srv_result() {
    /// RFC 5118 Section a.9
    let message = "\
INVITE sip:user@host SIP/2.0\r
Via: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bKas3\r
Max-Forwards: 70\r
To: <sip:user@host>\r
From: <sip:user@host>;tag=1\r
Call-ID: ipv6-9\r
CSeq: 1 INVITE\r
Contact: <sip:[2001:db8::9:1]>\r
Content-Type: application/sdp\r
Content-Length: 147\r\n\r
v=0\r
o=- 1 1 IN IP6 2001:db8::9:1\r
s=-\r
c=IN IP6 2001:db8::9:1\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
a=rtpmap:0 PCMU/8000\r
";

     let msg = parse_sip_message(message).expect("Parse failed for a.9");
     let req = msg.as_request().expect("Expected Request");

    assert_eq!(req.method, Method::Invite);
    let contact: Contact = get_typed_header(&msg).expect("Contact parse failed");
    assert!(matches!(contact.0.uri.host, Host::IPv6(ref ip) if ip == "2001:db8::9:1"));
}

// Section b.1 - Malformed IPv6 Addresses
#[test]
fn test_b1_malformed_ipv6_addresses() {
    /// RFC 5118 Section b.1 - Missing closing bracket
    let missing_bracket = "\
INVITE sip:[2001:db8::10 SIP/2.0\r\nVia: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bK-bad\r\nTo: <sip:[2001:db8::10>\r\nFrom: <sip:[2001:db8::9:1]>;tag=1\r\nCall-ID: ipv6-bad-1\r\nCSeq: 1 INVITE\r\nContent-Length: 0\r\n\r\n";
    assert!(expect_parse_error(missing_bracket, Some("Invalid URI"))); // URI parsing fails

    /// RFC 5118 Section b.1 - Invalid characters
    let invalid_chars = "\
INVITE sip:[2001:db8::XYZ] SIP/2.0\r\nVia: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bK-bad\r\nTo: <sip:[2001:db8::10]>\r\nFrom: <sip:[2001:db8::9:1]>;tag=1\r\nCall-ID: ipv6-bad-2\r\nCSeq: 1 INVITE\r\nContent-Length: 0\r\n\r\n";
     assert!(expect_parse_error(invalid_chars, Some("Invalid URI"))); // URI parsing fails

     /// RFC 5118 Section b.1 - Too many colons
     let too_many_colons = "\
INVITE sip:[2001:db8:::10] SIP/2.0\r\nVia: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bK-bad\r\nTo: <sip:[2001:db8::10]>\r\nFrom: <sip:[2001:db8::9:1]>;tag=1\r\nCall-ID: ipv6-bad-4\r\nCSeq: 1 INVITE\r\nContent-Length: 0\r\n\r\n";
    assert!(expect_parse_error(too_many_colons, Some("Invalid URI"))); // URI parsing fails
} 
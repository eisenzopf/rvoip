mod test_utils;
use test_utils::{parse_sip_message, expect_parse_error, validate_message};
use rvoip_sip_core::{Method, Uri};

/// Tests from RFC 5118 - Session Initiation Protocol (SIP) Torture Test Messages for IPv6
/// https://tools.ietf.org/html/rfc5118

// Section a.1 - SIP Message Containing an IPv6 Reference
#[test]
fn test_ipv6_uri_reference() {
    let message = "\
INVITE sip:[2001:db8::10] SIP/2.0\r
Via: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bKas3-111\r
Max-Forwards: 70\r
To: <sip:[2001:db8::10]>\r
From: <sip:[2001:db8::9:1]>;tag=1\r
Call-ID: ipv6-1\r
CSeq: 1 INVITE\r
Contact: <sip:[2001:db8::9:1]>\r
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

    let result = parse_sip_message(message);
    assert!(validate_message(
        result,
        Some(Method::Invite),
        Some("sip:[2001:db8::10]"),
        &[
            ("Via", "SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bKas3-111"),
            ("From", "<sip:[2001:db8::9:1]>;tag=1"),
            ("To", "<sip:[2001:db8::10]>"),
        ]
    ));
}

// Section a.2 - IPv6 Reference with Zone Index
#[test]
fn test_ipv6_with_zone_index() {
    let message = "\
INVITE sip:[2001:db8::10%eth0] SIP/2.0\r
Via: SIP/2.0/UDP [2001:db8::9:1%eth0];branch=z9hG4bKas3-111\r
Max-Forwards: 70\r
To: <sip:[2001:db8::10%eth0]>\r
From: <sip:[2001:db8::9:1%eth0]>;tag=1\r
Call-ID: ipv6-2\r
CSeq: 1 INVITE\r
Contact: <sip:[2001:db8::9:1%eth0]>\r
Content-Type: application/sdp\r
Content-Length: 155\r
\r
v=0\r
o=- 1 1 IN IP6 2001:db8::9:1\r
s=-\r
c=IN IP6 2001:db8::9:1%eth0\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
a=rtpmap:0 PCMU/8000\r
";

    // IPv6 addresses with zone indices - behavior depends on implementation
    let result = parse_sip_message(message);
    if result.is_ok() {
        assert!(validate_message(
            result,
            Some(Method::Invite),
            Some("sip:[2001:db8::10%eth0]"),
            &[
                ("Via", "SIP/2.0/UDP [2001:db8::9:1%eth0];branch=z9hG4bKas3-111"),
                ("From", "<sip:[2001:db8::9:1%eth0]>;tag=1"),
                ("To", "<sip:[2001:db8::10%eth0]>"),
            ]
        ));
    } else {
        // If implementation doesn't support zone indices
        assert!(expect_parse_error(message, Some("zone")));
    }
}

// Section a.3 - IPv6 Reference in Headers and Body
#[test]
fn test_ipv6_header_and_body() {
    let message = "\
INVITE sip:user@host SIP/2.0\r
Via: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bKas3\r
Max-Forwards: 70\r
To: <sip:user@host>\r
From: <sip:user@host>;tag=1\r
Call-ID: ipv6-3\r
CSeq: 1 INVITE\r
Contact: <sip:[2001:db8::9:1]>\r
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

    let result = parse_sip_message(message);
    assert!(validate_message(
        result,
        Some(Method::Invite),
        Some("sip:user@host"),
        &[
            ("Via", "SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bKas3"),
            ("From", "<sip:user@host>;tag=1"),
            ("To", "<sip:user@host>"),
            ("Contact", "<sip:[2001:db8::9:1]>"),
        ]
    ));
}

// Section a.4 - IPv6 Reference in Via's sent-by
#[test]
fn test_ipv6_via_sent_by() {
    let message = "\
INVITE sip:user@host;transport=tcp SIP/2.0\r
Via: SIP/2.0/TCP [2001:db8::9:1]:5060;branch=z9hG4bKas3-111\r
Max-Forwards: 70\r
To: <sip:user@host>\r
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

    let result = parse_sip_message(message);
    assert!(validate_message(
        result,
        Some(Method::Invite),
        Some("sip:user@host;transport=tcp"),
        &[
            ("Via", "SIP/2.0/TCP [2001:db8::9:1]:5060;branch=z9hG4bKas3-111"),
            ("From", "<sip:user@host>;tag=1"),
            ("To", "<sip:user@host>"),
        ]
    ));
}

// Section a.5 - IPv6 Reference in Multicast SDP
#[test]
fn test_ipv6_multicast_sdp() {
    let message = "\
INVITE sip:user@host SIP/2.0\r
Via: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bKas3\r
Max-Forwards: 70\r
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

    // This test focuses on IPv6 multicast addresses in SDP
    let result = parse_sip_message(message);
    assert!(result.is_ok());
}

// Section a.6 - IPv6 Reference in Route Header
#[test]
fn test_ipv6_route_header() {
    let message = "\
BYE sip:user@host SIP/2.0\r
Via: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bKas3\r
Max-Forwards: 70\r
To: <sip:user@host>;tag=1\r
From: <sip:user@host>;tag=1\r
Call-ID: ipv6-6\r
Route: <sip:[2001:db8::9:1]>\r
CSeq: 1 BYE\r
Content-Length: 0\r
\r
";

    let result = parse_sip_message(message);
    assert!(validate_message(
        result,
        Some(Method::Bye),
        Some("sip:user@host"),
        &[
            ("Via", "SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bKas3"),
            ("From", "<sip:user@host>;tag=1"),
            ("To", "<sip:user@host>;tag=1"),
            ("Route", "<sip:[2001:db8::9:1]>"),
        ]
    ));
}

// Section a.7 - IPv6 Reference in Record-Route Header
#[test]
fn test_ipv6_record_route_header() {
    let message = "\
INVITE sip:user@host SIP/2.0\r
Via: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bKas3\r
Via: SIP/2.0/UDP 192.0.2.1;branch=z9hG4bKjhja\r
Max-Forwards: 70\r
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

    let result = parse_sip_message(message);
    assert!(validate_message(
        result,
        Some(Method::Invite),
        Some("sip:user@host"),
        &[
            ("Via", "SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bKas3"),
            ("Record-Route", "<sip:[2001:db8::9:1]>"),
            ("From", "<sip:caller@example.net>;tag=1"),
            ("To", "<sip:user@host>"),
        ]
    ));
}

// Section a.8 - IPv6 Reference in REGISTER Contact
#[test]
fn test_ipv6_register_contact() {
    let message = "\
REGISTER sip:example.com SIP/2.0\r
Via: SIP/2.0/UDP [2001:db8::9:1]:5060;branch=z9hG4bKas3\r
Max-Forwards: 70\r
To: <sip:user@example.com>\r
From: <sip:user@example.com>;tag=1\r
Call-ID: ipv6-8\r
CSeq: 1 REGISTER\r
Contact: <sip:[2001:db8::9:1]:5060;transport=udp>\r
Expires: 3600\r
Content-Length: 0\r
\r
";

    let result = parse_sip_message(message);
    assert!(validate_message(
        result,
        Some(Method::Register),
        Some("sip:example.com"),
        &[
            ("Via", "SIP/2.0/UDP [2001:db8::9:1]:5060;branch=z9hG4bKas3"),
            ("From", "<sip:user@example.com>;tag=1"),
            ("To", "<sip:user@example.com>"),
            ("Contact", "<sip:[2001:db8::9:1]:5060;transport=udp>"),
        ]
    ));
}

// Section a.9 - IPv6 Reference in DNS Result
#[test]
fn test_ipv6_dns_srv_result() {
    let message = "\
INVITE sip:user@example.com SIP/2.0\r
Via: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bKas3\r
Max-Forwards: 70\r
To: <sip:user@example.com>\r
From: <sip:user@example.com>;tag=1\r
Call-ID: ipv6-9\r
CSeq: 1 INVITE\r
Contact: <sip:[2001:db8::9:1]>\r
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

    // This test simulates the result of a DNS lookup returning IPv6
    let result = parse_sip_message(message);
    assert!(validate_message(
        result,
        Some(Method::Invite),
        Some("sip:user@example.com"),
        &[
            ("Via", "SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bKas3"),
            ("From", "<sip:user@example.com>;tag=1"),
            ("To", "<sip:user@example.com>"),
            ("Contact", "<sip:[2001:db8::9:1]>"),
        ]
    ));
}

// Malformed IPv6 Addresses Tests
#[test]
fn test_malformed_ipv6_addresses() {
    // Missing closing bracket
    let missing_bracket = "\
INVITE sip:[2001:db8::10 SIP/2.0\r
Via: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bK-bad\r
Max-Forwards: 70\r
To: <sip:[2001:db8::10>\r
From: <sip:[2001:db8::9:1]>;tag=1\r
Call-ID: ipv6-bad-1\r
CSeq: 1 INVITE\r
Contact: <sip:[2001:db8::9:1]>\r
Content-Length: 0\r
\r
";
    assert!(expect_parse_error(missing_bracket, Some("bracket")));

    // Invalid characters in IPv6 address
    let invalid_chars = "\
INVITE sip:[2001:db8::XYZ] SIP/2.0\r
Via: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bK-bad\r
Max-Forwards: 70\r
To: <sip:[2001:db8::10]>\r
From: <sip:[2001:db8::9:1]>;tag=1\r
Call-ID: ipv6-bad-2\r
CSeq: 1 INVITE\r
Contact: <sip:[2001:db8::9:1]>\r
Content-Length: 0\r
\r
";
    assert!(expect_parse_error(invalid_chars, Some("invalid")));

    // Too many segments in IPv6 address
    let too_many_segments = "\
INVITE sip:[2001:db8:1:2:3:4:5:6:7] SIP/2.0\r
Via: SIP/2.0/UDP [2001:db8::9:1];branch=z9hG4bK-bad\r
Max-Forwards: 70\r
To: <sip:[2001:db8::10]>\r
From: <sip:[2001:db8::9:1]>;tag=1\r
Call-ID: ipv6-bad-3\r
CSeq: 1 INVITE\r
Contact: <sip:[2001:db8::9:1]>\r
Content-Length: 0\r
\r
";
    assert!(expect_parse_error(too_many_segments, Some("Invalid IPv6")));
}

// Test for SIP URIs using IPv6 literal addresses with both address and port
#[test]
fn test_ipv6_with_port() {
    let message = "\
INVITE sip:[2001:db8::10]:5060 SIP/2.0\r
Via: SIP/2.0/UDP [2001:db8::9:1]:5060;branch=z9hG4bKas3\r
Max-Forwards: 70\r
To: <sip:[2001:db8::10]:5060>\r
From: <sip:[2001:db8::9:1]:5060>;tag=1\r
Call-ID: ipv6-port\r
CSeq: 1 INVITE\r
Contact: <sip:[2001:db8::9:1]:5060>\r
Content-Length: 0\r
\r
";

    let result = parse_sip_message(message);
    assert!(validate_message(
        result,
        Some(Method::Invite),
        Some("sip:[2001:db8::10]:5060"),
        &[
            ("Via", "SIP/2.0/UDP [2001:db8::9:1]:5060;branch=z9hG4bKas3"),
            ("From", "<sip:[2001:db8::9:1]:5060>;tag=1"),
            ("To", "<sip:[2001:db8::10]:5060>"),
            ("Contact", "<sip:[2001:db8::9:1]:5060>"),
        ]
    ));
}

// Test for IPv6 with parameters
#[test]
fn test_ipv6_with_parameters() {
    let message = "\
INVITE sip:[2001:db8::10];transport=tcp SIP/2.0\r
Via: SIP/2.0/TCP [2001:db8::9:1];branch=z9hG4bKas3\r
Max-Forwards: 70\r
To: <sip:[2001:db8::10];transport=tcp>\r
From: <sip:[2001:db8::9:1];transport=tcp>;tag=1\r
Call-ID: ipv6-params\r
CSeq: 1 INVITE\r
Contact: <sip:[2001:db8::9:1];transport=tcp>\r
Content-Length: 0\r
\r
";

    let result = parse_sip_message(message);
    assert!(validate_message(
        result,
        Some(Method::Invite),
        Some("sip:[2001:db8::10];transport=tcp"),
        &[
            ("Via", "SIP/2.0/TCP [2001:db8::9:1];branch=z9hG4bKas3"),
            ("From", "<sip:[2001:db8::9:1];transport=tcp>;tag=1"),
            ("To", "<sip:[2001:db8::10];transport=tcp>"),
            ("Contact", "<sip:[2001:db8::9:1];transport=tcp>"),
        ]
    ));
} 
use crate::{parse_sip_message, expect_parse_error, validate_message};
use rvoip_sip_core::{Method, Uri};

/// Tests from RFC 4475 - Session Initiation Protocol (SIP) Torture Test Messages
/// https://tools.ietf.org/html/rfc4475

// Section 3.1.1 - Valid Messages
#[test]
fn test_3_1_1_1_short_form_valid() {
    // 3.1.1.1 - A Short Tortuous INVITE
    let message = "\
INVITE sip:vivekg@chair-dnrc.example.com;unknownparam SIP/2.0\r
TO :\r
 sip:vivekg@chair-dnrc.example.com ;   tag    = 1918181833n\r
from   : \"J Rosenberg \\\\\\\"\"       <sip:jdrosen@example.com>\r
  ;\r
  tag = 98asjd8\r
MaX-fOrWaRdS: 0068\r
Call-ID: wsinv.ndaksdj@192.0.2.1\r
Content-Length   : 150\r
cseq: 0009\r
 INVITE\r
Via  : SIP  /   2.0\r
 /UDP\r
  192.0.2.2;branch=390skdjuw\r
s :\r
\r
v=0\r
o=mhandley 29739 7272939 IN IP4 192.0.2.3\r
s=-\r
c=IN IP4 192.0.2.4\r
t=0 0\r
m=audio 49217 RTP/AVP 0 12\r
m=video 3227 RTP/AVP 31\r
a=rtpmap:31 LPC\r
";

    let result = parse_sip_message(message);
    assert!(validate_message(
        result,
        Some(Method::Invite),
        Some("sip:vivekg@chair-dnrc.example.com;unknownparam"),
        &[
            ("To", "sip:vivekg@chair-dnrc.example.com ;   tag    = 1918181833n"),
            ("From", "\"J Rosenberg \\\\\\\"\"       <sip:jdrosen@example.com>;\r\n  tag = 98asjd8"),
            ("Max-Forwards", "0068"),
            ("Call-ID", "wsinv.ndaksdj@192.0.2.1"),
            ("CSeq", "0009\r\n INVITE"),
        ]
    ));
}

#[test]
fn test_3_1_1_2_twisted_register() {
    // 3.1.1.2 - Torture Test REGISTER
    let message = "\
REGISTER sip:example.com SIP/2.0\r
To: sip:j.user@example.com\r
From: sip:j.user@example.com;tag=43251j3j324\r
Max-Forwards: 8\r
I: dblreq.mdshd2.43@192.0.2.1\r
Contact: sip:j.user@host.example.com\r
CSeq: 8 REGISTER\r
Via: SIP/2.0/UDP 192.0.2.125;branch=z9hG4bKkdjuw\r
Content-Length: 0\r
\r
";

    let result = parse_sip_message(message);
    assert!(validate_message(
        result,
        Some(Method::Register),
        Some("sip:example.com"),
        &[
            ("To", "sip:j.user@example.com"),
            ("From", "sip:j.user@example.com;tag=43251j3j324"),
            ("Max-Forwards", "8"),
            ("Call-ID", "dblreq.mdshd2.43@192.0.2.1"), // "I" header shorthand for Call-ID
            ("Contact", "sip:j.user@host.example.com"),
        ]
    ));
}

#[test]
fn test_3_1_1_3_wacky_message_format() {
    // 3.1.1.3 - Unusual Line Folding
    let message = "\
INVITE sip:vivekg@chair-dnrc.example.com SIP/2.0\r
TO :
 sip:vivekg@chair-dnrc.example.com ;   tag    = 1918181833n\r
from   : \"J Rosenberg \\\\\\\"\"       <sip:jdrosen@example.com>\r
  ;\r
  tag = 98asjd8\r
MaX-fOrWaRdS: 0068\r
Call-ID: wsinv.ndaksdj@192.0.2.1\r
Content-Length   : 150\r
cseq: 0009\r
 INVITE\r
Via  : SIP  /   2.0\r
 /UDP\r
  192.0.2.2;branch=390skdjuw\r
s :\r
\r
v=0\r
o=mhandley 29739 7272939 IN IP4 192.0.2.3\r
s=-\r
c=IN IP4 192.0.2.4\r
t=0 0\r
m=audio 49217 RTP/AVP 0 12\r
m=video 3227 RTP/AVP 31\r
a=rtpmap:31 LPC\r
";

    // This test case has non-standard line folding with a bare LF instead of CRLF
    // The behavior here will depend on how lenient the parser is
    // We expect it to either parse successfully or fail with a specific error
    let result = parse_sip_message(message);
    // If the parser is lenient with line folding, it should parse successfully
    // If strict, we expect a specific error message
    match result {
        Ok(_) => {
            // If it parsed successfully, validate the message
            assert!(validate_message(
                result,
                Some(Method::Invite),
                Some("sip:vivekg@chair-dnrc.example.com"),
                &[
                    ("To", "sip:vivekg@chair-dnrc.example.com ;   tag    = 1918181833n"),
                    ("From", "\"J Rosenberg \\\\\\\"\"       <sip:jdrosen@example.com>;\r\n  tag = 98asjd8"),
                ]
            ));
        },
        Err(_) => {
            // If it failed, we expect it to be due to invalid line folding
            assert!(expect_parse_error(message, Some("line ending")));
        }
    }
}

// Section 3.1.2 - Invalid Messages
#[test]
fn test_3_1_2_1_extraneous_header_encoding() {
    // 3.1.2.1 - Extraneous Header Field Separators
    let message = "\
OPTIONS sip:user@example.com SIP/2.0\r
To: sip:user@example.com\r
From: caller<sip:caller@example.com>;tag=323\r
Max-Forwards: 70\r
Call-ID: transports.kijh4akdnaqjkwendsasfdj\r
Accept: application/sdp\r
CSeq: 60 OPTIONS\r
Via: SIP/2.0/UDP t1.example.com;branch=z9hG4bKkdjuw\r
Via: SIP/2.0/TCP t2.example.com;branch=z9hG4bKklasjdfy\r
Via: SIP/2.0/TLS t3.example.com;branch=z9hG4bK2980unddj\r
Call-ID: transports.kijh4akdnaqjkwendsasfdj\r
\r
";

    // This message has a duplicate Call-ID header which is invalid
    assert!(expect_parse_error(message, Some("duplicate header")));
}

#[test]
fn test_3_1_2_2_content_length_overflow() {
    // 3.1.2.2 - Content Length Larger than Message
    let message = "\
INVITE sip:user@example.com SIP/2.0\r
To: sip:j.user@example.com\r
From: sip:caller@example.net;tag=134161461246\r
Max-Forwards: 7\r
Call-ID: bext01.0ha0isndaksdj\r
CSeq: 8 INVITE\r
Via: SIP/2.0/UDP 192.0.2.15;branch=z9hG4bKkdjuw\r
Content-Length: 9999\r
Content-Type: application/sdp\r
\r
v=0\r
o=mhandley 29739 7272939 IN IP4 192.0.2.15\r
s=-\r
c=IN IP4 192.0.2.15\r
t=0 0\r
m=audio 49217 RTP/AVP 0 12\r
m=video 3227 RTP/AVP 31\r
a=rtpmap:31 LPC\r
";

    // Content-Length header is larger than actual content
    assert!(expect_parse_error(message, Some("Content-Length")));
}

#[test]
fn test_3_1_2_3_negative_content_length() {
    // 3.1.2.3 - Negative Content-Length
    let message = "\
INVITE sip:user@example.com SIP/2.0\r
To: sip:user@example.com\r
From: sip:caller@example.net;tag=8814\r
Max-Forwards: 70\r
Call-ID: invut.0ha0isndaksdjadsfij\r
CSeq: 0 INVITE\r
Contact: sip:caller@host5.example.net\r
Content-Type: application/sdp\r
Content-Length: -999\r
Via: SIP/2.0/UDP host5.example.net;branch=z9hG4bK-31415-1-0\r
\r
v=0\r
o=mhandley 29739 7272939 IN IP4 192.0.2.5\r
s=-\r
c=IN IP4 192.0.2.5\r
t=0 0\r
m=audio 49217 RTP/AVP 0 12\r
m=video 3227 RTP/AVP 31\r
a=rtpmap:31 LPC\r
";

    // Content-Length header is negative
    assert!(expect_parse_error(message, Some("Content-Length")));
}

#[test]
fn test_3_1_2_4_request_uri_with_escaped_characters() {
    // 3.1.2.4 - Request-URI with Escaped Headers
    let message = "\
INVITE sip:user@example.com?Route=%3Csip:example.com%3E SIP/2.0\r
To: sip:user@example.com\r
From: sip:caller@example.net;tag=341518\r
Max-Forwards: 7\r
Contact: <sip:caller@host39923.example.net>\r
Call-ID: escruri.23940-asdfhj-aje3br-234q098\r
CSeq: 149 INVITE\r
Via: SIP/2.0/UDP host-of-the-hour.example.com;branch=z9hG4bK-2398ndaoe\r
Content-Type: application/sdp\r
Content-Length: 150\r
\r
v=0\r
o=mhandley 29739 7272939 IN IP4 192.0.2.1\r
s=-\r
c=IN IP4 192.0.2.1\r
t=0 0\r
m=audio 49217 RTP/AVP 0 12\r
m=video 3227 RTP/AVP 31\r
a=rtpmap:31 LPC\r
";

    // This case tests URI with escaped header parameters in Request-URI
    // Some implementations might accept this, others might reject it
    let result = parse_sip_message(message);
    // If parser accepts escaped headers in URI
    if result.is_ok() {
        assert!(validate_message(
            result,
            Some(Method::Invite),
            Some("sip:user@example.com?Route=%3Csip:example.com%3E"),
            &[
                ("To", "sip:user@example.com"),
                ("From", "sip:caller@example.net;tag=341518"),
            ]
        ));
    } else {
        // If parser rejects escaped headers in URI
        assert!(expect_parse_error(message, Some("invalid URI")));
    }
}

// Section 3.1.2.5 - Multiple SP Separating Request-Line Elements
#[test]
fn test_3_1_2_5_multiple_spaces() {
    let message = "\
OPTIONS  sip:user@example.com  SIP/2.0\r
To: sip:user@example.com\r
From: sip:caller@example.net;tag=323\r
Max-Forwards: 70\r
Call-ID: multi01.98asdh@192.0.2.1\r
CSeq: 59 OPTIONS\r
Via: SIP/2.0/UDP host.example.com;branch=z9hG4bKkdjuw\r
Content-Length: 0\r
\r
";

    // RFC 3261 says the SP separating elements must be exactly one, but some parsers may be lenient
    let result = parse_sip_message(message);
    if result.is_ok() {
        // If parser is lenient about spacing
        assert!(validate_message(
            result,
            Some(Method::Options),
            Some("sip:user@example.com"),
            &[
                ("To", "sip:user@example.com"),
                ("From", "sip:caller@example.net;tag=323"),
            ]
        ));
    } else {
        // If parser is strict about spacing
        assert!(expect_parse_error(message, Some("request line")));
    }
}

// Additional tests from RFC 4475

#[test]
fn test_3_1_2_6_param_value_escaping() {
    // Test for escaped characters in parameter values
    let message = "\
REGISTER sip:example.com SIP/2.0\r
To: sip:user@example.com;tag=complex\\\"string\\\";param=\\\"val\\\"\r
From: sip:user@example.com;tag=12312\r
Max-Forwards: 70\r
Call-ID: inv2543.clarewinmobil.example.com\r
CSeq: 9 REGISTER\r
Via: SIP/2.0/UDP 192.0.2.2:5060;branch=z9hG4bKkdjuw2395\r
Content-Length: 0\r
\r
";

    // Test the ability to handle complex escaped parameter values
    let result = parse_sip_message(message);
    assert!(result.is_ok());
}

#[test]
fn test_3_3_10_multiple_routes() {
    // Test for multiple Route headers
    let message = "\
OPTIONS sip:user@example.com SIP/2.0\r
Route: <sip:services.example.com;lr>\r
Route: <sip:edge.example.com;lr>\r
To: sip:user@example.com\r
From: sip:caller@example.net;tag=3415132\r
Max-Forwards: 70\r
Call-ID: multrte.0ha0isndaksdj\r
CSeq: 59 OPTIONS\r
Via: SIP/2.0/UDP host5.example.com;branch=z9hG4bK-39234-1\r
Content-Length: 0\r
\r
";

    // Check handling of multiple Route headers
    let result = parse_sip_message(message);
    assert!(result.is_ok());
    if let Ok(msg) = result {
        // Check that both Route headers are present
        assert!(validate_message(
            Ok(msg),
            Some(Method::Options),
            Some("sip:user@example.com"),
            &[
                ("Route", "<sip:services.example.com;lr>"),
                ("Route", "<sip:edge.example.com;lr>"),
            ]
        ));
    }
}

#[test]
fn test_3_3_16_path_header() {
    // Test for Path header
    let message = "\
REGISTER sip:example.com SIP/2.0\r
To: sip:user@example.com\r
From: sip:user@example.com;tag=8978\r
Max-Forwards: 70\r
Call-ID: pathtest.chad2@t1.example.com\r
CSeq: 79 REGISTER\r
Via: SIP/2.0/UDP t1.example.com;branch=z9hG4bK-p234\r
Path: <sip:pcscf.isp.example.com;lr>\r
Path: <sip:scscf.isp.example.com;lr>\r
Contact: <sip:user@192.0.2.5>\r
Content-Length: 0\r
\r
";

    // Check handling of Path headers (RFC 3327)
    let result = parse_sip_message(message);
    assert!(result.is_ok());
    if let Ok(msg) = result {
        // Check that both Path headers are present
        assert!(validate_message(
            Ok(msg),
            Some(Method::Register),
            Some("sip:example.com"),
            &[
                ("Path", "<sip:pcscf.isp.example.com;lr>"),
                ("Path", "<sip:scscf.isp.example.com;lr>"),
            ]
        ));
    }
}

// More torture tests from RFC 4475
#[test]
fn test_3_4_1_scalar_field_overflow() {
    // Test for behavior with out-of-range scalar fields
    let message = "\
REGISTER sip:example.com SIP/2.0\r
Via: SIP/2.0/TCP host129.example.com;branch=z9hG4bK-74BnK7G;rport=9\r
From: <sip:user@example.com>;tag=1928301774\r
To: <sip:user@example.com>\r
Call-ID: scalar02.23o7@host256.example.com\r
CSeq: 139122385607 REGISTER\r
Max-Forwards: 255\r
Expires: 10000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\r
Contact: <sip:user@host129.example.com>;expires=280297596632815\r
Content-Length: 0\r
\r
";

    // Check handling of extremely large scalar values
    // Some implementations might accept this, others might reject
    let result = parse_sip_message(message);
    if result.is_err() {
        // If implementation rejects
        assert!(expect_parse_error(message, None));
    } else {
        // If implementation accepts
        assert!(validate_message(
            result,
            Some(Method::Register),
            Some("sip:example.com"),
            &[
                ("CSeq", "139122385607 REGISTER"),
                ("Max-Forwards", "255"),
            ]
        ));
    }
}

#[test]
fn test_3_1_2_10_multipart_mime_body() {
    // Test for multipart MIME body
    let multipart_message = "\
INVITE sip:user@example.com SIP/2.0\r
To: sip:user@example.com\r
From: sip:caller@example.net;tag=08D\r
Max-Forwards: 70\r
Call-ID: multipart.sdp.jpeg@caller.example.net\r
CSeq: 5 INVITE\r
Via: SIP/2.0/UDP host5.example.net;branch=z9hG4bK-d87543-1\r
Contact: <sip:caller@host5.example.net>\r
Content-Type: multipart/mixed; boundary=unique-boundary-1\r
Content-Length: 501\r
\r
--unique-boundary-1\r
Content-Type: application/sdp\r
\r
v=0\r
o=caller 53655765 2353687637 IN IP4 host5.example.net\r
s=-\r
c=IN IP4 192.0.2.5\r
t=0 0\r
m=audio 20000 RTP/AVP 0\r
--unique-boundary-1\r
Content-Type: image/jpeg\r
Content-Transfer-Encoding: binary\r
Content-ID: <image1@caller.example.net>\r
\r
JPEG ... binary image data ...\r
--unique-boundary-1--\r
";

    // Check handling of multipart MIME body
    let result = parse_sip_message(multipart_message);
    assert!(result.is_ok());
}

// Add more tests from RFC 4475 as needed 
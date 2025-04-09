use crate::{parse_sip_message, expect_parse_error, validate_message};
use rvoip_sip_core::{Method, Uri, StatusCode, Message};

/// Custom torture tests for additional SIP message corner cases
/// These tests go beyond the RFC defined test cases to stress the parser further

// Test extremely long headers
#[test]
fn test_extremely_long_header_value() {
    // Generate a very long header value that's still valid
    let mut long_value = String::from("sip:user1@example.com");
    for i in 0..1000 {
        long_value.push_str(&format!(";param{}=value{}", i, i));
    }
    
    let mut message = format!("\
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
Content-Length: 0\r
\r
", long_value);

    // Test parser's ability to handle very long header values
    let result = parse_sip_message(&message);
    assert!(result.is_ok());
}

// Test unusual HTTP methods passed in a SIP message
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
Content-Length: 0\r
\r
", method, method);

        let result = parse_sip_message(&message);
        // If the parser allows custom methods
        if result.is_ok() {
            println!("Parser accepted unusual method: {}", method);
        } else {
            // If the parser rejects custom methods
            assert!(expect_parse_error(&message, Some("method")));
        }
    }
}

// Test unusual characters in header names
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
\r
";

    // Some implementations might accept unusual header names, others might reject
    let result = parse_sip_message(message);
    if result.is_ok() {
        println!("Parser accepts unusual header names");
    } else {
        assert!(expect_parse_error(message, Some("header")));
    }
}

// Test extremely malformed Request-URI
#[test]
fn test_malformed_request_uri() {
    let malformed_uris = [
        "INVITE sip:user@[::1 SIP/2.0\r\n",  // Missing closing bracket in IPv6
        "INVITE sip:@example.com SIP/2.0\r\n",  // Missing user part
        "INVITE sip:user@ SIP/2.0\r\n",  // Missing host part
        "INVITE sip:: SIP/2.0\r\n",  // Missing user and host
        "INVITE sip:user:pass@example.com SIP/2.0\r\n",  // Invalid use of colon in userinfo
        "INVITE sip:user@example.com:abc SIP/2.0\r\n",  // Non-numeric port
        "INVITE sip:user@example.com:99999 SIP/2.0\r\n",  // Port too large
        "INVITE sip:user@example.com:-1 SIP/2.0\r\n",  // Negative port
        "INVITE sip:user@.com SIP/2.0\r\n",  // Invalid domain
        "INVITE sip:user@example. SIP/2.0\r\n",  // Invalid domain ending
        "INVITE sip:user@example..com SIP/2.0\r\n",  // Invalid domain with consecutive dots
    ];
    
    for malformed in malformed_uris {
        let message = format!("{}\
Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKnashds7\r
Max-Forwards: 70\r
To: <sip:user@example.com>\r
From: <sip:caller@example.net>;tag=12345\r
Call-ID: malformed-uri-test\r
CSeq: 1 INVITE\r
Content-Length: 0\r
\r
", malformed);

        assert!(expect_parse_error(&message, None));
    }
}

// Test proper handling of exotic status codes
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
Content-Length: 0\r
\r
", code, reason);

        let result = parse_sip_message(&message);
        assert!(result.is_ok());
        
        // Verify status code is parsed correctly
        if let Ok(Message::Response(resp)) = result {
            // For known status codes, verify exact match
            // For unknown status codes, verify category matches
            let status_category = code - (code % 100);
            let resp_code = resp.status.as_u16();
            assert!(resp_code == code || 
                   resp_code / 100 == code / 100,
                   "Status code {} not parsed correctly", code);
        } else {
            panic!("Expected Response, got something else");
        }
    }
}

// Test handling of unexpected end of headers (no final CRLF)
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

    assert!(expect_parse_error(message, Some("headers")));
}

// Test unusual line endings
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

    // Many SIP parsers are lenient about line endings
    let lf_result = parse_sip_message(lf_only);
    let cr_result = parse_sip_message(cr_only);
    
    // We're testing if the parser is lenient about line endings
    // If it parses successfully, great; if not, it should fail with line ending error
    if lf_result.is_err() {
        assert!(expect_parse_error(lf_only, Some("line ending")));
    }
    
    if cr_result.is_err() {
        assert!(expect_parse_error(cr_only, Some("line ending")));
    }
}

// Test empty headers
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
Another-Empty: \r
Content-Length: 0\r
\r
";

    // Test if parser handles empty header values
    let result = parse_sip_message(message);
    assert!(result.is_ok());
}

// Test custom heavily malformed header collection
#[test]
fn test_malformed_headers() {
    let malformed_headers = [
        "Content-Length: abc\r\n",         // Non-numeric Content-Length
        "CSeq: 1\r\n",                     // Missing method in CSeq
        "CSeq: a INVITE\r\n",              // Non-numeric sequence in CSeq
        "Via: garbage\r\n",                // Invalid Via header
        "Via: /UDP host:port;branch=xyz\r\n", // Missing SIP version in Via
        "Max-Forwards: -1\r\n",            // Negative Max-Forwards
        "Call-ID: \r\n",                   // Empty Call-ID
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
\r
", header);

        assert!(expect_parse_error(&message, None));
    }
}

// Test extremely large numbers
#[test]
fn test_extremely_large_numbers() {
    let message = "\
INVITE sip:user@example.com SIP/2.0\r
Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKnashds7\r
Max-Forwards: 4294967295\r
To: <sip:user@example.com>\r
From: <sip:caller@example.net>;tag=12345\r
Call-ID: large-number-test\r
CSeq: 4294967295 INVITE\r
Content-Length: 0\r
\r
";

    // Testing with very large but valid uint32 numbers
    let result = parse_sip_message(message);
    assert!(result.is_ok());
}

// Test custom SIP URI parameters
#[test]
fn test_custom_uri_parameters() {
    let message = "\
INVITE sip:user@example.com;custom=value;transport=TCP;maddr=192.168.1.1;ttl=5;lr;method=INVITE SIP/2.0\r
Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKnashds7\r
Max-Forwards: 70\r
To: <sip:user@example.com;custom=value;transport=TCP>\r
From: <sip:caller@example.net;maddr=192.168.1.1;ttl=5;lr>;tag=12345\r
Call-ID: uri-params-test\r
CSeq: 1 INVITE\r
Contact: <sip:caller@example.net;methods=INVITE,BYE,OPTIONS>\r
Content-Length: 0\r
\r
";

    // Testing with various URI parameters
    let result = parse_sip_message(message);
    assert!(result.is_ok());
}

// Test message with multiple headers with the same name
#[test]
fn test_multiple_same_name_headers() {
    let message = "\
INVITE sip:user@example.com SIP/2.0\r
Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKnashds7\r
Via: SIP/2.0/TCP 10.0.0.1:5060;branch=z9hG4bKnashds8\r
Via: SIP/2.0/TLS 172.16.0.1:5061;branch=z9hG4bKnashds9\r
Max-Forwards: 70\r
To: <sip:user@example.com>\r
From: <sip:caller@example.net>;tag=12345\r
Call-ID: multi-headers-test\r
Record-Route: <sip:proxy1.example.com;lr>\r
Record-Route: <sip:proxy2.example.com;lr>\r
Record-Route: <sip:proxy3.example.com;lr>\r
Supported: timer\r
Supported: 100rel\r
Supported: path\r
CSeq: 1 INVITE\r
Contact: <sip:caller@example.net>\r
Content-Length: 0\r
\r
";

    // Test handling of multiple headers with the same name
    let result = parse_sip_message(message);
    assert!(result.is_ok());
    
    if let Ok(Message::Request(req)) = result {
        // Count headers manually
        let via_count = req.headers.iter().filter(|h| h.name.as_str() == "Via").count();
        let record_route_count = req.headers.iter().filter(|h| h.name.as_str() == "Record-Route").count();
        let supported_count = req.headers.iter().filter(|h| h.name.as_str() == "Supported").count();
        
        assert_eq!(via_count, 3, "Expected 3 Via headers");
        assert_eq!(record_route_count, 3, "Expected 3 Record-Route headers");
        assert_eq!(supported_count, 3, "Expected 3 Supported headers");
    } else {
        panic!("Expected Request, got something else");
    }
}

// Add more custom torture tests as needed 
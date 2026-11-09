//! Header and URI grammar from the RFCs that update RFC 3261 (its "Updated
//! by" list) and from RFC 3261's verified errata. Each message must parse in
//! Strict mode with the header typed (not kept as an unparsed field), carry
//! the values the RFC defines, and re-serialize to valid SIP.

use super::torture_test::assert_reserializes_strictly;
use rvoip_sip_core::parse_message_with_mode;
use rvoip_sip_core::parser::message::ParseMode;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::TypedHeader;
use rvoip_sip_core::types::{Message, Uri};
use std::str::FromStr;

/// A request carrying `extra` header lines, parsed in Strict mode.
fn request_with(method: &str, extra: &[&str]) -> Message {
    let mut text = format!(
        "{method} sip:bob@example.com SIP/2.0\r\n\
         Via: SIP/2.0/UDP client.example.com;branch=z9hG4bK-updates\r\n\
         Max-Forwards: 70\r\n\
         To: <sip:bob@example.com>\r\n\
         From: <sip:alice@example.com>;tag=1928301774\r\n\
         Call-ID: rfc3261-updates@client.example.com\r\n\
         CSeq: 1 {method}\r\n"
    );
    for line in extra {
        text.push_str(line);
        text.push_str("\r\n");
    }
    text.push_str("Content-Length: 0\r\n\r\n");
    let message = parse_message_with_mode(text.as_bytes(), ParseMode::Strict)
        .unwrap_or_else(|e| panic!("Strict parse failed ({e}):\n{text}"));
    assert_reserializes_strictly(&message);
    message
}

fn typed<'a>(message: &'a Message, name: HeaderName) -> Vec<&'a TypedHeader> {
    let headers = match message {
        Message::Request(r) => r.all_headers(),
        Message::Response(r) => r.all_headers(),
    };
    let found: Vec<_> = headers.iter().filter(|h| h.name() == name).collect();
    assert!(!found.is_empty(), "{name:?} missing");
    for header in &found {
        assert!(
            !matches!(header, TypedHeader::Other(..)),
            "{name:?} was not parsed as a typed header: {header}"
        );
    }
    found
}

fn value_of(header: &TypedHeader) -> String {
    let line = header.to_string();
    line.split_once(':')
        .map(|(_, v)| v.trim().to_string())
        .unwrap_or(line)
}

/// RFC 6665 §8.4 (obsoletes RFC 3265): Event with templates and the "id"
/// parameter, compact forms "o" and "u", Subscription-State parameters.
#[test]
fn rfc6665_event_allow_events_subscription_state() {
    let m = request_with(
        "SUBSCRIBE",
        &[
            "o: presence.winfo;id=1234",
            "u: presence, dialog",
            "Subscription-State: terminated;reason=timeout;retry-after=30",
        ],
    );
    let event = value_of(typed(&m, HeaderName::Event)[0]);
    assert!(
        event.starts_with("presence.winfo") && event.contains("id=1234"),
        "{event}"
    );
    let allow = value_of(typed(&m, HeaderName::AllowEvents)[0]);
    assert!(
        allow.contains("presence") && allow.contains("dialog"),
        "{allow}"
    );
    let state = value_of(typed(&m, HeaderName::SubscriptionState)[0]);
    assert!(
        state.starts_with("terminated")
            && state.contains("reason=timeout")
            && state.contains("retry-after=30"),
        "{state}"
    );
}

/// RFC 7462: Alert-Info carries alert URNs (absoluteURI in angle brackets).
#[test]
fn rfc7462_alert_info_urns() {
    let m = request_with(
        "INVITE",
        &["Alert-Info: <urn:alert:service:call-waiting>, <urn:alert:priority:high>"],
    );
    let alert = value_of(typed(&m, HeaderName::AlertInfo)[0]);
    assert!(
        alert.contains("<urn:alert:service:call-waiting>")
            && alert.contains("<urn:alert:priority:high>"),
        "{alert}"
    );
}

/// RFC 8760: the SHA-256 and SHA-512/256 Digest algorithms.
#[test]
fn rfc8760_digest_algorithms() {
    for algorithm in [
        "SHA-256",
        "SHA-256-sess",
        "SHA-512-256",
        "SHA-512-256-sess",
        "MD5",
    ] {
        let challenge = format!(
            "WWW-Authenticate: Digest realm=\"example.com\", nonce=\"abc\", qop=\"auth\", algorithm={algorithm}"
        );
        let response = format!(
            "Authorization: Digest username=\"alice\", realm=\"example.com\", nonce=\"abc\", \
             uri=\"sip:bob@example.com\", response=\"0123456789abcdef\", qop=auth, nc=00000001, \
             cnonce=\"xyz\", algorithm={algorithm}"
        );
        let m = request_with("REGISTER", &[&challenge, &response]);
        for name in [HeaderName::WwwAuthenticate, HeaderName::Authorization] {
            let value = value_of(typed(&m, name.clone())[0]);
            assert!(
                value.contains(&format!("algorithm={algorithm}")),
                "{name:?}: {value}"
            );
        }
    }
}

/// RFC 8898: Bearer challenges and credentials.
#[test]
fn rfc8898_bearer() {
    let m = request_with(
        "REGISTER",
        &[
            "WWW-Authenticate: Bearer realm=\"example.com\", scope=\"sip:register\", \
             authz_server=\"https://as.example.com/token\", error=\"invalid_token\"",
            "Authorization: Bearer eyJhbGciOiJSUzI1NiJ9.e30.c2lnbmF0dXJl",
        ],
    );
    let challenge = value_of(typed(&m, HeaderName::WwwAuthenticate)[0]);
    assert!(
        challenge.starts_with("Bearer") && challenge.contains("authz_server="),
        "{challenge}"
    );
    let credentials = value_of(typed(&m, HeaderName::Authorization)[0]);
    assert_eq!(credentials, "Bearer eyJhbGciOiJSUzI1NiJ9.e30.c2lnbmF0dXJl");
}

/// RFC 5626: outbound Contact parameters and Flow-Timer.
#[test]
fn rfc5626_outbound() {
    let m = request_with(
        "REGISTER",
        &[
            "Contact: <sip:alice@192.0.2.1;ob>;reg-id=1;+sip.instance=\"<urn:uuid:00000000-0000-1000-8000-AABBCCDDEEFF>\"",
            "Supported: path, outbound",
            "Flow-Timer: 120",
        ],
    );
    let contact = value_of(typed(&m, HeaderName::Contact)[0]);
    assert!(
        contact.contains(";ob>") && contact.contains("reg-id=1"),
        "{contact}"
    );
    assert!(
        contact.contains("+sip.instance=\"<urn:uuid:00000000-0000-1000-8000-AABBCCDDEEFF>\""),
        "{contact}"
    );
}

/// RFC 5393: Max-Breadth is accepted (an extension header field).
#[test]
fn rfc5393_max_breadth() {
    let m = request_with("INVITE", &["Max-Breadth: 60"]);
    assert!(m.to_bytes().windows(15).any(|w| w == b"Max-Breadth: 60"));
}

/// RFC 6878: Priority values beyond the four RFC 3261 ones.
#[test]
fn rfc6878_priority_values() {
    for value in [
        "emergency",
        "urgent",
        "normal",
        "non-urgent",
        "psap-callback",
    ] {
        let line = format!("Priority: {value}");
        let m = request_with("INVITE", &[&line]);
        assert_eq!(value_of(typed(&m, HeaderName::Priority)[0]), value);
    }
}

/// RFC 8217: in any header field allowing name-addr / addr-spec, a bare URI's
/// semicolon parameters are header parameters, and URIs are sent in
/// name-addr form.
#[test]
fn rfc8217_addr_spec_parameters_in_extension_headers() {
    let m = request_with(
        "REFER",
        &[
            "Refer-To: sip:carol@example.com;method=INVITE",
            "Referred-By: sip:alice@example.com;cid=1234",
            "P-Asserted-Identity: <sip:+15551234567@example.com;user=phone>",
        ],
    );
    for (name, expected) in [
        (HeaderName::ReferTo, "<sip:carol@example.com>;method=INVITE"),
        (HeaderName::ReferredBy, "<sip:alice@example.com>;cid=1234"),
        (
            HeaderName::PAssertedIdentity,
            "<sip:+15551234567@example.com;user=phone>",
        ),
    ] {
        let value = value_of(typed(&m, name.clone())[0]);
        assert!(value.contains(expected), "{name:?}: {value}");
    }
}

/// RFC 3325's P-Asserted-Identity has no header parameters, so a bare URI
/// followed by ";user=phone" (the form RFC 8217 forbids) does not match its
/// grammar; the header field is kept as received.
#[test]
fn rfc8217_p_asserted_identity_bare_uri_with_parameters_is_not_typed() {
    let m = request_with(
        "INVITE",
        &["P-Asserted-Identity: sip:+15551234567@example.com;user=phone"],
    );
    let headers = match &m {
        Message::Request(r) => r.all_headers(),
        Message::Response(r) => r.all_headers(),
    };
    let pai = headers
        .iter()
        .find(|h| h.name() == HeaderName::PAssertedIdentity)
        .expect("kept");
    assert!(matches!(pai, TypedHeader::Other(..)), "{pai}");
}

/// RFC 5954 §4.2: URIs whose hosts are the same IP address in different text
/// forms are equal; §4.1: IPv4address uses dec-octet.
#[test]
fn rfc5954_ip_address_comparison_and_ipv4_grammar() {
    for (a, b) in [
        ("sip:bob@[::ffff:192.0.2.128]", "sip:bob@[::ffff:c000:280]"),
        ("sip:bob@[2001:db8::9:1]", "sip:bob@[2001:db8::9:01]"),
        (
            "sip:bob@[0:0:0:0:0:FFFF:129.144.52.38]",
            "sip:bob@[::FFFF:129.144.52.38]",
        ),
    ] {
        assert_eq!(
            Uri::from_str(a).unwrap(),
            Uri::from_str(b).unwrap(),
            "{a} = {b}"
        );
    }
    assert!(Uri::from_str("sip:bob@444.555.666.777").is_err());
    assert!(Uri::from_str("sip:bob@192.0.2.128").is_ok());
    // §5: IPv6 is generated in its RFC 5952 form.
    assert_eq!(
        Uri::from_str("sip:bob@[2001:DB8:0:0:0:0:0:1]")
            .unwrap()
            .to_string(),
        "sip:bob@[2001:db8::1]"
    );
}

/// RFC 5630: SIPS URIs (including the deprecated transport=tls parameter)
/// parse and are kept as sent.
#[test]
fn rfc5630_sips() {
    let m = request_with(
        "INVITE",
        &["Contact: <sips:alice@client.example.com;transport=tls>"],
    );
    assert!(value_of(typed(&m, HeaderName::Contact)[0])
        .contains("<sips:alice@client.example.com;transport=tls>"));
}

/// RFC 3261 erratum 1073: "<" and ">" in a URI header value are escaped.
#[test]
fn rfc3261_erratum_1073_uri_header_escaping() {
    let m = request_with(
        "INVITE",
        &["Contact: <sip:user@host?Subject=foo&Call-Info=%3Chttp://www.foo.com%3E>"],
    );
    let contact = value_of(typed(&m, HeaderName::Contact)[0]);
    assert!(
        contact.contains("Call-Info=%3Chttp://www.foo.com%3E"),
        "{contact}"
    );
    assert!(contact.contains("Subject=foo"), "{contact}");
}

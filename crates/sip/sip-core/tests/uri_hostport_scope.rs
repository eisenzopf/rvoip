//! The port pre-validation must look only at the hostport component.
//!
//! `parse_sip_uri_fixed` screens for a malformed port before handing the input
//! to `hostport`. That screen used to scan the entire remainder of the URI for
//! the first `:`, past the `;` that opens the URI parameters and the `?` that
//! opens the URI headers. Both of those may legitimately contain a colon: RFC
//! 3261 §25.1 puts ":" in `hnv-unreserved`, so an `hvalue` may carry one
//! unescaped.
//!
//! The visible symptom was an attended transfer's `Refer-To`. A `Replaces`
//! naming a Call-ID with a port, `<sip:target?Replaces=cid@host:5060;...>`,
//! was read as though `5060;...` were the URI's own port and rejected.

use rvoip_sip_core::types::uri::Uri;
use std::str::FromStr;

/// A colon inside a URI header value is not a port.
#[test]
fn a_colon_in_a_uri_header_value_is_not_read_as_a_port() {
    let uri = Uri::from_str("sip:charlie@example.test?Replaces=consult@192.168.0.1:5060")
        .expect("a colon inside a URI header value must not be screened as a port");

    assert_eq!(uri.host.to_string(), "example.test");
    assert_eq!(uri.port, None);
    assert_eq!(
        uri.headers.get("Replaces").map(String::as_str),
        Some("consult@192.168.0.1:5060")
    );
}

/// The same, with a real port on the URI as well, so the two cannot be
/// confused for each other.
#[test]
fn a_real_port_and_a_colon_in_a_header_value_coexist() {
    let uri = Uri::from_str("sip:charlie@example.test:5070?Replaces=consult@host:5060")
        .expect("parse URI with both a port and a colon in a header value");

    assert_eq!(uri.port, Some(5070));
    assert_eq!(
        uri.headers.get("Replaces").map(String::as_str),
        Some("consult@host:5060")
    );
}

/// A colon inside a URI parameter value is not a port either.
#[test]
fn a_colon_in_a_uri_parameter_value_is_not_read_as_a_port() {
    let uri = Uri::from_str("sip:charlie@example.test;custom=host:5060")
        .expect("parse URI with a colon in a parameter value");
    assert_eq!(uri.port, None);
}

/// The screen still has to do its job. These are the cases it exists for, and
/// narrowing its scope must not have widened what it accepts.
#[test]
fn a_malformed_port_is_still_rejected() {
    for malformed in [
        "sip:charlie@example.test:notaport",
        "sip:charlie@example.test:99999",
        "sip:charlie@example.test:5060x",
    ] {
        assert!(
            Uri::from_str(malformed).is_err(),
            "{:?} carries a malformed port and must not parse",
            malformed
        );
    }
}

/// An IPv6 reference is full of colons and must keep working, with and without
/// a port, since the scoping change rewrote the bracket handling too.
#[test]
fn ipv6_references_still_parse() {
    let with_port = Uri::from_str("sip:charlie@[2001:db8::1]:5060").expect("IPv6 with port");
    assert_eq!(with_port.port, Some(5060));

    let without_port = Uri::from_str("sip:charlie@[2001:db8::1]").expect("IPv6 without port");
    assert_eq!(without_port.port, None);

    let with_header = Uri::from_str("sip:charlie@[2001:db8::1]:5060?Replaces=cid@host:5060")
        .expect("IPv6 with port and a colon in a header value");
    assert_eq!(with_header.port, Some(5060));
    assert_eq!(
        with_header.headers.get("Replaces").map(String::as_str),
        Some("cid@host:5060")
    );
}

/// sips shares the same screen, and shared the same bug.
#[test]
fn the_sips_parser_is_fixed_too() {
    let uri = Uri::from_str("sips:charlie@example.test?Replaces=consult@host:5060")
        .expect("sips URI with a colon in a header value");
    assert_eq!(
        uri.headers.get("Replaces").map(String::as_str),
        Some("consult@host:5060")
    );
    assert!(Uri::from_str("sips:charlie@example.test:notaport").is_err());
}

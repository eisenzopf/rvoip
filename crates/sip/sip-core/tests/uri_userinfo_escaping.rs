//! RFC 3261 §25.1 / §19.1.4 — escaping of the `user` and `password` parts
//!
//! The parser and the serialiser used to disagree inside this crate: the
//! parser accepted the full `user-unreserved` set unescaped, while the
//! serialiser preserved only `unreserved` and percent-encoded the rest. The
//! visible symptom was an E.164 target coming back as `%2B1555...`.
//!
//! §19.1.4 is what makes that a correctness bug rather than a cosmetic one:
//!
//!     Characters other than those in the "reserved" set (see RFC 2396)
//!     are equivalent to their ""%" HEX HEX" encoding.
//!
//! RFC 2396's `reserved` set is `; / ? : @ & = + $ ,`, which contains every
//! one of the eight `user-unreserved` characters. So escaping any of them
//! yields a URI a compliant peer compares as *different* — a gateway matching
//! an E.164 number literally will not match the escaped form.

use rvoip_sip_core::types::uri::Uri;
use std::str::FromStr;

/// The case that motivated this: an E.164 number must survive serialisation
/// with its leading `+` intact.
#[test]
fn e164_plus_survives_serialisation() {
    let uri = Uri::from_str("sip:+15551234567@gw.example.test").expect("parse E.164 URI");

    assert_eq!(uri.user.as_deref(), Some("+15551234567"));
    assert_eq!(uri.to_string(), "sip:+15551234567@gw.example.test");
    assert!(
        !uri.to_string().contains("%2B"),
        "leading + must not be escaped: RFC 2396 lists + as reserved, so \
         %2B is a different URI under RFC 3261 §19.1.4"
    );
}

/// `user-unreserved = "&" / "=" / "+" / "$" / "," / ";" / "?" / "/"`
///
/// All eight are permitted literally by the `user` production, and all eight
/// are reserved under RFC 2396 — escaping any of them changes URI identity.
#[test]
fn all_user_unreserved_characters_are_emitted_literally() {
    for c in ['&', '=', '+', '$', ',', ';', '?', '/'] {
        let raw = format!("sip:a{}b@example.test", c);
        let uri = Uri::from_str(&raw).unwrap_or_else(|_| panic!("parse {}", raw));

        assert_eq!(uri.user.as_deref(), Some(format!("a{}b", c).as_str()));
        assert_eq!(
            uri.to_string(),
            raw,
            "user-unreserved character {:?} must round-trip unescaped",
            c
        );
    }
}

/// `:` delimits the password and `@` delimits the host, so neither may appear
/// literally in the user part however permissive the rest of the rule is.
#[test]
fn user_part_delimiters_stay_escaped() {
    let uri = Uri {
        user: Some("a:b@c".to_string()),
        ..Uri::from_str("sip:placeholder@example.test").expect("parse base URI")
    };

    let serialised = uri.to_string();
    assert_eq!(serialised, "sip:a%3Ab%40c@example.test");

    // And the result must parse back to the value we started from, which is
    // the property that would break if either delimiter leaked through.
    let reparsed = Uri::from_str(&serialised).expect("reparse escaped delimiters");
    assert_eq!(reparsed.user.as_deref(), Some("a:b@c"));
}

/// A literal `%` has to become `%25`, otherwise it would be read back as the
/// start of an escape sequence.
#[test]
fn percent_is_escaped_in_user_part() {
    let uri = Uri {
        user: Some("100%pure".to_string()),
        ..Uri::from_str("sip:placeholder@example.test").expect("parse base URI")
    };

    assert_eq!(uri.to_string(), "sip:100%25pure@example.test");
    let reparsed = Uri::from_str(&uri.to_string()).expect("reparse escaped percent");
    assert_eq!(reparsed.user.as_deref(), Some("100%pure"));
}

/// `password = *( unreserved / escaped / "&" / "=" / "+" / "$" / "," )`
///
/// A strictly smaller set than `user`: `;`, `?` and `/` are absent. This is
/// the reason the two components do not share one escaping function.
#[test]
fn password_allows_only_its_own_smaller_sub_delimiter_set() {
    let base = Uri::from_str("sip:alice@example.test").expect("parse base URI");

    for c in ['&', '=', '+', '$', ','] {
        let uri = Uri {
            password: Some(format!("p{}q", c)),
            ..base.clone()
        };
        assert_eq!(
            uri.to_string(),
            format!("sip:alice:p{}q@example.test", c),
            "{:?} is permitted literally by the password production",
            c
        );
    }

    for c in [';', '?', '/'] {
        let uri = Uri {
            password: Some(format!("p{}q", c)),
            ..base.clone()
        };
        let serialised = uri.to_string();
        assert!(
            !serialised.contains(c),
            "{:?} is absent from the password production and must be escaped, got {}",
            c,
            serialised
        );
        assert_eq!(
            Uri::from_str(&serialised)
                .expect("reparse escaped password")
                .password
                .as_deref(),
            Some(format!("p{}q", c).as_str())
        );
    }
}

/// Known limitation, pinned deliberately so a future change to `Uri` has to
/// confront it rather than discover it.
///
/// The parser unescapes into `Uri::user`, so a URI that arrived with `%2B` and
/// one that arrived with a literal `+` become the same in-memory value. Under
/// §19.1.4 those are *different* URIs, so no serialisation choice can preserve
/// both — the distinction is lost at parse time, not at write time.
///
/// Emitting the literal form is the right side to land on: it is what the
/// `user` production prefers and what deployed gateways send. Preserving both
/// would require `Uri` to retain the original wire bytes of the user part.
#[test]
fn escaped_input_normalises_to_the_literal_form() {
    let from_escaped = Uri::from_str("sip:%2B15551234567@gw.example.test").expect("parse escaped");
    let from_literal = Uri::from_str("sip:+15551234567@gw.example.test").expect("parse literal");

    assert_eq!(from_escaped.user, from_literal.user);
    assert_eq!(from_escaped.to_string(), from_literal.to_string());
    assert_eq!(from_escaped.to_string(), "sip:+15551234567@gw.example.test");
}

//! RFC 4475 element behaviour: for every request in the torture archive, the
//! response an endpoint (UAS) must send instead of processing it, decided by
//! `validate_request` and `validate_request_for_uas` with the default UAS
//! capabilities. `None` means the request is processed. A request that is not
//! `is_answerable` is discarded instead.

use super::torture_test::{torture_fixture_bytes, torture_fixture_text};
use rvoip_sip_core::parse_message_with_mode;
use rvoip_sip_core::parser::message::ParseMode;
use rvoip_sip_core::types::headers::TypedHeader;
use rvoip_sip_core::types::{Message, StatusCode};
use rvoip_sip_core::validation::{
    is_answerable, validate_request, validate_request_for_uas, UasCapabilities,
};

/// (fixture, RFC 4475 section, status an endpoint answers with)
const ENDPOINT_RESPONSES: &[(&str, &str, Option<u16>)] = &[
    ("wsinv", "§3.1.1.1", None),
    // "Endpoints will generate a 501 listing the methods they understand in an
    // Allow header field."
    ("intmeth", "§3.1.1.2", Some(501)),
    ("esc01", "§3.1.1.3", None),
    ("escnull", "§3.1.1.4", None),
    // "An endpoint would reject this message with a 501."
    ("esc02", "§3.1.1.5", Some(501)),
    ("lwsdisp", "§3.1.1.6", None),
    ("longreq", "§3.1.1.7", None),
    ("dblreq", "§3.1.1.8", None),
    ("semiuri", "§3.1.1.9", None),
    ("transports", "§3.1.1.10", None),
    ("mpart01", "§3.1.1.11", None),
    // Accepted liberally by the Lenient parser, then processed normally.
    ("ltgtruri", "§3.1.2.7", None),
    ("lwsstart", "§3.1.2.9", None),
    ("trws", "§3.1.2.10", None),
    ("escruri", "§3.1.2.11", None),
    ("baddate", "§3.1.2.12", None),
    ("regbadct", "§3.1.2.13", None),
    ("badaspec", "§3.1.2.14", None),
    ("baddn", "§3.1.2.15", None),
    ("badvers", "§3.1.2.16", Some(505)),
    ("mismatch01", "§3.1.2.17", Some(400)),
    // 501 preferred over 400.
    ("mismatch02", "§3.1.2.18", Some(501)),
    ("badbranch", "§3.2.1", Some(400)),
    ("unkscm", "§3.3.2", Some(416)),
    ("novelsc", "§3.3.3", Some(416)),
    // The registrar, not the request validator, rejects a non-SIP AOR.
    ("unksm2", "§3.3.4", None),
    ("bext01", "§3.3.5", Some(420)),
    ("invut", "§3.3.6", Some(415)),
    ("regaut01", "§3.3.7", None),
    ("multi01", "§3.3.8", Some(400)),
    // "An endpoint should process the request as if the Max-Forwards field
    // value were still positive."
    ("zeromf", "§3.3.11", None),
    ("cparam01", "§3.3.12", None),
    ("cparam02", "§3.3.13", None),
    ("regescrt", "§3.3.14", None),
    ("sdp01", "§3.3.15", Some(406)),
    ("inv2543", "§3.4.1", None),
];

#[test]
fn rfc4475_endpoint_responses() {
    let capabilities = UasCapabilities::default();
    let mut failures = Vec::new();
    for (id, section, expected) in ENDPOINT_RESPONSES {
        let text = torture_fixture_bytes("rfc4475", id);
        let Message::Request(request) = parse_message_with_mode(&text, ParseMode::Lenient)
            .unwrap_or_else(|e| panic!("{id}: {e}"))
        else {
            panic!("{id}: not a request");
        };
        if !is_answerable(&request) {
            failures.push(format!(
                "{id} (RFC 4475 {section}): discarded as unanswerable"
            ));
            continue;
        }
        let rejection = validate_request(&request)
            .and_then(|()| validate_request_for_uas(&request, &capabilities))
            .err();
        let actual = rejection.as_ref().map(|r| r.status.as_u16());
        if actual != *expected {
            failures.push(format!(
                "{id} (RFC 4475 {section}): expected {expected:?}, got {actual:?} ({})",
                rejection.map_or("processed", |r| r.reason)
            ));
            continue;
        }
        if let Some(rejection) = rejection {
            let has = |f: fn(&TypedHeader) -> bool| rejection.headers.iter().any(f);
            match rejection.status {
                StatusCode::NotImplemented if *id != "mismatch02" => assert!(
                    has(|h| matches!(h, TypedHeader::Allow(_))),
                    "{id}: 501 lists the methods in Allow"
                ),
                StatusCode::BadExtension => {
                    let unsupported = rejection
                        .headers
                        .iter()
                        .find(|h| matches!(h, TypedHeader::Unsupported(_)))
                        .map(|h| h.to_string())
                        .unwrap_or_default();
                    assert!(
                        unsupported.contains("nothingSupportsThis")
                            && unsupported.contains("nothingSupportsThisEither")
                            && !unsupported.contains("noProxiesSupportThis"),
                        "{id}: Unsupported lists the Require tags only: {unsupported}"
                    );
                }
                StatusCode::UnsupportedMediaType => assert!(
                    has(|h| matches!(h, TypedHeader::Accept(_))),
                    "{id}: 415 lists acceptable types in Accept"
                ),
                _ => {}
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A request without Call-ID, From and To parses, but no valid response to it
/// exists (every response copies them, RFC 3261 §8.2.6.2), so it is discarded
/// rather than answered with the 400 RFC 4475 §3.3.1 would ideally send.
#[test]
fn rfc4475_request_without_call_id_from_to_is_discarded() {
    let text = torture_fixture_bytes("rfc4475", "insuf");
    let Message::Request(request) =
        parse_message_with_mode(&text, ParseMode::Lenient).unwrap_or_else(|e| panic!("insuf: {e}"))
    else {
        panic!("insuf: not a request");
    };
    assert!(!is_answerable(&request));
}

/// Messages the parser rejects: requests whose Via, From, To, Call-ID and
/// CSeq still parse are answered with 400 (RFC 4475 §3.1.2); when one of
/// those is itself malformed or missing, no valid response can be formed and
/// the message is discarded (RFC 3261 §18.3). Responses are never answered.
const UNPARSEABLE: &[(&str, &str, bool)] = &[
    // Via is malformed (";;,;,,"): a response must copy it.
    ("badinv01", "§3.1.2.1", false),
    ("clerr", "§3.1.2.2", true),
    ("ncl", "§3.1.2.3", true),
    // CSeq is malformed (overlarge): a response must copy it.
    ("scalar02", "§3.1.2.4", false),
    ("scalarlg", "§3.1.2.5 (response)", false),
    // To is malformed (unterminated quote): a response must copy it.
    ("quotbal", "§3.1.2.6", false),
    ("lwsruri", "§3.1.2.8", true),
    ("bigcode", "§3.1.2.19 (response)", false),
    ("mcl01", "§3.3.9", true),
    // No Via and no CSeq.
    ("test", "Appendix A (archive-only)", false),
];

#[test]
fn rfc4475_unparseable_requests_answered_with_400_when_possible() {
    let mut failures = Vec::new();
    for (id, section, answerable) in UNPARSEABLE {
        let text = torture_fixture_text("rfc4475", id);
        assert!(
            parse_message_with_mode(text.as_bytes(), ParseMode::Lenient).is_err(),
            "{id} must not parse"
        );
        let recovered = rvoip_sip_core::validation::recover_request_for_rejection(text.as_bytes());
        if recovered.is_some() != *answerable {
            failures.push(format!(
                "{id} (RFC 4475 {section}): answerable={answerable}, recovered={}",
                recovered.is_some()
            ));
        }
        if let Some(request) = recovered {
            let call_id = request.call_id().map(|c| c.value()).unwrap_or_default();
            assert!(
                text.contains(&format!("Call-ID: {call_id}"))
                    || text.contains(&format!("i: {call_id}")),
                "{id}: the 400 echoes the request's Call-ID"
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

//! RFC 4475 (SIP Torture Test Messages) and RFC 5118 (SIP Torture Test
//! Messages for IPv6) parser conformance.
//!
//! The fixtures under `torture/` are the bit-exact messages from each RFC's
//! Appendix A archive (see `torture/README.md`; `SHA256SUMS` pins them).
//! Every message is parsed in both parse modes:
//!
//! - `ParseMode::Strict` accepts only messages that conform to the RFC 3261
//!   grammar (with the RFC 5118 corrections) and never infers anything.
//! - `ParseMode::Lenient` is the datagram/stream path. It applies only the
//!   liberal handling each RFC explicitly allows for a message, and the
//!   result must not carry the error forward ("If the element forwards the
//!   request, it must not include ...").
//!
//! Messages that are syntactically valid but semantically wrong (for example
//! `badvers`, `mismatch01`, `insuf`) are accepted by the parser; the status an
//! element must answer with is checked by the request validator tests.
//!
//! Every accepted message must also re-serialize to bytes that parse in
//! Strict mode, and serializing that result again must be a fixpoint, so
//! nothing rvoip sends after accepting a torture message is malformed.

use rvoip_sip_core::parse_message_with_mode;
use rvoip_sip_core::parser::message::ParseMode;
use rvoip_sip_core::types::address::Address;
use rvoip_sip_core::types::contact::ContactValue;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::TypedHeader;
use rvoip_sip_core::types::via::ViaHeader;
use rvoip_sip_core::types::{Host, Message, Method, Request, Response, Scheme, Uri};
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Accept,
    Reject,
}
use Outcome::{Accept, Reject};

type Check = fn(&Message);

struct Case {
    /// Fixture file stem.
    id: &'static str,
    /// Where the expectation comes from.
    source: &'static str,
    strict: Outcome,
    lenient: Outcome,
    /// Assertions on every accepted parse.
    check: Option<Check>,
    /// Assertions for Lenient mode, when they differ from `check`.
    lenient_check: Option<Check>,
}

const fn case(
    id: &'static str,
    source: &'static str,
    strict: Outcome,
    lenient: Outcome,
    check: Option<Check>,
) -> Case {
    Case {
        id,
        source,
        strict,
        lenient,
        check,
        lenient_check: None,
    }
}

/// A case whose accepted parse differs between the modes.
const fn case_by_mode(
    id: &'static str,
    source: &'static str,
    strict: (Outcome, Option<Check>),
    lenient: (Outcome, Option<Check>),
) -> Case {
    Case {
        id,
        source,
        strict: strict.0,
        lenient: lenient.0,
        check: strict.1,
        lenient_check: lenient.1,
    }
}

// ---------------------------------------------------------------------------
// RFC 4475 expectations
// ---------------------------------------------------------------------------

const RFC4475: &[Case] = &[
    // 3.1.1 Valid Messages
    case(
        "wsinv",
        "RFC 4475 §3.1.1.1",
        Accept,
        Accept,
        Some(check_wsinv),
    ),
    case(
        "intmeth",
        "RFC 4475 §3.1.1.2",
        Accept,
        Accept,
        Some(check_intmeth),
    ),
    case(
        "esc01",
        "RFC 4475 §3.1.1.3",
        Accept,
        Accept,
        Some(check_esc01),
    ),
    case(
        "escnull",
        "RFC 4475 §3.1.1.4",
        Accept,
        Accept,
        Some(check_escnull),
    ),
    case(
        "esc02",
        "RFC 4475 §3.1.1.5",
        Accept,
        Accept,
        Some(check_esc02),
    ),
    case(
        "lwsdisp",
        "RFC 4475 §3.1.1.6",
        Accept,
        Accept,
        Some(check_lwsdisp),
    ),
    case(
        "longreq",
        "RFC 4475 §3.1.1.7",
        Accept,
        Accept,
        Some(check_longreq),
    ),
    // Lenient (datagrams): octets after the body are discarded (RFC 3261
    // §18.3). Strict (WebSocket): one SIP message per WebSocket message
    // (RFC 7118 §5), so the trailing octets make it invalid.
    case(
        "dblreq",
        "RFC 4475 §3.1.1.8, RFC 7118 §5",
        Reject,
        Accept,
        Some(check_dblreq),
    ),
    case(
        "semiuri",
        "RFC 4475 §3.1.1.9",
        Accept,
        Accept,
        Some(check_semiuri),
    ),
    case(
        "transports",
        "RFC 4475 §3.1.1.10",
        Accept,
        Accept,
        Some(check_transports),
    ),
    case(
        "mpart01",
        "RFC 4475 §3.1.1.11",
        Accept,
        Accept,
        Some(check_mpart01),
    ),
    case(
        "unreason",
        "RFC 4475 §3.1.1.12",
        Accept,
        Accept,
        Some(check_unreason),
    ),
    case(
        "noreason",
        "RFC 4475 §3.1.1.13",
        Accept,
        Accept,
        Some(check_noreason),
    ),
    // 3.1.2 Invalid Messages
    case("badinv01", "RFC 4475 §3.1.2.1", Reject, Reject, None),
    case("clerr", "RFC 4475 §3.1.2.2", Reject, Reject, None),
    case("ncl", "RFC 4475 §3.1.2.3", Reject, Reject, None),
    case("scalar02", "RFC 4475 §3.1.2.4", Reject, Reject, None),
    case("scalarlg", "RFC 4475 §3.1.2.5", Reject, Reject, None),
    case("quotbal", "RFC 4475 §3.1.2.6", Reject, Reject, None),
    // Liberal: ignore the brackets; never forward them.
    case(
        "ltgtruri",
        "RFC 4475 §3.1.2.7",
        Reject,
        Accept,
        Some(check_ltgtruri),
    ),
    case("lwsruri", "RFC 4475 §3.1.2.8", Reject, Reject, None),
    // Liberal: ignore the extra SP; never forward it.
    case(
        "lwsstart",
        "RFC 4475 §3.1.2.9",
        Reject,
        Accept,
        Some(check_lwsstart),
    ),
    case(
        "trws",
        "RFC 4475 §3.1.2.10",
        Reject,
        Accept,
        Some(check_trws),
    ),
    // Liberal: ignore the escaped headers; never forward them.
    case(
        "escruri",
        "RFC 4475 §3.1.2.11",
        Reject,
        Accept,
        Some(check_escruri),
    ),
    // Date is not needed to process the request, so the message is accepted
    // ("shouldn't [reject] unless the contents of the Date header field were
    // actually important"). Strict keeps the value as received; Lenient
    // interprets it and adjusts it to GMT.
    case_by_mode(
        "baddate",
        "RFC 4475 §3.1.2.12",
        (Accept, Some(check_baddate_kept)),
        (Accept, Some(check_baddate_gmt)),
    ),
    // Liberal: infer the angle brackets (unambiguous here).
    case(
        "regbadct",
        "RFC 4475 §3.1.2.13",
        Reject,
        Accept,
        Some(check_regbadct),
    ),
    // Liberal: ignore the spaces.
    case(
        "badaspec",
        "RFC 4475 §3.1.2.14",
        Reject,
        Accept,
        Some(check_badaspec),
    ),
    // Liberal: infer the missing quotes; never forward the error.
    case(
        "baddn",
        "RFC 4475 §3.1.2.15",
        Reject,
        Accept,
        Some(check_baddn),
    ),
    // Grammatically valid (SIP-Version = "SIP" "/" 1*DIGIT "." 1*DIGIT): 505.
    case(
        "badvers",
        "RFC 4475 §3.1.2.16",
        Accept,
        Accept,
        Some(check_badvers),
    ),
    // Grammatically valid; the element answers 400 / 501.
    case(
        "mismatch01",
        "RFC 4475 §3.1.2.17",
        Accept,
        Accept,
        Some(check_mismatch01),
    ),
    case(
        "mismatch02",
        "RFC 4475 §3.1.2.18",
        Accept,
        Accept,
        Some(check_mismatch02),
    ),
    // Status-Code = 3DIGIT.
    case("bigcode", "RFC 4475 §3.1.2.19", Reject, Reject, None),
    // 3.2 Transaction Layer Semantics
    case(
        "badbranch",
        "RFC 4475 §3.2.1",
        Accept,
        Accept,
        Some(check_badbranch),
    ),
    // 3.3 Application-Layer Semantics
    case(
        "insuf",
        "RFC 4475 §3.3.1",
        Accept,
        Accept,
        Some(check_insuf),
    ),
    case(
        "unkscm",
        "RFC 4475 §3.3.2",
        Accept,
        Accept,
        Some(check_unkscm),
    ),
    case(
        "novelsc",
        "RFC 4475 §3.3.3",
        Accept,
        Accept,
        Some(check_novelsc),
    ),
    case(
        "unksm2",
        "RFC 4475 §3.3.4",
        Accept,
        Accept,
        Some(check_unksm2),
    ),
    case(
        "bext01",
        "RFC 4475 §3.3.5",
        Accept,
        Accept,
        Some(check_bext01),
    ),
    case(
        "invut",
        "RFC 4475 §3.3.6",
        Accept,
        Accept,
        Some(check_invut),
    ),
    case(
        "regaut01",
        "RFC 4475 §3.3.7",
        Accept,
        Accept,
        Some(check_regaut01),
    ),
    case(
        "multi01",
        "RFC 4475 §3.3.8",
        Accept,
        Accept,
        Some(check_multi01),
    ),
    // Conflicting Content-Length values are a framing error.
    case("mcl01", "RFC 4475 §3.3.9", Reject, Reject, None),
    case(
        "bcast",
        "RFC 4475 §3.3.10",
        Accept,
        Accept,
        Some(check_bcast),
    ),
    case(
        "zeromf",
        "RFC 4475 §3.3.11",
        Accept,
        Accept,
        Some(check_zeromf),
    ),
    case(
        "cparam01",
        "RFC 4475 §3.3.12",
        Accept,
        Accept,
        Some(check_cparam01),
    ),
    case(
        "cparam02",
        "RFC 4475 §3.3.13",
        Accept,
        Accept,
        Some(check_cparam02),
    ),
    case(
        "regescrt",
        "RFC 4475 §3.3.14",
        Accept,
        Accept,
        Some(check_regescrt),
    ),
    case(
        "sdp01",
        "RFC 4475 §3.3.15",
        Accept,
        Accept,
        Some(check_sdp01),
    ),
    // 3.4 Backward Compatibility
    case(
        "inv2543",
        "RFC 4475 §3.4.1",
        Accept,
        Accept,
        Some(check_inv2543),
    ),
    // In the Appendix A archive but not described in the RFC: a header line
    // without a colon violates `message-header = header-name HCOLON ...`.
    case(
        "test",
        "RFC 4475 Appendix A (archive-only)",
        Reject,
        Reject,
        None,
    ),
];

// ---------------------------------------------------------------------------
// RFC 5118 expectations
// ---------------------------------------------------------------------------

/// The RFC 5118 archive uses LF line endings, two messages lack the empty
/// line that ends the header section, and two declare a Content-Length their
/// body does not have. Each message is therefore checked twice:
///
/// - as published (bit-exact), applying RFC 3261 to it: Strict rejects LF
///   line endings and a missing header terminator, Lenient tolerates both,
///   and a body shorter than Content-Length is an error in both (§18.3);
/// - normalized (CRLF, header terminator present, Content-Length matching
///   the body) to test what RFC 5118 describes, in both modes.
struct Rfc5118Case {
    id: &'static str,
    source: &'static str,
    /// As published, Lenient mode (Strict always rejects the LF framing).
    published_lenient: Outcome,
    /// Normalized, Strict mode.
    normalized_strict: Outcome,
    /// Normalized, Lenient mode.
    normalized_lenient: Outcome,
    check: Option<Check>,
}

const RFC5118: &[Rfc5118Case] = &[
    Rfc5118Case {
        id: "ipv6-good",
        source: "RFC 5118 §4.1",
        published_lenient: Accept,
        normalized_strict: Accept,
        normalized_lenient: Accept,
        check: Some(check_ipv6_good),
    },
    Rfc5118Case {
        id: "ipv6-bad",
        source: "RFC 5118 §4.2",
        published_lenient: Reject,
        normalized_strict: Reject,
        normalized_lenient: Reject,
        check: None,
    },
    Rfc5118Case {
        id: "port-ambiguous",
        source: "RFC 5118 §4.3 (erratum 1311)",
        published_lenient: Accept,
        normalized_strict: Accept,
        normalized_lenient: Accept,
        check: Some(check_port_ambiguous),
    },
    Rfc5118Case {
        id: "port-unambiguous",
        source: "RFC 5118 §4.4",
        published_lenient: Accept,
        normalized_strict: Accept,
        normalized_lenient: Accept,
        check: Some(check_port_unambiguous),
    },
    Rfc5118Case {
        id: "via-received-param-with-delim",
        source: "RFC 5118 §4.5",
        published_lenient: Accept,
        normalized_strict: Accept,
        normalized_lenient: Accept,
        check: Some(check_via_received),
    },
    Rfc5118Case {
        id: "via-received-param-no-delim",
        source: "RFC 5118 §4.5",
        published_lenient: Accept,
        normalized_strict: Accept,
        normalized_lenient: Accept,
        check: Some(check_via_received),
    },
    Rfc5118Case {
        // Content-Length: 268 with a 242-octet body as published.
        id: "ipv6-in-sdp",
        source: "RFC 5118 §4.6",
        published_lenient: Reject,
        normalized_strict: Accept,
        normalized_lenient: Accept,
        check: Some(check_ipv6_in_sdp),
    },
    Rfc5118Case {
        id: "mult-ip-in-header",
        source: "RFC 5118 §4.7",
        published_lenient: Accept,
        normalized_strict: Accept,
        normalized_lenient: Accept,
        check: Some(check_mult_ip_in_header),
    },
    Rfc5118Case {
        // Content-Length: 181 with a 180-octet body as published.
        id: "mult-ip-in-sdp",
        source: "RFC 5118 §4.8",
        published_lenient: Reject,
        normalized_strict: Accept,
        normalized_lenient: Accept,
        check: Some(check_mult_ip_in_sdp),
    },
    Rfc5118Case {
        id: "ipv4-mapped-ipv6",
        source: "RFC 5118 §4.9",
        published_lenient: Accept,
        normalized_strict: Accept,
        normalized_lenient: Accept,
        check: Some(check_ipv4_mapped),
    },
    Rfc5118Case {
        // RFC 5954 §4.1 replaced the RFC 3261 IPv6 ABNF that allowed the
        // extra colon, so Strict rejects it; RFC 5118 §4.10 asks for
        // tolerance, which Lenient provides.
        id: "ipv6-bug-abnf-3-colons",
        source: "RFC 5118 §4.10, RFC 5954 §4.1",
        published_lenient: Accept,
        normalized_strict: Reject,
        normalized_lenient: Accept,
        check: Some(check_ipv6_abnf_colons),
    },
    Rfc5118Case {
        id: "ipv6-correct-abnf-2-colons",
        source: "RFC 5118 §4.10",
        published_lenient: Accept,
        normalized_strict: Accept,
        normalized_lenient: Accept,
        check: Some(check_ipv6_abnf_colons),
    },
];

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn torture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/rfc_compliance/torture")
}

fn fixture(dir: &str, id: &str) -> Vec<u8> {
    let path = torture_dir().join(dir).join(format!("{id}.dat"));
    fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// A torture fixture for tests that build on these messages. RFC 5118
/// messages are returned normalized (see [`normalize_rfc5118`]).
pub(super) fn torture_fixture_bytes(dir: &str, id: &str) -> Vec<u8> {
    let bytes = fixture(dir, id);
    if dir == "rfc5118" {
        normalize_rfc5118(&bytes)
    } else {
        bytes
    }
}

/// [`torture_fixture_bytes`] as text, for fixtures that are UTF-8.
pub(super) fn torture_fixture_text(dir: &str, id: &str) -> String {
    String::from_utf8(torture_fixture_bytes(dir, id)).expect("UTF-8 fixture")
}

fn mode_name(mode: &ParseMode) -> &'static str {
    match mode {
        ParseMode::Strict => "Strict",
        ParseMode::Lenient => "Lenient",
    }
}

/// Parse `bytes` and hold the result to `expected`, running `check` and the
/// re-serialization invariants on every accepted message. Returns a failure
/// description instead of panicking, so one run reports every case.
fn evaluate(
    label: &str,
    bytes: &[u8],
    mode: ParseMode,
    expected: Outcome,
    check: Option<Check>,
) -> Option<String> {
    let mode_label = mode_name(&mode);
    let parsed = match std::panic::catch_unwind(|| parse_message_with_mode(bytes, mode)) {
        Ok(parsed) => parsed,
        Err(_) => return Some(format!("{label} [{mode_label}]: parser panicked")),
    };
    match (expected, parsed) {
        (Reject, Err(_)) => None,
        (Reject, Ok(message)) => Some(format!(
            "{label} [{}]: expected rejection, parsed as:\n{}",
            mode_label,
            String::from_utf8_lossy(&message.to_bytes())
        )),
        (Accept, Err(error)) => Some(format!(
            "{label} [{}]: expected acceptance, got {error}",
            mode_label
        )),
        (Accept, Ok(message)) => {
            let outcome = std::panic::catch_unwind(|| {
                if let Some(check) = check {
                    check(&message);
                }
                assert_reserializes_strictly(&message);
            });
            outcome.err().map(|panic| {
                let detail = panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                format!("{label} [{mode_label}]: {detail}")
            })
        }
    }
}

/// Anything rvoip sends after accepting a message must be valid SIP: the
/// serialization parses in Strict mode, and serializing that again yields
/// the same bytes.
pub(super) fn assert_reserializes_strictly(message: &Message) {
    let first = message.to_bytes();
    let reparsed = parse_message_with_mode(&first, ParseMode::Strict).unwrap_or_else(|error| {
        panic!(
            "serialization does not parse in Strict mode ({error}):\n{}",
            String::from_utf8_lossy(&first)
        )
    });
    let second = reparsed.to_bytes();
    assert!(
        first == second,
        "serialization is not a fixpoint:\n--- first\n{}\n--- second\n{}",
        String::from_utf8_lossy(&first),
        String::from_utf8_lossy(&second)
    );
}

/// An RFC 5118 message with CRLF line endings, a header terminator, and a
/// Content-Length matching the body.
fn normalize_rfc5118(bytes: &[u8]) -> Vec<u8> {
    let (head, body): (&[u8], &[u8]) = match bytes.windows(2).position(|w| w == b"\n\n") {
        Some(at) => (&bytes[..at], &bytes[at + 2..]),
        None => (bytes.strip_suffix(b"\n").unwrap_or(bytes), &[]),
    };
    let mut crlf_body = Vec::with_capacity(body.len() + 32);
    for &b in body {
        if b == b'\n' {
            crlf_body.push(b'\r');
        }
        crlf_body.push(b);
    }
    let mut out = Vec::new();
    for line in head.split(|&b| b == b'\n') {
        let lower = String::from_utf8_lossy(line).to_ascii_lowercase();
        if lower.starts_with("content-length:") || lower.starts_with("l:") {
            out.extend_from_slice(format!("Content-Length: {}", crlf_body.len()).as_bytes());
        } else {
            out.extend_from_slice(line);
        }
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&crlf_body);
    out
}

#[test]
fn fixtures_are_the_bit_exact_rfc_archives() {
    let sums = fs::read_to_string(torture_dir().join("SHA256SUMS")).expect("SHA256SUMS");
    let mut listed = 0;
    for line in sums.lines().filter(|l| !l.trim().is_empty()) {
        let (sum, file) = line.split_once("  ").expect("sha256sum line");
        let bytes = fs::read(torture_dir().join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(
            sha256_hex(&bytes),
            sum,
            "{file} differs from the RFC archive"
        );
        listed += 1;
    }
    assert_eq!(
        listed,
        RFC4475.len() + RFC5118.len(),
        "every fixture is pinned"
    );
}

#[test]
fn rfc4475_torture_messages() {
    let failures: Vec<String> = RFC4475
        .iter()
        .flat_map(|case| {
            let bytes = fixture("rfc4475", case.id);
            let label = format!("{} ({})", case.id, case.source);
            [
                evaluate(&label, &bytes, ParseMode::Strict, case.strict, case.check),
                evaluate(
                    &label,
                    &bytes,
                    ParseMode::Lenient,
                    case.lenient,
                    case.lenient_check.or(case.check),
                ),
            ]
        })
        .flatten()
        .collect();
    assert!(
        failures.is_empty(),
        "{} RFC 4475 expectation(s) failed:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn rfc5118_ipv6_torture_messages() {
    let failures: Vec<String> = RFC5118
        .iter()
        .flat_map(|case| {
            let published = fixture("rfc5118", case.id);
            let normalized = normalize_rfc5118(&published);
            let label = format!("{} ({})", case.id, case.source);
            let published_label = format!("{label} as published");
            let normalized_label = format!("{label} normalized");
            [
                evaluate(
                    &published_label,
                    &published,
                    ParseMode::Strict,
                    Reject,
                    None,
                ),
                evaluate(
                    &published_label,
                    &published,
                    ParseMode::Lenient,
                    case.published_lenient,
                    case.check,
                ),
                evaluate(
                    &normalized_label,
                    &normalized,
                    ParseMode::Strict,
                    case.normalized_strict,
                    case.check,
                ),
                evaluate(
                    &normalized_label,
                    &normalized,
                    ParseMode::Lenient,
                    case.normalized_lenient,
                    case.check,
                ),
            ]
        })
        .flatten()
        .collect();
    assert!(
        failures.is_empty(),
        "{} RFC 5118 expectation(s) failed:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

// ---------------------------------------------------------------------------
// Accessors
// ---------------------------------------------------------------------------

fn request(message: &Message) -> &Request {
    match message {
        Message::Request(request) => request,
        Message::Response(_) => panic!("expected a request"),
    }
}

fn response(message: &Message) -> &Response {
    match message {
        Message::Response(response) => response,
        Message::Request(_) => panic!("expected a response"),
    }
}

fn headers(message: &Message) -> &[TypedHeader] {
    match message {
        Message::Request(request) => request.all_headers(),
        Message::Response(response) => response.all_headers(),
    }
}

fn named(message: &Message, name: HeaderName) -> Vec<&TypedHeader> {
    headers(message)
        .iter()
        .filter(|h| h.name() == name)
        .collect()
}

fn other_header<'a>(message: &'a Message, name: &str) -> Vec<&'a TypedHeader> {
    headers(message)
        .iter()
        .filter(|h| matches!(h.name(), HeaderName::Other(n) if n == name))
        .collect()
}

/// The value part of a rendered header line.
fn value_of(header: &TypedHeader) -> String {
    let line = header.to_string();
    line.split_once(':')
        .map(|(_, v)| v.trim().to_string())
        .unwrap_or(line)
}

/// The value of the first `name:` line of a fixture, exactly as received.
fn raw_value(dir: &str, id: &str, name: &str) -> String {
    let raw = fixture(dir, id);
    let text = String::from_utf8_lossy(&raw).to_string();
    text.split("\r\n")
        .find_map(|line| {
            let (n, v) = line.split_once(':')?;
            n.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_string())
        })
        .unwrap_or_else(|| panic!("{id} has no {name} line"))
}

fn from_addr(message: &Message) -> (&Address, Option<&str>) {
    let from = match message {
        Message::Request(r) => r.from(),
        Message::Response(r) => r.from(),
    }
    .expect("typed From");
    (from.address(), from.tag())
}

fn to_addr(message: &Message) -> (&Address, Option<&str>) {
    let to = match message {
        Message::Request(r) => r.to(),
        Message::Response(r) => r.to(),
    }
    .expect("typed To");
    (to.address(), to.tag())
}

fn vias(message: &Message) -> Vec<ViaHeader> {
    headers(message)
        .iter()
        .filter_map(|h| match h {
            TypedHeader::Via(via) => Some(via.0.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

fn contacts(message: &Message) -> Vec<Address> {
    headers(message)
        .iter()
        .filter_map(|h| match h {
            TypedHeader::Contact(contact) => Some(contact.0.clone()),
            _ => None,
        })
        .flatten()
        .flat_map(|value| match value {
            ContactValue::Params(params) => params.into_iter().map(|p| p.address).collect(),
            ContactValue::Star => Vec::new(),
        })
        .collect()
}

fn cseq(message: &Message) -> (u32, Method) {
    let cseq = headers(message)
        .iter()
        .find_map(|h| match h {
            TypedHeader::CSeq(cseq) => Some(cseq.clone()),
            _ => None,
        })
        .expect("typed CSeq");
    (cseq.sequence(), cseq.method().clone())
}

fn max_forwards(message: &Message) -> u8 {
    headers(message)
        .iter()
        .find_map(|h| match h {
            TypedHeader::MaxForwards(mf) => Some(mf.0),
            _ => None,
        })
        .expect("typed Max-Forwards")
}

fn uri_param(uri: &Uri, key: &str) -> Option<Option<String>> {
    uri.parameters
        .iter()
        .find(|p| p.key().eq_ignore_ascii_case(key))
        .map(|p| p.value())
}

fn addr_param(address: &Address, key: &str) -> Option<Option<String>> {
    address
        .get_param(key)
        .map(|value| value.map(str::to_string))
}

fn host_string(host: &Host) -> String {
    host.to_string()
}

/// The start line of the message as rvoip would send it.
fn sent_start_line(message: &Message) -> String {
    let bytes = message.to_bytes();
    let text = String::from_utf8_lossy(&bytes);
    text.split("\r\n").next().unwrap_or_default().to_string()
}

fn sent(message: &Message) -> String {
    String::from_utf8_lossy(&message.to_bytes()).to_string()
}

fn body(message: &Message) -> &[u8] {
    match message {
        Message::Request(r) => r.body(),
        Message::Response(r) => r.body(),
    }
}

fn raw_body(dir: &str, id: &str) -> Vec<u8> {
    let raw = fixture(dir, id);
    let at = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("header end")
        + 4;
    raw[at..].to_vec()
}

fn ipv6(host: &Host) -> String {
    match host {
        Host::Address(IpAddr::V6(addr)) => addr.to_string(),
        other => panic!("expected an IPv6 host, got {other}"),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    // FIPS 180-4 SHA-256, so pinning the fixtures needs no extra dependency.
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut data = bytes.to_vec();
    let bit_len = (bytes.len() as u64) * 8;
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in data.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }
    h.iter().map(|v| format!("{v:08x}")).collect()
}

// ---------------------------------------------------------------------------
// RFC 4475 §3.1.1 Valid Messages
// ---------------------------------------------------------------------------

fn check_wsinv(m: &Message) {
    let r = request(m);
    assert_eq!(r.method(), Method::Invite);
    assert_eq!(r.uri().user.as_deref(), Some("vivekg"));
    assert_eq!(host_string(&r.uri().host), "chair-dnrc.example.com");
    assert_eq!(
        uri_param(r.uri(), "unknownparam"),
        Some(None),
        "uri param without a value"
    );

    let (to, to_tag) = to_addr(m);
    assert_eq!(to.uri.user.as_deref(), Some("vivekg"));
    assert_eq!(to_tag, Some("1918181833n"));

    let (from, from_tag) = from_addr(m);
    // The quoted-string "J Rosenberg \\\"" holds: J Rosenberg \"
    assert_eq!(from.display_name(), Some("J Rosenberg \\\""));
    assert_eq!(from.uri.user.as_deref(), Some("jdrosen"));
    assert_eq!(from_tag, Some("98asjd8"));

    assert_eq!(max_forwards(m), 68, "leading zeros");
    assert_eq!(cseq(m), (9, Method::Invite), "leading zeros");

    let v = vias(m);
    assert_eq!(v.len(), 3, "one Via plus a compact v: with two values");
    assert_eq!(
        (v[0].transport(), host_string(v[0].host()), v[0].branch()),
        ("UDP", "192.0.2.2".to_string(), Some("390skdjuw"))
    );
    assert_eq!(
        (v[1].transport(), host_string(v[1].host()), v[1].branch()),
        (
            "TCP",
            "spindle.example.com".to_string(),
            Some("z9hG4bK9ikj8")
        )
    );
    assert_eq!(
        (v[2].transport(), host_string(v[2].host()), v[2].branch()),
        ("UDP", "192.168.255.111".to_string(), Some("z9hG4bK30239"))
    );

    let subjects = named(m, HeaderName::Subject);
    assert_eq!(subjects.len(), 1, "s: compact Subject");
    assert_eq!(value_of(subjects[0]), "", "empty subject");

    let newfangled = other_header(m, "NewFangledHeader");
    assert_eq!(newfangled.len(), 1);
    assert_eq!(
        value_of(newfangled[0]),
        "newfangled value continued newfangled value",
        "folded value"
    );
    let unusual = other_header(m, "UnknownHeaderWithUnusualValue");
    assert_eq!(unusual.len(), 1);
    assert_eq!(
        value_of(unusual[0]),
        ";;,,;;,;",
        "an unknown header value is kept as is"
    );

    let routes = named(m, HeaderName::Route);
    assert_eq!(routes.len(), 1);
    assert_eq!(
        value_of(routes[0]),
        "<sip:services.example.com;lr;unknownwith=value;unknown-no-value>"
    );

    let c = contacts(m);
    assert_eq!(c.len(), 1, "m: compact Contact");
    assert_eq!(c[0].display_name(), Some("Quoted string \"\""));
    assert_eq!(c[0].uri.user.as_deref(), Some("jdrosen"));
    assert_eq!(addr_param(&c[0], "newparam"), Some(Some("newvalue".into())));
    assert_eq!(addr_param(&c[0], "secondparam"), Some(None));
    assert_eq!(addr_param(&c[0], "q"), Some(Some("0.33".into())));

    assert_eq!(body(m).len(), 150);
}

fn check_intmeth(m: &Message) {
    let method = "!interesting-Method0123456789_*+`.%indeed'~";
    let user = "1_unusual.URI~(to-be!sure)&isn't+it$/crazy?,/;;*";
    let r = request(m);
    assert_eq!(r.method(), Method::Extension(method.into()));
    assert_eq!(r.uri().user.as_deref(), Some(user));
    assert_eq!(
        r.uri().password.as_deref(),
        Some("&it+has=1,weird!*pas$wo~d_too.(doesn't-it)")
    );
    assert_eq!(host_string(&r.uri().host), "example.com");

    assert_eq!(vias(m)[0].branch(), Some("z9hG4bK-.!%66*_+`'~"));

    let (to, _) = to_addr(m);
    assert_eq!(
        to.display_name(),
        Some("BEL:\u{7} NUL:\u{0} DEL:\u{7f}"),
        "quoted-pair escapes control characters"
    );
    assert_eq!(to.uri.user.as_deref(), Some(user));

    let (from, from_tag) = from_addr(m);
    assert_eq!(from.display_name(), Some("token1~` token2'+_ token3*%!.-"));
    assert_eq!(from.uri.user.as_deref(), Some("mundane"));
    assert_eq!(
        addr_param(from, "fromParam''~+*_!.-%"),
        Some(Some("работающий".into())),
        "quoted-string generic-param value"
    );
    assert_eq!(from_tag, Some("_token~1'+`*%!-."));

    assert_eq!(
        value_of(named(m, HeaderName::CallId)[0]),
        "intmeth.word%ZK-!.*_+'@word`~)(><:\\/\"][?}{"
    );
    assert_eq!(cseq(m), (139122385, Method::Extension(method.into())));
    assert_eq!(max_forwards(m), 255);

    let ext = other_header(m, "extensionHeader-!.%*+_`'~");
    assert_eq!(ext.len(), 1);
    assert_eq!(value_of(ext[0]), "\u{feff}大停電");
}

fn check_esc01(m: &Message) {
    let r = request(m);
    assert_eq!(r.uri().user.as_deref(), Some("sips:user@example.com"));
    assert_eq!(host_string(&r.uri().host), "example.net");
    let (to, _) = to_addr(m);
    assert_eq!(to.uri.user.as_deref(), Some("user"), "%75se%72");
    let (from, from_tag) = from_addr(m);
    assert_eq!(from.uri.user.as_deref(), Some("I have spaces"));
    assert_eq!(from_tag, Some("938"));
    assert_eq!(named(m, HeaderName::CallId).len(), 1, "i: compact Call-ID");
    assert_eq!(
        named(m, HeaderName::ContentType).len(),
        1,
        "c: compact Content-Type"
    );
    let c = contacts(m);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].uri.user.as_deref(), Some("caller"), "cal%6Cer");
    assert_eq!(uri_param(&c[0].uri, "lr"), Some(None), "%6C%72");
    assert_eq!(
        uri_param(&c[0].uri, "name"),
        Some(Some("value%41".into())),
        "n%61me=v%61lue%25%34%31"
    );
    assert_eq!(body(m).len(), 150);
}

fn check_escnull(m: &Message) {
    let (to, _) = to_addr(m);
    assert_eq!(
        to.uri.user.as_deref(),
        Some("null-\u{0}-null"),
        "the AOR keeps its length"
    );
    let (from, from_tag) = from_addr(m);
    assert_eq!(from.uri.user.as_deref(), Some("null-\u{0}-null"));
    assert_eq!(
        from_tag,
        Some("839923423"),
        "addr-spec parameters are header parameters"
    );
    let c = contacts(m);
    assert_eq!(c.len(), 2, "two distinct contacts");
    assert_eq!(c[0].uri.user.as_deref(), Some("\u{0}"));
    assert_eq!(c[1].uri.user.as_deref(), Some("\u{0}\u{0}"));
}

fn check_esc02(m: &Message) {
    let r = request(m);
    assert_eq!(
        r.method(),
        Method::Extension("RE%47IST%45R".into()),
        "not REGISTER"
    );
    let (to, _) = to_addr(m);
    assert_eq!(to.display_name(), Some("%Z%45"));
    let (from, _) = from_addr(m);
    assert_eq!(from.display_name(), Some("%Z%45"));
    assert_eq!(vias(m)[0].branch(), Some("z9hG4bK209%fzsnel234"));
    assert_eq!(cseq(m).1, Method::Extension("RE%47IST%45R".into()));
    assert_eq!(contacts(m).len(), 2, "two Contact values, not three");
    assert_eq!(
        other_header(m, "C%6Fntact").len(),
        1,
        "C%6Fntact is an extension header"
    );
}

fn check_lwsdisp(m: &Message) {
    let (from, tag) = from_addr(m);
    assert_eq!(from.display_name(), Some("caller"));
    assert_eq!(from.uri.user.as_deref(), Some("caller"));
    assert_eq!(tag, Some("323"));
}

fn check_longreq(m: &Message) {
    let (to, _) = to_addr(m);
    let display = to.display_name().expect("long display name");
    assert!(display.starts_with("I have a user name of ") && display.ends_with(" proportion"));
    assert_eq!(to.uri.port, Some(6000));
    assert!(to.uri.parameters.len() >= 2, "long uri parameters");
    let (from, tag) = from_addr(m);
    assert!(from.uri.user.as_deref().unwrap_or_default().len() > 100);
    assert!(tag.unwrap_or_default().len() > 100, "long tag");
    assert!(value_of(named(m, HeaderName::CallId)[0]).len() > 100);
    // Every Via value of the fixture survives: each one starts with
    // "SIP/2.0/" (in Via, "Via :" and compact "v:" lines).
    let raw = fixture("rfc4475", "longreq");
    let raw_vias = String::from_utf8_lossy(&raw).matches("SIP/2.0/").count();
    assert_eq!(vias(m).len(), raw_vias);
    assert_eq!(body(m).len(), 150);
}

fn check_dblreq(m: &Message) {
    assert_eq!(request(m).method(), Method::Register);
    assert!(
        body(m).is_empty(),
        "the trailing INVITE octets are not part of the message"
    );
    assert!(!sent(m).contains("INVITE"));
}

fn check_semiuri(m: &Message) {
    let r = request(m);
    assert_eq!(r.uri().user.as_deref(), Some("user;par=u@example.net"));
    assert_eq!(host_string(&r.uri().host), "example.com");
    assert!(r.uri().parameters.is_empty());
    let (from, tag) = from_addr(m);
    assert!(from.uri.parameters.is_empty());
    assert_eq!(tag, Some("33242"));
}

fn check_transports(m: &Message) {
    let t: Vec<String> = vias(m).iter().map(|v| v.transport().to_string()).collect();
    assert_eq!(t, ["UDP", "SCTP", "TLS", "UNKNOWN", "TCP"]);
}

fn check_mpart01(m: &Message) {
    assert_eq!(request(m).method(), Method::Message);
    assert_eq!(
        body(m),
        raw_body("rfc4475", "mpart01"),
        "binary body framed by Content-Length"
    );
    assert!(body(m).contains(&0), "the binary part contains NUL octets");
    assert_eq!(m.content_length(), Some(body(m).len() as u64));
    assert_eq!(
        value_of(named(m, HeaderName::Date)[0]),
        raw_value("rfc4475", "mpart01", "Date"),
        "SIP-date is sent in its RFC 1123 form"
    );
}

fn check_unreason(m: &Message) {
    let r = response(m);
    assert_eq!(r.status_code(), 200);
    assert_eq!(
        r.reason_phrase(),
        "= 2**3 * 5**2 но сто девяносто девять - простое"
    );
    assert_eq!(body(m).len(), 154);
}

fn check_noreason(m: &Message) {
    let r = response(m);
    assert_eq!(r.status_code(), 100);
    assert_eq!(r.reason_phrase(), "");
    assert_eq!(
        sent_start_line(m),
        "SIP/2.0 100 ",
        "the SP after the code is required"
    );
}

// ---------------------------------------------------------------------------
// RFC 4475 §3.1.2 Invalid Messages accepted liberally
// ---------------------------------------------------------------------------

fn check_ltgtruri(m: &Message) {
    let r = request(m);
    assert_eq!(r.uri().scheme, Scheme::Sip);
    assert_eq!(r.uri().user.as_deref(), Some("user"));
    assert_eq!(host_string(&r.uri().host), "example.com");
    assert_eq!(sent_start_line(m), "INVITE sip:user@example.com SIP/2.0");
}

fn check_lwsstart(m: &Message) {
    assert_eq!(sent_start_line(m), "INVITE sip:user@example.com SIP/2.0");
}

fn check_trws(m: &Message) {
    assert_eq!(
        sent_start_line(m),
        "OPTIONS sip:remote-target@example.com SIP/2.0"
    );
}

fn check_escruri(m: &Message) {
    assert!(
        request(m).uri().headers.is_empty(),
        "escaped headers are ignored"
    );
    assert_eq!(sent_start_line(m), "INVITE sip:user@example.com SIP/2.0");
    assert!(
        named(m, HeaderName::Route).is_empty(),
        "never translated into a header"
    );
}

fn check_baddate_kept(m: &Message) {
    let dates = named(m, HeaderName::Date);
    assert_eq!(dates.len(), 1);
    assert_eq!(
        value_of(dates[0]),
        "Fri, 01 Jan 2010 16:00:00 EST",
        "kept as received"
    );
}

fn check_baddate_gmt(m: &Message) {
    let dates = named(m, HeaderName::Date);
    assert_eq!(dates.len(), 1);
    // Fri, 01 Jan 2010 16:00:00 EST is 21:00:00 GMT.
    assert_eq!(value_of(dates[0]), "Fri, 01 Jan 2010 21:00:00 GMT");
    assert!(matches!(dates[0], TypedHeader::Date(_)));
}

fn check_regbadct(m: &Message) {
    let c = contacts(m);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].uri.user.as_deref(), Some("user"));
    assert_eq!(
        c[0].uri.headers.get("Route").map(String::as_str),
        Some("<sip:sip.example.com>")
    );
    let sent = sent(m);
    assert!(
        sent.contains("Contact: <sip:user@example.com?Route=%3Csip:sip.example.com%3E>"),
        "name-addr form when the URI has headers:\n{sent}"
    );
}

fn check_badaspec(m: &Message) {
    let (to, _) = to_addr(m);
    assert_eq!(to.display_name(), Some("Watson, Thomas"));
    assert_eq!(to.uri.user.as_deref(), Some("t.watson"));
    assert_eq!(host_string(&to.uri.host), "example.org");
}

fn check_baddn(m: &Message) {
    let (from, tag) = from_addr(m);
    assert_eq!(from.display_name(), Some("Bell, Alexander"));
    assert_eq!(tag, Some("43"));
    let (to, _) = to_addr(m);
    assert_eq!(to.display_name(), Some("Watson, Thomas"));
    let sent = sent(m);
    assert!(
        sent.contains("\"Bell, Alexander\""),
        "quotes are added when sent:\n{sent}"
    );
    assert!(sent.contains("\"Watson, Thomas\""), "{sent}");
}

fn check_badvers(m: &Message) {
    let version = request(m).version();
    assert_eq!((version.major, version.minor), (7, 0));
}

fn check_mismatch01(m: &Message) {
    assert_eq!(request(m).method(), Method::Options);
    assert_eq!(cseq(m).1, Method::Invite);
}

fn check_mismatch02(m: &Message) {
    assert_eq!(request(m).method(), Method::Extension("NEWMETHOD".into()));
    assert_eq!(cseq(m).1, Method::Invite);
}

// ---------------------------------------------------------------------------
// RFC 4475 §3.2, §3.3, §3.4
// ---------------------------------------------------------------------------

fn check_badbranch(m: &Message) {
    assert_eq!(vias(m)[0].branch(), Some("z9hG4bK"));
}

fn check_insuf(m: &Message) {
    assert!(named(m, HeaderName::CallId).is_empty());
    assert!(named(m, HeaderName::From).is_empty());
    assert!(named(m, HeaderName::To).is_empty());
}

fn check_unkscm(m: &Message) {
    assert_eq!(
        request(m).uri().scheme,
        Scheme::Custom("nobodyKnowsThisScheme".into())
    );
    assert_eq!(
        sent_start_line(m),
        "OPTIONS nobodyKnowsThisScheme:totallyopaquecontent SIP/2.0"
    );
}

fn check_novelsc(m: &Message) {
    assert_eq!(
        sent_start_line(m),
        "OPTIONS soap.beep://192.0.2.103:3002 SIP/2.0"
    );
}

fn check_unksm2(m: &Message) {
    let (to, _) = to_addr(m);
    assert_eq!(to.uri.to_string(), "isbn:2983792873");
    let (from, tag) = from_addr(m);
    assert_eq!(from.uri.to_string(), "http://www.example.com");
    assert_eq!(tag, Some("3234233"));
    let c = contacts(m);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].uri.to_string(), "name:John_Smith");
}

fn check_bext01(m: &Message) {
    assert_eq!(
        value_of(named(m, HeaderName::Require)[0]),
        raw_value("rfc4475", "bext01", "Require")
    );
    assert_eq!(
        value_of(named(m, HeaderName::ProxyRequire)[0]),
        raw_value("rfc4475", "bext01", "Proxy-Require")
    );
}

fn check_invut(m: &Message) {
    assert_eq!(
        value_of(named(m, HeaderName::ContentType)[0]),
        "application/unknownformat"
    );
    assert_eq!(body(m).len(), 40);
}

fn check_regaut01(m: &Message) {
    // A proxy forwards this Authorization header field unmodified.
    let auth = named(m, HeaderName::Authorization);
    assert_eq!(auth.len(), 1);
    assert_eq!(
        value_of(auth[0]),
        raw_value("rfc4475", "regaut01", "Authorization")
    );
}

fn check_multi01(m: &Message) {
    assert_eq!(named(m, HeaderName::CSeq).len(), 2);
    assert_eq!(named(m, HeaderName::To).len(), 2);
    assert_eq!(named(m, HeaderName::From).len(), 2);
}

fn check_bcast(m: &Message) {
    let v = vias(m);
    assert_eq!(v.len(), 2);
    assert_eq!(host_string(v[1].host()), "255.255.255.255");
}

fn check_zeromf(m: &Message) {
    assert_eq!(max_forwards(m), 0);
}

fn check_cparam01(m: &Message) {
    let c = contacts(m);
    assert_eq!(c.len(), 1);
    assert!(
        c[0].uri.parameters.is_empty(),
        "unknownparam is a contact-param, not a url-parameter"
    );
    assert_eq!(addr_param(&c[0], "unknownparam"), Some(None));
}

fn check_cparam02(m: &Message) {
    let c = contacts(m);
    assert_eq!(c.len(), 1);
    assert_eq!(uri_param(&c[0].uri, "unknownparam"), Some(None));
    assert_eq!(addr_param(&c[0], "unknownparam"), None);
}

fn check_regescrt(m: &Message) {
    let c = contacts(m);
    assert_eq!(c.len(), 1);
    assert_eq!(
        c[0].uri.headers.get("Route").map(String::as_str),
        Some("<sip:sip.example.com>")
    );
}

fn check_sdp01(m: &Message) {
    // Media types are case-insensitive (RFC 2045 §5.1).
    assert!(value_of(named(m, HeaderName::Accept)[0]).eq_ignore_ascii_case("text/nobodyKnowsThis"));
}

fn check_inv2543(m: &Message) {
    assert_eq!(request(m).method(), Method::Invite);
    assert_eq!(vias(m)[0].branch(), None, "no branch");
    assert_eq!(from_addr(m).1, None, "no From tag");
    assert!(named(m, HeaderName::MaxForwards).is_empty());
    let (to, _) = to_addr(m);
    assert_eq!(
        uri_param(&to.uri, "user"),
        None,
        "addr-spec: ;user=phone is a header param"
    );
    assert_eq!(addr_param(to, "user"), Some(Some("phone".into())));
    assert_eq!(
        body(m),
        raw_body("rfc4475", "inv2543"),
        "no Content-Length: the rest of the datagram"
    );
}

// ---------------------------------------------------------------------------
// RFC 5118
// ---------------------------------------------------------------------------

fn check_ipv6_good(m: &Message) {
    assert_eq!(ipv6(&request(m).uri().host), "2001:db8::10");
    assert!(matches!(vias(m)[0].host(), Host::Address(IpAddr::V6(_))));
    assert_eq!(ipv6(&contacts(m)[0].uri.host), "2001:db8::1");
}

fn check_port_ambiguous(m: &Message) {
    // The intended port became the last octet pair of the reference.
    let r = request(m);
    assert_eq!(ipv6(&r.uri().host), "2001:db8::10:5070");
    assert_eq!(r.uri().port, None);
}

fn check_port_unambiguous(m: &Message) {
    let r = request(m);
    assert_eq!(ipv6(&r.uri().host), "2001:db8::10");
    assert_eq!(r.uri().port, Some(5070));
}

fn check_via_received(m: &Message) {
    let v = vias(m);
    assert_eq!(ipv6(v[0].host()), "2001:db8::9:1");
    assert_eq!(v[0].received().as_deref(), Some("2001:db8::9:255"));
    // When sending, the delimiters are not used.
    let sent = sent(m);
    assert!(sent.contains(";received=2001:db8::9:255"), "{sent}");
    assert!(!sent.contains("received=["), "{sent}");
}

fn check_ipv6_in_sdp(m: &Message) {
    assert_eq!(ipv6(&request(m).uri().host), "2001:db8::10");
    assert_eq!(ipv6(vias(m)[0].host()), "2001:db8::20");
    assert!(String::from_utf8_lossy(body(m)).contains("c=IN IP6 2001:db8::20"));
}

fn check_mult_ip_in_header(m: &Message) {
    let v = vias(m);
    assert_eq!(v.len(), 3);
    assert_eq!(
        (ipv6(v[0].host()), v[0].port()),
        ("2001:db8::9:1".to_string(), Some(6050))
    );
    assert_eq!(host_string(v[1].host()), "192.0.2.1");
    assert_eq!(ipv6(v[2].host()), "2001:db8::9:255");
    assert_eq!(v[2].received().as_deref(), Some("192.0.2.200"));
}

fn check_mult_ip_in_sdp(m: &Message) {
    let sdp = String::from_utf8_lossy(body(m));
    assert!(sdp.contains("c=IN IP4 192.0.2.1"));
    assert!(sdp.contains("c=IN IP6 2001:db8::1"));
}

fn check_ipv4_mapped(m: &Message) {
    let v = vias(m);
    assert_eq!(v.len(), 2);
    assert_eq!(ipv6(v[0].host()), "::ffff:192.0.2.10");
    assert_eq!(v[0].port(), Some(19823), "port outside the reference");
    assert_eq!(ipv6(v[1].host()), "::ffff:192.0.2.2");
    assert_eq!(ipv6(&contacts(m)[0].uri.host), "::ffff:192.0.2.2");
}

fn check_ipv6_abnf_colons(m: &Message) {
    // Both denote 2001:db8::c000:201, which is sent in its RFC 5952 form
    // (RFC 5954 §5).
    let r = request(m);
    assert_eq!(ipv6(&r.uri().host), "2001:db8::c000:201");
    assert_eq!(r.uri().user.as_deref(), Some("user"));
    assert_eq!(
        sent_start_line(m),
        "OPTIONS sip:user@[2001:db8::c000:201] SIP/2.0"
    );
}

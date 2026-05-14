//! SIP_API_DESIGN_2 §10 verification #31 — `TraceRedactor` strips
//! sensitive headers from the trace stream without affecting the wire
//! output.
//!
//! The redaction site lives in `SipTraceRuntime::publish` (dialog-core
//! `transaction/transport/trace.rs`) and is plumbed through
//! `UnifiedCoordinator::new` from `Config.trace_redaction`. This file
//! is the §10-named entry point; companion redactor unit-style tests
//! live in `sip_api_design_2_section_10_skeletons.rs`.

use rvoip_sip::api::trace_redactor::{
    apply_message_redactor, PassthroughRedactor, RedactionDecision, TraceRedactor,
};
use rvoip_sip_core::types::header::HeaderName;

#[derive(Debug)]
struct AuthRedactor;

impl TraceRedactor for AuthRedactor {
    fn redact(&self, header: &HeaderName, _value: &str) -> RedactionDecision {
        match header {
            HeaderName::Authorization | HeaderName::ProxyAuthorization => {
                RedactionDecision::Redact("<redacted>".to_string())
            }
            _ => RedactionDecision::Keep,
        }
    }
}

/// §10 #31 — Trace output has Authorization / Proxy-Authorization
/// rewritten to `<redacted>`; non-auth headers untouched.
#[test]
fn redactor_strips_authorization_from_trace_payload() {
    let raw = concat!(
        "REGISTER sip:registrar.example.com SIP/2.0\r\n",
        "Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-abc\r\n",
        "Authorization: Digest username=\"alice\", response=\"deadbeef\"\r\n",
        "Proxy-Authorization: Digest username=\"bob\", response=\"cafebabe\"\r\n",
        "Call-ID: trace-call@127.0.0.1\r\n",
        "Content-Length: 0\r\n",
        "\r\n",
    );

    let scrubbed = apply_message_redactor(&AuthRedactor, raw);

    assert!(
        scrubbed.contains("Authorization: <redacted>"),
        "Authorization must be rewritten; got:\n{scrubbed}"
    );
    assert!(
        scrubbed.contains("Proxy-Authorization: <redacted>"),
        "Proxy-Authorization must be rewritten; got:\n{scrubbed}"
    );
    assert!(
        !scrubbed.contains("response=\"deadbeef\""),
        "Authorization payload bytes must not survive redaction"
    );
    assert!(
        !scrubbed.contains("response=\"cafebabe\""),
        "Proxy-Authorization payload bytes must not survive redaction"
    );

    // Non-sensitive headers must be present verbatim.
    assert!(scrubbed.contains("Call-ID: trace-call@127.0.0.1"));
    assert!(scrubbed.contains("Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-abc"));
}

/// `PassthroughRedactor` is the configured default for the trace site;
/// it must be the identity on the message stream so trace output stays
/// unchanged when no redactor is configured.
#[test]
fn passthrough_redactor_is_identity_on_trace_payload() {
    let raw = concat!(
        "OPTIONS sip:bob@example.com SIP/2.0\r\n",
        "Via: SIP/2.0/UDP 127.0.0.1:5060\r\n",
        "Authorization: Digest secret\r\n",
        "\r\n",
    );
    let out = apply_message_redactor(&PassthroughRedactor, raw);
    assert_eq!(
        out, raw,
        "PassthroughRedactor must not alter the trace payload"
    );
}

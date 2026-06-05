//! SIP_API_DESIGN_2 §10 verification #31 — `TraceRedactor` strips
//! sensitive headers from the trace stream without affecting the wire
//! output.
//!
//! The redaction site lives in `SipTraceRuntime::publish` (dialog-core
//! `transaction/transport/trace.rs`) and is plumbed through
//! `UnifiedCoordinator::new` from `Config.trace_redaction`. This file
//! is the §10-named entry point; companion redactor unit-style tests
//! live in `sip_api_design_2_section_10_skeletons.rs`.

use proptest::prelude::*;
use rvoip_infra_common::events::cross_crate::{format_sip_trace_message, SipTraceConfig};
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

#[test]
fn default_trace_policy_redacts_identity_tokens_and_sdp_keying() {
    let raw = concat!(
        "INVITE sip:bob@example.com SIP/2.0\r\n",
        "Authorization: Digest response=\"deadbeef\"\r\n",
        "Identity: signed-passport;info=<https://cert.example>\r\n",
        "P-Asserted-Identity: <sip:+15551234567@example.com>\r\n",
        "X-Carrier-Token: carrier-secret-token\r\n",
        "Content-Type: application/sdp\r\n",
        "\r\n",
        "v=0\r\n",
        "a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:sdes-master-key\r\n",
        "a=ice-pwd:ice-secret\r\n",
        "a=rtpmap:0 PCMU/8000\r\n",
    );

    let config = SipTraceConfig {
        enabled: true,
        redact_sensitive_headers: true,
        include_body: true,
        ..SipTraceConfig::default()
    };
    let (scrubbed, truncated) = format_sip_trace_message(raw, &config);

    assert!(!truncated);
    assert!(scrubbed.contains("Authorization: <redacted>"));
    assert!(scrubbed.contains("Identity: <redacted>"));
    assert!(scrubbed.contains("P-Asserted-Identity: <redacted>"));
    assert!(scrubbed.contains("X-Carrier-Token: <redacted>"));
    assert!(scrubbed.contains("a=crypto:<redacted>"));
    assert!(scrubbed.contains("a=ice-pwd:<redacted>"));
    assert!(scrubbed.contains("a=rtpmap:0 PCMU/8000"));
    assert!(!scrubbed.contains("deadbeef"));
    assert!(!scrubbed.contains("signed-passport"));
    assert!(!scrubbed.contains("carrier-secret-token"));
    assert!(!scrubbed.contains("sdes-master-key"));
    assert!(!scrubbed.contains("ice-secret"));
}

fn secret_fragment_strategy() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[A-Za-z0-9._~+=/-]{1,48}").unwrap()
}

proptest! {
    #[test]
    fn auth_redactor_never_leaks_generated_authorization_values(
        auth_fragment in secret_fragment_strategy(),
        proxy_fragment in secret_fragment_strategy(),
        call_fragment in secret_fragment_strategy(),
    ) {
        let auth_secret = format!("auth-secret-{auth_fragment}");
        let proxy_secret = format!("proxy-secret-{proxy_fragment}");
        let call_id = format!("call-{call_fragment}@example.test");
        let raw = format!(
            "MESSAGE sip:bob@example.com SIP/2.0\r\n\
             Authorization: Digest username=\"alice\", response=\"{auth_secret}\"\r\n\
             Proxy-Authorization: Bearer {proxy_secret}\r\n\
             Call-ID: {call_id}\r\n\
             Content-Length: 0\r\n\
             \r\n"
        );

        let scrubbed = apply_message_redactor(&AuthRedactor, &raw);

        prop_assert!(scrubbed.contains("Authorization: <redacted>"));
        prop_assert!(scrubbed.contains("Proxy-Authorization: <redacted>"));
        prop_assert!(
            !scrubbed.contains(&auth_secret),
            "Authorization secret leaked: {}",
            scrubbed
        );
        prop_assert!(
            !scrubbed.contains(&proxy_secret),
            "Proxy-Authorization secret leaked: {}",
            scrubbed
        );
        let expected_call_id = format!("Call-ID: {}", call_id);
        prop_assert!(scrubbed.contains(&expected_call_id));
        prop_assert!(scrubbed.contains("Content-Length: 0"));
    }
}

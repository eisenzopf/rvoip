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
    apply_message_redactor, apply_redaction, BodyRedactionDecision, DefaultTraceRedactor,
    PassthroughRedactor, RedactionDecision, TraceRedactor, REDACTED_BODY_MARKER,
};
use rvoip_sip::Config;
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

/// `PassthroughRedactor` is an explicit operator/development escape hatch;
/// when selected it remains the identity on the message stream.
#[test]
fn passthrough_redactor_is_identity_on_trace_payload() {
    let raw = concat!(
        "MESSAGE sip:bob@example.com SIP/2.0\r\n",
        "Via: SIP/2.0/UDP 127.0.0.1:5060\r\n",
        "Authorization: Digest secret\r\n",
        "Content-Type: application/json\r\n",
        "\r\n",
        "{\"context\":\"visible-by-explicit-opt-in\"}",
    );
    let out = apply_message_redactor(&PassthroughRedactor, raw);
    assert_eq!(
        out, raw,
        "PassthroughRedactor must not alter the trace payload"
    );
}

#[test]
fn production_default_redacts_credentials_and_application_context() {
    let config = Config::local("trace-default", 0);
    let redactor = config
        .trace_redaction
        .as_ref()
        .expect("Config defaults to an explicit safe redactor");
    let raw = concat!(
        "MESSAGE sip:bob@example.com SIP/2.0\r\n",
        "Authorization: Bearer credential-secret\r\n",
        "X-Bridgefu-Context: application-secret\r\n",
        "Call-ID: operational-call-id\r\n",
        "\r\n",
    );

    let scrubbed = apply_message_redactor(redactor.as_ref(), raw);
    assert!(scrubbed.contains("Authorization: <redacted>"));
    assert!(scrubbed.contains("X-Bridgefu-Context: <redacted>"));
    assert!(scrubbed.contains("Call-ID: operational-call-id"));
    assert!(!scrubbed.contains("credential-secret"));
    assert!(!scrubbed.contains("application-secret"));

    let custom = HeaderName::Other("X-Context".into());
    assert_eq!(
        apply_redaction(None, &custom, "default-secret").as_deref(),
        Some("<redacted>")
    );
}

#[test]
fn passthrough_requires_explicit_development_configuration() {
    let config = Config::local("trace-development", 0).trace_passthrough_for_development();
    assert!(!config.sip_trace.redact_sensitive_headers);
    let redactor = config.trace_redaction.as_ref().unwrap();
    let raw = "OPTIONS sip:bob@example.com SIP/2.0\r\nX-Context: visible-by-opt-in\r\n\r\n";
    assert_eq!(apply_message_redactor(redactor.as_ref(), raw), raw);
}

#[test]
fn folded_sensitive_headers_inherit_redaction_for_crlf_and_lf() {
    for ending in ["\r\n", "\n"] {
        let raw = format!(
            "INVITE sip:bob@example.com SIP/2.0{ending}\
             Authorization: Digest first-secret{ending}\
             \tsecond-auth-secret{ending}\
             Proxy-Authorization: Bearer proxy-secret{ending}\
             \x20proxy-fold-secret{ending}\
             X-Bridgefu-Context: context-secret{ending}\
             \tcontext-fold-secret{ending}\
             Supported: timer,{ending}\
             \tpath{ending}\
             {ending}\
             Authorization: body-value-must-remain{ending}\
             \tbody-fold-must-remain"
        );

        let scrubbed = apply_message_redactor(&DefaultTraceRedactor, &raw);
        assert!(scrubbed.contains("Authorization: <redacted>"));
        assert!(scrubbed.contains("Proxy-Authorization: <redacted>"));
        assert!(scrubbed.contains("X-Bridgefu-Context: <redacted>"));
        assert!(scrubbed.contains(&format!("Supported: timer,{ending}\tpath")));
        for secret in [
            "first-secret",
            "second-auth-secret",
            "proxy-secret",
            "proxy-fold-secret",
            "context-secret",
            "context-fold-secret",
        ] {
            assert!(!scrubbed.contains(secret), "folded secret leaked: {secret}");
        }
        assert!(scrubbed.ends_with(REDACTED_BODY_MARKER));
        assert!(!scrubbed.contains("body-value-must-remain"));
        assert!(!scrubbed.contains("body-fold-must-remain"));
        assert!(
            !scrubbed.ends_with('\n'),
            "missing final newline was invented"
        );
    }
}

#[derive(Debug)]
struct FoldPolicy;

impl TraceRedactor for FoldPolicy {
    fn redact(&self, header: &HeaderName, _value: &str) -> RedactionDecision {
        match header {
            HeaderName::Other(name) if name.eq_ignore_ascii_case("X-Drop-Me") => {
                RedactionDecision::Drop
            }
            HeaderName::Other(name) if name.eq_ignore_ascii_case("X-Redact-Me") => {
                RedactionDecision::Redact("<hidden>".into())
            }
            _ => RedactionDecision::Keep,
        }
    }
}

#[test]
fn folded_keep_redact_drop_and_body_boundary_are_preserved() {
    for ending in ["\r\n", "\n"] {
        let raw = format!(
            "MESSAGE sip:bob@example.com SIP/2.0{ending}\
             X-Drop-Me: drop-secret{ending}\
             \tdrop-fold-secret{ending}\
             X-Redact-Me: redact-secret{ending}\
             \x20redact-fold-secret{ending}\
             Supported: timer,{ending}\
             \tpath{ending}\
             {ending}\
             X-Drop-Me: body-drop-value{ending}\
             \tbody-drop-fold"
        );
        let scrubbed = apply_message_redactor(&FoldPolicy, &raw);

        assert!(!scrubbed.contains("X-Drop-Me: drop-secret"));
        assert!(!scrubbed.contains("drop-fold-secret"));
        assert!(scrubbed.contains(&format!("X-Redact-Me: <hidden>{ending} <hidden>{ending}")));
        assert!(!scrubbed.contains("redact-secret"));
        assert!(!scrubbed.contains("redact-fold-secret"));
        assert!(scrubbed.contains(&format!("Supported: timer,{ending}\tpath")));
        assert!(scrubbed.ends_with(REDACTED_BODY_MARKER));
        assert!(!scrubbed.contains("body-drop-value"));
        assert!(!scrubbed.contains("body-drop-fold"));
        assert!(!scrubbed.ends_with('\n'));
    }
}

#[test]
fn safe_default_redacts_message_json_and_multiline_sdp_bodies() {
    let json = concat!(
        "MESSAGE sip:bob@example.com SIP/2.0\r\n",
        "Content-Type: application/json\r\n",
        "Content-Length: 68\r\n",
        "\r\n",
        "{\"type\":\"bridgefu.context.v1\",\"token\":\"json-body-secret\"}",
    );
    let scrubbed_json = apply_message_redactor(&DefaultTraceRedactor, json);
    assert!(scrubbed_json.ends_with("\r\n\r\n<redacted body>"));
    assert_eq!(scrubbed_json.matches(REDACTED_BODY_MARKER).count(), 1);
    assert!(!scrubbed_json.contains("bridgefu.context.v1"));
    assert!(!scrubbed_json.contains("json-body-secret"));
    assert!(!scrubbed_json.ends_with('\n'));

    for ending in ["\r\n", "\n"] {
        let sdp = format!(
            "INVITE sip:bob@example.com SIP/2.0{ending}\
             Content-Type: application/sdp{ending}\
             Content-Length: 99{ending}\
             {ending}\
             v=0{ending}\
             a=ice-pwd:sdp-body-secret{ending}\
             a=rtpmap:111 opus/48000/2{ending}"
        );
        let scrubbed_sdp = apply_message_redactor(&DefaultTraceRedactor, &sdp);
        assert_eq!(scrubbed_sdp.matches(REDACTED_BODY_MARKER).count(), 1);
        assert!(scrubbed_sdp.ends_with(&format!("{ending}{ending}{REDACTED_BODY_MARKER}{ending}")));
        assert!(!scrubbed_sdp.contains("sdp-body-secret"));
        assert!(!scrubbed_sdp.contains("a=rtpmap"));
    }
}

#[derive(Debug)]
struct DropBodyPolicy;

impl TraceRedactor for DropBodyPolicy {
    fn redact(&self, _header: &HeaderName, _value: &str) -> RedactionDecision {
        RedactionDecision::Keep
    }

    fn redact_body(&self, content_type: Option<&str>) -> BodyRedactionDecision {
        assert_eq!(content_type, Some("application/json"));
        BodyRedactionDecision::Drop
    }
}

#[test]
fn body_drop_omits_body_but_preserves_header_boundary() {
    for ending in ["\r\n", "\n"] {
        let raw = format!(
            "MESSAGE sip:bob@example.com SIP/2.0{ending}\
             Content-Type: application/json{ending}\
             Content-Length: 21{ending}\
             {ending}\
             drop-body-secret"
        );
        let scrubbed = apply_message_redactor(&DropBodyPolicy, &raw);
        assert!(scrubbed.ends_with(&format!("Content-Length: 21{ending}{ending}")));
        assert!(!scrubbed.contains("drop-body-secret"));
        assert!(!scrubbed.contains(REDACTED_BODY_MARKER));
    }
}

#[derive(Debug)]
struct ExplicitBodyKeepPolicy;

impl TraceRedactor for ExplicitBodyKeepPolicy {
    fn redact(&self, _header: &HeaderName, _value: &str) -> RedactionDecision {
        RedactionDecision::Keep
    }

    fn redact_body(&self, _content_type: Option<&str>) -> BodyRedactionDecision {
        BodyRedactionDecision::Keep
    }
}

#[test]
fn custom_policy_must_explicitly_keep_body_bytes() {
    let raw = "MESSAGE sip:bob@example.com SIP/2.0\nContent-Type: text/plain\n\nexplicit-body";
    assert_eq!(apply_message_redactor(&ExplicitBodyKeepPolicy, raw), raw);
    let safe_custom = apply_message_redactor(&AuthRedactor, raw);
    assert!(safe_custom.ends_with(REDACTED_BODY_MARKER));
    assert!(!safe_custom.contains("explicit-body"));
}

#[test]
fn redaction_debug_never_formats_replacement_values() {
    let decision = RedactionDecision::Redact("debug-secret".into());
    let debug = format!("{decision:?} {default:?}", default = DefaultTraceRedactor);
    assert!(!debug.contains("debug-secret"));
    assert!(debug.contains("Redact([redacted])"));
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

//! SIP-trace helpers: receiver config that enables tracing, inbound-trace
//! waiter, and the `X-Test: smoke` sentinel constants used across §10
//! tests.

#![allow(dead_code)]

use std::time::Duration;

use rvoip_sip::api::events::Event;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::Config;
use rvoip_sip::{SipTrace, SipTraceConfig, SipTraceDirection};

/// Sentinel header name used by smoke tests to assert wire emission.
pub const SMOKE_HEADER_NAME: &str = "X-Test";
/// Sentinel value paired with [`SMOKE_HEADER_NAME`].
pub const SMOKE_HEADER_VALUE: &str = "smoke";

/// `Config::local(name, port)` plus an explicit development-only verbatim
/// trace policy. This loopback test helper intentionally treats SIP trace as
/// a packet-capture oracle for §10 tests that inspect inbound wire values.
pub fn receiver_config(name: &str, port: u16) -> Config {
    let mut cfg = Config::local(name, port);
    cfg.sip_trace = SipTraceConfig {
        enabled: true,
        redact_sensitive_headers: false,
        include_body: true,
        ..SipTraceConfig::default()
    };
    // The compatibility booleans alone retain production-safe redaction.
    // Verbatim packet values require this deliberate test-only opt-in.
    cfg.trace_passthrough_for_development()
}

/// Drains `events` until an inbound `SipTrace` whose `start_line`
/// begins with `method_prefix` (e.g. `"INVITE"`, `"BYE"`, `"REGISTER"`)
/// arrives, or `timeout` elapses.
pub async fn wait_for_inbound_method(
    events: &mut EventReceiver,
    method_prefix: &str,
    timeout: Duration,
) -> Option<SipTrace> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, events.next()).await {
            Err(_) | Ok(None) => return None,
            Ok(Some(Event::SipTrace(trace))) => {
                if trace.direction == SipTraceDirection::Inbound
                    && trace.start_line.starts_with(method_prefix)
                {
                    return Some(trace);
                }
            }
            Ok(Some(_)) => continue,
        }
    }
}

/// Asserts the raw wire bytes contain both a header name and value.
/// Defaults to the [`SMOKE_HEADER_NAME`]/[`SMOKE_HEADER_VALUE`] pair.
pub fn assert_header_on_wire(raw_message: &str, name: &str, value: &str) {
    assert!(
        raw_message.contains(name),
        "expected `{name}` on the wire; got:\n{raw_message}"
    );
    assert!(
        raw_message.contains(value),
        "expected value `{value}` on the wire; got:\n{raw_message}"
    );
}

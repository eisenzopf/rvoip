//! SIP_API_DESIGN_2 §12.4 — pluggable trace-output redaction.
//!
//! Trace sinks (sip-trace logs, structured trace events, transport-level
//! capture) are operator-facing surfaces. When PII or carrier tokens
//! appear in headers (typically `Authorization`, `Proxy-Authorization`,
//! `P-Asserted-Identity`, `X-Customer-Token`-style extras), it is the
//! operator's responsibility to decide whether each header is logged
//! verbatim, scrubbed, or dropped entirely.
//!
//! `TraceRedactor` is the policy hook called by the trace path at the
//! [`DialogAdapter`](crate::adapters::dialog_adapter::DialogAdapter)
//! boundary just before a header value lands in the trace sink. The
//! returned [`RedactionDecision`] controls the wire-vs-trace divergence:
//! the wire form is untouched, the trace form follows the decision.
//!
//! Configure via [`Config::trace_redaction`](crate::Config::trace_redaction).

use std::sync::Arc;

use rvoip_sip_core::types::headers::HeaderName;

/// Outcome of a trace-redaction policy decision for one header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RedactionDecision {
    /// Trace the header verbatim.
    Keep,
    /// Replace the value with a fixed marker (the trace sink writes
    /// the header name + the supplied placeholder).
    Redact(String),
    /// Omit the header entirely from the trace output. The wire form
    /// is unaffected.
    Drop,
}

/// Policy hook consulted per header at the trace boundary. Implement
/// for application-specific redaction (e.g. log Authorization headers
/// as `Authorization: <redacted>`, drop `X-Customer-Token` entirely,
/// keep everything else verbatim).
pub trait TraceRedactor: Send + Sync + std::fmt::Debug {
    /// Decide what trace output should record for this header.
    fn redact(&self, header: &HeaderName, value: &str) -> RedactionDecision;
}

/// Default no-op redactor: returns [`RedactionDecision::Keep`] for
/// every header. Useful as the documented default and for tests.
#[derive(Clone, Debug, Default)]
pub struct PassthroughRedactor;

impl TraceRedactor for PassthroughRedactor {
    fn redact(&self, _header: &HeaderName, _value: &str) -> RedactionDecision {
        RedactionDecision::Keep
    }
}

/// Apply a redactor (if any) to a header/value pair and return the
/// trace-friendly form. `None` means "drop entirely; do not emit to
/// trace". This is the canonical helper for trace-emitting paths in
/// `DialogAdapter`.
pub fn apply_redaction(
    redactor: Option<&Arc<dyn TraceRedactor>>,
    header: &HeaderName,
    value: &str,
) -> Option<String> {
    match redactor {
        None => Some(value.to_string()),
        Some(r) => match r.redact(header, value) {
            RedactionDecision::Keep => Some(value.to_string()),
            RedactionDecision::Redact(replacement) => Some(replacement),
            RedactionDecision::Drop => None,
        },
    }
}

/// Apply a `TraceRedactor` to a rendered SIP message, walking each
/// header line and consulting the redactor per-header. Lines that the
/// redactor returns `Drop` for are omitted from the trace output; lines
/// that return `Redact(replacement)` are rewritten to
/// `<HeaderName>: <replacement>`; lines marked `Keep` pass through
/// verbatim.
///
/// Request/response start line and body bytes are preserved unchanged
/// — the redactor only sees header values.
pub fn apply_message_redactor(redactor: &dyn TraceRedactor, raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_headers = true;
    for line in raw.split_inclusive('\n') {
        // Strip the trailing newline for inspection so the parse logic
        // works on either CRLF or LF line endings.
        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
        if !in_headers {
            // Body: pass through verbatim.
            out.push_str(line);
            continue;
        }
        if trimmed.is_empty() {
            // Header/body boundary.
            out.push_str(line);
            in_headers = false;
            continue;
        }
        // Header lines have the shape `Name: value`. Continuation lines
        // (RFC 3261 §7.3.1) start with whitespace; pass them through
        // verbatim — they belong to the prior header.
        let bytes = trimmed.as_bytes();
        if matches!(bytes.first(), Some(b' ' | b'\t')) {
            out.push_str(line);
            continue;
        }
        let Some(colon) = trimmed.find(':') else {
            // No colon — not a header (start line); pass verbatim.
            out.push_str(line);
            continue;
        };
        let name = trimmed[..colon].trim();
        let value = trimmed[colon + 1..].trim();
        let header_name = name
            .parse::<HeaderName>()
            .unwrap_or_else(|_| HeaderName::Other(name.to_string()));
        match redactor.redact(&header_name, value) {
            RedactionDecision::Keep => out.push_str(line),
            RedactionDecision::Redact(replacement) => {
                out.push_str(name);
                out.push_str(": ");
                out.push_str(&replacement);
                // Preserve the original line ending.
                if line.ends_with("\r\n") {
                    out.push_str("\r\n");
                } else {
                    out.push('\n');
                }
            }
            RedactionDecision::Drop => {
                // Omit the header entirely from the trace.
            }
        }
    }
    out
}

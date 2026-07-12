//! SIP_API_DESIGN_2 §12.4 — pluggable trace-output redaction.
//!
//! Trace sinks (sip-trace logs, structured trace events, transport-level
//! capture) are operator-facing surfaces. The default policy redacts
//! credentials, identity-bearing fields, and every application-defined
//! header value. Verbatim tracing requires an explicit
//! [`PassthroughRedactor`] development/operator override.
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
#[derive(Clone, PartialEq, Eq)]
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

impl std::fmt::Debug for RedactionDecision {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Keep => formatter.write_str("Keep"),
            Self::Redact(_) => formatter.write_str("Redact([redacted])"),
            Self::Drop => formatter.write_str("Drop"),
        }
    }
}

/// Policy hook consulted per header at the trace boundary. Implement
/// for application-specific redaction (e.g. log Authorization headers
/// as `Authorization: <redacted>`, drop `X-Customer-Token` entirely,
/// keep everything else verbatim).
pub trait TraceRedactor: Send + Sync + std::fmt::Debug {
    /// Decide what trace output should record for this header.
    fn redact(&self, header: &HeaderName, value: &str) -> RedactionDecision;
}

/// Production-safe default trace policy.
///
/// Authentication material, asserted identities, addressing fields that may
/// carry PII, and every application-defined header value are redacted. Common
/// protocol-routing and capability headers remain available for diagnostics.
#[derive(Clone, Debug, Default)]
pub struct DefaultTraceRedactor;

impl TraceRedactor for DefaultTraceRedactor {
    fn redact(&self, header: &HeaderName, _value: &str) -> RedactionDecision {
        match header {
            HeaderName::Authorization
            | HeaderName::ProxyAuthorization
            | HeaderName::WwwAuthenticate
            | HeaderName::ProxyAuthenticate
            | HeaderName::AuthenticationInfo
            | HeaderName::Identity
            | HeaderName::PAssertedIdentity
            | HeaderName::PPreferredIdentity
            | HeaderName::From
            | HeaderName::To
            | HeaderName::Contact
            | HeaderName::ReplyTo
            | HeaderName::ReferTo
            | HeaderName::ReferredBy
            | HeaderName::Subject
            | HeaderName::AlertInfo
            | HeaderName::CallInfo
            | HeaderName::ErrorInfo
            | HeaderName::Other(_) => RedactionDecision::Redact("<redacted>".to_string()),
            _ => RedactionDecision::Keep,
        }
    }
}

/// Construct the production-safe default as a shared policy object.
pub fn default_trace_redactor() -> Arc<dyn TraceRedactor> {
    Arc::new(DefaultTraceRedactor)
}

/// Explicit no-op policy for controlled development/operator diagnostics.
///
/// Selecting this policy can expose credentials, PII, and application context
/// in trace sinks. Production/default configuration never selects it.
#[derive(Clone, Debug, Default)]
pub struct PassthroughRedactor;

impl TraceRedactor for PassthroughRedactor {
    fn redact(&self, _header: &HeaderName, _value: &str) -> RedactionDecision {
        RedactionDecision::Keep
    }
}

/// Apply a redactor (if any) to a header/value pair and return the
/// trace-friendly form. An absent configured policy uses
/// [`DefaultTraceRedactor`]; verbatim output requires an explicit
/// [`PassthroughRedactor`]. This is the canonical helper for trace-emitting
/// paths in `DialogAdapter`.
pub fn apply_redaction(
    redactor: Option<&Arc<dyn TraceRedactor>>,
    header: &HeaderName,
    value: &str,
) -> Option<String> {
    match redactor {
        None => match DefaultTraceRedactor.redact(header, value) {
            RedactionDecision::Keep => Some(value.to_string()),
            RedactionDecision::Redact(replacement) => Some(replacement),
            RedactionDecision::Drop => None,
        },
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
    enum ContinuationDecision {
        None,
        Keep,
        Redact(String),
        Drop,
    }

    fn push_line_ending(out: &mut String, line: &str) {
        if line.ends_with("\r\n") {
            out.push_str("\r\n");
        } else if line.ends_with('\n') {
            out.push('\n');
        }
    }

    let mut out = String::with_capacity(raw.len());
    let mut in_headers = true;
    let mut continuation = ContinuationDecision::None;
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
            continuation = ContinuationDecision::None;
            continue;
        }
        // RFC 3261 §7.3.1 continuation lines inherit the complete decision
        // made for their owning header. A redacted or dropped header must
        // never leak through a folded continuation.
        let bytes = trimmed.as_bytes();
        if matches!(bytes.first(), Some(b' ' | b'\t')) {
            match &continuation {
                ContinuationDecision::None | ContinuationDecision::Keep => out.push_str(line),
                ContinuationDecision::Redact(replacement) => {
                    let leading_whitespace_len = trimmed.len() - trimmed.trim_start().len();
                    out.push_str(&trimmed[..leading_whitespace_len]);
                    out.push_str(replacement);
                    push_line_ending(&mut out, line);
                }
                ContinuationDecision::Drop => {}
            }
            continue;
        }
        let Some(colon) = trimmed.find(':') else {
            // No colon — not a header (start line); pass verbatim.
            out.push_str(line);
            continuation = ContinuationDecision::None;
            continue;
        };
        let name = trimmed[..colon].trim();
        let value = trimmed[colon + 1..].trim();
        let header_name = name
            .parse::<HeaderName>()
            .unwrap_or_else(|_| HeaderName::Other(name.to_string()));
        match redactor.redact(&header_name, value) {
            RedactionDecision::Keep => {
                out.push_str(line);
                continuation = ContinuationDecision::Keep;
            }
            RedactionDecision::Redact(replacement) => {
                out.push_str(name);
                out.push_str(": ");
                out.push_str(&replacement);
                push_line_ending(&mut out, line);
                continuation = ContinuationDecision::Redact(replacement);
            }
            RedactionDecision::Drop => {
                // Omit the header entirely from the trace.
                continuation = ContinuationDecision::Drop;
            }
        }
    }
    out
}

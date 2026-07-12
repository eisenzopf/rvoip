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
//! boundary just before a header value or body lands in the trace sink. The
//! returned decisions control the wire-vs-trace divergence:
//! the wire form is untouched, the trace form follows the decision.
//!
//! Configure via [`Config::trace_redaction`](crate::Config::trace_redaction).

use std::sync::Arc;

use rvoip_sip_core::types::headers::HeaderName;

/// Fixed diagnostic marker emitted in place of a redacted SIP body.
///
/// Body policies cannot supply their own replacement text, which prevents a
/// policy from accidentally reflecting body-derived secrets into a trace.
pub const REDACTED_BODY_MARKER: &str =
    rvoip_infra_common::events::cross_crate::SIP_TRACE_REDACTED_BODY;

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

/// Outcome of a trace-redaction policy decision for a SIP message body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyRedactionDecision {
    /// Trace the body verbatim.
    ///
    /// This must be an explicit policy override. The trait default is
    /// [`BodyRedactionDecision::Redact`].
    Keep,
    /// Replace the complete body with [`REDACTED_BODY_MARKER`].
    Redact,
    /// Omit the complete body from trace output.
    Drop,
}

/// Policy hook consulted for the request target, headers, and body at the trace
/// boundary.
/// Implement for application-specific redaction (e.g. log Authorization
/// headers as `Authorization: <redacted>`, drop `X-Customer-Token` entirely,
/// and retain the safe default body decision).
pub trait TraceRedactor: Send + Sync + std::fmt::Debug {
    /// Decide what trace output should record for this header.
    fn redact(&self, header: &HeaderName, value: &str) -> RedactionDecision;

    /// Decide what trace output should record for the complete message body.
    ///
    /// The safe additive default redacts every non-empty body. Existing custom
    /// header policies therefore cannot begin leaking MESSAGE JSON, SDP, or
    /// other content when body tracing is enabled. Implementations receive the
    /// declared content type, but never the body bytes. Verbatim body tracing
    /// requires an explicit [`BodyRedactionDecision::Keep`] override.
    fn redact_body(&self, _content_type: Option<&str>) -> BodyRedactionDecision {
        BodyRedactionDecision::Redact
    }

    /// Whether this policy deliberately permits a fully verbatim SIP trace,
    /// including the Request-URI and body bytes.
    ///
    /// The safe additive default is `false`. Only controlled development or
    /// operator policies should override it. Returning `true` also disables the
    /// lower transport layer's defense-in-depth sanitizer.
    fn allows_verbatim_trace(&self) -> bool {
        false
    }
}

/// Production-safe default trace policy.
///
/// Authentication material, asserted identities, addressing fields that may
/// carry PII, every application-defined header value, and every non-empty body
/// are redacted. A deliberately small set of sequencing, framing, capability,
/// and expiry headers remains available for diagnostics.
#[derive(Clone, Debug, Default)]
pub struct DefaultTraceRedactor;

impl TraceRedactor for DefaultTraceRedactor {
    fn redact(&self, header: &HeaderName, _value: &str) -> RedactionDecision {
        match header {
            // Deliberately retained protocol sequencing, framing, capability,
            // and expiry values. New typed headers fail closed in the branch
            // below until they are explicitly reviewed.
            HeaderName::CallId
            | HeaderName::ContentLength
            | HeaderName::ContentType
            | HeaderName::CSeq
            | HeaderName::MaxForwards
            | HeaderName::Allow
            | HeaderName::Expires
            | HeaderName::MinExpires
            | HeaderName::Supported
            | HeaderName::RAck
            | HeaderName::Accept
            | HeaderName::AcceptEncoding
            | HeaderName::ContentEncoding
            | HeaderName::Require
            | HeaderName::Timestamp
            | HeaderName::Priority
            | HeaderName::Date
            | HeaderName::MimeVersion
            | HeaderName::ProxyRequire
            | HeaderName::Unsupported
            | HeaderName::SessionExpires
            | HeaderName::MinSE
            | HeaderName::RSeq
            | HeaderName::AllowEvents => RedactionDecision::Keep,
            // Authentication, identities, addressing, routing, descriptive
            // free text, opaque entity tags, and every application-defined
            // header use a fixed non-reflective marker.
            _ => RedactionDecision::Redact("<redacted>".to_string()),
        }
    }
}

/// Construct the production-safe default as a shared policy object.
pub fn default_trace_redactor() -> Arc<dyn TraceRedactor> {
    Arc::new(DefaultTraceRedactor)
}

/// Explicit no-op policy for controlled development/operator diagnostics.
///
/// Selecting this policy can expose credentials, PII, application context,
/// SDP, and other body content in trace sinks. Production/default
/// configuration never selects it.
#[derive(Clone, Debug, Default)]
pub struct PassthroughRedactor;

impl TraceRedactor for PassthroughRedactor {
    fn redact(&self, _header: &HeaderName, _value: &str) -> RedactionDecision {
        RedactionDecision::Keep
    }

    fn redact_body(&self, _content_type: Option<&str>) -> BodyRedactionDecision {
        BodyRedactionDecision::Keep
    }

    fn allows_verbatim_trace(&self) -> bool {
        true
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

/// Apply a `TraceRedactor` to a rendered SIP message. Request targets use a
/// fixed marker unless [`TraceRedactor::allows_verbatim_trace`] explicitly opts
/// into development/operator output. Each header line then consults the
/// redactor per-header. Lines that the
/// redactor returns `Drop` for are omitted from the trace output; lines
/// that return `Redact(replacement)` are rewritten to
/// `<HeaderName>: <replacement>`; lines marked `Keep` pass through
/// verbatim.
///
/// The complete body is handled as a separate decision. The safe trait
/// default replaces any non-empty body with [`REDACTED_BODY_MARKER`]; only an
/// explicit [`BodyRedactionDecision::Keep`] override preserves body bytes.
/// The header/body boundary and the body's final-newline state are retained.
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

    // A fully verbatim policy is the one deliberate exception to the
    // fail-closed parser below. Keeping this fast path explicit also prevents
    // malformed diagnostic input from being partly rewritten despite the
    // operator's passthrough selection.
    if redactor.allows_verbatim_trace() {
        return raw.to_string();
    }

    let mut out = String::with_capacity(raw.len());
    let mut continuation = ContinuationDecision::None;
    let mut content_type = None;
    let mut offset = 0;
    let mut first_line = true;
    for line in raw.split_inclusive('\n') {
        offset += line.len();
        // Strip the trailing newline for inspection so the parse logic
        // works on either CRLF or LF line endings.
        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
        if first_line {
            out.push_str(
                &rvoip_infra_common::events::cross_crate::redact_sip_trace_start_line(trimmed),
            );
            push_line_ending(&mut out, line);
            first_line = false;
            continue;
        }
        if trimmed.is_empty() {
            // Header/body boundary.
            out.push_str(line);
            let body = &raw[offset..];
            if !body.is_empty() {
                match redactor.redact_body(content_type.as_deref()) {
                    BodyRedactionDecision::Keep => out.push_str(body),
                    BodyRedactionDecision::Redact => {
                        out.push_str(REDACTED_BODY_MARKER);
                        if body.ends_with("\r\n") {
                            out.push_str("\r\n");
                        } else if body.ends_with('\n') {
                            out.push('\n');
                        }
                    }
                    BodyRedactionDecision::Drop => {}
                }
            }
            return out;
        }
        // RFC 3261 §7.3.1 continuation lines inherit the complete decision
        // made for their owning header. A redacted or dropped header must
        // never leak through a folded continuation.
        let bytes = trimmed.as_bytes();
        if matches!(bytes.first(), Some(b' ' | b'\t')) {
            match &continuation {
                ContinuationDecision::None => {
                    let leading_whitespace_len = trimmed.len() - trimmed.trim_start().len();
                    out.push_str(&trimmed[..leading_whitespace_len]);
                    out.push_str(
                        rvoip_infra_common::events::cross_crate::SIP_TRACE_REDACTED_HEADER_VALUE,
                    );
                    push_line_ending(&mut out, line);
                }
                ContinuationDecision::Keep => out.push_str(line),
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
            // The start line was consumed above. Any later colonless line in
            // the header section is malformed and must fail closed.
            out.push_str(rvoip_infra_common::events::cross_crate::SIP_TRACE_REDACTED_HEADER_VALUE);
            push_line_ending(&mut out, line);
            continuation = ContinuationDecision::Redact(
                rvoip_infra_common::events::cross_crate::SIP_TRACE_REDACTED_HEADER_VALUE.into(),
            );
            continue;
        };
        let name = trimmed[..colon].trim();
        let value = trimmed[colon + 1..].trim();
        let header_name = name
            .parse::<HeaderName>()
            .unwrap_or_else(|_| HeaderName::Other(name.to_string()));
        if header_name == HeaderName::ContentType {
            content_type = Some(value.to_string());
        }
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

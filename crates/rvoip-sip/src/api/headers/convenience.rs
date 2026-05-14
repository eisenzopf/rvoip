//! Typed-but-shape-free header constructors for headers that
//! `rvoip-sip-core` does not (yet) model as their own `TypedHeader`
//! variant. Each helper returns a `TypedHeader::Other` with the
//! canonical header name so policy classification and wire output stay
//! correct as sip-core grows native variants.
//!
//! All inputs travel through the `Other(HeaderName::Other(...))`
//! channel, so the policy module classifies them as
//! `ApplicationControlled` (modulo carry-through rules). Use these
//! when you need readable call-site code without hand-typing
//! `TypedHeader::Other(HeaderName::Other("…".into()), …)` each time.

use rvoip_sip_core::types::headers::{HeaderName, HeaderValue, TypedHeader};

fn other(name: &str, value: impl Into<String>) -> TypedHeader {
    TypedHeader::Other(
        HeaderName::Other(name.to_string()),
        HeaderValue::Raw(value.into().into_bytes()),
    )
}

/// `Diversion` (RFC 5806).
pub fn diversion(value: impl Into<String>) -> TypedHeader {
    other("Diversion", value)
}

/// `History-Info` (RFC 7044).
pub fn history_info(value: impl Into<String>) -> TypedHeader {
    other("History-Info", value)
}

/// `Privacy` (RFC 3323).
pub fn privacy(value: impl Into<String>) -> TypedHeader {
    other("Privacy", value)
}

/// `Replaces` (RFC 3891).
pub fn replaces(value: impl Into<String>) -> TypedHeader {
    other("Replaces", value)
}

/// `Target-Dialog` (RFC 4538).
pub fn target_dialog(value: impl Into<String>) -> TypedHeader {
    other("Target-Dialog", value)
}

/// `Session-Expires` (RFC 4028). Use `min_se` for the matching `Min-SE:`.
pub fn session_expires(value: impl Into<String>) -> TypedHeader {
    other("Session-Expires", value)
}

/// `Min-SE` (RFC 4028).
pub fn min_se(seconds: u32) -> TypedHeader {
    other("Min-SE", seconds.to_string())
}

/// `P-Charging-Vector` (RFC 7315).
pub fn p_charging_vector(value: impl Into<String>) -> TypedHeader {
    other("P-Charging-Vector", value)
}

/// `P-Called-Party-ID` (RFC 3455).
pub fn p_called_party_id(value: impl Into<String>) -> TypedHeader {
    other("P-Called-Party-ID", value)
}

// ─────────────────────────────────────────────────────────────────────
// SIP_API_DESIGN_2 §10 #24 — multipart/mixed body composition + parsing.
// RFC 5621 §3 spells out the wire form: a `Content-Type: multipart/mixed;
// boundary=<token>` header on the outer message, then `--<token>` line
// separators between parts, each part with its own header block. Used
// by isUP-over-INFO trunks (`application/isup;...`) bundled with SDP,
// SIP-INFO DTMF (`application/dtmf-relay`) bundled with annotations,
// and PIDF presence bodies with metadata extensions.
// ─────────────────────────────────────────────────────────────────────

use bytes::Bytes;

/// One leaf of a `multipart/mixed` body: a header block plus the part's
/// raw bytes. Build via [`MultipartPart::new`].
#[derive(Debug, Clone)]
pub struct MultipartPart {
    /// Headers preceding the part body. Typically at least
    /// `Content-Type:` and `Content-Disposition:`.
    pub headers: Vec<TypedHeader>,
    /// Raw part bytes (already-encoded payload).
    pub body: Bytes,
}

impl MultipartPart {
    /// Construct a part with a content type, optional content
    /// disposition, and raw body bytes. Convenience wrapper —
    /// applications can also populate `headers` directly.
    pub fn new(
        content_type: impl Into<String>,
        disposition: Option<&str>,
        body: impl Into<Bytes>,
    ) -> Self {
        let mut headers = Vec::with_capacity(2);
        headers.push(other("Content-Type", content_type));
        if let Some(disp) = disposition {
            headers.push(other("Content-Disposition", disp));
        }
        Self {
            headers,
            body: body.into(),
        }
    }
}

/// Parse errors for [`multipart_parse`]. Each variant captures enough
/// context for the application to surface a useful 4xx response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultipartParseError {
    /// The `Content-Type` header value was missing the required
    /// `boundary=<token>` parameter.
    MissingBoundary,
    /// The boundary parameter was present but empty.
    EmptyBoundary,
    /// A part separator was found but the body did not include the
    /// closing `--<boundary>--` marker.
    UnterminatedBody,
    /// A part's header block was malformed (missing `\r\n\r\n`
    /// terminator before the body).
    MalformedPartHeaders,
}

impl std::fmt::Display for MultipartParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingBoundary => write!(f, "multipart Content-Type missing boundary parameter"),
            Self::EmptyBoundary => write!(f, "multipart boundary parameter is empty"),
            Self::UnterminatedBody => write!(f, "multipart body missing closing --<boundary>-- marker"),
            Self::MalformedPartHeaders => write!(f, "multipart part headers missing CRLF CRLF terminator"),
        }
    }
}

impl std::error::Error for MultipartParseError {}

/// Construct a `multipart/mixed` body. Returns the
/// `(Content-Type: multipart/mixed; boundary=<token>, body)` pair so
/// the caller can stamp the header on the outbound message and attach
/// the body. The boundary token is a stable random string for the
/// lifetime of the body; callers do not need to coordinate it.
pub fn multipart_mixed(parts: Vec<MultipartPart>) -> (TypedHeader, Bytes) {
    let boundary = format!("rvoip-{}", uuid::Uuid::new_v4().simple());
    let mut body: Vec<u8> = Vec::new();
    for part in &parts {
        body.extend_from_slice(b"--");
        body.extend_from_slice(boundary.as_bytes());
        body.extend_from_slice(b"\r\n");
        for h in &part.headers {
            body.extend_from_slice(h.to_string().as_bytes());
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(&part.body);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"--\r\n");

    let content_type = other(
        "Content-Type",
        format!("multipart/mixed; boundary={}", boundary),
    );
    (content_type, Bytes::from(body))
}

/// Parse a `multipart/mixed` body. `content_type` is the full
/// `Content-Type:` header value (e.g.
/// `multipart/mixed; boundary=foo`); `body` is the wire payload.
/// Returns one [`MultipartPart`] per logical part on success.
///
/// Part headers are parsed leniently: each `Name: value` line up to the
/// blank line is preserved as a `TypedHeader::Other`. The body bytes
/// up to (but not including) the next `--<boundary>` separator are
/// captured verbatim.
pub fn multipart_parse(
    content_type: &str,
    body: &[u8],
) -> Result<Vec<MultipartPart>, MultipartParseError> {
    let boundary_param = content_type
        .split(';')
        .map(str::trim)
        .find_map(|p| p.strip_prefix("boundary="))
        .ok_or(MultipartParseError::MissingBoundary)?
        .trim_matches('"');
    if boundary_param.is_empty() {
        return Err(MultipartParseError::EmptyBoundary);
    }
    let marker = format!("--{}", boundary_param);
    let close = format!("--{}--", boundary_param);

    // Find segments between `--boundary` markers; require the close
    // marker to appear so unterminated bodies are caught.
    let body_str = String::from_utf8_lossy(body);
    if !body_str.contains(&close) {
        return Err(MultipartParseError::UnterminatedBody);
    }

    let mut parts = Vec::new();
    let trimmed = body_str
        .split(&close)
        .next()
        .unwrap_or("");
    let segments: Vec<&str> = trimmed.split(&marker).collect();
    for seg in segments.iter().skip(1) {
        // Each segment begins with `\r\n` (after the marker) and
        // contains headers + CRLFCRLF + body + trailing CRLF.
        let seg = seg.trim_start_matches("\r\n").trim_end_matches("\r\n");
        if seg.is_empty() {
            continue;
        }
        let (headers_blob, body_blob) = seg
            .split_once("\r\n\r\n")
            .ok_or(MultipartParseError::MalformedPartHeaders)?;
        let mut headers = Vec::new();
        for line in headers_blob.split("\r\n") {
            if let Some((name, value)) = line.split_once(':') {
                headers.push(other(name.trim(), value.trim()));
            }
        }
        parts.push(MultipartPart {
            headers,
            body: Bytes::copy_from_slice(body_blob.as_bytes()),
        });
    }
    Ok(parts)
}

#[cfg(test)]
mod multipart_tests {
    use super::*;

    #[test]
    fn round_trip_two_parts() {
        let parts = vec![
            MultipartPart::new("application/sdp", None, "v=0\r\no=- 1 1 IN IP4 0.0.0.0\r\n"),
            MultipartPart::new(
                "application/isup;version=ansi92",
                Some("signal"),
                vec![0x01, 0x02, 0x03],
            ),
        ];
        let (ct, body) = multipart_mixed(parts);
        // The header's stringified form has `Content-Type:` prefix; we
        // need just the value for the parser. Extract by splitting on
        // the first colon.
        let ct_str = ct.to_string();
        let value = ct_str.split_once(':').map(|(_, v)| v.trim()).unwrap();
        let round = multipart_parse(value, &body).expect("parse round-trip");
        assert_eq!(round.len(), 2);
        assert!(round[0]
            .headers
            .iter()
            .any(|h| h.to_string().contains("application/sdp")));
        assert_eq!(&round[1].body[..], &[0x01, 0x02, 0x03]);
    }

    #[test]
    fn missing_boundary_returns_err() {
        let err = multipart_parse("multipart/mixed", b"--x--\r\n").unwrap_err();
        assert_eq!(err, MultipartParseError::MissingBoundary);
    }

    #[test]
    fn unterminated_body_returns_err() {
        let err = multipart_parse("multipart/mixed; boundary=x", b"--x\r\nfoo\r\n")
            .unwrap_err();
        assert_eq!(err, MultipartParseError::UnterminatedBody);
    }
}

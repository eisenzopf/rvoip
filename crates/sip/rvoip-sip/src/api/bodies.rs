//! SIP_API_DESIGN_2 §3.6 — Convenience body constructors.
//!
//! Each helper produces a `(content_type, Bytes)` tuple suitable for
//! attaching to a SIP body via `with_body(..)` + `with_content_type(..)`
//! on any of the outbound builders.
//!
//! ```rust,no_run
//! # async fn demo(coord: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, session: &rvoip_sip::CallId) -> rvoip_sip::Result<()> {
//! use rvoip_sip::bodies;
//!
//! let (ct, body) = bodies::dtmf_relay('5', 200);
//! coord
//!     .info(session, ct)
//!     .with_body(body)
//!     .send()
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! Forward-compatibility: each helper is `#[non_exhaustive]` in spirit
//! — its return shape will not change, but additional helpers may be
//! added in future releases.

use bytes::Bytes;

/// SDP body (RFC 4566). Returns
/// `("application/sdp".to_string(), bytes)`.
pub fn sdp(s: impl Into<String>) -> (String, Bytes) {
    let body: String = s.into();
    (
        "application/sdp".to_string(),
        Bytes::from(body.into_bytes()),
    )
}

/// `application/dtmf-relay` body (RFC 2833 / draft-choudhuri-sip-info-digit).
/// `signal` is the DTMF digit (`'0'`..`'9'`, `'*'`, `'#'`, `'A'`..`'D'`);
/// `duration_ms` is the digit duration in milliseconds.
pub fn dtmf_relay(signal: char, duration_ms: u32) -> (String, Bytes) {
    let body = format!("Signal={}\r\nDuration={}\r\n", signal, duration_ms);
    (
        "application/dtmf-relay".to_string(),
        Bytes::from(body.into_bytes()),
    )
}

/// Minimal `Presence` document model used by [`pidf_xml`].
///
/// `#[non_exhaustive]` so additional fields can be added without
/// breaking external callers (RFC 3863 has a richer schema; this
/// covers the open / closed basic states most applications need).
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct Presence {
    /// Presentity entity URI (e.g. `sip:alice@example.com`).
    pub entity: String,
    /// `"open"` (available) or `"closed"` (unavailable).
    pub basic: String,
    /// Optional human-readable note carried alongside the basic
    /// status.
    pub note: Option<String>,
}

impl Presence {
    /// Build a `Presence` for the given entity URI and basic status
    /// (`"open"` or `"closed"`), with no note.
    pub fn new(entity: impl Into<String>, basic: impl Into<String>) -> Self {
        Self {
            entity: entity.into(),
            basic: basic.into(),
            note: None,
        }
    }
    /// Attach a human-readable note to the presence document.
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

/// `application/pidf+xml` body (RFC 3863). Produces a minimal valid
/// PIDF document carrying the basic `open` / `closed` status with an
/// optional human-readable note.
pub fn pidf_xml(presence: &Presence) -> (String, Bytes) {
    let note_block = match presence.note.as_ref() {
        Some(n) => format!("    <note>{}</note>\n", xml_escape(n)),
        None => String::new(),
    };
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <presence xmlns=\"urn:ietf:params:xml:ns:pidf\" entity=\"{}\">\n\
         \x20 <tuple id=\"t1\">\n\
         \x20   <status><basic>{}</basic></status>\n\
         {}\
         \x20 </tuple>\n\
         </presence>\n",
        xml_escape(&presence.entity),
        xml_escape(&presence.basic),
        note_block,
    );
    (
        "application/pidf+xml".to_string(),
        Bytes::from(body.into_bytes()),
    )
}

/// `application/simple-message-summary` body (RFC 3842). Indicates
/// pending voicemail messages alongside the configured account URI.
pub fn simple_message_summary(
    account_uri: impl Into<String>,
    messages_waiting: bool,
    new_count: u32,
    old_count: u32,
) -> (String, Bytes) {
    let body = format!(
        "Messages-Waiting: {}\r\n\
         Message-Account: {}\r\n\
         Voice-Message: {}/{}\r\n",
        if messages_waiting { "yes" } else { "no" },
        account_uri.into(),
        new_count,
        old_count,
    );
    (
        "application/simple-message-summary".to_string(),
        Bytes::from(body.into_bytes()),
    )
}

/// `application/isup` body (RFC 3204). Wraps raw ISUP layer-3 bytes
/// with the canonical Content-Type for SIP-I gateways.
pub fn isup_l3(bytes: impl Into<Bytes>) -> (String, Bytes) {
    ("application/isup".to_string(), bytes.into())
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdp_returns_application_sdp() {
        let (ct, body) = sdp("v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\n");
        assert_eq!(ct, "application/sdp");
        assert!(body.starts_with(b"v=0"));
    }

    #[test]
    fn dtmf_relay_emits_signal_and_duration() {
        let (ct, body) = dtmf_relay('5', 200);
        assert_eq!(ct, "application/dtmf-relay");
        let rendered = std::str::from_utf8(&body).unwrap();
        assert!(rendered.contains("Signal=5"));
        assert!(rendered.contains("Duration=200"));
    }

    #[test]
    fn pidf_xml_escapes_entity_and_note() {
        let presence =
            Presence::new("sip:alice@example.com", "open").with_note("In a <meeting> & away");
        let (ct, body) = pidf_xml(&presence);
        assert_eq!(ct, "application/pidf+xml");
        let rendered = std::str::from_utf8(&body).unwrap();
        assert!(rendered.contains("entity=\"sip:alice@example.com\""));
        assert!(rendered.contains("In a &lt;meeting&gt; &amp; away"));
    }

    #[test]
    fn simple_message_summary_emits_required_headers() {
        let (ct, body) = simple_message_summary("sip:vm@example.com", true, 3, 5);
        assert_eq!(ct, "application/simple-message-summary");
        let rendered = std::str::from_utf8(&body).unwrap();
        assert!(rendered.contains("Messages-Waiting: yes"));
        assert!(rendered.contains("Voice-Message: 3/5"));
    }

    #[test]
    fn isup_l3_preserves_bytes() {
        let raw = Bytes::from(&b"\x01\x02\x03"[..]);
        let (ct, body) = isup_l3(raw.clone());
        assert_eq!(ct, "application/isup");
        assert_eq!(body, raw);
    }
}

//! Byte-level locator for the top `Via` header line in a serialized
//! SIP message.
//!
//! Used by stateless-proxy forwarders that need to push or pop the
//! top Via WITHOUT round-tripping the whole message through the
//! structured parser — because a re-serialization would invalidate
//! the RFC 8224 `Identity` JWS signature (the JWT covers a specific
//! canonical wire form of the headers it signs over; parser
//! re-emission rarely matches byte-for-byte).
//!
//! The locator returns the byte range of the first Via line,
//! inclusive of the trailing CRLF. Long-form (`Via:`) and compact
//! form (`v:`) header names are both recognised, case-insensitively
//! per RFC 3261 §7.3.3.
//!
//! ## Limitations
//!
//! - Folded header values (continuation via CRLF SP/HTAB, RFC 3261
//!   §7.3.1) are not unfolded — the locator stops at the first CRLF.
//!   The rvoip serializer never folds, so this is correct for the
//!   in-tree path; external messages with folded Vias will lose the
//!   continuation lines on pop.
//! - Comma-separated multi-value Via headers (`Via: SIP/2.0/UDP h1,
//!   SIP/2.0/UDP h2`) are treated as a single line — pop removes all
//!   entries on that line. The rvoip serializer emits one Via per
//!   line, so this is correct for in-tree messages.

use std::ops::Range;

/// Locate the first `Via:` (or compact `v:`) header line in `bytes`.
///
/// Returns the byte range of the full line, **inclusive** of the
/// trailing `\r\n`. The range can be passed directly to
/// `BytesMut::splice` or used to slice for replacement.
///
/// Returns `None` if no Via header is found or the file is malformed
/// (no CRLF terminator before end-of-buffer).
pub fn find_top_via_line(bytes: &[u8]) -> Option<Range<usize>> {
    let mut cursor = 0;
    while cursor < bytes.len() {
        // Find the end of the current line.
        let line_end = match find_crlf(bytes, cursor) {
            Some(end) => end,
            None => return None,
        };
        let line = &bytes[cursor..line_end];

        if is_via_header(line) {
            // Return inclusive of CRLF.
            return Some(cursor..(line_end + 2));
        }

        cursor = line_end + 2;
    }
    None
}

/// Find the offset of the `\r\n` that terminates the line starting at
/// `from`. Returns the index of the `\r`.
fn find_crlf(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\r' && bytes[i + 1] == b'\n' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// True when `line` starts with a Via header name (`Via` long form or
/// `v` compact form) followed by optional whitespace and a colon.
/// Per RFC 3261 §7.3.1 / §7.3.3, header names are case-insensitive.
fn is_via_header(line: &[u8]) -> bool {
    // Long form: "Via" + optional WSP + ":"
    if line.len() >= 4 && line[..3].eq_ignore_ascii_case(b"Via") {
        return has_colon_after_optional_wsp(&line[3..]);
    }
    // Compact form: "v" + optional WSP + ":"
    if line.len() >= 2 && line[..1].eq_ignore_ascii_case(b"v") {
        return has_colon_after_optional_wsp(&line[1..]);
    }
    false
}

fn has_colon_after_optional_wsp(rest: &[u8]) -> bool {
    let mut i = 0;
    while i < rest.len() && (rest[i] == b' ' || rest[i] == b'\t') {
        i += 1;
    }
    i < rest.len() && rest[i] == b':'
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE: &[u8] = b"INVITE sip:b@x SIP/2.0\r\n\
Via: SIP/2.0/UDP a.example.com:5060;branch=z9hG4bKabc\r\n\
From: <sip:a@x>;tag=t\r\n\
To: <sip:b@x>\r\n\
\r\n";

    #[test]
    fn finds_long_form_via() {
        let range = find_top_via_line(SIMPLE).expect("Via present");
        let line = &SIMPLE[range.clone()];
        assert!(line.starts_with(b"Via: "));
        assert!(line.ends_with(b"\r\n"));
        assert!(line.windows(7).any(|w| w == b"z9hG4bK"));
    }

    #[test]
    fn finds_compact_form_via() {
        let msg = b"INVITE sip:b@x SIP/2.0\r\n\
v: SIP/2.0/TCP a.example.com:5060;branch=z9hG4bKxyz\r\n\
From: <sip:a@x>;tag=t\r\n\
\r\n";
        let range = find_top_via_line(msg).expect("v: present");
        assert!(msg[range].starts_with(b"v: "));
    }

    #[test]
    fn case_insensitive_match() {
        let msg = b"INVITE sip:b@x SIP/2.0\r\n\
VIA: SIP/2.0/UDP a:5060;branch=z9hG4bKABC\r\n\
\r\n";
        assert!(find_top_via_line(msg).is_some());

        let msg2 = b"INVITE sip:b@x SIP/2.0\r\n\
vIa: SIP/2.0/UDP a:5060;branch=z9hG4bK\r\n\
\r\n";
        assert!(find_top_via_line(msg2).is_some());
    }

    #[test]
    fn skips_non_via_headers_before_via() {
        let msg = b"INVITE sip:b@x SIP/2.0\r\n\
Max-Forwards: 70\r\n\
Via: SIP/2.0/UDP a:5060;branch=z9hG4bK1\r\n\
Via: SIP/2.0/UDP b:5060;branch=z9hG4bK2\r\n\
\r\n";
        let range = find_top_via_line(msg).expect("Via present");
        // Should match the FIRST Via line (top of stack), not the second.
        let line = &msg[range];
        assert!(line.windows(8).any(|w| w == b"z9hG4bK1"));
        assert!(!line.windows(8).any(|w| w == b"z9hG4bK2"));
    }

    #[test]
    fn returns_none_when_no_via() {
        let msg = b"INVITE sip:b@x SIP/2.0\r\n\
From: <sip:a@x>;tag=t\r\n\
\r\n";
        assert!(find_top_via_line(msg).is_none());
    }

    #[test]
    fn rejects_lookalike_header_names() {
        // "Via-Like:" must NOT match — colon comes after non-WSP chars.
        let msg = b"INVITE sip:b@x SIP/2.0\r\n\
Via-Like: junk\r\n\
\r\n";
        assert!(find_top_via_line(msg).is_none());

        // "Vary:" must NOT match (we test "Via" prefix exact).
        let msg2 = b"INVITE sip:b@x SIP/2.0\r\n\
Vary: Accept\r\n\
\r\n";
        assert!(find_top_via_line(msg2).is_none());
    }

    #[test]
    fn tolerates_whitespace_between_name_and_colon() {
        // RFC 3261 §7.3.1 allows linear whitespace between header
        // name and ":".
        let msg = b"INVITE sip:b@x SIP/2.0\r\n\
Via  : SIP/2.0/UDP a:5060;branch=z9hG4bK1\r\n\
\r\n";
        let range = find_top_via_line(msg).expect("Via present");
        assert!(msg[range].starts_with(b"Via  : "));
    }
}

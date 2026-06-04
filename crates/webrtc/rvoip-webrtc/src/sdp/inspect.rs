//! Lightweight SDP inspection helpers backed by the shared sip-core SDP parser.

use rvoip_sip_core::sdp::parser::parse_attribute;
use rvoip_sip_core::types::sdp::{ParsedAttribute, SdpSession};
use std::str::FromStr;

/// Returns true when SDP contains an `m=` line for `kind` (`audio`, `video`, …).
pub fn sdp_has_media_line(sdp: &str, kind: &str) -> bool {
    if let Ok(session) = SdpSession::from_str(sdp) {
        return session
            .media_descriptions
            .iter()
            .any(|media| media.media.eq_ignore_ascii_case(kind));
    }

    sdp.lines().any(|line| {
        let line = line.trim();
        line.starts_with("m=") && line[2..].starts_with(kind)
    })
}

/// Returns true when SDP advertises simulcast / multi-SSRC video semantics.
pub fn sdp_indicates_simulcast(sdp: &str) -> bool {
    if let Ok(session) = SdpSession::from_str(sdp) {
        return session.media_descriptions.iter().any(|media| {
            media.generic_attributes.iter().any(|attr| {
                matches!(
                    attr,
                    ParsedAttribute::Rid(_)
                        | ParsedAttribute::Simulcast(_, _)
                        | ParsedAttribute::SimulcastStructured(_)
                ) || matches!(
                    attr,
                    ParsedAttribute::SsrcGroup(group)
                        if group.semantics.eq_ignore_ascii_case("FID")
                )
            })
        });
    }

    sdp.contains("simulcast") || sdp.contains("a=ssrc-group:FID") || sdp.contains("a=rid:")
}

/// Returns true when SDP advertises RFC 4733 telephone-event in any audio
/// m-section (case-insensitive rtpmap match). Used by D1 to decide whether
/// the answerer should attach a local PT 101 track in response to a remote
/// offer.
pub fn sdp_advertises_telephone_event(sdp: &str) -> bool {
    if let Ok(session) = SdpSession::from_str(sdp) {
        return session
            .media_descriptions
            .iter()
            .filter(|media| media.media.eq_ignore_ascii_case("audio"))
            .flat_map(|media| media.rtpmaps())
            .any(|rtpmap| rtpmap.encoding_name.eq_ignore_ascii_case("telephone-event"));
    }

    sdp.lines().any(|line| {
        let l = line.trim();
        l.starts_with("a=rtpmap:") && l.to_ascii_lowercase().contains("telephone-event")
    })
}

/// Returns true when ICE candidates are embedded in SDP (full gather, not trickle-only).
pub fn sdp_has_inline_ice_candidates(sdp: &str) -> bool {
    if let Ok(session) = SdpSession::from_str(sdp) {
        return session.media_descriptions.iter().any(|media| {
            media
                .generic_attributes
                .iter()
                .any(|attr| matches!(attr, ParsedAttribute::Candidate(_)))
        });
    }

    sdp.lines()
        .any(|line| line.trim().starts_with("a=candidate:"))
}

/// G12 — produce a privacy-friendly version of an SDP for logging.
///
/// Strips:
/// - `a=candidate:` IPs (replaces the 5th and 6th whitespace-separated
///   fields — the address and port — with `***`).
/// - `c=IN IP4 …` and `c=IN IP6 …` connection address.
/// - `a=ice-ufrag:…` and `a=ice-pwd:…` values.
/// - `o=…` origin username and address.
///
/// Keeps the SDP structurally valid (m-lines, mids, rtpmap, fmtp, etc.)
/// so logs are still useful for diagnosis.
pub fn redact_for_log(sdp: &str) -> String {
    let mut out = String::with_capacity(sdp.len());
    for line in sdp.lines() {
        let trimmed = line.trim_end();
        if let Some(rest) = trimmed.strip_prefix("a=candidate:") {
            let attr_line = format!("candidate:{rest}");
            let parsed_candidate = matches!(
                parse_attribute(&attr_line),
                Ok(ParsedAttribute::Candidate(_))
            );

            // candidate:<foundation> <component> <proto> <prio> <addr> <port> typ <type> ...
            let mut parts: Vec<&str> = rest.split_whitespace().collect();
            if (parsed_candidate || parts.len() >= 6) && parts.len() >= 6 {
                parts[4] = "***";
                parts[5] = "***";
            }
            out.push_str("a=candidate:");
            out.push_str(&parts.join(" "));
        } else if trimmed.starts_with("c=IN IP4 ") || trimmed.starts_with("c=IN IP6 ") {
            out.push_str(&trimmed[..9]);
            out.push_str("***");
        } else if let Some(rest) = trimmed.strip_prefix("a=ice-ufrag:") {
            let _ = parse_attribute(&format!("ice-ufrag:{rest}"));
            out.push_str("a=ice-ufrag:***");
        } else if let Some(rest) = trimmed.strip_prefix("a=ice-pwd:") {
            let _ = parse_attribute(&format!("ice-pwd:{rest}"));
            out.push_str("a=ice-pwd:***");
        } else if let Some(rest) = trimmed.strip_prefix("o=") {
            // o=<user> <sess-id> <sess-vers> <nettype> <addrtype> <addr>
            let mut parts: Vec<&str> = rest.split_whitespace().collect();
            if !parts.is_empty() {
                parts[0] = "***";
            }
            if parts.len() >= 6 {
                parts[5] = "***";
            }
            out.push_str("o=");
            out.push_str(&parts.join(" "));
        } else {
            out.push_str(trimmed);
        }
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_strips_candidate_addresses() {
        let sdp = "a=candidate:1 1 udp 2122260223 192.168.1.5 50001 typ host\r\n";
        let red = redact_for_log(sdp);
        assert!(!red.contains("192.168.1.5"));
        assert!(!red.contains("50001"));
        assert!(red.contains("typ host"), "type label must survive");
    }

    #[test]
    fn redact_strips_ice_credentials() {
        let sdp = "a=ice-ufrag:abcd\r\na=ice-pwd:0123456789abcdef\r\n";
        let red = redact_for_log(sdp);
        assert!(!red.contains("abcd"));
        assert!(!red.contains("0123456789abcdef"));
        assert!(red.contains("a=ice-ufrag:***"));
        assert!(red.contains("a=ice-pwd:***"));
    }

    #[test]
    fn redact_keeps_codec_lines() {
        let sdp = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\na=fmtp:111 useinbandfec=1\r\n";
        let red = redact_for_log(sdp);
        assert!(red.contains("a=rtpmap:111 opus/48000/2"));
        assert!(red.contains("a=fmtp:111 useinbandfec=1"));
    }

    #[test]
    fn redact_strips_origin_username_and_address() {
        let sdp = "o=mozilla...THIS_IS_SDPARTA-99.0 0 0 IN IP4 198.51.100.7\r\n";
        let red = redact_for_log(sdp);
        assert!(!red.contains("mozilla"));
        assert!(!red.contains("198.51.100.7"));
        assert!(red.contains("o=***"));
    }

    #[test]
    fn redact_strips_connection_address() {
        let sdp = "c=IN IP4 198.51.100.7\r\n";
        let red = redact_for_log(sdp);
        assert!(!red.contains("198.51.100.7"));
        assert!(red.contains("c=IN IP4 ***"));
    }
}

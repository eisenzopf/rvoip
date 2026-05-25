//! Lightweight SDP inspection helpers (no full parser).

/// Returns true when SDP contains an `m=` line for `kind` (`audio`, `video`, …).
pub fn sdp_has_media_line(sdp: &str, kind: &str) -> bool {
    sdp.lines().any(|line| {
        let line = line.trim();
        line.starts_with("m=") && line[2..].starts_with(kind)
    })
}

/// Returns true when SDP advertises simulcast / multi-SSRC video semantics.
pub fn sdp_indicates_simulcast(sdp: &str) -> bool {
    sdp.contains("simulcast")
        || sdp.contains("a=ssrc-group:FID")
        || sdp.contains("a=rid:")
}

/// Returns true when ICE candidates are embedded in SDP (full gather, not trickle-only).
pub fn sdp_has_inline_ice_candidates(sdp: &str) -> bool {
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
            // candidate:<foundation> <component> <proto> <prio> <addr> <port> typ <type> ...
            let mut parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 6 {
                parts[4] = "***";
                parts[5] = "***";
            }
            out.push_str("a=candidate:");
            out.push_str(&parts.join(" "));
        } else if trimmed.starts_with("c=IN IP4 ") || trimmed.starts_with("c=IN IP6 ") {
            out.push_str(&trimmed[..9]);
            out.push_str("***");
        } else if let Some(rest) = trimmed.strip_prefix("a=ice-ufrag:") {
            let _ = rest;
            out.push_str("a=ice-ufrag:***");
        } else if let Some(rest) = trimmed.strip_prefix("a=ice-pwd:") {
            let _ = rest;
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

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

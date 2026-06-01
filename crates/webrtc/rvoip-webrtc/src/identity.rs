//! DTLS-SRTP fingerprint extraction for identity binding (H7.4).
//!
//! Reads `a=fingerprint:<algo> <hex-digest>` lines from a stored SDP and
//! exposes them as a typed struct. The rvoip-core [`IdentityAssurance`]
//! enum doesn't yet have a `DtlsFingerprint` variant, so the adapter surfaces
//! these via [`crate::adapter::WebRtcAdapter::remote_dtls_fingerprint`] for
//! callers that want to pin / verify the peer cert out-of-band.
//!
//! When `rvoip-core` gains a `DtlsFingerprint` variant on `IdentityAssurance`,
//! `verify_request_signature` can return it directly — for now it stays
//! `Anonymous`.

use serde::{Deserialize, Serialize};

/// One `a=fingerprint:` line extracted from an SDP.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DtlsFingerprint {
    /// Algorithm name (e.g. `sha-256`, `sha-384`, `sha-512`).
    pub algorithm: String,
    /// Lowercase hex with `:` separators, as it appears in SDP.
    pub value: String,
}

impl DtlsFingerprint {
    /// Try to parse one `a=fingerprint:<algo> <value>` line.
    pub fn parse_line(line: &str) -> Option<Self> {
        let rest = line.trim().strip_prefix("a=fingerprint:")?;
        let mut parts = rest.split_ascii_whitespace();
        let algorithm = parts.next()?.to_ascii_lowercase();
        let value = parts.next()?.to_ascii_lowercase();
        Some(Self { algorithm, value })
    }
}

/// Extract every `a=fingerprint:` line from an SDP body. Returns them in the
/// order they appear (session level first, then per-media).
pub fn extract_fingerprints(sdp: &str) -> Vec<DtlsFingerprint> {
    sdp.lines()
        .filter_map(DtlsFingerprint::parse_line)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_SDP: &str = "v=0\r\n\
o=- 0 0 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
a=fingerprint:sha-256 13:14:DD:9E:5F:91:00:46:11:50:6C:90:8B:9E:AA:F2:14:31:F3:18:C9:00:48:6F:1D:34:33:36:8B:DE:F0:23\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
a=fingerprint:sha-256 13:14:DD:9E:5F:91:00:46:11:50:6C:90:8B:9E:AA:F2:14:31:F3:18:C9:00:48:6F:1D:34:33:36:8B:DE:F0:23\r\n\
a=rtpmap:111 opus/48000/2\r\n";

    #[test]
    fn parse_single_line() {
        let fp = DtlsFingerprint::parse_line(
            "a=fingerprint:sha-256 13:14:DD:9E:5F:91:00:46:11:50:6C:90:8B:9E",
        )
        .expect("parses");
        assert_eq!(fp.algorithm, "sha-256");
        assert_eq!(fp.value, "13:14:dd:9e:5f:91:00:46:11:50:6c:90:8b:9e");
    }

    #[test]
    fn parse_line_rejects_non_fingerprint() {
        assert!(DtlsFingerprint::parse_line("a=mid:0").is_none());
        assert!(DtlsFingerprint::parse_line("a=fingerprint:incomplete").is_none());
        assert!(DtlsFingerprint::parse_line("").is_none());
    }

    #[test]
    fn extract_all_returns_session_then_media() {
        let fps = extract_fingerprints(SAMPLE_SDP);
        assert_eq!(fps.len(), 2, "session + audio m-section");
        assert!(fps.iter().all(|f| f.algorithm == "sha-256"));
    }

    #[test]
    fn extract_empty_when_no_fingerprint() {
        assert!(extract_fingerprints("v=0\r\no=- 0 0 IN IP4 1.1.1.1\r\n").is_empty());
    }
}

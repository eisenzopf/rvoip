//! Authoritative compatibility descriptor for this UCTP implementation.
//!
//! Keep wire-version and substrate identifiers here so diagnostics and
//! adapters do not infer compatibility from the crate's semver release.
//! Crate semver describes the Rust API; the envelope and datagram version
//! bytes describe the wire format.

use serde::Serialize;

/// Crate release that produced this build.
pub const UCTP_CRATE_RELEASE: &str = env!("CARGO_PKG_VERSION");

/// Envelope versions accepted and emitted by this implementation.
pub const UCTP_ENVELOPE_VERSIONS: &[u8] = &[1];

/// Media-datagram header versions accepted and emitted by this implementation.
pub const UCTP_DATAGRAM_VERSIONS: &[u8] = &[1];

/// Current envelope version emitted by [`crate::UctpEnvelope::new`].
pub const UCTP_ENVELOPE_VERSION: u8 = 1;

/// Current version byte in the eight-byte UCTP media-datagram header.
pub const UCTP_DATAGRAM_VERSION: u8 = 1;

/// ALPN used by native UCTP over QUIC.
pub const UCTP_RAW_QUIC_ALPN: &str = "uctp/1";

/// Byte form of [`UCTP_RAW_QUIC_ALPN`] for rustls and quinn configuration.
pub const UCTP_RAW_QUIC_ALPN_BYTES: &[u8] = b"uctp/1";

/// ALPN used by HTTP/3, the substrate for UCTP over WebTransport.
pub const UCTP_WEBTRANSPORT_ALPN: &str = "h3";

/// Byte form of [`UCTP_WEBTRANSPORT_ALPN`] for rustls and quinn configuration.
pub const UCTP_WEBTRANSPORT_ALPN_BYTES: &[u8] = b"h3";

/// Media profile carried after the UCTP datagram header.
pub const UCTP_RTP_DATAGRAM_PROFILE: &str = "rtp-datagram/1";

/// Machine-readable compatibility facts for APIs and diagnostics.
///
/// `crate_release` is intentionally separate from the wire versions. A crate
/// release can change without changing either wire format, and a breaking wire
/// revision requires its own version change even if the Rust API does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct UctpCompatibility {
    pub crate_release: &'static str,
    pub envelope_versions: &'static [u8],
    pub datagram_versions: &'static [u8],
    pub raw_quic_alpn: &'static str,
    pub webtransport_alpn: &'static str,
    pub media_profile: &'static str,
}

impl UctpCompatibility {
    /// Return whether this build can decode the supplied envelope version.
    pub fn supports_envelope(self, version: u8) -> bool {
        self.envelope_versions.contains(&version)
    }

    /// Return whether this build can decode the supplied datagram version.
    pub fn supports_datagram(self, version: u8) -> bool {
        self.datagram_versions.contains(&version)
    }
}

/// Compatibility facts for the current build.
pub const UCTP_COMPATIBILITY: UctpCompatibility = UctpCompatibility {
    crate_release: UCTP_CRATE_RELEASE,
    envelope_versions: UCTP_ENVELOPE_VERSIONS,
    datagram_versions: UCTP_DATAGRAM_VERSIONS,
    raw_quic_alpn: UCTP_RAW_QUIC_ALPN,
    webtransport_alpn: UCTP_WEBTRANSPORT_ALPN,
    media_profile: UCTP_RTP_DATAGRAM_PROFILE,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_descriptor_is_explicit_and_serializable() {
        assert!(UCTP_COMPATIBILITY.supports_envelope(UCTP_ENVELOPE_VERSION));
        assert!(UCTP_COMPATIBILITY.supports_datagram(UCTP_DATAGRAM_VERSION));
        assert!(!UCTP_COMPATIBILITY.supports_envelope(0));
        assert!(!UCTP_COMPATIBILITY.supports_datagram(2));

        let value = serde_json::to_value(UCTP_COMPATIBILITY).unwrap();
        assert_eq!(value["crate_release"], UCTP_CRATE_RELEASE);
        assert_eq!(value["envelope_versions"], serde_json::json!([1]));
        assert_eq!(value["datagram_versions"], serde_json::json!([1]));
        assert_eq!(value["raw_quic_alpn"], "uctp/1");
        assert_eq!(value["webtransport_alpn"], "h3");
        assert_eq!(value["media_profile"], "rtp-datagram/1");
    }
}

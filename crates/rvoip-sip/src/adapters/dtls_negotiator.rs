//! NEXT_STEPS C3 — DTLS-SRTP SDP-level negotiation (RFC 5763 / 8842).
//!
//! This module is the SDP-side scaffold for DTLS-SRTP. It handles:
//!
//! 1. Detecting a DTLS-SRTP offer (presence of `a=fingerprint:` and
//!    `a=setup:` on an audio m-line with `RTP/SAVP` proto).
//! 2. Computing the complementary `a=setup:` role per RFC 8842 §5.1
//!    so the answer carries the matching active/passive value.
//! 3. Selecting our advertised fingerprint hash.
//!
//! What this module does **not** do yet (separate workstream in
//! NEXT_STEPS Area C3.2 / C3.3):
//!
//! - Drive the actual DTLS handshake — that lives in
//!   `rtp-core::dtls` and needs an integration call site in
//!   `MediaAdapter::negotiate_sdp_as_uas`.
//! - Extract SRTP master keys from the DTLS extractor and feed them
//!   into rtp-core's `SrtpContext` (the existing SDES key-derivation
//!   path is at `media-core::relay::controller::install_srtp_contexts`).
//! - Manage long-lived certificate / private-key state (today the
//!   SDES path generates a random master key per session; DTLS needs
//!   a stable cert so peers can pin our fingerprint across calls).
//!
//! The split is intentional: this module lets us write byte-fixture
//! tests for the SDP shape today while the rtp-core DTLS engine is
//! brought online behind it.

use crate::errors::{Result, SessionError};
use rvoip_sip_core::types::sdp::{ParsedAttribute, SdpSession};

/// DTLS setup role per RFC 4145 / RFC 8842 §5.1. We use a string-typed
/// representation because the SDP parser already hands us
/// `ParsedAttribute::Setup(String)`; this is a thin wrapper that
/// enforces the four legal values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupRole {
    /// Endpoint initiates the DTLS handshake (TLS client role).
    Active,
    /// Endpoint waits for the peer to initiate (TLS server role).
    Passive,
    /// Endpoint accepts either role; the peer decides.
    Actpass,
    /// Existing DTLS association is being kept; no new handshake.
    Holdconn,
}

impl SetupRole {
    /// Parse the role from the lower-cased `a=setup:` value.
    /// Trailing whitespace and case variations are tolerated; unknown
    /// values surface as `Err(SDPNegotiationFailed)` so the state
    /// machine can route them to a 488.
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_lowercase().as_str() {
            "active" => Ok(SetupRole::Active),
            "passive" => Ok(SetupRole::Passive),
            "actpass" => Ok(SetupRole::Actpass),
            "holdconn" => Ok(SetupRole::Holdconn),
            other => Err(SessionError::SDPNegotiationFailed(format!(
                "Unknown a=setup role: {}",
                other
            ))),
        }
    }

    /// Lower-cased wire form.
    pub fn as_str(self) -> &'static str {
        match self {
            SetupRole::Active => "active",
            SetupRole::Passive => "passive",
            SetupRole::Actpass => "actpass",
            SetupRole::Holdconn => "holdconn",
        }
    }

    /// RFC 8842 §5.1 — the complementary role we MUST pick in the
    /// answer given the peer's offer.
    ///
    /// | Offer role | Answer role |
    /// |---|---|
    /// | `actpass` | `active` (we initiate) |
    /// | `active`  | `passive` (peer initiates) |
    /// | `passive` | `active`  (we initiate) |
    /// | `holdconn`| `holdconn` (re-use existing) |
    pub fn complementary(self) -> Self {
        match self {
            SetupRole::Actpass => SetupRole::Active,
            SetupRole::Active => SetupRole::Passive,
            SetupRole::Passive => SetupRole::Active,
            SetupRole::Holdconn => SetupRole::Holdconn,
        }
    }
}

/// What we extracted from a DTLS-SRTP offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DtlsOffer {
    /// Hash function (e.g. `"sha-256"`).
    pub hash_function: String,
    /// Colon-separated hex fingerprint.
    pub fingerprint: String,
    /// Peer's chosen setup role.
    pub setup_role: SetupRole,
}

/// Inspect the parsed offer's audio m-line for DTLS-SRTP attributes.
/// Returns `Some` only when *both* `a=fingerprint` AND `a=setup` are
/// present on the audio m-line. RFC 8842 §5.1 requires both.
pub fn detect_dtls_offer(sdp: &SdpSession) -> Option<DtlsOffer> {
    let audio = sdp.media_descriptions.iter().find(|m| m.media == "audio")?;

    let mut fingerprint = None;
    let mut setup = None;
    for attr in audio
        .generic_attributes
        .iter()
        .chain(sdp.generic_attributes.iter())
    {
        match attr {
            ParsedAttribute::Fingerprint(hash, fp) if fingerprint.is_none() => {
                fingerprint = Some((hash.clone(), fp.clone()));
            }
            ParsedAttribute::Setup(role) if setup.is_none() => {
                setup = SetupRole::parse(role).ok();
            }
            _ => {}
        }
    }

    match (fingerprint, setup) {
        (Some((hash_function, fp)), Some(setup_role)) => Some(DtlsOffer {
            hash_function,
            fingerprint: fp,
            setup_role,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::sdp::SdpBuilder;
    use std::str::FromStr;

    fn dtls_audio_offer(hash: &str, fp: &str, setup: &str) -> SdpSession {
        let sdp = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16000, "RTP/SAVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .attribute("fingerprint", Some(format!("{} {}", hash, fp)))
            .attribute("setup", Some(setup.to_string()))
            .attribute("sendrecv", None::<String>)
            .done()
            .build()
            .expect("offer builds")
            .to_string();
        SdpSession::from_str(&sdp).expect("offer parses")
    }

    #[test]
    fn rfc_8842_complementary_role_matrix() {
        assert_eq!(SetupRole::Actpass.complementary(), SetupRole::Active);
        assert_eq!(SetupRole::Active.complementary(), SetupRole::Passive);
        assert_eq!(SetupRole::Passive.complementary(), SetupRole::Active);
        assert_eq!(SetupRole::Holdconn.complementary(), SetupRole::Holdconn);
    }

    #[test]
    fn parse_round_trip_preserves_role() {
        for role in ["active", "passive", "actpass", "holdconn"] {
            let parsed = SetupRole::parse(role).expect("legal role parses");
            assert_eq!(parsed.as_str(), role);
        }
    }

    #[test]
    fn parse_rejects_unknown_role() {
        assert!(SetupRole::parse("random-garbage").is_err());
        assert!(SetupRole::parse("").is_err());
    }

    #[test]
    fn parse_is_case_insensitive_and_trims_whitespace() {
        assert_eq!(
            SetupRole::parse("  ACTPASS  ").expect("trim+casefold"),
            SetupRole::Actpass
        );
    }

    #[test]
    fn detect_dtls_offer_returns_some_when_both_attributes_present() {
        let offer = dtls_audio_offer(
            "sha-256",
            "AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89",
            "actpass",
        );
        let detected = detect_dtls_offer(&offer).expect("detect");
        assert_eq!(detected.hash_function, "sha-256");
        assert!(detected.fingerprint.starts_with("AB:CD:EF"));
        assert_eq!(detected.setup_role, SetupRole::Actpass);
    }

    #[test]
    fn detect_dtls_offer_returns_none_for_plain_rtp_avp() {
        let plain = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16000, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .attribute("sendrecv", None::<String>)
            .done()
            .build()
            .expect("plain offer builds")
            .to_string();
        let parsed = SdpSession::from_str(&plain).expect("offer parses");
        assert!(detect_dtls_offer(&parsed).is_none());
    }

    #[test]
    fn detect_dtls_offer_returns_none_when_only_fingerprint_present() {
        // RFC 8842 §5.1 — fingerprint without setup is malformed for
        // DTLS-SRTP. Return None so the caller falls back to plain or
        // rejects via the higher-level policy.
        let sdp = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16000, "RTP/SAVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .attribute("fingerprint", Some("sha-256 AB:CD:EF"))
            .attribute("sendrecv", None::<String>)
            .done()
            .build()
            .expect("partial offer builds")
            .to_string();
        let parsed = SdpSession::from_str(&sdp).expect("offer parses");
        assert!(detect_dtls_offer(&parsed).is_none());
    }

    #[test]
    fn unknown_setup_value_short_circuits_detection() {
        // A peer offering `a=setup:bogus` is invalid per RFC 8842;
        // we treat that exactly like a missing setup line and return
        // None so the higher-level negotiator's plain/strict fallback
        // applies.
        let sdp = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16000, "RTP/SAVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .attribute("fingerprint", Some("sha-256 AB:CD:EF"))
            .attribute("setup", Some("bogus"))
            .attribute("sendrecv", None::<String>)
            .done()
            .build()
            .expect("offer with bogus setup builds")
            .to_string();
        let parsed = SdpSession::from_str(&sdp).expect("offer parses");
        assert!(detect_dtls_offer(&parsed).is_none());
    }
}

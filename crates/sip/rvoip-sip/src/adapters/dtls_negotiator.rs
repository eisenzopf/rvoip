//! NEXT_STEPS C3 — DTLS-SRTP SDP-level negotiation (RFC 5763 / 8842).
//!
//! This module is the SDP-side scaffold for DTLS-SRTP. It handles:
//!
//! 1. Detecting a DTLS-SRTP offer (presence of `a=fingerprint:` and
//!    `a=setup:` on an audio m-line with `UDP/TLS/RTP/SAVP` proto).
//! 2. Computing the complementary `a=setup:` role per RFC 8842 §5.1
//!    so the answer carries the matching active/passive value.
//! 3. Selecting our advertised fingerprint hash.
//!
//! The media adapter owns the matching per-call certificate identity and
//! hands the validated fingerprint to media-core before context installation.

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
    /// Parsed SHA-256 fingerprint used for the pre-install certificate check.
    pub fingerprint_sha256: [u8; 32],
    /// Peer's chosen setup role.
    pub setup_role: SetupRole,
}

fn parse_sha256_fingerprint(value: &str) -> Result<[u8; 32]> {
    let parts: Vec<&str> = value.split(':').collect();
    if parts.len() != 32 || parts.iter().any(|part| part.len() != 2) {
        return Err(SessionError::SDPNegotiationFailed(
            "DTLS-SRTP fingerprint must contain exactly 32 colon-separated SHA-256 bytes"
                .to_string(),
        ));
    }
    let mut out = [0_u8; 32];
    for (index, part) in parts.into_iter().enumerate() {
        out[index] = u8::from_str_radix(part, 16).map_err(|_| {
            SessionError::SDPNegotiationFailed(
                "DTLS-SRTP fingerprint contains non-hexadecimal bytes".to_string(),
            )
        })?;
    }
    Ok(out)
}

/// Parse and validate the DTLS-SRTP attributes for the audio media section.
/// Partial, malformed, unsupported-hash, and wrong-transport offers are
/// explicit negotiation failures rather than plaintext fallbacks.
pub fn parse_dtls_offer(sdp: &SdpSession) -> Result<Option<DtlsOffer>> {
    let Some(audio) = sdp
        .media_descriptions
        .iter()
        .find(|media| media.media.eq_ignore_ascii_case("audio"))
    else {
        return Ok(None);
    };

    let parse_scope = |attributes: &[ParsedAttribute], scope: &str| {
        let mut fingerprint = None;
        let mut setup = None;
        for attr in attributes {
            match attr {
                ParsedAttribute::Fingerprint(hash, value) => {
                    if fingerprint.is_some() {
                        return Err(SessionError::SDPNegotiationFailed(format!(
                            "DTLS-SRTP {scope} scope contains multiple a=fingerprint attributes"
                        )));
                    }
                    fingerprint = Some((hash.clone(), value.clone()));
                }
                ParsedAttribute::Setup(value) => {
                    if setup.is_some() {
                        return Err(SessionError::SDPNegotiationFailed(format!(
                            "DTLS-SRTP {scope} scope contains multiple a=setup attributes"
                        )));
                    }
                    setup = Some(value.clone());
                }
                _ => {}
            }
        }
        Ok((fingerprint, setup))
    };
    let (media_fingerprint, media_setup) = parse_scope(&audio.generic_attributes, "media")?;
    let (session_fingerprint, session_setup) = parse_scope(&sdp.generic_attributes, "session")?;
    // RFC 8122/8842 permit session defaults and media-level overrides.
    let fingerprint = media_fingerprint.or(session_fingerprint);
    let setup = media_setup.or(session_setup);

    if fingerprint.is_none() && setup.is_none() {
        return Ok(None);
    }
    if !audio.protocol.eq_ignore_ascii_case("UDP/TLS/RTP/SAVP") {
        return Err(SessionError::SDPNegotiationFailed(
            "DTLS-SRTP attributes require UDP/TLS/RTP/SAVP".to_string(),
        ));
    }
    let (hash_function, fingerprint) = fingerprint.ok_or_else(|| {
        SessionError::SDPNegotiationFailed("DTLS-SRTP offer is missing a=fingerprint".to_string())
    })?;
    if !hash_function.eq_ignore_ascii_case("sha-256") {
        return Err(SessionError::SDPNegotiationFailed(
            "DTLS-SRTP currently requires a SHA-256 certificate fingerprint".to_string(),
        ));
    }
    let setup_role = SetupRole::parse(&setup.ok_or_else(|| {
        SessionError::SDPNegotiationFailed("DTLS-SRTP offer is missing a=setup".to_string())
    })?)?;
    if setup_role == SetupRole::Holdconn {
        return Err(SessionError::SDPNegotiationFailed(
            "a=setup:holdconn is not valid for a new DTLS association".to_string(),
        ));
    }
    let fingerprint_sha256 = parse_sha256_fingerprint(&fingerprint)?;
    Ok(Some(DtlsOffer {
        hash_function,
        fingerprint,
        fingerprint_sha256,
        setup_role,
    }))
}

/// Inspect the parsed offer's audio m-line for DTLS-SRTP attributes.
/// Returns `Some` only when *both* `a=fingerprint` AND `a=setup` are
/// present on the audio m-line. RFC 8842 §5.1 requires both.
pub fn detect_dtls_offer(sdp: &SdpSession) -> Option<DtlsOffer> {
    parse_dtls_offer(sdp).ok().flatten()
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
            .media_audio(16000, "UDP/TLS/RTP/SAVP")
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
        assert_eq!(detected.fingerprint_sha256[0..3], [0xAB, 0xCD, 0xEF]);
        assert_eq!(detected.setup_role, SetupRole::Actpass);
    }

    #[test]
    fn parse_rejects_ambiguous_duplicate_fingerprint_and_setup_attributes() {
        let fingerprint = "AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89";
        let mut duplicate_fingerprint = dtls_audio_offer("sha-256", fingerprint, "actpass");
        duplicate_fingerprint.media_descriptions[0]
            .generic_attributes
            .push(ParsedAttribute::Fingerprint(
                "sha-256".to_string(),
                fingerprint.to_string(),
            ));
        assert!(parse_dtls_offer(&duplicate_fingerprint).is_err());

        let mut duplicate_setup = dtls_audio_offer("sha-256", fingerprint, "actpass");
        duplicate_setup.media_descriptions[0]
            .generic_attributes
            .push(ParsedAttribute::Setup("passive".to_string()));
        assert!(parse_dtls_offer(&duplicate_setup).is_err());
    }

    #[test]
    fn media_level_dtls_attributes_override_session_defaults() {
        let media_fingerprint = "AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89";
        let session_fingerprint = "10:11:12:13:14:15:16:17:18:19:1A:1B:1C:1D:1E:1F:20:21:22:23:24:25:26:27:28:29:2A:2B:2C:2D:2E:2F";
        let mut offer = dtls_audio_offer("sha-256", media_fingerprint, "actpass");
        offer.generic_attributes.push(ParsedAttribute::Fingerprint(
            "sha-256".to_string(),
            session_fingerprint.to_string(),
        ));
        offer
            .generic_attributes
            .push(ParsedAttribute::Setup("passive".to_string()));

        let parsed = parse_dtls_offer(&offer)
            .expect("session defaults plus media override are valid")
            .expect("DTLS attributes detected");
        assert_eq!(parsed.fingerprint, media_fingerprint);
        assert_eq!(parsed.setup_role, SetupRole::Actpass);
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

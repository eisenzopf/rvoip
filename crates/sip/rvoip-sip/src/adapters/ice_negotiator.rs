//! ICE (RFC 8445) SDP-level negotiation.
//!
//! This module's job stops at extracting `a=ice-ufrag`/`a=ice-pwd`/
//! `a=candidate` from a parsed offer or answer into
//! `rvoip-nat-core`'s SDP-agnostic [`rvoip_nat_core::IceCandidate`] —
//! same split `dtls_negotiator::detect_dtls_offer` uses for
//! `a=fingerprint`/`a=setup`. Driving the actual `IceAgent` lives in
//! `MediaAdapter`.

use rvoip_nat_core::{CandidateKind, IceCandidate};
use rvoip_sip_core::types::sdp::{CandidateAttribute, ParsedAttribute, SdpSession};

/// What we extracted from an SDP offer or answer's ICE attributes.
#[derive(Debug, Clone, PartialEq)]
pub struct IceOffer {
    pub ufrag: String,
    pub pwd: String,
    pub candidates: Vec<IceCandidate>,
}

/// Convert a parsed `a=candidate:` line into `rvoip-nat-core`'s
/// SDP-agnostic representation. Returns `None` for a candidate whose
/// `connection_address` doesn't parse as an `IpAddr` (an FQDN/mDNS
/// candidate) — hostname candidates are out of scope for this pass —
/// or whose `candidate_type` isn't one of the four RFC 8839 §5.1.1
/// values.
fn candidate_attribute_to_ice_candidate(a: &CandidateAttribute) -> Option<IceCandidate> {
    Some(IceCandidate {
        foundation: a.foundation.clone(),
        component: a.component_id,
        transport: a.transport.clone(),
        priority: a.priority,
        address: a.connection_address.parse().ok()?,
        port: a.port,
        kind: CandidateKind::parse(&a.candidate_type)?,
        related_address: a.related_address.as_ref().and_then(|s| s.parse().ok()),
        related_port: a.related_port,
    })
}

/// Inspect a parsed offer or answer's audio m-line for ICE attributes.
/// Returns `Some` only when both `a=ice-ufrag` AND `a=ice-pwd` are
/// present (RFC 8445 §5.1 requires both); candidates may be empty (a
/// peer can advertise credentials before/without any candidates in
/// degenerate cases, though a real agent always has at least one host
/// candidate).
pub fn detect_ice_offer(sdp: &SdpSession) -> Option<IceOffer> {
    let audio = sdp.media_descriptions.iter().find(|m| m.media == "audio")?;

    let mut ufrag = None;
    let mut pwd = None;
    for attr in audio
        .generic_attributes
        .iter()
        .chain(sdp.generic_attributes.iter())
    {
        match attr {
            ParsedAttribute::IceUfrag(u) if ufrag.is_none() => ufrag = Some(u.clone()),
            ParsedAttribute::IcePwd(p) if pwd.is_none() => pwd = Some(p.clone()),
            _ => {}
        }
    }

    let candidates = audio
        .candidates()
        .filter_map(candidate_attribute_to_ice_candidate)
        .collect();

    match (ufrag, pwd) {
        (Some(ufrag), Some(pwd)) => Some(IceOffer {
            ufrag,
            pwd,
            candidates,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::sdp::SdpBuilder;
    use std::str::FromStr;

    fn ice_audio_offer(ufrag: &str, pwd: &str, candidate_lines: &[&str]) -> SdpSession {
        let mut media_builder = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16000, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .ice_ufrag(ufrag)
            .ice_pwd(pwd);
        for line in candidate_lines {
            media_builder = media_builder.ice_candidate(*line);
        }
        let sdp = media_builder
            .attribute("sendrecv", None::<String>)
            .done()
            .build()
            .expect("offer builds")
            .to_string();
        SdpSession::from_str(&sdp).expect("offer parses")
    }

    #[test]
    fn detect_ice_offer_returns_some_with_ufrag_pwd_and_candidates() {
        let offer = ice_audio_offer(
            "F7gI",
            "x9cml/YzichV2+XlhiMu8g",
            &["1 1 udp 2130706431 192.168.1.5 5000 typ host"],
        );
        let detected = detect_ice_offer(&offer).expect("detect");
        assert_eq!(detected.ufrag, "F7gI");
        assert_eq!(detected.pwd, "x9cml/YzichV2+XlhiMu8g");
        assert_eq!(detected.candidates.len(), 1);
        assert_eq!(detected.candidates[0].kind, CandidateKind::Host);
        assert_eq!(
            detected.candidates[0].address,
            "192.168.1.5".parse::<std::net::IpAddr>().unwrap()
        );
        assert_eq!(detected.candidates[0].port, 5000);
    }

    #[test]
    fn detect_ice_offer_returns_none_without_ice_attributes() {
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
        assert!(detect_ice_offer(&parsed).is_none());
    }

    #[test]
    fn detect_ice_offer_returns_none_when_only_ufrag_present() {
        // RFC 8445 §5.1 — ufrag without pwd is malformed.
        let sdp = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16000, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .ice_ufrag("F7gI")
            .attribute("sendrecv", None::<String>)
            .done()
            .build()
            .expect("partial offer builds")
            .to_string();
        let parsed = SdpSession::from_str(&sdp).expect("offer parses");
        assert!(detect_ice_offer(&parsed).is_none());
    }

    #[test]
    fn detect_ice_offer_skips_a_hostname_form_candidate() {
        let offer = ice_audio_offer(
            "F7gI",
            "x9cml/YzichV2+XlhiMu8g",
            &["1 1 udp 2130706431 host.example.invalid 5000 typ host"],
        );
        let detected = detect_ice_offer(&offer).expect("detect");
        assert!(detected.candidates.is_empty());
    }

    #[test]
    fn detect_ice_offer_parses_srflx_candidate_with_raddr_rport() {
        let offer = ice_audio_offer(
            "F7gI",
            "x9cml/YzichV2+XlhiMu8g",
            &["2 1 udp 1694498815 203.0.113.1 6000 typ srflx raddr 192.168.1.5 rport 5000"],
        );
        let detected = detect_ice_offer(&offer).expect("detect");
        assert_eq!(detected.candidates.len(), 1);
        let c = &detected.candidates[0];
        assert_eq!(c.kind, CandidateKind::ServerReflexive);
        assert_eq!(
            c.related_address,
            Some("192.168.1.5".parse::<std::net::IpAddr>().unwrap())
        );
        assert_eq!(c.related_port, Some(5000));
    }
}

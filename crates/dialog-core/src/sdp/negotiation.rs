//! RFC 3264 SDP offer/answer matcher.
//!
//! ## Purpose
//!
//! Take a parsed SDP offer plus our local [`AnswerCapabilities`] and
//! produce per-`m=`-line accept/reject decisions plus the negotiated
//! payload-type set in the offerer's preference order.
//!
//! ## Why this lives in `dialog-core`
//!
//! Sprint 3 lifts the matching logic out of `session-core`'s
//! audio-only / SDES-aware path so future Sprint 4 work (DTLS-SRTP
//! `a=fingerprint`/`a=setup`, ICE `a=ice-ufrag`/`a=candidate`, video
//! `m=video`) can extend a single generic implementation rather than
//! splaying a third hardcoded SDP shape across the stack.
//!
//! ## Sprint 3 scope cut
//!
//! This module ships the generic matcher with comprehensive tests.
//! Swapping `session-core::adapters::media_adapter::negotiate_sdp_as_uas`
//! over to call into it is deferred — that path is SRTP-aware and the
//! conversion is non-trivial without re-doing the SRTP negotiation
//! plumbing. The matcher is ready for Sprint 4 D2/D3/D5 to consume,
//! and a follow-up can swap the existing answer path over once the
//! integration's been validated against carrier interop tests.
//!
//! ## RFC 3264 §6 — what the matcher does
//!
//! For each `m=` line in the offer:
//!
//! - Intersect the offered formats with `caps.supported_formats`,
//!   preserving the offerer's preference order (RFC 3264 §6 paragraph
//!   3: "the order of media formats … in the answer is significant
//!   only as a hint to the offerer").
//! - If the intersection is empty: emit a rejected media line
//!   (`accept = false`); the SDP serializer must render it as
//!   `m=<media> 0 <protocol> …` (RFC 3264 §6 paragraph 4).
//! - Carry over `rtpmap` / `fmtp` for surviving formats only — drop
//!   metadata for formats we removed.
//! - Carry over comprehension-optional attributes (crypto, fingerprint,
//!   setup, ice-ufrag, ice-pwd, candidate, mid, ssrc, …) verbatim.
//!   These belong to the consumer to inspect/process; the matcher's
//!   job is only the format intersection.
//!
//! ## SRTP policy hooks
//!
//! Two flags on [`AnswerCapabilities`] enforce session-core's RFC 4568
//! SRTP policy without the matcher growing crypto knowledge:
//!
//! - `accept_srtp = false` rejects any `RTP/SAVP` offer (port 0).
//! - `require_srtp = true` rejects any plain `RTP/AVP` offer (port 0).
//!
//! The actual `a=crypto:` parsing/answering stays in session-core.

use rvoip_sip_core::types::sdp::{
    MediaDescription, ParsedAttribute, RtpMapAttribute, SdpSession,
};

/// Local capabilities that drive the matcher's accept/reject
/// decisions. Populated by the caller (typically session-core's
/// media adapter) before calling [`match_offer`].
#[derive(Debug, Clone)]
pub struct AnswerCapabilities {
    /// Payload-type strings we support, in *our* preference order.
    /// The matcher uses this only as a filter — the answer's order
    /// follows the offerer's preference per RFC 3264 §6.
    pub supported_formats: Vec<String>,

    /// When `false`, an `RTP/SAVP` offer is rejected outright.
    /// Default behaviour: `true` (accept SRTP if the consumer can
    /// negotiate keys; the matcher leaves crypto-line handling to
    /// the consumer).
    pub accept_srtp: bool,

    /// When `true`, a plain `RTP/AVP` offer is rejected (port 0)
    /// because our policy requires SRTP. Mirrors RFC 3261 `Require:`
    /// semantics — fail loudly rather than silently downgrade.
    pub require_srtp: bool,
}

impl Default for AnswerCapabilities {
    fn default() -> Self {
        Self {
            supported_formats: Vec::new(),
            accept_srtp: true,
            require_srtp: false,
        }
    }
}

/// Result of matching an offer against [`AnswerCapabilities`].
///
/// One [`MediaLineMatch`] per `m=` line in the offer, in the same
/// order. Consumers walk this and either build a positive answer
/// (port = chosen RTP port, formats = `negotiated_formats`) or
/// emit a port-0 rejection line per RFC 3264 §6.
#[derive(Debug, Clone)]
pub struct OfferAnswerMatch {
    pub media_lines: Vec<MediaLineMatch>,
}

/// Per-`m=`-line matching outcome.
#[derive(Debug, Clone)]
pub struct MediaLineMatch {
    /// `false` → emit as `m=<media> 0 <protocol> …` to reject this
    /// stream per RFC 3264 §6 paragraph 4.
    pub accepted: bool,

    /// The original offer's media kind (`"audio"`, `"video"`, …) and
    /// transport profile (`"RTP/AVP"`, `"RTP/SAVP"`, …) verbatim.
    pub media: String,
    pub protocol: String,

    /// Surviving formats in the offerer's preference order. Empty
    /// when `accepted == false` (the consumer should still emit some
    /// placeholder format per RFC 3264 §6 to keep the m= line
    /// well-formed; the matcher's contract is to report intent).
    pub negotiated_formats: Vec<String>,

    /// `rtpmap` / `fmtp` lines for the surviving formats, plus
    /// comprehension-optional attributes (crypto, fingerprint, setup,
    /// ice-*, candidate, mid, ssrc, …) carried over verbatim from
    /// the offer.
    pub carry_over_attrs: Vec<ParsedAttribute>,

    /// Offer's media-line direction (sendrecv / sendonly / recvonly /
    /// inactive). Consumer is responsible for emitting the dual per
    /// RFC 3264 §6.1 (e.g. answer `recvonly` to a `sendonly` offer).
    pub direction: Option<rvoip_sip_core::MediaDirection>,
}

/// Errors the matcher can return. Today only one — kept as an enum
/// for forward compatibility.
#[derive(Debug, thiserror::Error)]
pub enum MatchError {
    #[error("RFC 3264 matcher: offer carries no media lines")]
    EmptyOffer,
}

/// Match an SDP offer against our local [`AnswerCapabilities`] per
/// RFC 3264 §6.
pub fn match_offer(
    offer: &SdpSession,
    caps: &AnswerCapabilities,
) -> Result<OfferAnswerMatch, MatchError> {
    if offer.media_descriptions.is_empty() {
        return Err(MatchError::EmptyOffer);
    }

    let mut lines = Vec::with_capacity(offer.media_descriptions.len());
    for m in &offer.media_descriptions {
        lines.push(match_one_media(m, caps));
    }
    Ok(OfferAnswerMatch { media_lines: lines })
}

fn match_one_media(m: &MediaDescription, caps: &AnswerCapabilities) -> MediaLineMatch {
    // Policy gates first — these short-circuit the format intersection.
    let is_srtp = m.protocol.contains("SAVP");
    if is_srtp && !caps.accept_srtp {
        return MediaLineMatch {
            accepted: false,
            media: m.media.clone(),
            protocol: m.protocol.clone(),
            negotiated_formats: Vec::new(),
            carry_over_attrs: Vec::new(),
            direction: m.direction,
        };
    }
    if !is_srtp && caps.require_srtp {
        return MediaLineMatch {
            accepted: false,
            media: m.media.clone(),
            protocol: m.protocol.clone(),
            negotiated_formats: Vec::new(),
            carry_over_attrs: Vec::new(),
            direction: m.direction,
        };
    }

    // Format intersection in the OFFERER's order (RFC 3264 §6: answer
    // formats follow offer order; our preferences are only a filter).
    let supported: std::collections::HashSet<&String> = caps.supported_formats.iter().collect();
    let negotiated: Vec<String> = m
        .formats
        .iter()
        .filter(|fmt| supported.contains(fmt))
        .cloned()
        .collect();

    if negotiated.is_empty() {
        return MediaLineMatch {
            accepted: false,
            media: m.media.clone(),
            protocol: m.protocol.clone(),
            negotiated_formats: Vec::new(),
            carry_over_attrs: Vec::new(),
            direction: m.direction,
        };
    }

    // Carry over rtpmap/fmtp ONLY for formats that survived. Other
    // attributes (crypto, fingerprint, setup, ice-*, candidate, mid,
    // ssrc, group, msid, rtcp-mux, rtcp-fb, extmap, …) survive
    // verbatim — the consumer is responsible for processing them.
    let mut carry_over = Vec::new();
    let kept: std::collections::HashSet<&String> = negotiated.iter().collect();
    for attr in &m.generic_attributes {
        match attr {
            ParsedAttribute::RtpMap(rtpmap) => {
                if kept.contains(&format_to_string(rtpmap)) {
                    carry_over.push(attr.clone());
                }
            }
            ParsedAttribute::Fmtp(fmtp) => {
                if kept.contains(&fmtp.format) {
                    carry_over.push(attr.clone());
                }
            }
            // RtcpFb is also format-keyed (first arg is the PT or "*").
            ParsedAttribute::RtcpFb(fmt, _, _) if fmt != "*" => {
                if kept.contains(fmt) {
                    carry_over.push(attr.clone());
                }
            }
            // Everything else: verbatim. The consumer decides.
            _ => carry_over.push(attr.clone()),
        }
    }

    MediaLineMatch {
        accepted: true,
        media: m.media.clone(),
        protocol: m.protocol.clone(),
        negotiated_formats: negotiated,
        carry_over_attrs: carry_over,
        direction: m.direction,
    }
}

/// rtpmap's `payload_type` is a `u8`; format strings in `MediaDescription::formats`
/// are `String`. Bridge them for the carry-over filter.
fn format_to_string(rtpmap: &RtpMapAttribute) -> String {
    rtpmap.payload_type.to_string()
}

// Tests live in `crates/session-core/tests/sdp_matcher_integration.rs`
// — dialog-core's own `--test` profile fails to compile due to a
// pre-existing crate-skew issue around rvoip-infra-common (out of
// Sprint 3 scope). Move back into a `#[cfg(test)] mod tests {}` here
// once that issue is resolved.

#[cfg(any())]
mod tests {
    use super::*;
    use rvoip_sip_core::sdp::SdpBuilder;
    use rvoip_sip_core::types::sdp::{ParsedAttribute, SdpSession};
    use rvoip_sip_core::MediaDirection;
    use std::str::FromStr;

    /// Helper: build a one-`m=`-line audio offer with the supplied
    /// formats (and optional rtpmap entries).
    fn audio_offer(formats: &[&str]) -> SdpSession {
        let mut b = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16000, "RTP/AVP")
                .formats(formats);
        for fmt in formats {
            // Add an rtpmap so the carry-over filter has something to
            // examine; map the format to a synthetic name.
            b = b.rtpmap(*fmt, &format!("CODEC{}/8000", fmt));
        }
        let sdp = b.attribute("sendrecv", None::<String>)
            .done()
            .build()
            .expect("offer builds")
            .to_string();
        SdpSession::from_str(&sdp).expect("parses")
    }

    fn caps(formats: &[&str]) -> AnswerCapabilities {
        AnswerCapabilities {
            supported_formats: formats.iter().map(|s| s.to_string()).collect(),
            accept_srtp: true,
            require_srtp: false,
        }
    }

    #[test]
    fn intersection_in_offerer_order() {
        // Offer prefers PCMA over PCMU. Our caps prefer PCMU. The
        // answer's order MUST follow the offerer.
        let offer = audio_offer(&["8", "0", "101"]);
        let our = caps(&["0", "8", "101"]);
        let m = match_offer(&offer, &our).unwrap();
        assert_eq!(m.media_lines.len(), 1);
        let line = &m.media_lines[0];
        assert!(line.accepted);
        assert_eq!(line.negotiated_formats, vec!["8", "0", "101"]);
    }

    #[test]
    fn empty_intersection_rejects_media_line() {
        // Offer asks for video PTs we don't support.
        let offer = audio_offer(&["97", "98"]);
        let our = caps(&["0", "8", "101"]);
        let m = match_offer(&offer, &our).unwrap();
        let line = &m.media_lines[0];
        assert!(!line.accepted, "no overlap → reject");
        assert!(line.negotiated_formats.is_empty());
    }

    #[test]
    fn rtpmap_carryover_filters_to_kept_formats() {
        // Offer: `0 8 101`; our caps support only `0`. The answer
        // should carry rtpmap for PT 0 and drop the others.
        let offer = audio_offer(&["0", "8", "101"]);
        let our = caps(&["0"]);
        let m = match_offer(&offer, &our).unwrap();
        let line = &m.media_lines[0];
        assert!(line.accepted);
        assert_eq!(line.negotiated_formats, vec!["0"]);

        let kept_rtpmaps: Vec<u8> = line
            .carry_over_attrs
            .iter()
            .filter_map(|a| match a {
                ParsedAttribute::RtpMap(r) => Some(r.payload_type),
                _ => None,
            })
            .collect();
        assert_eq!(kept_rtpmaps, vec![0]);
    }

    #[test]
    fn srtp_offer_rejected_when_accept_srtp_false() {
        // Build a one-line SAVP offer.
        let sdp = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16000, "RTP/SAVP")
                .formats(&["0", "8"])
                .rtpmap("0", "PCMU/8000")
                .rtpmap("8", "PCMA/8000")
                .attribute("sendrecv", None::<String>)
                .done()
            .build()
            .expect("savp offer builds")
            .to_string();
        let offer = SdpSession::from_str(&sdp).unwrap();

        let our = AnswerCapabilities {
            supported_formats: vec!["0".into(), "8".into()],
            accept_srtp: false,
            require_srtp: false,
        };
        let m = match_offer(&offer, &our).unwrap();
        let line = &m.media_lines[0];
        assert!(!line.accepted, "SAVP offer rejected when accept_srtp=false");
    }

    #[test]
    fn plain_rtp_offer_rejected_when_require_srtp_true() {
        let offer = audio_offer(&["0", "8"]);
        let our = AnswerCapabilities {
            supported_formats: vec!["0".into(), "8".into()],
            accept_srtp: true,
            require_srtp: true,
        };
        let m = match_offer(&offer, &our).unwrap();
        let line = &m.media_lines[0];
        assert!(!line.accepted, "RTP/AVP offer rejected when require_srtp=true");
    }

    #[test]
    fn savp_offer_accepted_when_accept_srtp_true() {
        let sdp = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16000, "RTP/SAVP")
                .formats(&["0"])
                .rtpmap("0", "PCMU/8000")
                .attribute("sendrecv", None::<String>)
                .done()
            .build()
            .expect("savp offer builds")
            .to_string();
        let offer = SdpSession::from_str(&sdp).unwrap();
        let our = caps(&["0"]);
        let m = match_offer(&offer, &our).unwrap();
        assert!(m.media_lines[0].accepted);
        assert_eq!(m.media_lines[0].protocol, "RTP/SAVP");
    }

    #[test]
    fn empty_offer_returns_error() {
        let mut sdp = audio_offer(&["0"]);
        sdp.media_descriptions.clear();
        assert!(matches!(
            match_offer(&sdp, &caps(&["0"])),
            Err(MatchError::EmptyOffer)
        ));
    }

    #[test]
    fn direction_carried_through_per_line() {
        let offer = audio_offer(&["0"]);
        let m = match_offer(&offer, &caps(&["0"])).unwrap();
        // Default offer above set sendrecv.
        assert_eq!(m.media_lines[0].direction, Some(MediaDirection::SendRecv));
    }

    #[test]
    fn unknown_attributes_carried_verbatim() {
        // Build an offer with an `a=fingerprint` line (DTLS-SRTP
        // territory) and an `a=ice-ufrag` line. Matcher must pass
        // both through unchanged so Sprint 4 D2/D3 consumers can
        // process them.
        let sdp = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16000, "RTP/AVP")
                .formats(&["0"])
                .rtpmap("0", "PCMU/8000")
                .attribute("ice-ufrag", Some("abcd1234"))
                .attribute("ice-pwd", Some("supersecretpassword"))
                .attribute("fingerprint", Some("sha-256 AB:CD:EF"))
                .attribute("setup", Some("active"))
                .attribute("sendrecv", None::<String>)
                .done()
            .build()
            .expect("d2-d3 offer builds")
            .to_string();
        let offer = SdpSession::from_str(&sdp).unwrap();
        let m = match_offer(&offer, &caps(&["0"])).unwrap();

        let attrs = &m.media_lines[0].carry_over_attrs;
        // Expect at least one of each forwarded.
        let has_ufrag = attrs
            .iter()
            .any(|a| matches!(a, ParsedAttribute::IceUfrag(_)));
        let has_pwd = attrs
            .iter()
            .any(|a| matches!(a, ParsedAttribute::IcePwd(_)));
        let has_fp = attrs
            .iter()
            .any(|a| matches!(a, ParsedAttribute::Fingerprint(..)));
        let has_setup = attrs.iter().any(|a| matches!(a, ParsedAttribute::Setup(_)));
        assert!(has_ufrag, "ice-ufrag must carry through:\n{:#?}", attrs);
        assert!(has_pwd, "ice-pwd must carry through:\n{:#?}", attrs);
        assert!(has_fp, "fingerprint must carry through:\n{:#?}", attrs);
        assert!(has_setup, "setup must carry through:\n{:#?}", attrs);
    }

    #[test]
    fn multi_m_line_offer_independently_matched() {
        // Audio + video offer. Caps support audio PT 0 only. Video
        // line gets rejected; audio line accepted.
        let sdp = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16000, "RTP/AVP")
                .formats(&["0"])
                .rtpmap("0", "PCMU/8000")
                .attribute("sendrecv", None::<String>)
                .done()
            .media_video(17000, "RTP/AVP")
                .formats(&["97"])
                .rtpmap("97", "VP8/90000")
                .attribute("sendrecv", None::<String>)
                .done()
            .build()
            .expect("multi-m offer builds")
            .to_string();
        let offer = SdpSession::from_str(&sdp).unwrap();
        let our = AnswerCapabilities {
            supported_formats: vec!["0".into()], // audio only
            accept_srtp: true,
            require_srtp: false,
        };
        let m = match_offer(&offer, &our).unwrap();
        assert_eq!(m.media_lines.len(), 2);
        assert!(m.media_lines[0].accepted, "audio line accepted");
        assert_eq!(m.media_lines[0].media, "audio");
        assert!(!m.media_lines[1].accepted, "video line rejected");
        assert_eq!(m.media_lines[1].media, "video");
    }

    #[test]
    fn rtpmap_for_dropped_format_filtered_out() {
        // Offer: `0 8 101`. Caps: `0 101`. The PCMA rtpmap (PT 8)
        // must not appear in the carry-over.
        let offer = audio_offer(&["0", "8", "101"]);
        let m = match_offer(&offer, &caps(&["0", "101"])).unwrap();
        let kept_pts: Vec<u8> = m.media_lines[0]
            .carry_over_attrs
            .iter()
            .filter_map(|a| match a {
                ParsedAttribute::RtpMap(r) => Some(r.payload_type),
                _ => None,
            })
            .collect();
        assert_eq!(kept_pts, vec![0, 101]);
    }
}

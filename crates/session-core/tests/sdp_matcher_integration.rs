//! Sprint 3 C2 — RFC 3264 §6 SDP matcher integration tests.
//!
//! Lives in `session-core/tests/` rather than `dialog-core/tests/`
//! because dialog-core's `--test` profile currently fails to compile
//! due to a pre-existing `StableCrateId` collision around
//! `rvoip-infra-common` that's outside Sprint 3's scope. Session-core
//! depends on dialog-core, so the matcher is reachable here, and this
//! test path compiles clean. Move into `dialog-core/tests/` once the
//! crate-skew issue is resolved (out of Sprint 3 scope).

use std::str::FromStr;

use rvoip_dialog_core::sdp::{match_offer, AnswerCapabilities, MatchError};
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::types::sdp::{ParsedAttribute, SdpSession};
use rvoip_sip_core::MediaDirection;

/// Helper: build a one-`m=`-line audio offer with the supplied
/// formats (and one rtpmap entry per format).
fn audio_offer(formats: &[&str]) -> SdpSession {
    let mut b = SdpBuilder::new("Session")
        .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
        .connection("IN", "IP4", "127.0.0.1")
        .time("0", "0")
        .media_audio(16000, "RTP/AVP")
        .formats(formats);
    for fmt in formats {
        b = b.rtpmap(*fmt, &format!("CODEC{}/8000", fmt));
    }
    let sdp = b
        .attribute("sendrecv", None::<String>)
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
    let offer = audio_offer(&["97", "98"]);
    let our = caps(&["0", "8", "101"]);
    let m = match_offer(&offer, &our).unwrap();
    let line = &m.media_lines[0];
    assert!(!line.accepted);
    assert!(line.negotiated_formats.is_empty());
}

#[test]
fn rtpmap_carryover_filters_to_kept_formats() {
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
    assert!(!m.media_lines[0].accepted);
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
    assert!(!m.media_lines[0].accepted);
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
    assert_eq!(m.media_lines[0].direction, Some(MediaDirection::SendRecv));
}

#[test]
fn ice_and_dtls_attributes_carried_verbatim() {
    // Sprint 4 D2 (DTLS-SRTP) and D3 (ICE) consumers depend on
    // the matcher passing these attributes through unchanged.
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
    assert!(attrs
        .iter()
        .any(|a| matches!(a, ParsedAttribute::IceUfrag(_))));
    assert!(attrs
        .iter()
        .any(|a| matches!(a, ParsedAttribute::IcePwd(_))));
    assert!(attrs
        .iter()
        .any(|a| matches!(a, ParsedAttribute::Fingerprint(..))));
    assert!(attrs.iter().any(|a| matches!(a, ParsedAttribute::Setup(_))));
}

#[test]
fn multi_m_line_offer_independently_matched() {
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
        supported_formats: vec!["0".into()],
        accept_srtp: true,
        require_srtp: false,
    };
    let m = match_offer(&offer, &our).unwrap();
    assert_eq!(m.media_lines.len(), 2);
    assert!(m.media_lines[0].accepted);
    assert_eq!(m.media_lines[0].media, "audio");
    assert!(!m.media_lines[1].accepted);
    assert_eq!(m.media_lines[1].media, "video");
}

#[test]
fn rtpmap_for_dropped_format_filtered_out() {
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

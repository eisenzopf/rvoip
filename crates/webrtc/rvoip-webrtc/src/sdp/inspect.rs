//! Lightweight SDP inspection helpers backed by the shared sip-core SDP parser.

use rvoip_sip_core::sdp::parser::parse_attribute;
use rvoip_sip_core::types::sdp::{ParsedAttribute, SdpSession};
use std::collections::{BTreeMap, HashSet};
use std::str::FromStr;

use crate::media::dtmf::TelephoneEventCodec;
use crate::peer::builder::HDREXT_SDES_MID;

/// Final audio MID header-extension binding for locally-originated RTP.
///
/// The alpha WebRTC engine does not put supplemental SSRCs in SDP. Browsers
/// therefore need the negotiated SDES MID extension to associate RFC 4733
/// packets with the audio m-section. Keep both the value and ID here so the
/// negotiation boundary can reject an ambiguous or non-one-byte binding
/// before any RTP is written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NegotiatedAudioMid {
    pub(crate) value: String,
    pub(crate) extension_id: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AudioMidCandidate {
    value: String,
    extension_id: u8,
    direction: Option<String>,
}

fn one_audio_mid_candidate(sdp: &str) -> Option<AudioMidCandidate> {
    let session = SdpSession::from_str(sdp).ok()?;
    let mut audio = session
        .media_descriptions
        .iter()
        .filter(|media| media.media.eq_ignore_ascii_case("audio") && media.port != 0);
    let media = audio.next()?;
    // One local audio sender cannot safely guess which MID owns an
    // unsignalled supplemental SSRC when SDP contains multiple active audio
    // sections.
    if audio.next().is_some() {
        return None;
    }

    let mut mids = media.generic_attributes.iter().filter_map(|attribute| {
        if let ParsedAttribute::Mid(mid) = attribute {
            Some(mid)
        } else {
            None
        }
    });
    let value = mids.next()?.trim().to_owned();
    if mids.next().is_some()
        || value.is_empty()
        || value.len() > 16
        || !value.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return None;
    }

    let media_extmaps = media
        .generic_attributes
        .iter()
        .filter_map(|attribute| match attribute {
            ParsedAttribute::ExtMap(id, direction, uri, _) if uri == HDREXT_SDES_MID => {
                Some((*id, direction.clone()))
            }
            _ => None,
        });
    let mut extmaps = media_extmaps.collect::<Vec<_>>();
    if extmaps.is_empty() {
        extmaps = session
            .generic_attributes
            .iter()
            .filter_map(|attribute| match attribute {
                ParsedAttribute::ExtMap(id, direction, uri, _) if uri == HDREXT_SDES_MID => {
                    Some((*id, direction.clone()))
                }
                _ => None,
            })
            .collect();
    }
    let [(extension_id, direction)] = extmaps.as_slice() else {
        return None;
    };
    // TrackLocalStaticRTP::write_rtp_with_extensions selects the RFC 8285
    // one-byte profile for a MID of at most 16 bytes. An ID outside 1..=14
    // would make that write fail after dropping the extension, so reject it at
    // negotiation time instead.
    if !(1..=14).contains(extension_id) {
        return None;
    }

    Some(AudioMidCandidate {
        value,
        extension_id: *extension_id,
        direction: direction.clone(),
    })
}

fn extmap_allows_send(direction: Option<&str>) -> bool {
    direction.is_none_or(|direction| {
        direction.eq_ignore_ascii_case("sendrecv") || direction.eq_ignore_ascii_case("sendonly")
    })
}

fn extmap_allows_receive(direction: Option<&str>) -> bool {
    direction.is_none_or(|direction| {
        direction.eq_ignore_ascii_case("sendrecv") || direction.eq_ignore_ascii_case("recvonly")
    })
}

/// Resolve the exact SDES MID value required on locally-originated audio RTP.
///
/// `local_is_offerer` describes the current offer/answer exchange, rather than
/// the peer's long-lived application role (WHEP rollback can temporarily make
/// an application offerer answer a server counter-offer). Any missing,
/// duplicated, remapped, or directionally incompatible binding returns
/// `None`, allowing the RTP sender to fail closed.
pub(crate) fn negotiated_sdes_mid_for_outbound_audio(
    offer_sdp: &str,
    answer_sdp: &str,
    local_is_offerer: bool,
) -> Option<NegotiatedAudioMid> {
    let offer = one_audio_mid_candidate(offer_sdp)?;
    let answer = one_audio_mid_candidate(answer_sdp)?;
    if offer.value != answer.value || offer.extension_id != answer.extension_id {
        return None;
    }

    let (local, remote) = if local_is_offerer {
        (&offer, &answer)
    } else {
        (&answer, &offer)
    };
    if !extmap_allows_send(local.direction.as_deref())
        || !extmap_allows_receive(remote.direction.as_deref())
    {
        return None;
    }

    Some(NegotiatedAudioMid {
        value: local.value.clone(),
        extension_id: local.extension_id,
    })
}

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

/// Extract every RFC 4733 telephone-event payload mapping advertised in
/// remote audio SDP.
///
/// Payload type and clock rate are both negotiation results. Keeping the
/// complete set is important for browser offers that advertise more than one
/// telephone-event rate and then select a non-default dynamic payload type.
#[must_use]
pub fn telephone_event_codecs_in_sdp(sdp: &str) -> Vec<TelephoneEventCodec> {
    if let Ok(session) = SdpSession::from_str(sdp) {
        let mut codecs = Vec::new();
        let mut seen = HashSet::new();
        for media in session
            .media_descriptions
            .iter()
            .filter(|media| media.media.eq_ignore_ascii_case("audio") && media.port != 0)
        {
            // RFC 3264 makes the m-line format order the offerer's preference
            // order. Keep it: Chromium commonly offers PT 110/48 kHz before
            // PT 126/8 kHz, and outbound RFC 4733 must use the selected
            // mapping rather than a numeric-PT sort.
            for format in &media.formats {
                let Ok(payload_type) = format.parse::<u8>() else {
                    continue;
                };
                let Some(rtpmap) = media.get_rtpmap(payload_type) else {
                    continue;
                };
                if rtpmap.encoding_name.eq_ignore_ascii_case("telephone-event")
                    && rtpmap.clock_rate > 0
                    && seen.insert(payload_type)
                {
                    codecs.push(TelephoneEventCodec::new(payload_type, rtpmap.clock_rate));
                }
            }
        }
        return codecs;
    }

    {
        // Fail-soft parser for otherwise valid SDP variants not yet accepted
        // by sip-core. Record mappings first, then apply each active audio
        // m-line's payload order so selection stays deterministic.
        let mut mappings = BTreeMap::<u8, u32>::new();
        let mut audio_formats = Vec::<u8>::new();
        let mut in_active_audio = false;
        for line in sdp.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("m=") {
                let fields = rest.split_whitespace().collect::<Vec<_>>();
                in_active_audio = fields.first().is_some_and(|kind| {
                    kind.eq_ignore_ascii_case("audio")
                        && fields.get(1).is_some_and(|port| *port != "0")
                });
                if in_active_audio {
                    audio_formats.extend(
                        fields
                            .iter()
                            .skip(3)
                            .filter_map(|payload_type| payload_type.parse::<u8>().ok()),
                    );
                }
                continue;
            }
            if !in_active_audio {
                continue;
            }
            let Some(rest) = trimmed.strip_prefix("a=rtpmap:") else {
                continue;
            };
            let mut fields = rest.split_whitespace();
            let (Some(payload_type), Some(encoding)) = (fields.next(), fields.next()) else {
                continue;
            };
            let Ok(payload_type) = payload_type.parse::<u8>() else {
                continue;
            };
            let mut encoding = encoding.split('/');
            let (Some(name), Some(clock_rate_hz)) = (encoding.next(), encoding.next()) else {
                continue;
            };
            if !name.eq_ignore_ascii_case("telephone-event") {
                continue;
            }
            let Ok(clock_rate_hz) = clock_rate_hz.parse::<u32>() else {
                continue;
            };
            if clock_rate_hz > 0 {
                mappings.insert(payload_type, clock_rate_hz);
            }
        }
        let mut seen = HashSet::new();
        return audio_formats
            .into_iter()
            .filter_map(|payload_type| {
                let clock_rate_hz = *mappings.get(&payload_type)?;
                seen.insert(payload_type)
                    .then_some(TelephoneEventCodec::new(payload_type, clock_rate_hz))
            })
            .collect();
    }
}

/// Select the remote peer's preferred active-audio RFC 4733 mapping.
///
/// `None` is significant: the remote description contains no usable
/// telephone-event mapping, so outbound DTMF must fail closed after final SDP.
#[must_use]
pub fn preferred_telephone_event_codec_in_sdp(sdp: &str) -> Option<TelephoneEventCodec> {
    telephone_event_codecs_in_sdp(sdp).into_iter().next()
}

/// Select one RFC 4733 mapping present unchanged in both sides of a completed
/// offer/answer exchange.
///
/// Offer order remains authoritative when several telephone-event rates were
/// accepted. A payload type whose clock changed in the answer is not a valid
/// negotiation result and is ignored.
#[must_use]
pub fn negotiated_telephone_event_codec(
    offer_sdp: &str,
    answer_sdp: &str,
) -> Option<TelephoneEventCodec> {
    let answered = telephone_event_codecs_in_sdp(answer_sdp)
        .into_iter()
        .collect::<HashSet<_>>();
    telephone_event_codecs_in_sdp(offer_sdp)
        .into_iter()
        .find(|codec| answered.contains(codec))
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

    const MID_AUDIO_SDP: &str = concat!(
        "v=0\r\n",
        "o=- 1 1 IN IP4 127.0.0.1\r\n",
        "s=-\r\n",
        "t=0 0\r\n",
        "m=audio 9 UDP/TLS/RTP/SAVPF 111 110\r\n",
        "c=IN IP4 0.0.0.0\r\n",
        "a=mid:call-audio\r\n",
        "a=extmap:4 urn:ietf:params:rtp-hdrext:sdes:mid\r\n",
        "a=sendrecv\r\n",
        "a=rtpmap:111 opus/48000/2\r\n",
        "a=rtpmap:110 telephone-event/48000\r\n",
    );

    #[test]
    fn resolves_exact_negotiated_outbound_audio_mid() {
        let binding = negotiated_sdes_mid_for_outbound_audio(MID_AUDIO_SDP, MID_AUDIO_SDP, true)
            .expect("unique negotiated MID");
        assert_eq!(binding.value.as_bytes(), b"call-audio");
        assert_eq!(binding.extension_id, 4);
    }

    #[test]
    fn ambiguous_or_remapped_audio_mid_fails_closed() {
        let second_audio = concat!(
            "m=audio 9 UDP/TLS/RTP/SAVPF 111 110\r\n",
            "c=IN IP4 0.0.0.0\r\n",
            "a=mid:other-audio\r\n",
            "a=extmap:4 urn:ietf:params:rtp-hdrext:sdes:mid\r\n",
            "a=sendrecv\r\n",
            "a=rtpmap:111 opus/48000/2\r\n",
            "a=rtpmap:110 telephone-event/48000\r\n",
        );
        let ambiguous = format!("{MID_AUDIO_SDP}{second_audio}");
        assert_eq!(
            negotiated_sdes_mid_for_outbound_audio(&ambiguous, &ambiguous, true),
            None
        );

        let remapped = MID_AUDIO_SDP.replace("a=extmap:4 ", "a=extmap:5 ");
        assert_eq!(
            negotiated_sdes_mid_for_outbound_audio(MID_AUDIO_SDP, &remapped, true),
            None
        );
        let missing =
            MID_AUDIO_SDP.replace("a=extmap:4 urn:ietf:params:rtp-hdrext:sdes:mid\r\n", "");
        assert_eq!(
            negotiated_sdes_mid_for_outbound_audio(MID_AUDIO_SDP, &missing, true),
            None
        );
    }

    #[test]
    fn outbound_mid_honors_extmap_direction_from_each_local_perspective() {
        let send_offer = MID_AUDIO_SDP.replace("a=extmap:4 ", "a=extmap:4/sendonly ");
        let receive_answer = MID_AUDIO_SDP.replace("a=extmap:4 ", "a=extmap:4/recvonly ");
        assert!(
            negotiated_sdes_mid_for_outbound_audio(&send_offer, &receive_answer, true).is_some()
        );
        assert_eq!(
            negotiated_sdes_mid_for_outbound_audio(&send_offer, &receive_answer, false),
            None,
            "a recvonly local answer cannot originate the MID extension"
        );

        let receive_offer = MID_AUDIO_SDP.replace("a=extmap:4 ", "a=extmap:4/recvonly ");
        let send_answer = MID_AUDIO_SDP.replace("a=extmap:4 ", "a=extmap:4/sendonly ");
        assert!(
            negotiated_sdes_mid_for_outbound_audio(&receive_offer, &send_answer, false).is_some()
        );
        assert_eq!(
            negotiated_sdes_mid_for_outbound_audio(&receive_offer, &send_answer, true),
            None,
            "a recvonly local offer cannot originate the MID extension"
        );
    }

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
    fn extracts_chromium_dynamic_telephone_event_mappings() {
        let sdp = concat!(
            "v=0\r\n",
            "m=audio 9 UDP/TLS/RTP/SAVPF 111 110 126\r\n",
            "a=rtpmap:111 opus/48000/2\r\n",
            "a=rtpmap:110 telephone-event/48000\r\n",
            "a=rtpmap:126 telephone-event/8000\r\n",
            "m=video 9 UDP/TLS/RTP/SAVPF 96\r\n",
            "a=rtpmap:96 VP8/90000\r\n",
        );
        assert_eq!(
            telephone_event_codecs_in_sdp(sdp),
            vec![
                TelephoneEventCodec::new(110, 48_000),
                TelephoneEventCodec::new(126, 8_000),
            ]
        );
        assert_eq!(
            preferred_telephone_event_codec_in_sdp(sdp),
            Some(TelephoneEventCodec::new(110, 48_000))
        );
    }

    #[test]
    fn selects_eight_khz_dynamic_mapping_when_it_is_the_only_offer() {
        let sdp = concat!(
            "v=0\r\n",
            "m=audio 9 UDP/TLS/RTP/SAVPF 111 126\r\n",
            "a=rtpmap:111 opus/48000/2\r\n",
            "a=rtpmap:126 telephone-event/8000\r\n",
        );
        assert_eq!(
            preferred_telephone_event_codec_in_sdp(sdp),
            Some(TelephoneEventCodec::new(126, 8_000))
        );
    }

    #[test]
    fn no_telephone_event_offer_has_no_negotiated_mapping() {
        let sdp = concat!(
            "v=0\r\n",
            "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n",
            "a=rtpmap:111 opus/48000/2\r\n",
        );
        assert_eq!(preferred_telephone_event_codec_in_sdp(sdp), None);
    }

    #[test]
    fn final_mapping_requires_an_unchanged_offer_answer_pair() {
        let offer = concat!(
            "v=0\r\n",
            "m=audio 9 UDP/TLS/RTP/SAVPF 111 110 126\r\n",
            "a=rtpmap:111 opus/48000/2\r\n",
            "a=rtpmap:110 telephone-event/48000\r\n",
            "a=rtpmap:126 telephone-event/8000\r\n",
        );
        let answer = concat!(
            "v=0\r\n",
            "m=audio 9 UDP/TLS/RTP/SAVPF 111 110\r\n",
            "a=rtpmap:111 opus/48000/2\r\n",
            "a=rtpmap:110 telephone-event/48000\r\n",
        );
        assert_eq!(
            negotiated_telephone_event_codec(offer, answer),
            Some(TelephoneEventCodec::new(110, 48_000))
        );

        let changed_clock = answer.replace("telephone-event/48000", "telephone-event/8000");
        assert_eq!(
            negotiated_telephone_event_codec(offer, &changed_clock),
            None
        );
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

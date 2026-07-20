//! SDP parse/serialize helpers.

use rtc::peer_connection::sdp::{RTCSdpType, RTCSessionDescription};
use rvoip_core::capability::CodecInfo;
use rvoip_sip_core::types::sdp::{MediaDescription, RtpMapAttribute, SdpSession};
use std::str::FromStr;

use crate::errors::{Result, WebRtcError};

pub fn parse_sdp(sdp: &str, kind: RTCSdpType) -> Result<RTCSessionDescription> {
    if sdp.trim().is_empty() {
        return Err(WebRtcError::Sdp("empty SDP".into()));
    }

    let desc = match kind {
        RTCSdpType::Offer => RTCSessionDescription::offer(sdp.to_owned()),
        RTCSdpType::Answer => RTCSessionDescription::answer(sdp.to_owned()),
        other => {
            return Err(WebRtcError::Sdp(format!(
                "unsupported SDP type for parse: {other:?}"
            )));
        }
    };

    desc.map_err(|e| WebRtcError::Sdp(format!("{e}")))
}

pub fn sdp_to_string(desc: &RTCSessionDescription) -> Result<String> {
    if desc.sdp.is_empty() {
        return Err(WebRtcError::Sdp("empty SDP in description".into()));
    }
    Ok(desc.sdp.clone())
}

/// One deterministic primary-audio result from a completed offer/answer
/// exchange.
///
/// Payload type is deliberately retained beside the transport-neutral codec
/// descriptor. Dynamic RTP payload assignments are signaling results and must
/// not be reconstructed later from a process-wide codec table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NegotiatedAudioPayload {
    pub codec: CodecInfo,
    pub payload_type: u8,
}

/// Derive the one primary audio codec that remains in a final SDP answer.
///
/// An SDP answer may legally retain several primary codecs. That describes a
/// negotiated *set*, not the codec the remote sender will choose packet by
/// packet, so this helper fails closed unless exactly one primary codec
/// remains. Callers that deliberately advertise several primary codecs must
/// use payload-aware media handling rather than pretending the first format is
/// a selected codec.
pub fn negotiated_single_audio_payload(
    offer_sdp: &str,
    answer_sdp: &str,
) -> Result<NegotiatedAudioPayload> {
    let offer = SdpSession::from_str(offer_sdp)
        .map_err(|_| WebRtcError::Sdp("unable to parse the audio offer".into()))?;
    let answer = SdpSession::from_str(answer_sdp)
        .map_err(|_| WebRtcError::Sdp("unable to parse the audio answer".into()))?;
    let offer_audio = first_active_audio(&offer)
        .ok_or_else(|| WebRtcError::Sdp("offer has no active audio section".into()))?;
    let answer_audio = first_active_audio(&answer)
        .ok_or_else(|| WebRtcError::Sdp("answer rejected or omitted audio".into()))?;

    let mut primary = Vec::new();
    for format in &answer_audio.formats {
        let payload_type = format.parse::<u8>().map_err(|_| {
            WebRtcError::Sdp("audio answer contains a non-RTP format identifier".into())
        })?;
        let Some(codec) = codec_for_payload(&answer, answer_audio, payload_type)? else {
            continue;
        };

        if !offer_audio
            .formats
            .iter()
            .any(|offered| offered.parse::<u8>() == Ok(payload_type))
        {
            return Err(WebRtcError::Sdp(
                "audio answer selected a payload type absent from the offer".into(),
            ));
        }
        let offered = codec_for_payload(&offer, offer_audio, payload_type)?.ok_or_else(|| {
            WebRtcError::Sdp("audio answer selected an auxiliary offer payload as media".into())
        })?;
        if !same_rtp_codec(&offered, &codec) {
            return Err(WebRtcError::Sdp(
                "audio answer changed the offered RTP payload mapping".into(),
            ));
        }
        primary.push(NegotiatedAudioPayload {
            codec,
            payload_type,
        });
    }

    match primary.len() {
        1 => Ok(primary.remove(0)),
        0 => Err(WebRtcError::Sdp(
            "audio answer contains no supported primary codec".into(),
        )),
        _ => Err(WebRtcError::Sdp(
            "audio answer contains multiple primary codecs".into(),
        )),
    }
}

fn first_active_audio(session: &SdpSession) -> Option<&MediaDescription> {
    session
        .media_descriptions
        .iter()
        .find(|media| media.port != 0 && media.media.eq_ignore_ascii_case("audio"))
}

fn codec_for_payload(
    session: &SdpSession,
    media: &MediaDescription,
    payload_type: u8,
) -> Result<Option<CodecInfo>> {
    let mapping = media
        .rtpmaps()
        .find(|mapping| mapping.payload_type == payload_type)
        .or_else(|| {
            session
                .rtpmaps()
                .find(|mapping| mapping.payload_type == payload_type)
        });
    let static_mapping;
    let mapping = match mapping {
        Some(mapping) => mapping,
        None => {
            static_mapping = match payload_type {
                0 => RtpMapAttribute {
                    payload_type,
                    encoding_name: "PCMU".into(),
                    clock_rate: 8_000,
                    encoding_params: Some("1".into()),
                },
                8 => RtpMapAttribute {
                    payload_type,
                    encoding_name: "PCMA".into(),
                    clock_rate: 8_000,
                    encoding_params: Some("1".into()),
                },
                _ => {
                    return Err(WebRtcError::Sdp(
                        "dynamic audio payload is missing rtpmap".into(),
                    ));
                }
            };
            &static_mapping
        }
    };

    if is_auxiliary_audio_codec(&mapping.encoding_name) {
        return Ok(None);
    }
    let name = normalize_codec_name(&mapping.encoding_name);
    if !matches!(name.as_str(), "opus" | "g.711-mu" | "g.711-a") {
        return Err(WebRtcError::Sdp(
            "audio answer contains an unsupported primary codec".into(),
        ));
    }
    let channels = mapping
        .encoding_params
        .as_deref()
        .and_then(|value| value.split('/').next())
        .and_then(|value| value.parse::<u8>().ok())
        .unwrap_or(1);
    let format = payload_type.to_string();
    let fmtp = media
        .get_fmtp(&format)
        .or_else(|| session.get_fmtp(&format))
        .map(|attribute| attribute.parameters.clone())
        .filter(|value| !value.trim().is_empty());
    Ok(Some(CodecInfo {
        name,
        clock_rate_hz: mapping.clock_rate,
        channels,
        fmtp,
    }))
}

fn is_auxiliary_audio_codec(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "telephone-event" | "cn" | "red" | "rtx" | "ulpfec" | "flexfec-03"
    )
}

fn same_rtp_codec(offered: &CodecInfo, answered: &CodecInfo) -> bool {
    offered.name.eq_ignore_ascii_case(&answered.name)
        && offered.clock_rate_hz == answered.clock_rate_hz
        && offered.channels == answered.channels
}

/// Extract the first audio codec name from an SDP body (best-effort for capability tests).
pub fn audio_codecs_in_sdp(sdp: &str) -> Vec<String> {
    if let Ok(session) = SdpSession::from_str(sdp) {
        return session
            .media_descriptions
            .iter()
            .filter(|media| media.media.eq_ignore_ascii_case("audio"))
            .flat_map(|media| media.rtpmaps())
            .map(|rtpmap| normalize_codec_name(&rtpmap.encoding_name))
            .collect();
    }

    let mut codecs = Vec::new();
    for line in sdp.lines() {
        if let Some(rest) = line.strip_prefix("a=rtpmap:") {
            let mut parts = rest.split_whitespace();
            let _payload_type = parts.next();
            let codec = parts.next().unwrap_or("");
            let name = codec.split('/').next().unwrap_or(codec);
            if !name.is_empty() {
                codecs.push(normalize_codec_name(name));
            }
        }
    }
    codecs
}

fn normalize_codec_name(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "opus" => "opus".into(),
        "pcmu" => "g.711-mu".into(),
        "pcma" => "g.711-a".into(),
        other => other.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFFER: &str = concat!(
        "v=0\r\n",
        "o=- 1 1 IN IP4 127.0.0.1\r\n",
        "s=-\r\n",
        "t=0 0\r\n",
        "c=IN IP4 127.0.0.1\r\n",
        "m=audio 9 UDP/TLS/RTP/SAVPF 111 0 8 110\r\n",
        "a=rtpmap:111 opus/48000/2\r\n",
        "a=fmtp:111 minptime=10;useinbandfec=1\r\n",
        "a=rtpmap:0 PCMU/8000\r\n",
        "a=rtpmap:8 PCMA/8000\r\n",
        "a=rtpmap:110 telephone-event/48000\r\n",
    );

    #[test]
    fn derives_exact_single_primary_codec_and_payload_type() {
        let answer = concat!(
            "v=0\r\n",
            "o=- 2 2 IN IP4 127.0.0.1\r\n",
            "s=-\r\n",
            "t=0 0\r\n",
            "c=IN IP4 127.0.0.1\r\n",
            "m=audio 9 UDP/TLS/RTP/SAVPF 110 111\r\n",
            "a=rtpmap:110 telephone-event/48000\r\n",
            "a=rtpmap:111 opus/48000/2\r\n",
            "a=fmtp:111 minptime=10;useinbandfec=1\r\n",
        );
        let negotiated = negotiated_single_audio_payload(OFFER, answer).unwrap_or_else(|error| {
            if let WebRtcError::Sdp(detail) = error {
                panic!("negotiation failed: {detail}");
            }
            panic!("negotiation failed outside SDP parsing");
        });
        assert_eq!(negotiated.payload_type, 111);
        assert_eq!(negotiated.codec.name, "opus");
        assert_eq!(negotiated.codec.clock_rate_hz, 48_000);
        assert_eq!(negotiated.codec.channels, 2);
    }

    #[test]
    fn rejects_an_answer_with_multiple_primary_codecs() {
        let answer = concat!(
            "v=0\r\n",
            "o=- 2 2 IN IP4 127.0.0.1\r\n",
            "s=-\r\n",
            "t=0 0\r\n",
            "c=IN IP4 127.0.0.1\r\n",
            "m=audio 9 UDP/TLS/RTP/SAVPF 0 111\r\n",
            "a=rtpmap:0 PCMU/8000\r\n",
            "a=rtpmap:111 opus/48000/2\r\n",
        );
        assert!(negotiated_single_audio_payload(OFFER, answer).is_err());
    }

    #[test]
    fn accepts_static_g711_without_rtpmap() {
        let offer = concat!(
            "v=0\r\n",
            "o=- 1 1 IN IP4 127.0.0.1\r\n",
            "s=-\r\n",
            "t=0 0\r\n",
            "c=IN IP4 127.0.0.1\r\n",
            "m=audio 9 RTP/AVP 0 8\r\n",
        );
        let answer = concat!(
            "v=0\r\n",
            "o=- 2 2 IN IP4 127.0.0.1\r\n",
            "s=-\r\n",
            "t=0 0\r\n",
            "c=IN IP4 127.0.0.1\r\n",
            "m=audio 9 RTP/AVP 0\r\n",
        );
        let negotiated = negotiated_single_audio_payload(offer, answer).unwrap_or_else(|error| {
            if let WebRtcError::Sdp(detail) = error {
                panic!("negotiation failed: {detail}");
            }
            panic!("negotiation failed outside SDP parsing");
        });
        assert_eq!(negotiated.payload_type, 0);
        assert_eq!(negotiated.codec.name, "g.711-mu");
    }

    #[test]
    fn rejects_answer_payload_not_present_in_offer() {
        let answer = concat!(
            "v=0\r\n",
            "o=- 2 2 IN IP4 127.0.0.1\r\n",
            "s=-\r\n",
            "t=0 0\r\n",
            "c=IN IP4 127.0.0.1\r\n",
            "m=audio 9 UDP/TLS/RTP/SAVPF 109\r\n",
            "a=rtpmap:109 opus/48000/2\r\n",
        );
        assert!(negotiated_single_audio_payload(OFFER, answer).is_err());
    }
}

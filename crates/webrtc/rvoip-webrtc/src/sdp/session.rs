//! SDP parse/serialize helpers.

use rtc::peer_connection::sdp::{RTCSdpType, RTCSessionDescription};

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

/// Extract the first audio codec name from an SDP body (best-effort for capability tests).
pub fn audio_codecs_in_sdp(sdp: &str) -> Vec<String> {
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

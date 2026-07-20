//! `CapabilityDescriptor` ↔ codec preferences (INTERFACE_DESIGN §9.2).

use rvoip_core::capability::{CapabilityDescriptor, CodecInfo, NegotiatedCodecs};
use std::collections::BTreeSet;

use crate::errors::{Result, WebRtcError};
use crate::peer::builder::{MIME_TYPE_OPUS, MIME_TYPE_PCMA, MIME_TYPE_PCMU};

/// Default WebRTC interop capabilities: Opus + G.711.
pub fn default_webrtc_capabilities() -> CapabilityDescriptor {
    CapabilityDescriptor {
        audio_codecs: vec![
            CodecInfo {
                name: "opus".into(),
                clock_rate_hz: 48000,
                channels: 2,
                fmtp: Some("minptime=10;useinbandfec=1".into()),
            },
            CodecInfo {
                name: "g.711-mu".into(),
                clock_rate_hz: 8000,
                channels: 1,
                fmtp: None,
            },
            CodecInfo {
                name: "g.711-a".into(),
                clock_rate_hz: 8000,
                channels: 1,
                fmtp: None,
            },
        ],
        max_streams_per_connection: 4,
        ..CapabilityDescriptor::default()
    }
}

/// Ordered codec preferences from an offerer's `CapabilityDescriptor`.
pub fn offer_codec_preferences(caps: &CapabilityDescriptor) -> Vec<String> {
    caps.audio_codecs.iter().map(|c| c.name.clone()).collect()
}

/// Pick the first codec from `preferences` that `local` supports.
pub fn pick_codec(preferences: &[String], local: &CapabilityDescriptor) -> Result<CodecInfo> {
    for pref in preferences {
        if let Some(codec) = local.audio_codecs.iter().find(|c| &c.name == pref) {
            return Ok(codec.clone());
        }
    }
    Err(WebRtcError::IncompatibleCapabilities)
}

/// Build negotiated codecs for a 1:1 audio stream after intersection.
pub fn negotiate_audio(
    offer_caps: &CapabilityDescriptor,
    answer_caps: &CapabilityDescriptor,
) -> Result<NegotiatedCodecs> {
    let prefs = offer_codec_preferences(offer_caps);
    let selected = pick_codec(&prefs, answer_caps)?;
    Ok(NegotiatedCodecs {
        audio: Some(selected),
        video: None,
    })
}

pub fn codec_to_mime(name: &str) -> Option<&'static str> {
    match name {
        "opus" => Some(MIME_TYPE_OPUS),
        "g.711-mu" => Some(MIME_TYPE_PCMU),
        "g.711-a" => Some(MIME_TYPE_PCMA),
        _ => None,
    }
}

pub fn mime_to_codec_name(mime: &str) -> Option<&'static str> {
    match mime {
        MIME_TYPE_OPUS => Some("opus"),
        MIME_TYPE_PCMU => Some("g.711-mu"),
        MIME_TYPE_PCMA => Some("g.711-a"),
        _ => None,
    }
}

/// Count distinct primary audio codecs that the rvoip WebRTC media engine can
/// register for this descriptor. Supplemental codecs such as
/// `telephone-event` are intentionally not represented in
/// `CapabilityDescriptor::audio_codecs`.
pub(crate) fn supported_primary_audio_codec_count(caps: &CapabilityDescriptor) -> usize {
    caps.audio_codecs
        .iter()
        .filter_map(|codec| match codec.name.to_ascii_lowercase().as_str() {
            "opus" => Some("opus"),
            "g.711-mu" | "pcmu" => Some("g.711-mu"),
            "g.711-a" | "pcma" => Some("g.711-a"),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .len()
}

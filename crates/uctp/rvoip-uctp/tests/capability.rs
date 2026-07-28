//! Capability negotiation tests per `UCTP_IMPLEMENTATION_PLAN.md` §3.8.
//!
//! Post-migration: `CapabilityDescriptor` (formerly `UctpCapabilityDescriptor`)
//! lives in `rvoip-core`. Imports go through `rvoip_core::capability`.

use rvoip_core::capability::{
    negotiate_streams, CapabilityDescriptor, Codec, CodecInfo, NegotiationOutcome, StreamOffer,
};
use rvoip_uctp::state::default_v0_descriptor;

fn descriptor_with(audio: &[&str]) -> CapabilityDescriptor {
    CapabilityDescriptor {
        audio_codecs: audio
            .iter()
            .map(|s| CodecInfo {
                name: (*s).to_string(),
                clock_rate_hz: 48000,
                channels: 1,
                fmtp: None,
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn default_v0_descriptor_negotiates_pcma_without_codec_fallback() {
    let answerer = default_v0_descriptor();
    let prefs = ["g.711-a".to_string()];
    let offer = StreamOffer {
        id: "pcma-audio",
        kind: "audio",
        direction: "sendrecv",
        codec_preferences: &prefs,
    };

    match negotiate_streams(std::iter::once(offer), &answerer) {
        NegotiationOutcome::Ok(streams) => {
            assert_eq!(streams.len(), 1);
            assert_eq!(streams[0].chosen_codec.as_deref(), Some("g.711-a"));
        }
        NegotiationOutcome::NotAcceptable488 => panic!("default UCTP descriptor rejected PCMA"),
    }
}

#[test]
fn full_codec_overlap_picks_top_preference() {
    let answerer = descriptor_with(&["opus", "g.711-mu"]);
    let prefs = ["opus".to_string(), "g.711-mu".to_string()];
    let offer = StreamOffer {
        id: "strm_1",
        kind: "audio",
        direction: "sendrecv",
        codec_preferences: &prefs,
    };

    match negotiate_streams(std::iter::once(offer), &answerer) {
        NegotiationOutcome::Ok(streams) => {
            assert_eq!(streams.len(), 1);
            assert_eq!(streams[0].chosen_codec.as_deref(), Some("opus"));
        }
        NegotiationOutcome::NotAcceptable488 => panic!("expected Ok"),
    }
}

#[test]
fn partial_overlap_picks_first_supported() {
    let answerer = descriptor_with(&["g.711-mu"]);
    let prefs = [
        "opus".to_string(),
        "g.722".to_string(),
        "g.711-mu".to_string(),
    ];
    let offer = StreamOffer {
        id: "strm_1",
        kind: "audio",
        direction: "sendrecv",
        codec_preferences: &prefs,
    };

    match negotiate_streams(std::iter::once(offer), &answerer) {
        NegotiationOutcome::Ok(streams) => {
            assert_eq!(streams[0].chosen_codec.as_deref(), Some("g.711-mu"));
        }
        _ => panic!("expected Ok"),
    }
}

#[test]
fn connection_negotiate_488() {
    let answerer = descriptor_with(&["opus"]);
    let prefs = ["g.711-mu".to_string(), "g.722".to_string()];
    let offer = StreamOffer {
        id: "strm_1",
        kind: "audio",
        direction: "sendrecv",
        codec_preferences: &prefs,
    };

    matches!(
        negotiate_streams(std::iter::once(offer), &answerer),
        NegotiationOutcome::NotAcceptable488
    );
}

#[test]
fn wire_descriptor_roundtrips_through_json() {
    // The CapabilityDescriptor's serde shape uses the legacy flat-fields
    // `CodecInfo` for audio/video. The wire-spec nested-params `Codec`
    // shape is exercised separately by `codec_wire_shape_roundtrip`.
    let wire = serde_json::json!({
        "audio_codecs": [
            {"name": "opus", "clock_rate_hz": 48000, "channels": 2, "fmtp": null},
            {"name": "g.711-mu", "clock_rate_hz": 8000, "channels": 1, "fmtp": null}
        ],
        "video_codecs": [],
        "data_protocols": ["text", "json"],
        "dtmf_modes": ["rfc4733"],
        "max_streams_per_connection": 8,
        "transport_features": ["media-datagrams", "connection-migration", "0rtt"],
        "interop": ["sip"],
        "identity_assurance_offered": "identified",
        "identity_assurance_required": "task-scoped"
    });

    let descriptor: CapabilityDescriptor = serde_json::from_value(wire).expect("decode");
    assert_eq!(descriptor.audio_codecs.len(), 2);
    assert_eq!(descriptor.audio_codecs[0].name, "opus");
    assert_eq!(descriptor.audio_codecs[0].clock_rate_hz, 48000);
    assert!(descriptor.supports_dtmf_rfc4733());
    assert_eq!(descriptor.max_streams_per_connection, 8);

    let re_encoded = serde_json::to_value(&descriptor).expect("encode");
    let descriptor2: CapabilityDescriptor = serde_json::from_value(re_encoded).expect("decode");
    assert_eq!(descriptor2.audio_codecs[0].name, "opus");
}

#[test]
fn codec_wire_shape_roundtrip() {
    // Exercises the wire-spec `Codec` (nested params) ↔ `CodecInfo`
    // (flat) conversion. The spec §8 wire form lives on `Codec`; the
    // descriptor stores `CodecInfo` for compatibility with non-wire
    // consumers (rvoip-sip, rvoip-rtp-core).
    let wire = serde_json::json!({
        "name": "opus",
        "params": {"sample_rate": 48000, "channels": 2}
    });
    let codec: Codec = serde_json::from_value(wire).expect("decode codec");
    assert_eq!(codec.name, "opus");
    assert_eq!(
        codec.params.get("sample_rate").and_then(|v| v.as_u64()),
        Some(48000)
    );

    let info: CodecInfo = codec.clone().try_into().expect("codec->info");
    assert_eq!(info.clock_rate_hz, 48000);
    assert_eq!(info.channels, 2);

    let back: Codec = info.into();
    assert_eq!(back.name, "opus");
    assert_eq!(
        back.params.get("sample_rate").and_then(|v| v.as_u64()),
        Some(48000)
    );
}

use rvoip_core::capability::{CapabilityDescriptor, CodecInfo};
use rvoip_webrtc::sdp::{
    audio_codecs_in_sdp, default_webrtc_capabilities, negotiate_audio, pick_codec,
};
use rvoip_webrtc::WebRtcError;

fn opus_only() -> CapabilityDescriptor {
    CapabilityDescriptor {
        audio_codecs: vec![CodecInfo {
            name: "opus".into(),
            clock_rate_hz: 48000,
            channels: 2,
            fmtp: None,
        }],
        ..Default::default()
    }
}

fn g711_only() -> CapabilityDescriptor {
    CapabilityDescriptor {
        audio_codecs: vec![CodecInfo {
            name: "g.711-mu".into(),
            clock_rate_hz: 8000,
            channels: 1,
            fmtp: None,
        }],
        ..Default::default()
    }
}

#[test]
fn default_webrtc_capabilities_include_opus_and_g711() {
    let caps = default_webrtc_capabilities();
    assert!(caps.audio_codecs.iter().any(|c| c.name == "opus"));
    assert!(caps.audio_codecs.iter().any(|c| c.name == "g.711-mu"));
}

#[test]
fn negotiate_audio_picks_opus_when_both_support() {
    let offer = default_webrtc_capabilities();
    let answer = opus_only();
    let negotiated = negotiate_audio(&offer, &answer).expect("negotiate");
    assert_eq!(
        negotiated.audio.as_ref().map(|c| c.name.as_str()),
        Some("opus")
    );
}

#[test]
fn negotiate_audio_rejects_disjoint_codecs() {
    let offer = opus_only();
    let answer = g711_only();
    let err = negotiate_audio(&offer, &answer).unwrap_err();
    assert!(matches!(err, WebRtcError::IncompatibleCapabilities));
}

#[test]
fn pick_codec_walks_preferences_in_order() {
    let local = default_webrtc_capabilities();
    let picked = pick_codec(&["g.711-mu".into(), "opus".into()], &local).expect("pick");
    assert_eq!(picked.name, "g.711-mu");
}

#[test]
fn audio_codecs_in_sdp_parses_rtpmap() {
    let sdp = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\n";
    let codecs = audio_codecs_in_sdp(sdp);
    assert!(codecs.contains(&"opus".to_string()));
}

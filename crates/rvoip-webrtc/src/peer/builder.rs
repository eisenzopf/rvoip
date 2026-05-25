//! `PeerConnectionBuilder` wiring — MediaEngine, interceptors, configuration.

use rtc::interceptor::Registry;
use rtc::peer_connection::configuration::media_engine::MIME_TYPE_TELEPHONE_EVENT;
use rtc::peer_connection::configuration::interceptor_registry::register_default_interceptors;
use rtc::peer_connection::configuration::media_engine::MediaEngine;
use rtc::peer_connection::configuration::{
    RTCConfiguration, RTCConfigurationBuilder, RTCIceServer, RTCIceTransportPolicy,
};
use rtc::rtp_transceiver::rtp_sender::{
    RTCPFeedback, RTCRtpCodec, RTCRtpCodecParameters, RTCRtpHeaderExtensionCapability,
    RtpCodecKind, TYPE_RTCP_FB_CCM, TYPE_RTCP_FB_GOOG_REMB, TYPE_RTCP_FB_NACK,
    TYPE_RTCP_FB_TRANSPORT_CC,
};
use std::sync::Arc;

// G6 — RTP header extension URIs. Registered explicitly so that interop SDPs
// (Chrome / Safari / Firefox) round-trip the right `extmap:` IDs.
pub const HDREXT_SDES_MID: &str = "urn:ietf:params:rtp-hdrext:sdes:mid";
pub const HDREXT_SDES_RID: &str = "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id";
pub const HDREXT_SDES_RRID: &str = "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id";
pub const HDREXT_AUDIO_LEVEL: &str = "urn:ietf:params:rtp-hdrext:ssrc-audio-level";
pub const HDREXT_ABS_SEND_TIME: &str =
    "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time";
pub const HDREXT_TWCC: &str =
    "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";
use webrtc::peer_connection::PeerConnectionBuilder;
use webrtc::peer_connection::PeerConnectionEventHandler;
use webrtc::runtime::default_runtime;

use crate::config::WebRtcConfig;
use crate::errors::{Result, WebRtcError};

pub const MIME_TYPE_OPUS: &str = "audio/opus";
pub const MIME_TYPE_PCMU: &str = "audio/PCMU";
pub const MIME_TYPE_PCMA: &str = "audio/PCMA";
pub const MIME_TYPE_VP8: &str = "video/VP8";
pub const MIME_TYPE_VP9: &str = "video/VP9";
pub const MIME_TYPE_H264: &str = "video/H264";
pub const TELEPHONE_EVENT_PAYLOAD_TYPE: u8 = 101;
pub const VP8_PAYLOAD_TYPE: u8 = 96;
pub const VP9_PAYLOAD_TYPE: u8 = 98;
pub const H264_PAYLOAD_TYPE: u8 = 102;

/// Standard video RTCP feedback set: receiver-side packet loss recovery (NACK,
/// NACK+PLI), codec control (CCM+FIR), congestion control (REMB, TWCC).
pub fn video_track_rtcp_feedback() -> Vec<RTCPFeedback> {
    standard_video_rtcp_feedback()
}

/// Standard audio RTCP feedback set (TWCC).
pub fn audio_track_rtcp_feedback() -> Vec<RTCPFeedback> {
    standard_audio_rtcp_feedback()
}

fn standard_video_rtcp_feedback() -> Vec<RTCPFeedback> {
    vec![
        RTCPFeedback {
            typ: TYPE_RTCP_FB_GOOG_REMB.to_owned(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: TYPE_RTCP_FB_TRANSPORT_CC.to_owned(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: TYPE_RTCP_FB_CCM.to_owned(),
            parameter: "fir".to_owned(),
        },
        RTCPFeedback {
            typ: TYPE_RTCP_FB_NACK.to_owned(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: TYPE_RTCP_FB_NACK.to_owned(),
            parameter: "pli".to_owned(),
        },
    ]
}

/// Standard audio RTCP feedback: transport-wide congestion control only.
fn standard_audio_rtcp_feedback() -> Vec<RTCPFeedback> {
    vec![RTCPFeedback {
        typ: TYPE_RTCP_FB_TRANSPORT_CC.to_owned(),
        parameter: String::new(),
    }]
}

/// Build a configured `MediaEngine` with:
/// - Audio: Opus (TWCC), G.711 µ/a-law (SIP interop), telephone-event (DTMF).
/// - Video: VP8, VP9, H.264 (constrained-baseline + level 3.1 profile-level-id
///   `42e01f`, the modern web/Safari-compatible profile) — all with the
///   standard receiver feedback set (NACK, NACK+PLI, CCM+FIR, REMB, TWCC).
///
/// Backward-compatible: uses the default Opus settings
/// (`minptime=10;useinbandfec=1`). Prefer
/// [`build_media_engine_with_config`] when you want to thread an
/// [`OpusSettings`](crate::config::OpusSettings) through.
pub fn build_media_engine() -> Result<MediaEngine> {
    build_media_engine_with_opus(&crate::config::OpusSettings::default())
}

/// G12 — variant that takes an [`OpusSettings`](crate::config::OpusSettings)
/// for the Opus fmtp line. Other codecs and the registered header
/// extensions are unchanged.
pub fn build_media_engine_with_opus(
    opus_settings: &crate::config::OpusSettings,
) -> Result<MediaEngine> {
    let mut media_engine = MediaEngine::default();
    let video_feedback = standard_video_rtcp_feedback();
    let audio_feedback = standard_audio_rtcp_feedback();

    let opus = RTCRtpCodec {
        mime_type: MIME_TYPE_OPUS.to_owned(),
        clock_rate: 48000,
        channels: 2,
        sdp_fmtp_line: opus_settings.to_fmtp_line(),
        rtcp_feedback: audio_feedback.clone(),
    };
    media_engine.register_codec(
        RTCRtpCodecParameters {
            rtp_codec: opus,
            payload_type: 111,
            ..Default::default()
        },
        RtpCodecKind::Audio,
    )?;

    let pcmu = RTCRtpCodec {
        mime_type: MIME_TYPE_PCMU.to_owned(),
        clock_rate: 8000,
        channels: 1,
        sdp_fmtp_line: String::new(),
        rtcp_feedback: vec![],
    };
    media_engine.register_codec(
        RTCRtpCodecParameters {
            rtp_codec: pcmu,
            payload_type: 0,
            ..Default::default()
        },
        RtpCodecKind::Audio,
    )?;

    let pcma = RTCRtpCodec {
        mime_type: MIME_TYPE_PCMA.to_owned(),
        clock_rate: 8000,
        channels: 1,
        sdp_fmtp_line: String::new(),
        rtcp_feedback: vec![],
    };
    media_engine.register_codec(
        RTCRtpCodecParameters {
            rtp_codec: pcma,
            payload_type: 8,
            ..Default::default()
        },
        RtpCodecKind::Audio,
    )?;

    let telephone_event = RTCRtpCodec {
        mime_type: MIME_TYPE_TELEPHONE_EVENT.to_owned(),
        clock_rate: 8000,
        channels: 1,
        sdp_fmtp_line: "0-15".into(),
        rtcp_feedback: vec![],
    };
    media_engine.register_codec(
        RTCRtpCodecParameters {
            rtp_codec: telephone_event,
            payload_type: TELEPHONE_EVENT_PAYLOAD_TYPE,
            ..Default::default()
        },
        RtpCodecKind::Audio,
    )?;

    let vp8 = RTCRtpCodec {
        mime_type: MIME_TYPE_VP8.to_owned(),
        clock_rate: 90000,
        channels: 0,
        sdp_fmtp_line: String::new(),
        rtcp_feedback: video_feedback.clone(),
    };
    media_engine.register_codec(
        RTCRtpCodecParameters {
            rtp_codec: vp8,
            payload_type: VP8_PAYLOAD_TYPE,
            ..Default::default()
        },
        RtpCodecKind::Video,
    )?;

    let vp9 = RTCRtpCodec {
        mime_type: MIME_TYPE_VP9.to_owned(),
        clock_rate: 90000,
        channels: 0,
        sdp_fmtp_line: "profile-id=0".to_owned(),
        rtcp_feedback: video_feedback.clone(),
    };
    media_engine.register_codec(
        RTCRtpCodecParameters {
            rtp_codec: vp9,
            payload_type: VP9_PAYLOAD_TYPE,
            ..Default::default()
        },
        RtpCodecKind::Video,
    )?;

    // H.264 constrained-baseline level 3.1, packetization-mode=1
    // (profile-level-id=42e01f). Required for Safari and many SIP gateways /
    // SBCs. We expose a single profile to keep the codec table small.
    let h264 = RTCRtpCodec {
        mime_type: MIME_TYPE_H264.to_owned(),
        clock_rate: 90000,
        channels: 0,
        sdp_fmtp_line:
            "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f".to_owned(),
        rtcp_feedback: video_feedback,
    };
    media_engine.register_codec(
        RTCRtpCodecParameters {
            rtp_codec: h264,
            payload_type: H264_PAYLOAD_TYPE,
            ..Default::default()
        },
        RtpCodecKind::Video,
    )?;

    // G6 — explicitly register the canonical browser-interop RTP header
    // extensions so the `extmap:` ID survives offer/answer round-trips.
    // Order matters only for negotiated id stability (browsers don't care).
    register_default_header_extensions(&mut media_engine)?;

    Ok(media_engine)
}

/// G6 — register the canonical RTP header extensions for browser interop.
///
/// | URI                                                              | Audio | Video | Spec       |
/// |------------------------------------------------------------------|-------|-------|------------|
/// | `urn:ietf:params:rtp-hdrext:sdes:mid`                            |   ✓   |   ✓   | RFC 9335   |
/// | `urn:ietf:params:rtp-hdrext:ssrc-audio-level`                    |   ✓   |       | RFC 6464   |
/// | `urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id`                  |       |   ✓   | RFC 8852   |
/// | `urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id`         |       |   ✓   | RFC 8852   |
/// | `http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time`     |   ✓   |   ✓   | draft      |
/// | `http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01` | ✓ | ✓ | draft |
pub fn register_default_header_extensions(media_engine: &mut MediaEngine) -> Result<()> {
    fn ext(uri: &str) -> RTCRtpHeaderExtensionCapability {
        RTCRtpHeaderExtensionCapability {
            uri: uri.to_owned(),
        }
    }
    // MID — required by browsers using BUNDLE.
    media_engine.register_header_extension(ext(HDREXT_SDES_MID), RtpCodecKind::Audio, None)?;
    media_engine.register_header_extension(ext(HDREXT_SDES_MID), RtpCodecKind::Video, None)?;
    // Audio level — Chrome interop, active-speaker detection.
    media_engine.register_header_extension(ext(HDREXT_AUDIO_LEVEL), RtpCodecKind::Audio, None)?;
    // RID / repaired-RID — required for simulcast clients.
    media_engine.register_header_extension(ext(HDREXT_SDES_RID), RtpCodecKind::Video, None)?;
    media_engine.register_header_extension(ext(HDREXT_SDES_RRID), RtpCodecKind::Video, None)?;
    // abs-send-time — Safari & older congestion control paths.
    media_engine.register_header_extension(ext(HDREXT_ABS_SEND_TIME), RtpCodecKind::Audio, None)?;
    media_engine.register_header_extension(ext(HDREXT_ABS_SEND_TIME), RtpCodecKind::Video, None)?;
    // TWCC — every modern browser.
    media_engine.register_header_extension(ext(HDREXT_TWCC), RtpCodecKind::Audio, None)?;
    media_engine.register_header_extension(ext(HDREXT_TWCC), RtpCodecKind::Video, None)?;
    Ok(())
}

pub fn build_rtc_configuration(config: &WebRtcConfig) -> RTCConfiguration {
    let ice_servers: Vec<RTCIceServer> = config
        .ice_servers
        .iter()
        .map(|entry| RTCIceServer {
            urls: entry.urls.clone(),
            username: entry.username.clone().unwrap_or_default(),
            credential: entry.credential.clone().unwrap_or_default(),
            ..Default::default()
        })
        .collect();

    let policy = match config.ice_transport_policy {
        crate::config::IceTransportPolicy::All => RTCIceTransportPolicy::All,
        crate::config::IceTransportPolicy::Relay => RTCIceTransportPolicy::Relay,
    };

    RTCConfigurationBuilder::new()
        .with_ice_servers(ice_servers)
        .with_ice_transport_policy(policy)
        .build()
}

/// Construct a webrtc-rs peer connection with shared media engine settings.
pub async fn build_peer_connection(
    config: &WebRtcConfig,
    handler: Arc<dyn PeerConnectionEventHandler>,
) -> Result<Arc<dyn webrtc::peer_connection::PeerConnection>> {
    let runtime = default_runtime().ok_or_else(|| {
        WebRtcError::Webrtc("no async runtime found (enable webrtc runtime-tokio)".into())
    })?;

    let mut media_engine = build_media_engine_with_opus(&config.opus_settings)?;
    let registry = register_default_interceptors(Registry::new(), &mut media_engine)
        .map_err(|e| WebRtcError::Webrtc(format!("{e}")))?;
    let rtc_config = build_rtc_configuration(config);

    let pc = PeerConnectionBuilder::new()
        .with_configuration(rtc_config)
        .with_media_engine(media_engine)
        .with_interceptor_registry(registry)
        .with_handler(handler)
        .with_runtime(runtime)
        .with_udp_addrs(vec![config.udp_bind.clone()])
        .build()
        .await?;

    Ok(Arc::new(pc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_engine_registers_opus_and_g711() -> Result<()> {
        let _engine = build_media_engine()?;
        Ok(())
    }
}

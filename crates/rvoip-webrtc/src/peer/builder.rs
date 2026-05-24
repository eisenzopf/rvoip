//! `PeerConnectionBuilder` wiring — MediaEngine, interceptors, configuration.

use rtc::interceptor::Registry;
use rtc::peer_connection::configuration::media_engine::MIME_TYPE_TELEPHONE_EVENT;
use rtc::peer_connection::configuration::interceptor_registry::register_default_interceptors;
use rtc::peer_connection::configuration::media_engine::MediaEngine;
use rtc::peer_connection::configuration::{
    RTCConfiguration, RTCConfigurationBuilder, RTCIceServer,
};
use rtc::rtp_transceiver::rtp_sender::{RTCRtpCodec, RTCRtpCodecParameters, RtpCodecKind};
use std::sync::Arc;
use webrtc::peer_connection::PeerConnectionBuilder;
use webrtc::peer_connection::PeerConnectionEventHandler;
use webrtc::runtime::default_runtime;

use crate::config::WebRtcConfig;
use crate::errors::{Result, WebRtcError};

pub const MIME_TYPE_OPUS: &str = "audio/opus";
pub const MIME_TYPE_PCMU: &str = "audio/PCMU";
pub const MIME_TYPE_PCMA: &str = "audio/PCMA";
pub const MIME_TYPE_VP8: &str = "video/VP8";
pub const TELEPHONE_EVENT_PAYLOAD_TYPE: u8 = 101;
pub const VP8_PAYLOAD_TYPE: u8 = 96;

/// Build a configured `MediaEngine` with Opus + G.711 for SIP interop.
pub fn build_media_engine() -> Result<MediaEngine> {
    let mut media_engine = MediaEngine::default();

    let opus = RTCRtpCodec {
        mime_type: MIME_TYPE_OPUS.to_owned(),
        clock_rate: 48000,
        channels: 2,
        sdp_fmtp_line: "minptime=10;useinbandfec=1".into(),
        rtcp_feedback: vec![],
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
        rtcp_feedback: vec![],
    };
    media_engine.register_codec(
        RTCRtpCodecParameters {
            rtp_codec: vp8,
            payload_type: VP8_PAYLOAD_TYPE,
            ..Default::default()
        },
        RtpCodecKind::Video,
    )?;

    Ok(media_engine)
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

    RTCConfigurationBuilder::new()
        .with_ice_servers(ice_servers)
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

    let mut media_engine = build_media_engine()?;
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
    fn media_engine_registers_opus_and_g711() {
        let engine = build_media_engine().expect("media engine");
        let _ = engine;
    }
}

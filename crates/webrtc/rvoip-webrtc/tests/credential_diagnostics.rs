#[cfg(feature = "client")]
use rvoip_webrtc::client::{Answer, CallTarget, IceCandidate, Offer};
#[cfg(feature = "client-video")]
use rvoip_webrtc::client::{VideoCodec, VideoFrame, YuvFrame};
use rvoip_webrtc::config::{IceServerConfig, WebRtcConfig};
use rvoip_webrtc::data_message::EncodedDataMessage;
use rvoip_webrtc::identity::DtlsFingerprint;
use rvoip_webrtc::media::packetize::{H264Packet, Vp8Packet};
use rvoip_webrtc::peer::DataChannelOptions;
#[cfg(feature = "signaling-ws")]
use rvoip_webrtc::signaling::websocket::SignalingMessage;
#[cfg(feature = "signaling-ws")]
use rvoip_webrtc::signaling::AuthRejection;
use rvoip_webrtc::WebRtcError;

const CANARY: &str = "webrtc-turn-credential-canary\r\nAuthorization: exposed";

#[test]
fn turn_and_enclosing_config_debug_redact_credentials_and_urls() {
    let server = IceServerConfig::turn(CANARY, CANARY, CANARY);
    let mut config = WebRtcConfig::loopback();
    config.udp_bind = CANARY.into();
    config.ice_servers = vec![server.clone()];
    config.cors_origins = vec![CANARY.into()];

    for rendered in [format!("{server:?}"), format!("{config:?}")] {
        assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
    }
    assert_eq!(server.credential.as_deref(), Some(CANARY));
    assert_eq!(config.ice_servers[0].credential.as_deref(), Some(CANARY));

    let wire = serde_json::to_string(&config).unwrap();
    let restored: WebRtcConfig = serde_json::from_str(&wire).unwrap();
    assert_eq!(restored.ice_servers[0].credential.as_deref(), Some(CANARY));
}

#[test]
fn turn_and_handshake_containers_do_not_regain_derived_debug() {
    for (source, declaration) in [
        (
            include_str!("../src/config.rs"),
            "pub struct IceServerConfig",
        ),
        (include_str!("../src/config.rs"), "pub struct WebRtcConfig"),
        (
            include_str!("../src/signaling/websocket.rs"),
            "struct HandshakeMetadata",
        ),
        (
            include_str!("../src/peer/data_channel.rs"),
            "pub struct DataChannelOptions",
        ),
        (
            include_str!("../src/media/packetize/vp8.rs"),
            "pub struct Vp8Packet",
        ),
        (
            include_str!("../src/media/packetize/h264.rs"),
            "pub struct H264Packet",
        ),
    ] {
        assert_no_derived_debug(source, declaration);
    }
    #[cfg(feature = "client")]
    for (source, declaration) in [
        (include_str!("../src/client/native.rs"), "pub struct Offer"),
        (include_str!("../src/client/native.rs"), "pub struct Answer"),
        (
            include_str!("../src/client/native.rs"),
            "pub struct IceCandidate",
        ),
        (
            include_str!("../src/client/native.rs"),
            "pub enum CallTarget",
        ),
    ] {
        assert_no_derived_debug(source, declaration);
    }
    #[cfg(feature = "client-video")]
    for (source, declaration) in [
        (
            include_str!("../src/client/video.rs"),
            "pub struct YuvFrame",
        ),
        (
            include_str!("../src/client/video.rs"),
            "pub enum VideoFrame",
        ),
    ] {
        assert_no_derived_debug(source, declaration);
    }
}

#[test]
fn data_channel_and_packetizer_debug_is_metadata_only() {
    use bytes::Bytes;

    let options = DataChannelOptions::partial_reliable_retransmits(3)
        .with_protocol(CANARY)
        .with_negotiated_id(7);
    let payload = Bytes::copy_from_slice(CANARY.as_bytes());
    let vp8 = Vp8Packet {
        payload: payload.clone(),
        marker: true,
    };
    let h264 = H264Packet {
        payload,
        marker: false,
    };

    let options_debug = format!("{options:?}");
    let vp8_debug = format!("{vp8:?}");
    let h264_debug = format!("{h264:?}");
    for rendered in [&options_debug, &vp8_debug, &h264_debug] {
        assert!(!rendered.contains(CANARY), "payload leaked: {rendered}");
    }
    assert!(options_debug.contains("protocol_bytes"));
    assert!(vp8_debug.contains("payload_bytes"));
    assert!(h264_debug.contains("payload_bytes"));

    assert_eq!(options.protocol.as_deref(), Some(CANARY));
    assert_eq!(options.max_retransmits, Some(3));
    assert_eq!(options.negotiated_id, Some(7));
    assert_eq!(vp8.payload.as_ref(), CANARY.as_bytes());
    assert!(vp8.marker);
    assert_eq!(h264.payload.as_ref(), CANARY.as_bytes());
    assert!(!h264.marker);
}

fn assert_no_derived_debug(source: &str, declaration: &str) {
    let prefix = &source[..source.find(declaration).unwrap()];
    let attributes = prefix.rsplit("\n\n").next().unwrap_or_default();
    assert!(
        !attributes.contains("Debug"),
        "{declaration} regained derived Debug"
    );
}

#[cfg(feature = "client")]
#[test]
fn native_signaling_debug_is_metadata_only() {
    const SDP_CANARY: &str = "v=0\r\na=ice-pwd:native-sdp-canary\r\n";
    const CONNECTION_CANARY: &str = "native-connection-id-canary";
    const ICE_CANARY: &str =
        r#"{"candidate":"candidate:1 1 UDP 1 192.0.2.1 3478 typ host native-ice-canary"}"#;
    const URI_CANARY: &str = "sip:native-uri-canary@example.invalid";
    const PARTICIPANT_CANARY: &str = "native-participant-canary";

    let offer = Offer(SDP_CANARY.into());
    let answer = Answer {
        sdp: SDP_CANARY.into(),
        connection_id: Some(CONNECTION_CANARY.into()),
    };
    let candidate = IceCandidate(ICE_CANARY.into());
    let uri = CallTarget::Uri(URI_CANARY.into());
    let participant = CallTarget::Participant(PARTICIPANT_CANARY.into());

    for (rendered, canary) in [
        (format!("{offer:?}"), SDP_CANARY),
        (format!("{answer:?}"), SDP_CANARY),
        (format!("{answer:?}"), CONNECTION_CANARY),
        (format!("{candidate:?}"), ICE_CANARY),
        (format!("{uri:?}"), URI_CANARY),
        (format!("{participant:?}"), PARTICIPANT_CANARY),
    ] {
        assert!(!rendered.contains(canary), "payload leaked: {rendered}");
    }

    assert_eq!(offer.0, SDP_CANARY);
    assert_eq!(answer.sdp, SDP_CANARY);
    assert_eq!(answer.connection_id.as_deref(), Some(CONNECTION_CANARY));
    assert_eq!(candidate.0, ICE_CANARY);
    assert!(matches!(uri, CallTarget::Uri(value) if value == URI_CANARY));
    assert!(matches!(participant, CallTarget::Participant(value) if value == PARTICIPANT_CANARY));
}

#[cfg(feature = "client-video")]
#[test]
fn native_video_debug_is_metadata_only() {
    use bytes::Bytes;
    use std::time::Duration;

    const VIDEO_CANARY: &[u8] = b"native-video-payload-canary";
    const VIDEO_CANARY_TEXT: &str = "native-video-payload-canary";
    let plane = Bytes::copy_from_slice(VIDEO_CANARY);
    let yuv = YuvFrame {
        y: plane.clone(),
        u: plane.clone(),
        v: plane.clone(),
        capture_time: Duration::from_millis(7),
    };
    let encoded = VideoFrame::Encoded {
        codec: VideoCodec::Vp8,
        rtp_packets: vec![plane.clone()],
        timestamp_rtp: 42,
        keyframe: true,
    };
    let raw = VideoFrame::YuvI420 {
        width: 16,
        height: 16,
        y: plane.clone(),
        u: plane.clone(),
        v: plane,
        capture_time: Duration::from_millis(9),
    };

    for rendered in [
        format!("{yuv:?}"),
        format!("{encoded:?}"),
        format!("{raw:?}"),
    ] {
        assert!(
            !rendered.contains(VIDEO_CANARY_TEXT),
            "video payload leaked: {rendered}"
        );
    }

    assert_eq!(yuv.y.as_ref(), VIDEO_CANARY);
    assert!(matches!(
        encoded,
        VideoFrame::Encoded { rtp_packets, .. } if rtp_packets[0].as_ref() == VIDEO_CANARY
    ));
    assert!(matches!(
        raw,
        VideoFrame::YuvI420 { y, u, v, .. }
            if y.as_ref() == VIDEO_CANARY
                && u.as_ref() == VIDEO_CANARY
                && v.as_ref() == VIDEO_CANARY
    ));
}

#[test]
fn signaling_identity_and_errors_are_metadata_only() {
    let fingerprint = DtlsFingerprint {
        algorithm: CANARY.into(),
        value: CANARY.into(),
    };
    let encoded = EncodedDataMessage::Text(CANARY.into());
    let error = WebRtcError::Signaling(CANARY.into());

    for rendered in [
        format!("{fingerprint:?}"),
        format!("{encoded:?}"),
        format!("{error:?} {error}"),
    ] {
        assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
    }
    assert_eq!(fingerprint.value, CANARY);
    assert!(matches!(error, WebRtcError::Signaling(value) if value == CANARY));
}

#[cfg(feature = "signaling-ws")]
#[test]
fn signaling_messages_and_auth_rejections_are_metadata_only() {
    let message = SignalingMessage {
        msg_type: CANARY.into(),
        sdp: CANARY.into(),
        candidate: CANARY.into(),
        connection_id: CANARY.into(),
    };
    let rejection = AuthRejection::Unauthorized {
        www_authenticate: CANARY.into(),
    };
    for rendered in [format!("{message:?}"), format!("{rejection:?}")] {
        assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
    }
    assert_eq!(message.sdp, CANARY);
}

#[test]
fn route_ownership_and_candidate_logging_do_not_regain_raw_diagnostics() {
    let adapter = include_str!("../src/adapter.rs");
    assert!(
        !adapter.contains("#[derive(Clone, Debug, Eq, PartialEq)]\npub(crate) enum RouteOwnerKey")
    );
    assert!(!adapter.contains("candidate = %candidate.candidate"));
    assert!(!adapter.contains("warn!(conn = %conn, label,"));
    assert!(!adapter.contains("warn!(conn = %conn, label = %label"));
    assert!(!adapter.contains("warn!(conn = %conn, label = ?label"));
    assert!(!adapter.contains("label, \"WebRTC adapter event queue full"));
    assert!(!adapter.contains("label, error = %error, \"dropping invalid"));
}

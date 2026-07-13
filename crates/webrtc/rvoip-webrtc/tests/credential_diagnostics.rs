use rvoip_webrtc::config::{IceServerConfig, WebRtcConfig};
use rvoip_webrtc::data_message::EncodedDataMessage;
use rvoip_webrtc::identity::DtlsFingerprint;
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
    ] {
        let prefix = &source[..source.find(declaration).unwrap()];
        let attributes = prefix.rsplit("\n\n").next().unwrap_or_default();
        assert!(
            !attributes.contains("Debug"),
            "{declaration} regained derived Debug"
        );
    }
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
}

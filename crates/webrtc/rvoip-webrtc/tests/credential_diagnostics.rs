use rvoip_webrtc::config::{IceServerConfig, WebRtcConfig};

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

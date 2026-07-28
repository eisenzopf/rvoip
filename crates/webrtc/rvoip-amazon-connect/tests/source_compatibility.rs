//! Compile/runtime canary for the frozen public legacy surface.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use rvoip_amazon_connect::{
    AmazonConnectAdapter, ConnectConfig, ConnectContactStarter, ConnectionData, ContactTarget,
    MediaPlacement, StartContactRequest, StopContactRequest,
};

struct ExistingCustomStarter;

#[async_trait]
impl ConnectContactStarter for ExistingCustomStarter {
    async fn start_webrtc_contact(
        &self,
        request: StartContactRequest,
    ) -> rvoip_amazon_connect::Result<ConnectionData> {
        assert!(request.client_token.is_none());
        Err(rvoip_amazon_connect::ConnectError::Control(
            "legacy test stops before media".into(),
        ))
    }
}

#[tokio::test]
async fn legacy_struct_literals_and_none_token_wrapper_remain_source_compatible() {
    let mut config = ConnectConfig::new("legacy-instance", "legacy-flow");
    config.default_display_name = "legacy-display".into();
    let adapter = AmazonConnectAdapter::new(config, Arc::new(ExistingCustomStarter));
    let target = ContactTarget {
        instance_id: Some("legacy-override-instance".into()),
        contact_flow_id: Some("legacy-override-flow".into()),
        default_display_name: Some("legacy-override-display".into()),
    };
    let result = adapter
        .originate_contact_to(target, BTreeMap::new(), None, None)
        .await;
    assert!(result.is_err());

    let _request = StartContactRequest {
        instance_id: "instance".into(),
        contact_flow_id: "flow".into(),
        display_name: "display".into(),
        attributes: BTreeMap::new(),
        description: None,
        client_token: None,
    };
    let _stop = StopContactRequest {
        instance_id: "instance".into(),
        contact_id: "contact".into(),
    };
    let _connection = ConnectionData {
        contact_id: "contact".into(),
        participant_id: "participant".into(),
        participant_token: "participant-token".into(),
        meeting_id: "meeting".into(),
        media_region: "region".into(),
        attendee_id: "attendee".into(),
        join_token: "join-token".into(),
        media_placement: MediaPlacement {
            signaling_url: "wss://signal.example".into(),
            audio_host_url: "audio.example".into(),
            turn_control_url: None,
            audio_fallback_url: None,
            event_ingestion_url: None,
        },
    };
}

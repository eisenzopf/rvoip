use std::time::Duration;

use rvoip_core::adapter::{ConnectionAdapter, EndReason, OriginateRequest};
use rvoip_core::connection::{Direction, Transport};
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_vapi::{
    VapiAdapter, VapiApiKey, VapiAssistant, VapiAudioFormat, VapiCallOptions, VapiConfig,
};

/// Creates a real Vapi WebSocket call and immediately ends it.
///
/// This test is intentionally ignored because it uses credentials, consumes
/// provider concurrency, and may incur a small charge.
#[tokio::test]
#[ignore = "requires VAPI_API_KEY and VAPI_ASSISTANT_ID or VAPI_TRANSIENT_ASSISTANT_JSON"]
async fn live_vapi_websocket_activation_and_shutdown() {
    let api_key = std::env::var("VAPI_API_KEY").expect("VAPI_API_KEY");
    let assistant = match std::env::var("VAPI_ASSISTANT_ID") {
        Ok(assistant_id) => VapiAssistant::saved(assistant_id),
        Err(_) => {
            let definition = std::env::var("VAPI_TRANSIENT_ASSISTANT_JSON")
                .expect("VAPI_ASSISTANT_ID or VAPI_TRANSIENT_ASSISTANT_JSON");
            VapiAssistant::transient(
                serde_json::from_str(&definition).expect("valid transient assistant JSON"),
            )
        }
    };
    let adapter = VapiAdapter::new(VapiConfig::new(
        VapiApiKey::new(api_key).expect("valid Vapi API key"),
    ))
    .expect("Vapi adapter");
    let _events = adapter.subscribe_events();
    let request = OriginateRequest::new(
        SessionId::new(),
        ParticipantId::new(),
        "vapi.websocket",
        Direction::Outbound,
        VapiAudioFormat::MuLaw8Khz.capabilities(),
    )
    .with_transport(Transport::Vapi)
    .with_context(VapiCallOptions::new(assistant));
    let connection_id = adapter
        .originate(request)
        .await
        .expect("prepare Vapi route")
        .connection
        .id;
    tokio::time::timeout(
        Duration::from_secs(20),
        adapter.activate_outbound_with_receipt(connection_id.clone()),
    )
    .await
    .expect("activation timeout")
    .expect("activate Vapi route");
    adapter
        .end(connection_id, EndReason::Normal)
        .await
        .expect("end Vapi route");
}

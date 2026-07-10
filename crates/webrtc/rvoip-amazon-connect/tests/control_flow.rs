//! Adapter control-plane wiring tests using a mock `ConnectContactStarter`.
//!
//! These verify, without AWS or a live Chime meeting, that:
//!   * SIP-header → attribute translation feeds `StartWebRTCContact` correctly.
//!   * `originate_contact` invokes the control plane with those attributes.
//!   * The flow then proceeds to the Chime signaling step (which fails fast
//!     against an unreachable URL — proving control happened first).

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::Deserialize;

use rvoip_amazon_connect::control::{
    ConnectContactStarter, ConnectionData, MediaPlacement, StartContactRequest,
};
use rvoip_amazon_connect::{AmazonConnectAdapter, AttributeMapping, ConnectConfig};

/// Records the request it received and returns a `ConnectionData` whose
/// signaling URL is unreachable, so the media leg fails fast after control.
struct MockStarter {
    last: Arc<Mutex<Option<StartContactRequest>>>,
    signaling_url: String,
}

#[derive(Deserialize)]
struct ScreenPopFixture {
    instance_id: String,
    contact_flow_id: String,
    display_name: String,
    description: String,
    headers: Vec<(String, String)>,
    expected_attributes: BTreeMap<String, String>,
}

fn screen_pop_fixture() -> ScreenPopFixture {
    serde_json::from_str(include_str!("fixtures/vapi_screen_pop.json"))
        .expect("valid Vapi screen-pop fixture")
}

#[async_trait]
impl ConnectContactStarter for MockStarter {
    async fn start_webrtc_contact(
        &self,
        request: StartContactRequest,
    ) -> rvoip_amazon_connect::Result<ConnectionData> {
        *self.last.lock() = Some(request);
        Ok(ConnectionData {
            contact_id: "contact-1".into(),
            participant_id: "participant-1".into(),
            participant_token: "ptok".into(),
            meeting_id: "meeting-1".into(),
            media_region: "us-west-2".into(),
            attendee_id: "attendee-1".into(),
            join_token: "jtok".into(),
            media_placement: MediaPlacement {
                signaling_url: self.signaling_url.clone(),
                audio_host_url: "audio.example.invalid".into(),
                ..Default::default()
            },
        })
    }
}

#[tokio::test]
async fn originate_passes_translated_attributes_to_control_plane() {
    let fixture = screen_pop_fixture();
    let last = Arc::new(Mutex::new(None));
    // A localhost TCP double accepts and closes during the WebSocket handshake
    // after the mock control call, without DNS or any AWS/Chime dependency.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("ephemeral listener");
    let address = listener.local_addr().expect("listener address");
    let signaling_double = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("signaling connection");
        drop(stream);
    });
    let starter = Arc::new(MockStarter {
        last: Arc::clone(&last),
        signaling_url: format!("ws://{address}/control/m1"),
    });

    let mapping = AttributeMapping::default()
        .rename("X-Correlation-Id", "correlation_id")
        .rename("X-Vapi-Call-Id", "vapi_call_id");
    let config = ConnectConfig::new(&fixture.instance_id, &fixture.contact_flow_id)
        .with_attribute_mapping(mapping.clone());
    let adapter = AmazonConnectAdapter::new(config, starter);

    // Translate the golden Vapi transfer headers exactly as server glue does.
    let mapped = mapping.translate(fixture.headers);
    assert_eq!(mapped.attributes, fixture.expected_attributes);

    // The media leg will fail (unreachable signaling URL), but control must run.
    let result = adapter
        .originate_contact(
            mapped.attributes.clone(),
            Some(fixture.display_name.clone()),
            Some(fixture.description.clone()),
        )
        .await;
    assert!(
        result.is_err(),
        "expected signaling failure against invalid URL"
    );
    signaling_double.await.expect("signaling double task");

    let req = last.lock().take().expect("control plane was invoked");
    assert_eq!(req.instance_id, fixture.instance_id);
    assert_eq!(req.contact_flow_id, fixture.contact_flow_id);
    assert_eq!(req.display_name, fixture.display_name);
    assert_eq!(
        req.description.as_deref(),
        Some(fixture.description.as_str())
    );
    // The release-blocking screen-pop contract: the correlation id is passed
    // to StartWebRTCContact under Amazon's expected attribute key.
    assert_eq!(
        req.attributes.get("correlation_id"),
        Some(&"corr-golden-0001".to_string())
    );
    assert_eq!(req.attributes, fixture.expected_attributes);
    assert!(!req.attributes.contains_key("Subject"));

    // And the failure was counted.
    assert_eq!(adapter.metrics().contacts_started, 1);
    assert_eq!(adapter.metrics().failures, 1);
}

#[test]
fn attributes_translate_into_a_btreemap_for_the_request() {
    let mapping = AttributeMapping::default();
    let headers: BTreeMap<String, String> = [("X-Foo".to_string(), "bar".to_string())]
        .into_iter()
        .collect();
    let mapped = mapping.translate(headers);
    assert_eq!(mapped.attributes.get("Foo"), Some(&"bar".to_string()));
}

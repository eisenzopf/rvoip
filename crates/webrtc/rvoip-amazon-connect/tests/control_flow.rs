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
    let last = Arc::new(Mutex::new(None));
    let starter = Arc::new(MockStarter {
        last: Arc::clone(&last),
        // Reserved-for-docs TLD: connect attempt fails quickly, after control.
        signaling_url: "wss://signal.invalid/control/m1".into(),
    });

    let config = ConnectConfig::new("instance-123", "flow-abc")
        .with_attribute_mapping(AttributeMapping::default().rename("X-Vapi-Customer-Id", "customerId"));
    let adapter = AmazonConnectAdapter::new(config, starter);

    // Translate a SIP custom-header set the way application glue would.
    let headers: Vec<(String, String)> = vec![
        ("X-Vapi-Customer-Id".into(), "cust-42".into()),
        ("X-Account-Tier".into(), "gold".into()),
        ("Subject".into(), "ignored".into()),
    ];
    let mapping = AttributeMapping::default().rename("X-Vapi-Customer-Id", "customerId");
    let mapped = mapping.translate(headers);

    // The media leg will fail (unreachable signaling URL), but control must run.
    let result = adapter
        .originate_contact(mapped.attributes.clone(), Some("Caller".into()), None)
        .await;
    assert!(result.is_err(), "expected signaling failure against invalid URL");

    let req = last.lock().take().expect("control plane was invoked");
    assert_eq!(req.instance_id, "instance-123");
    assert_eq!(req.contact_flow_id, "flow-abc");
    assert_eq!(req.display_name, "Caller");
    // Renamed + sanitized attributes flowed through to StartWebRTCContact.
    assert_eq!(req.attributes.get("customerId"), Some(&"cust-42".to_string()));
    assert_eq!(req.attributes.get("Account-Tier"), Some(&"gold".to_string()));
    assert!(!req.attributes.contains_key("Subject"));

    // And the failure was counted.
    assert_eq!(adapter.metrics().contacts_started, 1);
    assert_eq!(adapter.metrics().failures, 1);
}

#[test]
fn attributes_translate_into_a_btreemap_for_the_request() {
    let mapping = AttributeMapping::default();
    let headers: BTreeMap<String, String> =
        [("X-Foo".to_string(), "bar".to_string())].into_iter().collect();
    let mapped = mapping.translate(headers);
    assert_eq!(mapped.attributes.get("Foo"), Some(&"bar".to_string()));
}

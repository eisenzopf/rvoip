//! Real-substrate lifecycle regression: adapter-event backpressure must not
//! retain the WebSocket peer's admission permit.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rvoip_auth_core::bearer_stub;
use rvoip_core::adapter::{AdapterEvent, AdapterLifecycleSink, ConnectionAdapter};
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::{
    auth,
    connection::{ConnectionOffer, StreamOffer},
    session::SessionInvite,
};
use rvoip_uctp::types::MessageType;
use rvoip_websocket::{UctpWsAdapter, UctpWsClient, UctpWsConfig, ADAPTER_EVENT_CAP};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Notify};
use url::Url;

struct RecordingLifecycleSink {
    terminals: AtomicUsize,
    notified: Notify,
}

#[async_trait]
impl AdapterLifecycleSink for RecordingLifecycleSink {
    async fn deliver_terminal(&self, event: AdapterEvent) {
        assert!(matches!(event, AdapterEvent::Ended { .. }));
        self.terminals.fetch_add(1, Ordering::AcqRel);
        self.notified.notify_one();
    }
}

async fn authenticate(client: &Arc<UctpWsClient>) -> mpsc::Receiver<UctpEnvelope> {
    let mut inbound = client.take_inbound().expect("take inbound once");
    client
        .send(UctpEnvelope::new(
            MessageType::AuthHello,
            serde_json::to_value(auth::AuthHello {
                device: auth::Device {
                    id: format!("dev_{}", uuid::Uuid::new_v4().simple()),
                    kind: "browser".into(),
                    platform: "test".into(),
                    sdk_version: "lifecycle-capacity/0.1".into(),
                },
                auth_methods: vec!["bearer".into()],
                capabilities: serde_json::Value::Object(Default::default()),
            })
            .expect("encode auth hello"),
        ))
        .await
        .expect("send auth hello");
    let challenge = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("auth challenge timeout")
        .expect("peer closed before auth challenge");
    assert_eq!(challenge.msg_type, MessageType::AuthChallenge);
    client
        .send(
            UctpEnvelope::new(
                MessageType::AuthResponse,
                serde_json::to_value(auth::AuthResponse {
                    method: "bearer".into(),
                    credential: "test-token".into(),
                    actor_token: None,
                })
                .expect("encode auth response"),
            )
            .with_in_reply_to(challenge.id),
        )
        .await
        .expect("send auth response");
    let session = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("auth session timeout")
        .expect("peer closed before auth session");
    assert_eq!(session.msg_type, MessageType::AuthSession);
    inbound
}

async fn connect_eventually(url: &Url) -> Arc<UctpWsClient> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        match tokio::time::timeout(Duration::from_millis(250), UctpWsClient::connect(url)).await {
            Ok(Ok(client)) => return client,
            _ if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Ok(Err(error)) => panic!("second WebSocket peer was not admitted: {error}"),
            Err(error) => panic!("second WebSocket peer admission timed out: {error}"),
        }
    }
}

#[tokio::test]
async fn full_adapter_event_queue_releases_permit_for_second_peer() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let server_addr = listener.local_addr().expect("local address");
    let mut config = UctpWsConfig::new(listener, bearer_stub());
    config.max_concurrent_connections = 1;
    let adapter = UctpWsAdapter::new(config).await.expect("adapter");

    let sink = Arc::new(RecordingLifecycleSink {
        terminals: AtomicUsize::new(0),
        notified: Notify::new(),
    });
    adapter
        .install_lifecycle_sink(sink.clone())
        .expect("install lifecycle sink");
    // Hold the sole receiver without draining it after route setup. Quality
    // events below fill this real adapter queue; terminal delivery must use
    // the direct lifecycle sink and still release the connection semaphore.
    let mut events = adapter.subscribe_events();

    let url = Url::parse(&format!("ws://{server_addr}")).expect("WebSocket URL");
    let first = UctpWsClient::connect(&url)
        .await
        .expect("first peer connect");
    let mut first_inbound = authenticate(&first).await;
    let sid = "sess_capacity";
    let wire_connid = "conn_capacity";
    first
        .send(
            UctpEnvelope::new(
                MessageType::SessionInvite,
                serde_json::to_value(SessionInvite {
                    from: "untrusted-client-value".into(),
                    to: vec!["server".into()],
                    medium: "voice".into(),
                    intent: "synchronous-engagement".into(),
                    capabilities_offer: serde_json::Value::Object(Default::default()),
                })
                .expect("encode invite"),
            )
            .with_sid(sid),
        )
        .await
        .expect("send invite");

    let _core_connection_id = loop {
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("inbound adapter event timeout")
            .expect("adapter event channel closed");
        if let AdapterEvent::InboundConnection { connection } = event {
            break connection.id;
        }
    };

    first
        .send(
            UctpEnvelope::new(
                MessageType::ConnectionOffer,
                serde_json::to_value(ConnectionOffer {
                    by_participant: "untrusted-client-value".into(),
                    substrate: "websocket".into(),
                    capabilities: serde_json::Value::Object(Default::default()),
                    streams_offered: vec![StreamOffer {
                        id: "strm_capacity".into(),
                        kind: "audio".into(),
                        direction: "sendrecv".into(),
                        codec_preferences: vec!["opus".into()],
                    }],
                    substrate_setup: serde_json::Value::Null,
                })
                .expect("encode offer"),
            )
            .with_sid(sid)
            .with_connid(wire_connid),
        )
        .await
        .expect("send offer");

    // Quality is explicitly best-effort at the adapter boundary. Sending more
    // than the channel capacity deterministically leaves the queue full while
    // allowing the event translator to continue to the terminal event.
    for index in 0..(ADAPTER_EVENT_CAP + 32) {
        first
            .send(
                UctpEnvelope::new(
                    MessageType::ConnectionQuality,
                    serde_json::json!({
                        "interval_ms": 20,
                        "streams": [{
                            "strm_id": "strm_capacity",
                            "loss_pct": 0.0,
                            "jitter_ms": 1,
                            "rtt_ms": 2,
                            "mos": 4.5,
                            "bitrate_bps": 32_000,
                            "packets_sent": index,
                            "packets_received": index
                        }]
                    }),
                )
                .with_sid(sid)
                .with_connid(wire_connid),
            )
            .await
            .expect("send quality event");
    }
    first
        .send(
            UctpEnvelope::new(
                MessageType::ConnectionEnd,
                serde_json::json!({"reason_code": 0, "reason": "test-complete"}),
            )
            .with_sid(sid)
            .with_connid(wire_connid),
        )
        .await
        .expect("send connection end");

    tokio::time::timeout(Duration::from_secs(5), sink.notified.notified())
        .await
        .expect("terminal fallback was not invoked");
    assert_eq!(sink.terminals.load(Ordering::Acquire), 1);

    // Drain signaling replies until the first server-side WebSocket closes.
    tokio::time::timeout(Duration::from_secs(5), async {
        while first_inbound.recv().await.is_some() {}
    })
    .await
    .expect("first peer did not close");
    drop(first);

    let second = connect_eventually(&url).await;
    let mut second_inbound = second.take_inbound().expect("second inbound");
    second
        .send(UctpEnvelope::new(
            MessageType::AuthHello,
            serde_json::to_value(auth::AuthHello {
                device: auth::Device {
                    id: "dev_second".into(),
                    kind: "browser".into(),
                    platform: "test".into(),
                    sdk_version: "lifecycle-capacity/0.1".into(),
                },
                auth_methods: vec!["bearer".into()],
                capabilities: serde_json::Value::Object(Default::default()),
            })
            .expect("encode second auth hello"),
        ))
        .await
        .expect("send second auth hello");
    let second_challenge = tokio::time::timeout(Duration::from_secs(2), second_inbound.recv())
        .await
        .expect("second peer auth challenge timeout")
        .expect("second peer was closed before auth challenge");
    assert_eq!(second_challenge.msg_type, MessageType::AuthChallenge);
}

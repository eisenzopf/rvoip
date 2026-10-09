//! Listener ownership: drain preserves peers, shutdown joins cleanup, Drop cancels admission.
use async_trait::async_trait;
use rvoip_auth_core::{bearer_stub, BearerAuthError, BearerValidator};
use rvoip_core::identity::IdentityAssurance;
use rvoip_uctp::{envelope::UctpEnvelope, payloads::auth, types::MessageType};
use rvoip_websocket::{UctpWsAdapter, UctpWsClient, UctpWsConfig};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, Semaphore},
};
use url::Url;

async fn listener() -> (Arc<UctpWsAdapter>, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut config = UctpWsConfig::new(listener, bearer_stub());
    config.max_concurrent_connections = 1;
    let adapter = UctpWsAdapter::new(config).await.unwrap();
    (adapter, address)
}

async fn confirm_slot_occupied(address: SocketAddr) {
    // The server can only reject this upgraded probe for capacity after it
    // has accepted the first raw socket and assigned that socket a permit.
    let probe = tokio::time::timeout(
        Duration::from_secs(2),
        UctpWsClient::connect(&Url::parse(&format!("ws://{address}")).unwrap()),
    )
    .await
    .expect("server must reject the capacity probe promptly");
    assert!(
        probe.is_err(),
        "unfinished first handshake must occupy the sole slot"
    );
}

async fn rebind(address: SocketAddr) -> TcpListener {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(listener) = TcpListener::bind(address).await {
                return listener;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("listener must release its port")
}

async fn receive(inbound: &mut mpsc::Receiver<UctpEnvelope>) -> UctpEnvelope {
    tokio::time::timeout(Duration::from_secs(2), inbound.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn hello(client: &UctpWsClient, inbound: &mut mpsc::Receiver<UctpEnvelope>) -> UctpEnvelope {
    client
        .send(UctpEnvelope::new(
            MessageType::AuthHello,
            serde_json::to_value(auth::AuthHello {
                device: auth::Device {
                    id: "dev_drain".into(),
                    kind: "desktop".into(),
                    platform: "test".into(),
                    sdk_version: "test/1".into(),
                },
                auth_methods: vec!["bearer".into()],
                capabilities: serde_json::json!({}),
            })
            .unwrap(),
        ))
        .await
        .unwrap();
    let challenge = receive(inbound).await;
    assert_eq!(challenge.msg_type, MessageType::AuthChallenge);
    challenge
}

fn response(challenge: &UctpEnvelope) -> UctpEnvelope {
    UctpEnvelope::new(
        MessageType::AuthResponse,
        serde_json::to_value(auth::AuthResponse {
            method: "bearer".into(),
            credential: "test-token".into(),
            actor_token: None,
        })
        .unwrap(),
    )
    .with_in_reply_to(challenge.id.clone())
}

#[tokio::test]
async fn drain_releases_listener_and_keeps_existing_peer_usable() {
    let (adapter, address) = listener().await;
    let client = UctpWsClient::connect(&Url::parse(&format!("ws://{address}")).unwrap())
        .await
        .unwrap();
    let mut inbound = client.take_inbound().unwrap();
    let challenge = hello(&client, &mut inbound).await;
    assert!(!adapter.is_draining());
    adapter.begin_drain();
    assert!(adapter.is_draining());
    let _replacement = rebind(address).await;
    // Authentication on the existing socket must still work after drain.
    client.send(response(&challenge)).await.unwrap();
    assert_eq!(
        receive(&mut inbound).await.msg_type,
        MessageType::AuthSession
    );
    let (first, second) = tokio::join!(
        adapter.shutdown(Duration::from_secs(2)),
        adapter.shutdown(Duration::from_secs(2))
    );
    assert!(first && second);
    assert!(tokio::time::timeout(Duration::from_secs(2), inbound.recv())
        .await
        .unwrap()
        .is_none());
    assert!(adapter.shutdown(Duration::from_secs(1)).await);
}

#[tokio::test]
async fn shutdown_cancels_an_incomplete_websocket_upgrade() {
    let (adapter, address) = listener().await;
    let _unfinished = TcpStream::connect(address).await.unwrap();
    confirm_slot_occupied(address).await;
    assert!(adapter.shutdown(Duration::from_secs(2)).await);
    let _replacement = rebind(address).await;
}

#[tokio::test]
async fn dropping_adapter_cancels_its_listener_and_connected_peer() {
    let (adapter, address) = listener().await;
    let client = UctpWsClient::connect(&Url::parse(&format!("ws://{address}")).unwrap())
        .await
        .unwrap();
    let mut inbound = client.take_inbound().unwrap();
    hello(&client, &mut inbound).await;
    drop(adapter);
    let _replacement = rebind(address).await;
    assert!(tokio::time::timeout(Duration::from_secs(2), inbound.recv())
        .await
        .unwrap()
        .is_none());
}

struct BlockingValidator {
    entered: Arc<Semaphore>,
    exited: Arc<Semaphore>,
}
struct ExitGuard(Arc<Semaphore>);
impl Drop for ExitGuard {
    fn drop(&mut self) {
        self.0.add_permits(1);
    }
}
#[async_trait]
impl BearerValidator for BlockingValidator {
    async fn validate(&self, _token: &str) -> Result<IdentityAssurance, BearerAuthError> {
        let _guard = ExitGuard(self.exited.clone());
        self.entered.add_permits(1);
        std::future::pending().await
    }
}

#[tokio::test]
async fn timeout_reports_incomplete_cleanup_and_later_wait_joins_it() {
    let entered = Arc::new(Semaphore::new(0));
    let exited = Arc::new(Semaphore::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut config = UctpWsConfig::new(
        listener,
        Arc::new(BlockingValidator {
            entered: entered.clone(),
            exited: exited.clone(),
        }),
    );
    config.coordinator_caps.signaling_send_timeout = Duration::from_millis(150);
    let adapter = UctpWsAdapter::new(config).await.unwrap();
    let client = UctpWsClient::connect(&Url::parse(&format!("ws://{address}")).unwrap())
        .await
        .unwrap();
    let mut inbound = client.take_inbound().unwrap();
    let challenge = hello(&client, &mut inbound).await;
    client.send(response(&challenge)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    // A stalled handler prevents orderly coordinator drain. The caller's
    // short budget must not falsely claim completion or detach peer work.
    assert!(!adapter.shutdown(Duration::from_millis(1)).await);
    assert!(adapter.shutdown(Duration::from_secs(2)).await);
    assert_eq!(
        exited.available_permits(),
        1,
        "validator future was dropped before successful shutdown"
    );
    let _replacement = rebind(address).await;
}

#[cfg(feature = "wss")]
#[tokio::test]
async fn shutdown_cancels_an_incomplete_tls_handshake() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (cert, key) = rvoip_uctp::substrate::self_signed_for_dev(&["localhost".into()]).unwrap();
    let tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut config = UctpWsConfig::new(listener, bearer_stub()).with_tls(Arc::new(tls));
    config.max_concurrent_connections = 1;
    let adapter = UctpWsAdapter::new(config).await.unwrap();
    let _unfinished = TcpStream::connect(address).await.unwrap();
    confirm_slot_occupied(address).await;
    assert!(adapter.shutdown(Duration::from_secs(2)).await);
    let _replacement = rebind(address).await;
}

#[tokio::test]
async fn shutdown_retires_inbound_route_and_reports_its_terminal_event() {
    use rvoip_core::adapter::{AdapterEvent, ConnectionAdapter, OrchestratorAdapterEvent};
    use rvoip_uctp::payloads::session::SessionInvite;
    let (adapter, address) = listener().await;
    let mut events = adapter.subscribe_orchestrator_events();
    let client = UctpWsClient::connect(&Url::parse(&format!("ws://{address}")).unwrap())
        .await
        .unwrap();
    let mut inbound = client.take_inbound().unwrap();
    let challenge = hello(&client, &mut inbound).await;
    client.send(response(&challenge)).await.unwrap();
    assert_eq!(
        receive(&mut inbound).await.msg_type,
        MessageType::AuthSession
    );
    client
        .send(
            UctpEnvelope::new(
                MessageType::SessionInvite,
                serde_json::to_value(SessionInvite {
                    from: "untrusted-peer".into(),
                    to: vec!["server".into()],
                    medium: "voice".into(),
                    intent: "synchronous-engagement".into(),
                    capabilities_offer: serde_json::json!({}),
                })
                .unwrap(),
            )
            .with_sid("sess_shutdown")
            .with_cid("conv_shutdown"),
        )
        .await
        .unwrap();
    let connection_id = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let OrchestratorAdapterEvent::AuthenticatedInboundConnection { connection, .. } =
                events.recv().await.unwrap()
            {
                return connection.id;
            }
        }
    })
    .await
    .unwrap();
    assert!(adapter.is_connection_live(&connection_id));
    assert!(adapter.shutdown(Duration::from_secs(2)).await);
    assert!(!adapter.is_connection_live(&connection_id));
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let OrchestratorAdapterEvent::Public(AdapterEvent::Ended {
                connection_id: ended,
                ..
            }) = events.recv().await.unwrap()
            {
                assert_eq!(ended, connection_id);
                return;
            }
        }
    })
    .await
    .expect("shutdown must deliver a terminal event for the exact retired route");
}

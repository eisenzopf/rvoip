//! Per-call outbound WSS client identity tests for `WebSocketTransport`.
//!
//! WSS mirror of `tls_per_call_identity_test.rs` — verifies the same
//! connection-pool-keying change applied to `WebSocketTransport`, so a
//! single instance can place calls under different client identities/trust
//! policies instead of being locked to whatever `TlsClientConfig` (if any)
//! it was constructed with.

#![cfg(feature = "wss")]

use std::io::Write;
use std::net::SocketAddr;
use std::time::Duration;

use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::{Message, Method};
use rvoip_sip_transport::transport::tls::{OutboundTlsConfig, TlsClientConfig};
use rvoip_sip_transport::transport::ws::WebSocketTransport;
use rvoip_sip_transport::{Transport, TransportEvent};
use tempfile::tempdir;

fn write_self_signed_cert_for_names(
    names: Vec<String>,
) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempdir().expect("tempdir");
    let cert_path = dir.path().join("server.crt");
    let key_path = dir.path().join("server.key");

    let cert = rcgen::generate_simple_self_signed(names).expect("rcgen self-signed");
    let cert_pem = cert.cert.pem();
    let key_pem = cert.signing_key.serialize_pem();

    std::fs::File::create(&cert_path)
        .and_then(|mut f| f.write_all(cert_pem.as_bytes()))
        .expect("write cert");
    std::fs::File::create(&key_path)
        .and_then(|mut f| f.write_all(key_pem.as_bytes()))
        .expect("write key");

    (dir, cert_path, key_path)
}

fn loopback_addr(port: u16) -> SocketAddr {
    format!("127.0.0.1:{}", port).parse().unwrap()
}

fn register_request(call_id: &str) -> Message {
    let request = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com")
        .unwrap()
        .from("alice", "sip:alice@example.com", Some("tag-per-call-wss"))
        .to("alice", "sip:alice@example.com", None)
        .call_id(call_id)
        .cseq(1)
        .build();
    Message::Request(request)
}

/// Two concurrent outbound calls from a single `WebSocketTransport`, each
/// using a distinct per-call identity (a different trusted CA) to reach a
/// distinct WSS server. The client transport is bound with no baked-in
/// client TLS config at all — both dials succeed purely from their
/// per-call `OutboundTlsConfig` override.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_concurrent_calls_use_distinct_identities_to_distinct_servers() {
    let _ = tracing_subscriber::fmt::try_init();

    let (_dir_a, cert_a, key_a) =
        write_self_signed_cert_for_names(vec!["wss-server-a.example.test".to_string()]);
    let (_dir_b, cert_b, key_b) =
        write_self_signed_cert_for_names(vec!["wss-server-b.example.test".to_string()]);
    let (_dir_client, client_cert, client_key) =
        write_self_signed_cert_for_names(vec!["wss-client.example.test".to_string()]);

    let (server_a, mut events_a) = WebSocketTransport::bind(
        loopback_addr(0),
        true,
        Some(cert_a.to_str().unwrap()),
        Some(key_a.to_str().unwrap()),
        None,
    )
    .await
    .expect("server A bind");
    let addr_a = server_a.local_addr().expect("server A addr");

    let (server_b, mut events_b) = WebSocketTransport::bind(
        loopback_addr(0),
        true,
        Some(cert_b.to_str().unwrap()),
        Some(key_b.to_str().unwrap()),
        None,
    )
    .await
    .expect("server B bind");
    let addr_b = server_b.local_addr().expect("server B addr");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Client transport: secure=true (so per-call WSS dials are eligible)
    // but no baked-in `TlsClientConfig` — plain `bind()`, not
    // `bind_with_client_tls()`. Its own cert/key are only needed because
    // this transport also has a listener; it never accepts a connection
    // in this test.
    let (client, _client_rx) = WebSocketTransport::bind(
        loopback_addr(0),
        true,
        Some(client_cert.to_str().unwrap()),
        Some(client_key.to_str().unwrap()),
        None,
    )
    .await
    .expect("client bind");

    let identity_a = OutboundTlsConfig {
        client: TlsClientConfig {
            extra_ca_path: Some(cert_a.clone()),
            insecure_skip_verify: false,
            ..Default::default()
        },
        server_name: Some("wss-server-a.example.test".to_string()),
    };
    let identity_b = OutboundTlsConfig {
        client: TlsClientConfig {
            extra_ca_path: Some(cert_b.clone()),
            insecure_skip_verify: false,
            ..Default::default()
        },
        server_name: Some("wss-server-b.example.test".to_string()),
    };

    let (result_a, result_b) = tokio::join!(
        client.send_message_with_tls_identity(
            register_request("wss-call-a"),
            addr_a,
            Some(&identity_a)
        ),
        client.send_message_with_tls_identity(
            register_request("wss-call-b"),
            addr_b,
            Some(&identity_b)
        ),
    );
    result_a.expect("identity A call must succeed against server A");
    result_b.expect("identity B call must succeed against server B");

    let received_a = tokio::time::timeout(Duration::from_secs(5), events_a.recv())
        .await
        .expect("server A timed out")
        .expect("server A channel closed");
    let received_b = tokio::time::timeout(Duration::from_secs(5), events_b.recv())
        .await
        .expect("server B timed out")
        .expect("server B channel closed");

    match received_a {
        TransportEvent::MessageReceived { message, .. } => match message {
            Message::Request(req) => {
                assert_eq!(req.call_id().unwrap().to_string(), "wss-call-a");
            }
            other => panic!("server A: expected request, got {:?}", other),
        },
        other => panic!("server A: unexpected event: {:?}", other),
    }
    match received_b {
        TransportEvent::MessageReceived { message, .. } => match message {
            Message::Request(req) => {
                assert_eq!(req.call_id().unwrap().to_string(), "wss-call-b");
            }
            other => panic!("server B: expected request, got {:?}", other),
        },
        other => panic!("server B: unexpected event: {:?}", other),
    }
}

/// Same destination, two different trust policies. Proves the WSS
/// connection pool is keyed by `(address, identity)` and not address
/// alone: a connection validated under a trusting identity must not be
/// handed to a send under a non-trusting identity for the same
/// destination.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn distinct_identities_to_same_destination_do_not_share_a_pooled_connection() {
    let (_dir, cert_path, key_path) =
        write_self_signed_cert_for_names(vec!["wss-shared.example.test".to_string()]);
    let (_dir_client, client_cert, client_key) =
        write_self_signed_cert_for_names(vec!["wss-client2.example.test".to_string()]);

    let (server, mut server_events) = WebSocketTransport::bind(
        loopback_addr(0),
        true,
        Some(cert_path.to_str().unwrap()),
        Some(key_path.to_str().unwrap()),
        None,
    )
    .await
    .expect("server bind");
    let server_addr = server.local_addr().expect("server addr");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let (client, _client_rx) = WebSocketTransport::bind(
        loopback_addr(0),
        true,
        Some(client_cert.to_str().unwrap()),
        Some(client_key.to_str().unwrap()),
        None,
    )
    .await
    .expect("client bind");

    let trusting_identity = OutboundTlsConfig {
        client: TlsClientConfig {
            extra_ca_path: Some(cert_path.clone()),
            insecure_skip_verify: false,
            ..Default::default()
        },
        server_name: Some("wss-shared.example.test".to_string()),
    };
    client
        .send_message_with_tls_identity(
            register_request("wss-call-trusting"),
            server_addr,
            Some(&trusting_identity),
        )
        .await
        .expect("trusting identity must complete the handshake");

    let received = tokio::time::timeout(Duration::from_secs(5), server_events.recv())
        .await
        .expect("server timed out on first call")
        .expect("server channel closed");
    match received {
        TransportEvent::MessageReceived { message, .. } => match message {
            Message::Request(req) => {
                assert_eq!(req.call_id().unwrap().to_string(), "wss-call-trusting");
            }
            other => panic!("expected request, got {:?}", other),
        },
        other => panic!("unexpected event: {:?}", other),
    }

    let non_trusting_identity = OutboundTlsConfig {
        client: TlsClientConfig::default(),
        server_name: Some("wss-shared.example.test".to_string()),
    };
    let result = client
        .send_message_with_tls_identity(
            register_request("wss-call-non-trusting"),
            server_addr,
            Some(&non_trusting_identity),
        )
        .await;

    assert!(
        result.is_err(),
        "a distrusting identity must not reuse another identity's already-validated \
         connection to the same destination: {:?}",
        result
    );
}

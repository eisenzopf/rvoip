//! Per-call outbound TLS client identity tests for `TlsTransport`.
//!
//! Verifies the connection-pool-keying change that lets one `TlsTransport`
//! instance place calls under different client identities/trust policies
//! instead of being locked to whatever `TlsClientConfig` it was constructed
//! with. Companion to `tls_handshake_test.rs`, which only exercises the
//! transport's baked-in default identity.

use std::io::Write;
use std::net::SocketAddr;
use std::time::Duration;

use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::Method;
use rvoip_sip_transport::transport::tls::{OutboundTlsConfig, TlsClientConfig, TlsTransport};
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

fn register_request(uri: &str, call_id: &str) -> rvoip_sip_core::Message {
    let request = SimpleRequestBuilder::new(Method::Register, uri)
        .unwrap()
        .from("alice", "sip:alice@example.com", Some("tag-per-call-tls"))
        .to("alice", "sip:alice@example.com", None)
        .call_id(call_id)
        .cseq(1)
        .build();
    rvoip_sip_core::Message::Request(request)
}

/// Two concurrent outbound calls from a single `TlsTransport`, each using a
/// distinct per-call identity (a different trusted CA) to reach a distinct
/// server. Proves the per-call `OutboundTlsConfig` override actually
/// isolates identities: neither call's trust policy leaks into the other,
/// and both complete concurrently against one shared transport instance.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_concurrent_calls_use_distinct_identities_to_distinct_servers() {
    let _ = tracing_subscriber::fmt::try_init();

    let (_dir_a, cert_a, key_a) =
        write_self_signed_cert_for_names(vec!["server-a.example.test".to_string()]);
    let (_dir_b, cert_b, key_b) =
        write_self_signed_cert_for_names(vec!["server-b.example.test".to_string()]);

    let (tx_a, mut events_a) = tokio::sync::mpsc::channel(16);
    let (server_a, _rx_a) = TlsTransport::bind(loopback_addr(0), &cert_a, &key_a, Some(tx_a))
        .await
        .expect("server A bind");
    let addr_a = server_a.local_addr().expect("server A addr");

    let (tx_b, mut events_b) = tokio::sync::mpsc::channel(16);
    let (server_b, _rx_b) = TlsTransport::bind(loopback_addr(0), &cert_b, &key_b, Some(tx_b))
        .await
        .expect("server B bind");
    let addr_b = server_b.local_addr().expect("server B addr");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // One client-only transport, no baked-in identity — both calls ride
    // per-call `OutboundTlsConfig` overrides instead.
    let (client, _client_rx) =
        TlsTransport::client_only(loopback_addr(0), None, TlsClientConfig::default())
            .await
            .expect("client bind");

    let identity_a = OutboundTlsConfig {
        client: TlsClientConfig {
            extra_ca_path: Some(cert_a.clone()),
            insecure_skip_verify: false,
            ..Default::default()
        },
        server_name: Some("server-a.example.test".to_string()),
    };
    let identity_b = OutboundTlsConfig {
        client: TlsClientConfig {
            extra_ca_path: Some(cert_b.clone()),
            insecure_skip_verify: false,
            ..Default::default()
        },
        server_name: Some("server-b.example.test".to_string()),
    };

    let msg_a = register_request("sip:server-a.example.test", "call-a");
    let msg_b = register_request("sip:server-b.example.test", "call-b");

    let (result_a, result_b) = tokio::join!(
        client.send_message_with_tls_identity(msg_a, addr_a, Some(&identity_a)),
        client.send_message_with_tls_identity(msg_b, addr_b, Some(&identity_b)),
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
            rvoip_sip_core::Message::Request(req) => {
                assert_eq!(
                    req.call_id().map(|c| c.to_string()).as_deref(),
                    Some("call-a")
                );
            }
            other => panic!("server A: expected request, got {:?}", other),
        },
        other => panic!("server A: unexpected event: {:?}", other),
    }
    match received_b {
        TransportEvent::MessageReceived { message, .. } => match message {
            rvoip_sip_core::Message::Request(req) => {
                assert_eq!(
                    req.call_id().map(|c| c.to_string()).as_deref(),
                    Some("call-b")
                );
            }
            other => panic!("server B: expected request, got {:?}", other),
        },
        other => panic!("server B: unexpected event: {:?}", other),
    }
}

/// Same destination, two different trust policies. Proves the connection
/// pool is keyed by `(address, identity)` and not address alone: a
/// connection established and validated under a trusting identity must
/// never be handed to a send under a different (here, non-trusting)
/// identity for the same destination. If the pool keyed on address only,
/// the second send would incorrectly reuse the first identity's
/// already-validated connection and succeed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn distinct_identities_to_same_destination_do_not_share_a_pooled_connection() {
    let (_dir, cert_path, key_path) =
        write_self_signed_cert_for_names(vec!["shared.example.test".to_string()]);

    let (server_tx, mut server_events) = tokio::sync::mpsc::channel(16);
    let (server, _server_rx) =
        TlsTransport::bind(loopback_addr(0), &cert_path, &key_path, Some(server_tx))
            .await
            .expect("server bind");
    let server_addr = server.local_addr().expect("server addr");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let (client, _client_rx) =
        TlsTransport::client_only(loopback_addr(0), None, TlsClientConfig::default())
            .await
            .expect("client bind");

    // Identity that trusts the self-signed cert — first send must succeed.
    let trusting_identity = OutboundTlsConfig {
        client: TlsClientConfig {
            extra_ca_path: Some(cert_path.clone()),
            insecure_skip_verify: false,
            ..Default::default()
        },
        server_name: Some("shared.example.test".to_string()),
    };
    client
        .send_message_with_tls_identity(
            register_request("sip:shared.example.test", "call-trusting"),
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
            rvoip_sip_core::Message::Request(req) => {
                assert_eq!(
                    req.call_id().map(|c| c.to_string()).as_deref(),
                    Some("call-trusting")
                );
            }
            other => panic!("expected request, got {:?}", other),
        },
        other => panic!("unexpected event: {:?}", other),
    }

    // Identity that does NOT trust the self-signed cert (no extra CA) —
    // same destination. Must dial fresh and fail validation rather than
    // silently reusing the already-open, already-trusted connection above.
    let non_trusting_identity = OutboundTlsConfig {
        client: TlsClientConfig::default(),
        server_name: Some("shared.example.test".to_string()),
    };
    let result = client
        .send_message_with_tls_identity(
            register_request("sip:shared.example.test", "call-non-trusting"),
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

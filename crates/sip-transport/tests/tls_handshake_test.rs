//! End-to-end TLS handshake test for `TlsTransport`.
//!
//! Generates a self-signed RSA cert via `rcgen`, binds a server-side
//! `TlsTransport` on it, then has a client-side `TlsTransport` (with
//! `insecure_skip_verify=true` to accept the self-signed cert) dial it
//! and exchange a real SIP REGISTER request. Asserts the server's
//! `TransportEvent::MessageReceived` carries the original method.
//!
//! This is the regression check for **Step 1B** of the TLS roadmap
//! (`crates/TLS_SIP_IMPLEMENTATION_PLAN.md`): the client connector at
//! `transport/tls/mod.rs:TlsTransport::connect` actually completes a
//! handshake against a real rustls server and pumps SIP bytes through.

use std::io::Write;
use std::net::SocketAddr;
use std::time::Duration;

use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::Method;
use rvoip_sip_transport::transport::tls::{TlsClientConfig, TlsTransport};
use rvoip_sip_transport::{Transport, TransportEvent};
use tempfile::tempdir;

/// Generate a self-signed cert for `localhost` and write it + the key
/// out as PEM files in a temp dir; return the dir handle (so tempfiles
/// live for the test) and the two paths.
fn write_self_signed_localhost_cert() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf)
{
    let dir = tempdir().expect("tempdir");
    let cert_path = dir.path().join("server.crt");
    let key_path = dir.path().join("server.key");

    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
        .expect("rcgen self-signed");
    let cert_pem = cert.serialize_pem().expect("cert PEM");
    let key_pem = cert.serialize_private_key_pem();

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tls_client_connect_and_send_succeeds_against_self_signed_server() {
    let _ = tracing_subscriber::fmt::try_init();

    let (_dir, cert_path, key_path) = write_self_signed_localhost_cert();

    // Server side: bind on an ephemeral port with the self-signed cert,
    // strict server-side TLS verification (no client auth, default
    // server config).
    let server_addr = loopback_addr(0);
    let (server_tx, mut server_events) = tokio::sync::mpsc::channel(16);
    let (server_transport, _server_rx_unused) =
        TlsTransport::bind(server_addr, &cert_path, &key_path, Some(server_tx))
            .await
            .expect("server bind");
    let server_actual = server_transport.local_addr().expect("server local addr");

    // Give the listener a moment to actually bind.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Client side: another TlsTransport on a different ephemeral port,
    // built with insecure_skip_verify=true so it accepts our
    // self-signed cert (production deployments load real certs via
    // `extra_ca_path` or a system trust store; see
    // `crates/TLS_SIP_IMPLEMENTATION_PLAN.md` Step 1C).
    let client_addr = loopback_addr(0);
    let (client_transport, _client_rx) = TlsTransport::bind_with_client_config(
        client_addr,
        &cert_path,
        &key_path,
        None,
        TlsClientConfig {
            extra_ca_path: None,
            insecure_skip_verify: true,
        },
    )
    .await
    .expect("client bind");

    // Build a real SIP REGISTER and send it. The TlsTransport's
    // auto-dial path (inside `send_to_addr`) should perform the TLS
    // handshake and push the message bytes over the encrypted stream.
    let request = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com")
        .unwrap()
        .from("alice", "sip:alice@example.com", Some("tag-tls"))
        .to("alice", "sip:alice@example.com", None)
        .call_id("tls-handshake-test")
        .cseq(1)
        .build();

    client_transport
        .send_message(rvoip_sip_core::Message::Request(request), server_actual)
        .await
        .expect("client send via TLS");

    // Server should observe a MessageReceived for our REGISTER.
    let received = tokio::time::timeout(Duration::from_secs(5), server_events.recv())
        .await
        .expect("server timed out waiting for inbound TLS message")
        .expect("server event channel closed");

    match received {
        TransportEvent::MessageReceived { message, .. } => match message {
            rvoip_sip_core::Message::Request(req) => {
                assert_eq!(req.method(), Method::Register);
                assert_eq!(req.uri().to_string(), "sip:registrar.example.com");
            }
            other => panic!("expected request, got {:?}", other),
        },
        other => panic!("unexpected event: {:?}", other),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tls_client_default_validation_rejects_self_signed_cert() {
    // Inverse of the above: with `insecure_skip_verify = false`
    // (default), a self-signed cert that isn't in any trust store must
    // cause the handshake to fail.
    let (_dir, cert_path, key_path) = write_self_signed_localhost_cert();

    let (server_transport, _server_rx) =
        TlsTransport::bind(loopback_addr(0), &cert_path, &key_path, None)
            .await
            .expect("server bind");
    let server_actual = server_transport.local_addr().expect("server local addr");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let (client_transport, _client_rx) =
        TlsTransport::bind(loopback_addr(0), &cert_path, &key_path, None)
            .await
            .expect("client bind");

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        client_transport.connect(server_actual),
    )
    .await
    .expect("connect did not return in time");

    assert!(
        result.is_err(),
        "default validation accepted a self-signed cert: {:?}",
        result
    );
}

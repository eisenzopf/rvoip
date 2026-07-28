//! End-to-end TLS handshake test for `TlsTransport`.
//!
//! Generates a self-signed RSA cert via `rcgen`, binds a server-side
//! `TlsTransport` on it, then has a client-only `TlsTransport` (with
//! the same cert added as an extra CA so default validation accepts it)
//! dial it without any local endpoint cert/key and exchange a real SIP
//! REGISTER request. Asserts the server's `TransportEvent::MessageReceived`
//! carries the original method.
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
use rvoip_sip_transport::{HandshakeAdmissionConfig, Transport, TransportEvent};
use tempfile::tempdir;
use tokio::io::AsyncReadExt;

/// Generate a self-signed cert for `localhost` and write it + the key
/// out as PEM files in a temp dir; return the dir handle (so tempfiles
/// live for the test) and the two paths.
fn write_self_signed_localhost_cert() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf)
{
    write_self_signed_cert_for_names(vec!["localhost".to_string()])
}

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tls_client_connect_and_send_succeeds_against_self_signed_server() {
    let _ = tracing_subscriber::fmt::try_init();

    // Cert must cover both "localhost" (loopback SNI fallback) and the
    // request URI's host — `tls_server_name_for_message` derives SNI
    // from the URI when the host is a domain, so the cert needs the
    // matching SAN.
    let (_dir, cert_path, key_path) = write_self_signed_cert_for_names(vec![
        "localhost".to_string(),
        "registrar.example.com".to_string(),
    ]);

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

    // Client side: client-only TLS, with no local endpoint cert/key.
    // It accepts our self-signed server cert by trusting the cert file
    // as an extra CA — the loopback SNI path resolves to "localhost",
    // which matches the cert's SAN.
    let client_addr = loopback_addr(0);
    let (client_transport, _client_rx) = TlsTransport::client_only(
        client_addr,
        None,
        TlsClientConfig {
            extra_ca_path: Some(cert_path.clone()),
            insecure_skip_verify: false,
            ..Default::default()
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
        TlsTransport::client_only(loopback_addr(0), None, TlsClientConfig::default())
            .await
            .expect("client config");

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tls_client_uses_request_uri_host_for_sni_and_certificate_validation() {
    let (_dir, cert_path, key_path) =
        write_self_signed_cert_for_names(vec!["pbx.example.test".to_string()]);

    let (server_tx, mut server_events) = tokio::sync::mpsc::channel(16);
    let (server_transport, _server_rx) =
        TlsTransport::bind(loopback_addr(0), &cert_path, &key_path, Some(server_tx))
            .await
            .expect("server bind");
    let server_actual = server_transport.local_addr().expect("server local addr");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let (client_transport, _client_rx) = TlsTransport::client_only(
        loopback_addr(0),
        None,
        TlsClientConfig {
            extra_ca_path: Some(cert_path.clone()),
            insecure_skip_verify: false,
            ..Default::default()
        },
    )
    .await
    .expect("client config");

    let request =
        SimpleRequestBuilder::new(Method::Register, "sips:pbx.example.test;transport=tls")
            .unwrap()
            .from("alice", "sips:alice@example.test", Some("tag-tls-sni"))
            .to("alice", "sips:alice@example.test", None)
            .call_id("tls-sni-test")
            .cseq(1)
            .build();

    client_transport
        .send_message(rvoip_sip_core::Message::Request(request), server_actual)
        .await
        .expect("client send via TLS with URI-host SNI");

    let received = tokio::time::timeout(Duration::from_secs(5), server_events.recv())
        .await
        .expect("server timed out waiting for URI-host SNI request")
        .expect("server event channel closed");

    match received {
        TransportEvent::MessageReceived { message, .. } => match message {
            rvoip_sip_core::Message::Request(req) => {
                assert_eq!(req.method(), Method::Register);
                assert_eq!(req.uri().to_string(), "sips:pbx.example.test;transport=tls");
            }
            other => panic!("expected request, got {:?}", other),
        },
        other => panic!("unexpected event: {:?}", other),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tls_server_only_refuses_new_outbound_connections() {
    let (_dir, cert_path, key_path) = write_self_signed_localhost_cert();

    let (server_transport, _server_rx) = TlsTransport::bind_server_only_with_client_config(
        loopback_addr(0),
        &cert_path,
        &key_path,
        None,
        TlsClientConfig::default(),
    )
    .await
    .expect("server-only bind");

    let request = SimpleRequestBuilder::new(Method::Options, "sips:pbx.example.test;transport=tls")
        .unwrap()
        .from("alice", "sips:alice@example.test", Some("tag-server-only"))
        .to("pbx", "sips:pbx.example.test", None)
        .call_id("tls-server-only-test")
        .cseq(1)
        .build();

    let result = server_transport
        .send_message(
            rvoip_sip_core::Message::Request(request),
            loopback_addr(65000),
        )
        .await;

    assert!(
        result.is_err(),
        "server-only TLS transport unexpectedly opened an outbound connection"
    );
}

fn trusted_client_config(cert_path: &std::path::Path) -> TlsClientConfig {
    TlsClientConfig {
        extra_ca_path: Some(cert_path.to_path_buf()),
        insecure_skip_verify: false,
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_tls_client_hello_does_not_block_another_handshake() {
    let (_dir, cert_path, key_path) = write_self_signed_localhost_cert();
    let (server, _events) = TlsTransport::bind_with_handshake_config(
        loopback_addr(0),
        &cert_path,
        &key_path,
        None,
        HandshakeAdmissionConfig::new(Duration::from_secs(2), 2),
    )
    .await
    .expect("TLS bind");
    let address = server.local_addr().unwrap();

    let _slow_peer = tokio::net::TcpStream::connect(address)
        .await
        .expect("slow TCP peer");
    tokio::time::sleep(Duration::from_millis(50)).await;

    let (client, _client_events) =
        TlsTransport::client_only(loopback_addr(0), None, trusted_client_config(&cert_path))
            .await
            .expect("TLS client");
    tokio::time::timeout(Duration::from_millis(750), client.connect(address))
        .await
        .expect("valid TLS handshake was serialized behind slow peer")
        .expect("valid TLS handshake");

    client.close().await.unwrap();
    server.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tls_handshake_deadline_releases_saturated_admission() {
    let (_dir, cert_path, key_path) = write_self_signed_localhost_cert();
    let (server, _events) = TlsTransport::bind_with_handshake_config(
        loopback_addr(0),
        &cert_path,
        &key_path,
        None,
        HandshakeAdmissionConfig::new(Duration::from_millis(150), 1),
    )
    .await
    .expect("TLS bind");
    let address = server.local_addr().unwrap();

    let _slow_peer = tokio::net::TcpStream::connect(address)
        .await
        .expect("slow TCP peer");
    tokio::time::sleep(Duration::from_millis(30)).await;

    let (client, _client_events) =
        TlsTransport::client_only(loopback_addr(0), None, trusted_client_config(&cert_path))
            .await
            .expect("TLS client");
    let connect = tokio::spawn({
        let client = client;
        async move {
            client.connect(address).await?;
            Ok::<_, rvoip_sip_transport::Error>(client)
        }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !connect.is_finished(),
        "second TLS handshake bypassed the configured admission limit"
    );
    let client = tokio::time::timeout(Duration::from_secs(1), connect)
        .await
        .expect("TLS slot was not released after deadline")
        .expect("connect task")
        .expect("valid TLS handshake after timeout");

    client.close().await.unwrap();
    server.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tls_close_cancels_pending_and_live_peers_then_releases_listener() {
    let (_dir, cert_path, key_path) = write_self_signed_localhost_cert();
    let config = HandshakeAdmissionConfig::new(Duration::from_secs(30), 2);
    let (server_tx, mut server_events) = tokio::sync::mpsc::channel(16);
    let (server, _unused_events) = TlsTransport::bind_with_handshake_config(
        loopback_addr(0),
        &cert_path,
        &key_path,
        Some(server_tx),
        config,
    )
    .await
    .expect("TLS bind");
    let address = server.local_addr().unwrap();

    let mut slow_peer = tokio::net::TcpStream::connect(address)
        .await
        .expect("slow TCP peer");
    let (client, mut client_events) =
        TlsTransport::client_only(loopback_addr(0), None, trusted_client_config(&cert_path))
            .await
            .expect("TLS client");
    client.connect(address).await.expect("live TLS peer");

    tokio::time::timeout(Duration::from_millis(500), server.close())
        .await
        .expect("TLS close waited for peer or handshake timeout")
        .expect("TLS close");
    server.close().await.expect("idempotent TLS close");

    let mut byte = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_millis(500), slow_peer.read(&mut byte))
        .await
        .expect("slow TLS socket remained open");
    assert!(matches!(read, Ok(0) | Err(_)));

    let closed = tokio::time::timeout(Duration::from_secs(1), client_events.recv())
        .await
        .expect("live TLS peer was not closed")
        .expect("client event channel closed");
    assert!(matches!(closed, TransportEvent::ConnectionClosed { .. }));

    let (replacement, _replacement_events) =
        TlsTransport::bind_with_handshake_config(address, &cert_path, &key_path, None, config)
            .await
            .expect("rebind released TLS address");
    replacement.close().await.unwrap();

    // A cancelled live reader or handshake must not emit after close returns.
    assert!(
        tokio::time::timeout(Duration::from_millis(100), server_events.recv())
            .await
            .is_err()
    );
    client.close().await.unwrap();
}

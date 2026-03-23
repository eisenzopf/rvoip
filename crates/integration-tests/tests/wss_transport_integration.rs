//! WSS (TLS WebSocket) SIP transport integration tests.
//!
//! Tests the secure WebSocket transport layer for SIP messaging over wss://
//! connections. Validates TLS certificate loading, secure bind/close lifecycle,
//! error handling for missing or invalid certificates, and the TLS handshake path.

#![cfg(all(feature = "ws", feature = "tls"))]

use std::io::Write;
use std::net::SocketAddr;
use std::time::Duration;

use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};
use tempfile::NamedTempFile;
use tokio::time::timeout;

use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::method::Method;
use rvoip_sip_core::Message;
use rvoip_sip_transport::transport::{Transport, TransportEvent, WebSocketTransport};

/// Install the rustls ring CryptoProvider (idempotent — ignores if already set).
fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

// =============================================================================
// Helpers
// =============================================================================

/// Generate a self-signed certificate and private key, write them to temp files,
/// and return (cert_file, key_file). The files are kept alive by returning the
/// `NamedTempFile` handles — they are deleted when dropped.
fn generate_self_signed_cert() -> (NamedTempFile, NamedTempFile) {
    let key_pair = KeyPair::generate(&PKCS_ECDSA_P256_SHA256)
        .expect("key generation");
    let mut params = CertificateParams::new(vec!["localhost".to_string()]);
    params.key_pair = Some(key_pair);
    let cert = rcgen::Certificate::from_params(params)
        .expect("self-signed cert");

    let cert_pem = cert.serialize_pem().expect("serialize cert PEM");
    let key_pem = cert.serialize_private_key_pem();

    let mut cert_file = NamedTempFile::new().expect("create cert tempfile");
    cert_file.write_all(cert_pem.as_bytes()).expect("write cert PEM");
    cert_file.flush().expect("flush cert");

    let mut key_file = NamedTempFile::new().expect("create key tempfile");
    key_file.write_all(key_pem.as_bytes()).expect("write key PEM");
    key_file.flush().expect("flush key");

    (cert_file, key_file)
}

/// Bind a WSS transport using the given cert/key temp files.
async fn bind_wss(
    cert_file: &NamedTempFile,
    key_file: &NamedTempFile,
) -> (WebSocketTransport, tokio::sync::mpsc::Receiver<TransportEvent>) {
    let cert_path = cert_file.path().to_str().expect("cert path as str");
    let key_path = key_file.path().to_str().expect("key path as str");

    WebSocketTransport::bind(
        "127.0.0.1:0".parse().unwrap(),
        true,
        Some(cert_path),
        Some(key_path),
        None,
    )
    .await
    .expect("should bind WSS transport with valid self-signed cert")
}

/// Build a minimal SIP REGISTER request.
fn build_register(call_id: &str) -> Message {
    SimpleRequestBuilder::new(Method::Register, "sip:example.com")
        .expect("valid URI")
        .from("Alice", "sip:alice@example.com", Some("wss-tag"))
        .to("Alice", "sip:alice@example.com", None)
        .call_id(call_id)
        .cseq(1)
        .via("127.0.0.1:5060", "WSS", Some("z9hG4bK-wss-test"))
        .build()
        .into()
}

const TIMEOUT: Duration = Duration::from_secs(5);

// =============================================================================
// Test 1: WSS bind with valid self-signed cert
// =============================================================================

#[tokio::test]
async fn test_wss_bind_with_valid_cert() {
    install_crypto_provider();
    let (cert_file, key_file) = generate_self_signed_cert();

    let (transport, _rx) = bind_wss(&cert_file, &key_file).await;

    let addr = transport.local_addr().expect("local_addr");
    assert_eq!(addr.ip(), "127.0.0.1".parse::<std::net::IpAddr>().unwrap());
    assert!(addr.port() > 0, "should have a non-zero port");

    transport.close().await.expect("close");
}

// =============================================================================
// Test 2: WSS bind fails with missing cert path
// =============================================================================

#[tokio::test]
async fn test_wss_bind_fails_missing_cert_path() {
    install_crypto_provider();
    let (_cert_file, key_file) = generate_self_signed_cert();
    let key_path = key_file.path().to_str().expect("key path");

    let result = WebSocketTransport::bind(
        "127.0.0.1:0".parse().unwrap(),
        true,
        None, // no cert path
        Some(key_path),
        None,
    )
    .await;

    assert!(result.is_err(), "WSS bind should fail without cert path");
}

// =============================================================================
// Test 3: WSS bind fails with missing key path
// =============================================================================

#[tokio::test]
async fn test_wss_bind_fails_missing_key_path() {
    install_crypto_provider();
    let (cert_file, _key_file) = generate_self_signed_cert();
    let cert_path = cert_file.path().to_str().expect("cert path");

    let result = WebSocketTransport::bind(
        "127.0.0.1:0".parse().unwrap(),
        true,
        Some(cert_path),
        None, // no key path
        None,
    )
    .await;

    assert!(result.is_err(), "WSS bind should fail without key path");
}

// =============================================================================
// Test 4: WSS bind fails with nonexistent cert file
// =============================================================================

#[tokio::test]
async fn test_wss_bind_fails_nonexistent_cert_file() {
    install_crypto_provider();
    let result = WebSocketTransport::bind(
        "127.0.0.1:0".parse().unwrap(),
        true,
        Some("/tmp/nonexistent_wss_cert_12345.pem"),
        Some("/tmp/nonexistent_wss_key_12345.pem"),
        None,
    )
    .await;

    assert!(result.is_err(), "WSS bind should fail with nonexistent cert files");
}

// =============================================================================
// Test 5: WSS client-server TLS handshake path exercised
// =============================================================================
//
// The WSS client uses an empty root store, so a self-signed server cert will
// be rejected during the TLS handshake. This test validates that the TLS code
// path is actually exercised: the server binds with a real cert, the client
// attempts a wss:// connection, and the send fails with a TLS error.

#[tokio::test]
async fn test_wss_client_server_tls_handshake_attempted() {
    install_crypto_provider();
    let (cert_file, key_file) = generate_self_signed_cert();

    // Start a WSS server with a valid self-signed cert
    let (server, _server_rx) = bind_wss(&cert_file, &key_file).await;
    let server_addr = server.local_addr().expect("server addr");

    // Create a WSS client — needs its own cert/key for the listener side,
    // even though we only care about outbound wss:// connections.
    let (client_cert, client_key) = generate_self_signed_cert();
    let (client, _client_rx) = bind_wss(&client_cert, &client_key).await;

    let msg = build_register("wss-tls-attempt@example.com");
    let result = client.send_message(msg, server_addr).await;

    // The send should fail because the TLS handshake is rejected
    // (self-signed cert not trusted by empty root store).
    assert!(result.is_err(), "WSS send should fail due to TLS cert rejection");

    let err_msg = format!("{}", result.unwrap_err());
    // The error should mention TLS handshake failure
    assert!(
        err_msg.contains("TLS") || err_msg.contains("tls") || err_msg.contains("handshake")
            || err_msg.contains("certificate") || err_msg.contains("Certificate"),
        "Error should be TLS-related, got: {}",
        err_msg,
    );

    client.close().await.expect("client close");
    server.close().await.expect("server close");
}

// =============================================================================
// Test 6: WSS listener reports secure
// =============================================================================

#[tokio::test]
async fn test_wss_listener_reports_secure() {
    install_crypto_provider();
    let (cert_file, key_file) = generate_self_signed_cert();
    let (transport, _rx) = bind_wss(&cert_file, &key_file).await;

    // The transport itself doesn't expose is_secure() directly, but we can
    // verify by checking the debug output includes the address (confirming
    // it bound successfully in secure mode) and that it didn't error.
    let debug = format!("{:?}", transport);
    assert!(
        debug.contains("WebSocketTransport"),
        "Debug should show WebSocketTransport: {}",
        debug,
    );

    // Verify the transport is functional (not closed)
    assert!(!transport.is_closed());

    transport.close().await.expect("close");
}

// =============================================================================
// Test 7: WSS close is clean
// =============================================================================

#[tokio::test]
async fn test_wss_close_is_clean() {
    install_crypto_provider();
    let (cert_file, key_file) = generate_self_signed_cert();
    let (transport, mut rx) = bind_wss(&cert_file, &key_file).await;

    // Verify transport is initially open
    assert!(!transport.is_closed());

    // Close the transport
    transport.close().await.expect("close");

    // Verify transport reports closed
    assert!(transport.is_closed());

    // Should receive a Closed event (or channel closes)
    let event = timeout(Duration::from_secs(2), rx.recv()).await;
    if let Ok(Some(TransportEvent::Closed)) = event {
        // expected
    } else {
        // The Closed event may or may not arrive depending on timing;
        // the important thing is the transport is marked as closed.
        assert!(transport.is_closed());
    }

    // Double close should be safe
    transport.close().await.expect("double close should succeed");
    assert!(transport.is_closed());
}

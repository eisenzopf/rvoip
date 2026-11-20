//! Phase 4 acceptance — WebSocket client + WSS server round-trip.
//!
//! Exercises the WS client wired in `ws/mod.rs::connect_to()` and the
//! WSS TLS accept wired in `ws/listener.rs` against a locally bound
//! `WebSocketTransport` server:
//!
//!   1. Plain `ws://` — server bind, client dial via `send_message()`,
//!      assert the SIP REGISTER round-trips through
//!      `TransportEvent::MessageReceived`.
//!   2. `wss://` — same flow, but the server is bound with a
//!      self-signed cert and the client dials over TLS. Gated on the
//!      `wss` + `dev-insecure-tls` features so production builds don't
//!      enable the test (the client side would need real cert chain
//!      validation).
//!
//! These tests are the regression check for Phase 4 of the
//! `STIR_SHAKEN_AND_PROXY_PLAN.md` roadmap.

#![cfg(feature = "ws")]

use std::net::SocketAddr;
use std::time::Duration;

use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::{Message, Method};
use rvoip_sip_transport::transport::ws::WebSocketTransport;
use rvoip_sip_transport::{Transport, TransportEvent};

// The self-signed-cert helper below is only built when both `wss`
// and `dev-insecure-tls` are enabled — keep its imports together
// behind the same gate so the default `--features ws` build doesn't
// see them as unused.
#[cfg(all(feature = "wss", feature = "dev-insecure-tls"))]
use std::io::Write;
#[cfg(all(feature = "wss", feature = "dev-insecure-tls"))]
use tempfile::tempdir;

fn loopback_addr(port: u16) -> SocketAddr {
    format!("127.0.0.1:{}", port).parse().unwrap()
}

fn build_register(call_id: &str) -> rvoip_sip_core::Message {
    let request = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com")
        .unwrap()
        .from("alice", "sip:alice@example.com", Some("tag-ws"))
        .to("alice", "sip:alice@example.com", None)
        .call_id(call_id)
        .cseq(1)
        .build();
    Message::Request(request)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn plain_ws_round_trip_delivers_register_to_server_event_bus() {
    let _ = tracing_subscriber::fmt::try_init();

    // Server-side: bind plain WS on an ephemeral port.
    let (server_transport, mut server_rx) =
        WebSocketTransport::bind(loopback_addr(0), false, None, None, None)
            .await
            .expect("server bind ws");
    let server_addr = server_transport.local_addr().expect("server local addr");

    // Let the accept loop come up.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Client-side: separate transport instance dialing the server.
    // `connect_to()` is invoked transitively by `send_message()`'s
    // pool-miss path.
    let (client_transport, _client_rx_unused) =
        WebSocketTransport::bind(loopback_addr(0), false, None, None, None)
            .await
            .expect("client bind ws");

    let register = build_register("ws-roundtrip-plain");
    client_transport
        .send_message(register.clone(), server_addr)
        .await
        .expect("client send ws");

    // Wait for the server-side accept + read loop to deliver the
    // MessageReceived event. Bound the wait so a regression doesn't
    // hang CI.
    let event = tokio::time::timeout(Duration::from_secs(3), server_rx.recv())
        .await
        .expect("timed out waiting for server-side MessageReceived")
        .expect("server channel closed");

    match event {
        TransportEvent::MessageReceived {
            message,
            transport_type,
            ..
        } => {
            assert_eq!(
                transport_type,
                rvoip_sip_transport::transport::TransportType::Ws,
                "server should observe Ws transport on plain ws"
            );
            match message {
                Message::Request(req) => {
                    assert_eq!(req.method(), Method::Register);
                    assert_eq!(req.call_id().unwrap().to_string(), "ws-roundtrip-plain");
                }
                _ => panic!("expected REGISTER request"),
            }
        }
        other => panic!("unexpected first event: {:?}", other),
    }
}

#[cfg(all(feature = "wss", feature = "dev-insecure-tls"))]
fn write_self_signed_localhost_cert() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf)
{
    let dir = tempdir().expect("tempdir");
    let cert_path = dir.path().join("server.crt");
    let key_path = dir.path().join("server.key");

    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
        .expect("rcgen self-signed");
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

/// WSS server-side accept smoke test. Binds a WSS server with a
/// self-signed cert and runs the TLS handshake against it using the
/// tokio-tungstenite client (configured to trust the self-signed cert
/// via `dev-insecure-tls`). Asserts that the TLS handshake succeeds
/// and the WS upgrade completes — i.e., the new
/// `ws/listener.rs::accept()` TLS branch wires through.
///
/// Note: this test still reaches under the `WebSocketTransport` API
/// to drive the rustls handshake manually so the server-side accept
/// branch is the unit under test in isolation. The end-to-end client
/// round-trip via `WebSocketTransport::bind_with_client_tls` is
/// covered by `wss_client_round_trip_delivers_register_to_server_event_bus`.
#[cfg(all(feature = "wss", feature = "dev-insecure-tls"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wss_server_accepts_tls_handshake_and_negotiates_sip_subprotocol() {
    use std::sync::Arc as StdArc;
    use tokio_rustls::rustls::{
        client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        pki_types::{CertificateDer, ServerName, UnixTime},
        ClientConfig, DigitallySignedStruct, SignatureScheme,
    };
    use tokio_rustls::TlsConnector;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let _ = tracing_subscriber::fmt::try_init();

    let (_dir, cert_path, key_path) = write_self_signed_localhost_cert();

    // Server-side: WSS bind. The bind() path builds the TlsAcceptor up
    // front so per-accept handshakes don't re-parse cert material.
    let (server_transport, server_rx) = WebSocketTransport::bind(
        loopback_addr(0),
        true,
        Some(cert_path.to_str().unwrap()),
        Some(key_path.to_str().unwrap()),
        None,
    )
    .await
    .expect("wss server bind");
    let server_addr = server_transport.local_addr().expect("server local addr");

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Client-side: dial TCP, then run a rustls handshake that accepts
    // self-signed certs (dev-only verifier). The `dangerous` API is
    // gated on the `dev-insecure-tls` feature so production builds
    // can't accidentally enable it. After TLS is up, run the WS
    // handshake on the TLS stream.
    let tcp_stream = tokio::net::TcpStream::connect(server_addr)
        .await
        .expect("client tcp connect");

    // Accept the self-signed server cert. This is the same shape as
    // the TLS transport's `dev-insecure-tls` verifier.
    #[derive(Debug)]
    struct AcceptAll;
    impl ServerCertVerifier for AcceptAll {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, tokio_rustls::rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![
                SignatureScheme::ECDSA_NISTP256_SHA256,
                SignatureScheme::ECDSA_NISTP384_SHA384,
                SignatureScheme::ED25519,
                SignatureScheme::RSA_PSS_SHA256,
                SignatureScheme::RSA_PSS_SHA384,
                SignatureScheme::RSA_PSS_SHA512,
                SignatureScheme::RSA_PKCS1_SHA256,
                SignatureScheme::RSA_PKCS1_SHA384,
                SignatureScheme::RSA_PKCS1_SHA512,
            ]
        }
    }
    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(StdArc::new(AcceptAll))
        .with_no_client_auth();

    let connector = TlsConnector::from(StdArc::new(config));
    let server_name = ServerName::try_from("localhost".to_string()).expect("server name");
    let tls_stream = connector
        .connect(server_name, tcp_stream)
        .await
        .expect("client tls handshake");

    let url = format!("wss://localhost:{}/", server_addr.port());
    let mut request = url.into_client_request().expect("ws request");
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        http::HeaderValue::from_static("sip"),
    );

    let (mut ws_stream, response) = tokio_tungstenite::client_async(request, tls_stream)
        .await
        .expect("ws upgrade over tls");
    assert_eq!(
        response
            .headers()
            .get("Sec-WebSocket-Protocol")
            .and_then(|value| value.to_str().ok()),
        Some("sip"),
        "RFC 7118 requires the exact `sip` subprotocol for WSS"
    );

    // The successful upgrade proves the WSS accept path:
    //  - TlsAcceptor::accept() handled the rustls handshake
    //  - SipWsStream::ServerTls() wrapped the resulting stream
    //  - accept_async() drove the WS upgrade on the encrypted stream
    //
    // We don't pump a SIP REGISTER through here because that would
    // require a WebSocket → SIP forwarder on the client side, and the
    // primary goal of this test is the WSS accept branch.
    //
    // Close cleanly so the server-side read loop logs at debug, not
    // ERROR. tokio-tungstenite's `close` sends a Close frame and waits
    // for the peer's response.
    let _ = ws_stream.close(None).await;

    let _ = server_transport;
    let _ = server_rx;
}

/// End-to-end WSS client + server smoke test via the public
/// `WebSocketTransport` API. Binds a WSS server with a self-signed
/// cert, builds a client transport via `bind_with_client_tls(..., Some(
/// TlsClientConfig { insecure_skip_verify: true, .. }))`, sends a
/// REGISTER, and asserts the server receives it on its event bus with
/// `TransportType::Wss`.
///
/// Gated on `wss + dev-insecure-tls` because the self-signed cert
/// requires the insecure verifier. Production deployments use real CA
/// chains; the default `TlsClientConfig` (system roots, no insecure
/// skip) handles that path with no API change.
#[cfg(all(feature = "wss", feature = "dev-insecure-tls"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wss_client_round_trip_delivers_register_to_server_event_bus() {
    use rvoip_sip_transport::transport::ws::TlsClientConfig;

    let _ = tracing_subscriber::fmt::try_init();

    let (_dir, cert_path, key_path) = write_self_signed_localhost_cert();

    // Server-side: WSS bind with the self-signed cert.
    let (server_transport, mut server_rx) = WebSocketTransport::bind(
        loopback_addr(0),
        true,
        Some(cert_path.to_str().unwrap()),
        Some(key_path.to_str().unwrap()),
        None,
    )
    .await
    .expect("wss server bind");
    let server_addr = server_transport.local_addr().expect("server local addr");

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Client-side: WSS bind WITH a TlsClientConfig that trusts the
    // self-signed cert via the `dev-insecure-tls` skip-verify path.
    // Bind to a separate ephemeral port. The bind requires cert/key
    // for the server side of the same transport, but the listener
    // never accepts here — only the outbound dial is exercised.
    let client_tls = TlsClientConfig {
        insecure_skip_verify: true,
        ..TlsClientConfig::default()
    };
    let (client_transport, _client_rx_unused) = WebSocketTransport::bind_with_client_tls(
        loopback_addr(0),
        true,
        Some(cert_path.to_str().unwrap()),
        Some(key_path.to_str().unwrap()),
        None,
        Some(client_tls),
    )
    .await
    .expect("wss client bind");

    let register = build_register("wss-roundtrip-tls");
    client_transport
        .send_message(register.clone(), server_addr)
        .await
        .expect("client send wss");

    let event = tokio::time::timeout(Duration::from_secs(3), server_rx.recv())
        .await
        .expect("timed out waiting for server-side MessageReceived")
        .expect("server channel closed");

    match event {
        TransportEvent::MessageReceived {
            message,
            transport_type,
            ..
        } => {
            assert_eq!(
                transport_type,
                rvoip_sip_transport::transport::TransportType::Wss,
                "server should observe Wss transport on wss://"
            );
            match message {
                Message::Request(req) => {
                    assert_eq!(req.method(), Method::Register);
                    assert_eq!(req.call_id().unwrap().to_string(), "wss-roundtrip-tls");
                }
                _ => panic!("expected REGISTER request"),
            }
        }
        other => panic!("unexpected first event: {:?}", other),
    }
}

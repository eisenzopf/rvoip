#![cfg(feature = "ws")]

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::SinkExt;
use http::HeaderValue;
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::{Message, Method};
use rvoip_sip_transport::transport::ws::WebSocketTransport;
use rvoip_sip_transport::{HandshakeAdmissionConfig, Transport, TransportEvent};
use tokio::io::AsyncReadExt;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as WsMessage;

#[cfg(feature = "wss")]
use rvoip_sip_transport::transport::tls::{TlsClientConfig, TlsServerClientAuthConfig};

fn loopback(port: u16) -> SocketAddr {
    format!("127.0.0.1:{port}").parse().unwrap()
}

fn register_bytes(call_id: &str) -> Vec<u8> {
    Message::Request(
        SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.test")
            .unwrap()
            .from("alice", "sip:alice@example.test", Some("ws-admission"))
            .to("alice", "sip:alice@example.test", None)
            .call_id(call_id)
            .cseq(1)
            .build(),
    )
    .to_bytes()
}

async fn connect_sip_ws(
    address: SocketAddr,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let mut request = format!("ws://{address}/")
        .into_client_request()
        .expect("WebSocket request");
    request
        .headers_mut()
        .insert("Sec-WebSocket-Protocol", HeaderValue::from_static("sip"));
    let (stream, response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("SIP WebSocket connect");
    assert_eq!(
        response
            .headers()
            .get("Sec-WebSocket-Protocol")
            .and_then(|value| value.to_str().ok()),
        Some("sip")
    );
    stream
}

async fn expect_register(events: &mut tokio::sync::mpsc::Receiver<TransportEvent>, call_id: &str) {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .expect("server event timeout")
            .expect("server event channel closed");
        if let TransportEvent::MessageReceived { message, .. } = event {
            let Message::Request(request) = message else {
                panic!("expected request");
            };
            assert_eq!(request.method(), Method::Register);
            assert_eq!(request.call_id().unwrap().to_string(), call_id);
            return;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_http_peer_does_not_block_another_ws_upgrade() {
    let (server, mut events) = WebSocketTransport::bind_with_handshake_config(
        loopback(0),
        false,
        None,
        None,
        None,
        HandshakeAdmissionConfig::new(Duration::from_secs(2), 2),
    )
    .await
    .expect("WS bind");
    let address = server.local_addr().unwrap();

    let _slow_peer = tokio::net::TcpStream::connect(address)
        .await
        .expect("slow TCP peer");
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut client = tokio::time::timeout(Duration::from_millis(750), connect_sip_ws(address))
        .await
        .expect("valid WS upgrade was serialized behind slow peer");
    client
        .send(WsMessage::Binary(register_bytes("ws-parallel").into()))
        .await
        .expect("send REGISTER");
    expect_register(&mut events, "ws-parallel").await;
    server.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ws_handshake_deadline_releases_saturated_admission() {
    let (server, _events) = WebSocketTransport::bind_with_handshake_config(
        loopback(0),
        false,
        None,
        None,
        None,
        HandshakeAdmissionConfig::new(Duration::from_millis(150), 1),
    )
    .await
    .expect("WS bind");
    let address = server.local_addr().unwrap();

    let _slow_peer = tokio::net::TcpStream::connect(address)
        .await
        .expect("slow TCP peer");
    tokio::time::sleep(Duration::from_millis(30)).await;

    let client = tokio::spawn(connect_sip_ws(address));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !client.is_finished(),
        "second handshake bypassed the configured admission limit"
    );
    tokio::time::timeout(Duration::from_secs(1), client)
        .await
        .expect("handshake slot was not released after deadline")
        .expect("client task failed");
    server.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ws_close_cancels_slow_handshake_and_releases_listener() {
    let config = HandshakeAdmissionConfig::new(Duration::from_secs(30), 1);
    let (server, mut events) = WebSocketTransport::bind_with_handshake_config(
        loopback(0),
        false,
        None,
        None,
        None,
        config,
    )
    .await
    .expect("WS bind");
    let address = server.local_addr().unwrap();
    let mut slow_peer = tokio::net::TcpStream::connect(address)
        .await
        .expect("slow TCP peer");
    tokio::time::sleep(Duration::from_millis(30)).await;

    tokio::time::timeout(Duration::from_millis(500), server.close())
        .await
        .expect("WS close waited for handshake timeout")
        .expect("WS close");
    server.close().await.expect("idempotent WS close");

    let mut byte = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_millis(500), slow_peer.read(&mut byte))
        .await
        .expect("slow peer socket remained open");
    assert!(matches!(read, Ok(0) | Err(_)));

    // The listener socket is owned by the managed accept task and is dropped
    // before close returns.
    let (replacement, _replacement_events) =
        WebSocketTransport::bind_with_handshake_config(address, false, None, None, None, config)
            .await
            .expect("rebind released WS address");
    replacement.close().await.unwrap();

    // Closed is the only allowed shutdown event; no late connection/message
    // event may be emitted by the cancelled handshake.
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(25), events.recv()).await
    {
        assert!(matches!(event, TransportEvent::Closed));
    }
}

#[cfg(feature = "wss")]
fn write_wss_cert() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    use std::io::Write;

    let directory = tempfile::tempdir().expect("tempdir");
    let cert_path = directory.path().join("wss.crt");
    let key_path = directory.path().join("wss.key");
    let cert = rcgen::generate_simple_self_signed(vec![
        "localhost".to_string(),
        "registrar.example.test".to_string(),
    ])
    .expect("self-signed WSS certificate");
    std::fs::File::create(&cert_path)
        .and_then(|mut file| file.write_all(cert.cert.pem().as_bytes()))
        .expect("write WSS certificate");
    std::fs::File::create(&key_path)
        .and_then(|mut file| file.write_all(cert.signing_key.serialize_pem().as_bytes()))
        .expect("write WSS key");
    (directory, cert_path, key_path)
}

#[cfg(feature = "wss")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_wss_client_hello_does_not_block_authenticated_upgrade() {
    let (_directory, cert_path, key_path) = write_wss_cert();
    let cert = cert_path.to_str().unwrap();
    let key = key_path.to_str().unwrap();
    let (server, mut events) = WebSocketTransport::bind_with_tls_configs_and_handshake(
        loopback(0),
        true,
        Some(cert),
        Some(key),
        None,
        None,
        TlsServerClientAuthConfig::default(),
        HandshakeAdmissionConfig::new(Duration::from_secs(2), 2),
    )
    .await
    .expect("WSS bind");
    let address = server.local_addr().unwrap();

    let _slow_peer = tokio::net::TcpStream::connect(address)
        .await
        .expect("slow WSS TCP peer");
    tokio::time::sleep(Duration::from_millis(50)).await;

    let (client, _client_events) = WebSocketTransport::bind_with_client_tls(
        loopback(0),
        true,
        Some(cert),
        Some(key),
        None,
        Some(TlsClientConfig {
            extra_ca_path: Some(cert_path.clone()),
            ..Default::default()
        }),
    )
    .await
    .expect("WSS client");
    tokio::time::timeout(
        Duration::from_millis(750),
        client.send_message(
            Message::Request(
                SimpleRequestBuilder::new(Method::Register, "sips:registrar.example.test")
                    .unwrap()
                    .from("alice", "sips:alice@example.test", Some("wss-admission"))
                    .to("alice", "sips:alice@example.test", None)
                    .call_id("wss-parallel")
                    .cseq(1)
                    .build(),
            ),
            address,
        ),
    )
    .await
    .expect("valid WSS upgrade was serialized behind slow ClientHello")
    .expect("send WSS REGISTER");
    expect_register(&mut events, "wss-parallel").await;

    client.close().await.unwrap();
    server.close().await.unwrap();
}

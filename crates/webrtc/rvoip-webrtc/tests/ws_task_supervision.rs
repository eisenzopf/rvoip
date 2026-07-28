//! Owned inbound WebSocket task admission and stalled-peer shutdown.

#![cfg(feature = "signaling-ws")]

use std::time::Duration;

use rvoip_webrtc::{WebRtcConfig, WebRtcServerBuilder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn stalled_handshake_is_bounded_rejected_and_joined_on_shutdown() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut config = WebRtcConfig::loopback();
    config.max_concurrent_sessions = 1;
    let server = WebRtcServerBuilder::new(config)
        .with_ws("127.0.0.1:0")
        .build()
        .await
        .expect("WS server");
    let adapter = server.adapter();
    let address = server.ws_addr().expect("WS address");

    // The first peer deliberately never completes its HTTP upgrade.
    let _stalled = tokio::net::TcpStream::connect(address)
        .await
        .expect("stalled connection");
    wait_until(Duration::from_secs(1), || {
        adapter.metrics().inbound_ws_connection_tasks == 1
    })
    .await;

    // A second accepted socket cannot allocate another connection task. The
    // listener attempts a retryable HTTP rejection; a kernel may reset a
    // socket whose request raced the pre-upgrade close, which is also a valid
    // fail-closed result.
    let mut overloaded = tokio::net::TcpStream::connect(address)
        .await
        .expect("overloaded connection");
    overloaded
        .write_all(
            b"GET /signal HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
        )
        .await
        .expect("upgrade request");
    let mut response = Vec::new();
    let read = tokio::time::timeout(
        Duration::from_secs(1),
        overloaded.read_to_end(&mut response),
    )
    .await
    .expect("overload response timeout");
    if read.is_ok() || !response.is_empty() {
        assert!(
            response.starts_with(b"HTTP/1.1 503 Service Unavailable"),
            "unexpected overload response: {}",
            String::from_utf8_lossy(&response)
        );
    } else if let Err(error) = read {
        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
    }
    assert_eq!(adapter.metrics().inbound_ws_connections_rejected_total, 1);

    server
        .shutdown_with_deadline(Duration::from_millis(250))
        .await;
    assert_eq!(adapter.metrics().inbound_ws_connection_tasks, 0);
    assert_eq!(adapter.metrics().peer_session_tasks, 0);
    assert_eq!(adapter.metrics().media_tasks, 0);
    assert!(adapter.routes().is_empty());
}

async fn wait_until(mut remaining: Duration, mut predicate: impl FnMut() -> bool) {
    while !predicate() {
        assert!(!remaining.is_zero(), "condition deadline exceeded");
        let slice = remaining.min(Duration::from_millis(10));
        tokio::time::sleep(slice).await;
        remaining = remaining.saturating_sub(slice);
    }
}

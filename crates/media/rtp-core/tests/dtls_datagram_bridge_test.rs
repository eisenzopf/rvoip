//! Proves a real DTLS-SRTP handshake completes over `DtlsDatagramBridge`
//! with no real socket anywhere in the picture — the bridge's whole
//! purpose is letting a caller-owned reactor (e.g. `mio`) supply
//! datagrams itself instead of this crate owning a `tokio::net::UdpSocket`.
//!
//! The two bridges here are wired together by plain channels standing in
//! for "an external reactor moved these bytes between two real sockets" —
//! mirrors `dtls_srtp_handshake_test.rs`'s structure exactly, just with
//! the transport swapped out.

#![cfg(feature = "dtls-webrtc")]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use rvoip_rtp_core::dtls_srtp::{
    default_srtp_profiles, generate_identity, handshake_client, handshake_server,
};
use rvoip_rtp_core::transport::dtls_datagram_bridge::DtlsDatagramBridge;
use tokio::sync::mpsc;

/// Spawns a task standing in for the external reactor: drains one side's
/// outbound channel and feeds every datagram straight into the other
/// side's bridge, as if it had gone out over the wire and come back in.
fn pump_outbound_into_peer(
    mut outbound_rx: mpsc::Receiver<(Bytes, SocketAddr)>,
    peer: Arc<DtlsDatagramBridge>,
) {
    tokio::spawn(async move {
        while let Some((bytes, _target)) = outbound_rx.recv().await {
            if !peer.feed_inbound(bytes) {
                break;
            }
        }
    });
}

#[tokio::test]
async fn independent_bridges_complete_a_real_handshake_with_no_real_socket() {
    let client_addr: SocketAddr = "127.0.0.1:40000".parse().unwrap();
    let server_addr: SocketAddr = "127.0.0.1:40001".parse().unwrap();

    let (client_out_tx, client_out_rx) = mpsc::channel(32);
    let (server_out_tx, server_out_rx) = mpsc::channel(32);

    let client_bridge = Arc::new(DtlsDatagramBridge::new(
        client_addr,
        server_addr,
        client_out_tx,
    ));
    let server_bridge = Arc::new(DtlsDatagramBridge::new(
        server_addr,
        client_addr,
        server_out_tx,
    ));

    pump_outbound_into_peer(server_out_rx, client_bridge.clone());
    pump_outbound_into_peer(client_out_rx, server_bridge.clone());

    let client_identity = generate_identity().unwrap();
    let server_identity = generate_identity().unwrap();

    let client_task = tokio::spawn(handshake_client(
        client_bridge,
        client_identity,
        default_srtp_profiles(),
    ));
    let server_task = tokio::spawn(handshake_server(
        server_bridge,
        server_identity,
        default_srtp_profiles(),
    ));

    let (client_result, server_result) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(client_task, server_task)
    })
    .await
    .expect("handshake over a socket-free bridge must complete within 10s");

    let client_result = client_result
        .expect("client task panicked")
        .expect("client handshake failed");
    let server_result = server_result
        .expect("server task panicked")
        .expect("server handshake failed");

    assert_eq!(client_result.profile, server_result.profile);
    assert_eq!(
        client_result.client_write_key.key(),
        server_result.client_write_key.key()
    );
    assert_eq!(
        client_result.client_write_key.salt(),
        server_result.client_write_key.salt()
    );
    assert_eq!(
        client_result.server_write_key.key(),
        server_result.server_write_key.key()
    );
    assert_eq!(
        client_result.remote_fingerprint_sha256,
        server_result.local_fingerprint_sha256
    );
    assert_eq!(
        server_result.remote_fingerprint_sha256,
        client_result.local_fingerprint_sha256
    );
}

#[tokio::test]
async fn feed_inbound_reports_backpressure_without_blocking() {
    let client_addr: SocketAddr = "127.0.0.1:40002".parse().unwrap();
    let server_addr: SocketAddr = "127.0.0.1:40003".parse().unwrap();
    let (outbound_tx, _outbound_rx) = mpsc::channel(32);
    let bridge = DtlsDatagramBridge::new(client_addr, server_addr, outbound_tx);

    // Nothing is draining the inbound side (no handshake running), so
    // filling past the inbound channel's capacity must report backpressure
    // via a plain `false` return — never block the caller's (synchronous,
    // in a real `mio` reactor) call site.
    let mut accepted = 0;
    let mut rejected = 0;
    for _ in 0..64 {
        if bridge.feed_inbound(Bytes::from_static(b"probe")) {
            accepted += 1;
        } else {
            rejected += 1;
        }
    }
    assert!(
        accepted > 0,
        "some datagrams should fit before the channel fills"
    );
    assert!(
        rejected > 0,
        "pushing past capacity must be reported, not silently accepted"
    );
}

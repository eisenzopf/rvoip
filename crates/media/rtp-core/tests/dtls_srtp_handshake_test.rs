//! Proves a real DTLS-SRTP handshake completes between two independent
//! parties over actual UDP sockets, and that they derive identical SRTP
//! keys and see each other's real certificate fingerprint.
//!
//! This is exactly the property the in-house `rtp_core::dtls` handshake
//! never demonstrated: its `HandshakeState` has a `force_verification_buffer`
//! escape hatch whose own doc comment admits independent `Finished`-message
//! verification "for the current verification issues" was never solved, and
//! a saved log of its own examples shows every attempt timing out on
//! loopback. This test drives `client` and `server` as two separate async
//! tasks that only communicate over real sockets, no shared state, no
//! forced buffer syncing.

#![cfg(feature = "dtls-webrtc")]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rvoip_rtp_core::dtls_srtp::{default_srtp_profiles, handshake_client, handshake_server};
use tokio::net::UdpSocket;

async fn connected_socket_pair() -> (Arc<UdpSocket>, Arc<UdpSocket>) {
    let a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let b = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let a_addr: SocketAddr = a.local_addr().unwrap();
    let b_addr: SocketAddr = b.local_addr().unwrap();
    a.connect(b_addr).await.unwrap();
    b.connect(a_addr).await.unwrap();
    (Arc::new(a), Arc::new(b))
}

#[tokio::test]
async fn independent_client_and_server_complete_a_real_handshake_with_matching_keys() {
    let (client_socket, server_socket) = connected_socket_pair().await;

    let client_task = tokio::spawn(handshake_client(client_socket, default_srtp_profiles()));
    let server_task = tokio::spawn(handshake_server(server_socket, default_srtp_profiles()));

    let (client_result, server_result) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(client_task, server_task)
    })
    .await
    .expect(
        "handshake must complete within 10s, not time out like the in-house implementation did",
    );

    let client_result = client_result
        .expect("client task panicked")
        .expect("client handshake failed");
    let server_result = server_result
        .expect("server task panicked")
        .expect("server handshake failed");

    // Both sides must agree on the negotiated SRTP profile.
    assert_eq!(client_result.profile, server_result.profile);

    // The whole point: two independent parties deriving the SAME keys from
    // a real, unforced handshake, not a shared buffer forced to match.
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
        client_result.server_write_key.salt(),
        server_result.server_write_key.salt()
    );

    // Client and server keys must differ from each other (distinct
    // directions), or the split step silently produced the same bytes
    // twice instead of genuinely dividing the exported keying material.
    assert_ne!(
        client_result.client_write_key.key(),
        client_result.server_write_key.key()
    );

    // Each side's view of the other's certificate must match what the
    // other side reports as its own certificate: this is exactly the
    // binding a real `a=fingerprint` check in a SIP/SDP caller would rely
    // on to prevent an on-path attacker from swapping in your own
    // certificate.
    assert_eq!(
        client_result.remote_fingerprint_sha256,
        server_result.local_fingerprint_sha256
    );
    assert_eq!(
        server_result.remote_fingerprint_sha256,
        client_result.local_fingerprint_sha256
    );

    // Sanity: client and server used different self-signed certificates.
    assert_ne!(
        client_result.local_fingerprint_sha256,
        server_result.local_fingerprint_sha256
    );
}

#[tokio::test]
async fn each_handshake_uses_a_fresh_certificate() {
    // Two back-to-back handshakes should not reuse the same self-signed
    // certificate (each call to handshake_client/handshake_server generates
    // its own), so repeated calls must produce different fingerprints.
    let (client_socket_1, server_socket_1) = connected_socket_pair().await;
    let (client_socket_2, server_socket_2) = connected_socket_pair().await;

    let run = |client_socket: Arc<UdpSocket>, server_socket: Arc<UdpSocket>| async move {
        let client_task = tokio::spawn(handshake_client(client_socket, default_srtp_profiles()));
        let server_task = tokio::spawn(handshake_server(server_socket, default_srtp_profiles()));
        let (c, s) = tokio::join!(client_task, server_task);
        (c.unwrap().unwrap(), s.unwrap().unwrap())
    };

    let (first_client, _first_server) = tokio::time::timeout(
        Duration::from_secs(10),
        run(client_socket_1, server_socket_1),
    )
    .await
    .expect("first handshake timed out");

    let (second_client, _second_server) = tokio::time::timeout(
        Duration::from_secs(10),
        run(client_socket_2, server_socket_2),
    )
    .await
    .expect("second handshake timed out");

    assert_ne!(
        first_client.local_fingerprint_sha256,
        second_client.local_fingerprint_sha256
    );
}

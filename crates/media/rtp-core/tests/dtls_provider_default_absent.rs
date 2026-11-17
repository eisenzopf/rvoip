//! With both rustls providers compiled (the `aws_lc_rs` dev-dependency
//! feature) and no process-level default installed, a DTLS-SRTP handshake
//! still completes: rtp-core installs ring as the process default first,
//! because the `dtls` crate can only use that default.
//!
//! Process-level provider state is global, so this scenario has its own test
//! binary.
#![cfg(feature = "dtls-webrtc")]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rvoip_rtp_core::dtls_srtp::{
    default_srtp_profiles, generate_identity, handshake_client, handshake_server,
};
use tokio::net::UdpSocket;

async fn handshake() {
    let a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let b = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let a_addr: SocketAddr = a.local_addr().unwrap();
    let b_addr: SocketAddr = b.local_addr().unwrap();
    a.connect(b_addr).await.unwrap();
    b.connect(a_addr).await.unwrap();
    let client = tokio::spawn(handshake_client(
        Arc::new(a),
        generate_identity().unwrap(),
        default_srtp_profiles(),
    ));
    let server = tokio::spawn(handshake_server(
        Arc::new(b),
        generate_identity().unwrap(),
        default_srtp_profiles(),
    ));
    let (client, server) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(client, server)
    })
    .await
    .expect("handshake completes");
    let client = client.expect("client task").expect("client handshake");
    let server = server.expect("server task").expect("server handshake");
    assert_eq!(client.profile, server.profile);
}

fn installed_key_provider() -> Option<String> {
    rustls::crypto::CryptoProvider::get_default()
        .map(|provider| format!("{:?}", provider.key_provider))
}

#[tokio::test]
async fn handshake_installs_ring_when_no_process_provider_is_installed() {
    assert_eq!(installed_key_provider(), None, "no provider installed yet");
    handshake().await;
    assert_eq!(installed_key_provider().as_deref(), Some("Ring"));
}

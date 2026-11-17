//! An application's own process-level rustls provider is never replaced:
//! with aws-lc-rs installed at startup, the DTLS-SRTP handshake runs on it
//! and it is still the installed default afterwards.
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
async fn handshake_keeps_an_application_installed_provider() {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("first install in this process");
    let installed = rustls::crypto::CryptoProvider::get_default()
        .expect("installed")
        .clone();
    handshake().await;
    let after = rustls::crypto::CryptoProvider::get_default().expect("still installed");
    assert!(
        Arc::ptr_eq(&installed, after),
        "the application's provider is kept"
    );
    assert_eq!(installed_key_provider().as_deref(), Some("AwsLcRs"));
}

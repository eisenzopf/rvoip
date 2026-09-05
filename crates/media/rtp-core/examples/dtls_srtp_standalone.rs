//! Demonstrates the whole DTLS-SRTP flow a SIP-over-WS/WSS application
//! actually needs, using only `rvoip-rtp-core`: no WebRTC, no `rvoip-webrtc`,
//! just a signaling exchange (simulated here instead of going over a real
//! WS/WSS connection) plus a real DTLS handshake over plain UDP sockets.
//!
//! The important ordering this example is built around:
//! 1. Both sides generate their `DtlsIdentity` up front, because its
//!    fingerprint has to go into the SDP offer/answer.
//! 2. The offer/answer exchange happens (here: two in-memory structs instead
//!    of real SIP INVITE/200 OK messages) and decides, via `a=setup`, which
//!    side is the DTLS client and which is the DTLS server.
//! 3. Only then does the DTLS handshake run, over the UDP sockets that will
//!    also carry the SRTP media once the handshake derives keys.
//!
//! Run with: cargo run -p rvoip-rtp-core --features dtls-webrtc --example dtls_srtp_standalone

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::UdpSocket;

use rvoip_rtp_core::dtls_srtp::{
    default_srtp_profiles, generate_identity, handshake_client, handshake_server,
};

/// The handful of SDP attributes a DTLS-SRTP offer/answer actually needs.
/// A real SIP UA would put these in `a=fingerprint` and `a=setup` lines
/// inside the `m=audio` section; everything else about the SDP (codecs,
/// ICE candidates, etc.) is unrelated to DTLS-SRTP and left out here.
#[derive(Debug, Clone)]
struct SdpMediaDescription {
    media_addr: SocketAddr,
    fingerprint_sha256: [u8; 32],
    setup: SetupRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupRole {
    /// RFC 4145: "the endpoint will initiate an outgoing connection".
    Active,
    /// RFC 4145: "the endpoint will accept an incoming connection".
    Passive,
    /// RFC 4145/5763: "the endpoint is willing to accept either role"; only
    /// valid in an offer, the answer must pick Active or Passive.
    ActPass,
}

fn fingerprint_hex(fp: &[u8; 32]) -> String {
    fp.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();

    // Each side's media socket. In a real app this is the same socket
    // already used for SIP-negotiated RTP; DTLS-SRTP runs over that same
    // port, multiplexed from RTP/RTCP by looking at the first byte of each
    // datagram (RFC 5764 §5.1.2). This example skips the demux since it
    // only ever sends DTLS on these sockets.
    let uac_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
    let uas_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
    let uac_addr = uac_socket.local_addr()?;
    let uas_addr = uas_socket.local_addr()?;
    uac_socket.connect(uas_addr).await?;
    uas_socket.connect(uac_addr).await?;

    // Step 1: generate identities before either side knows its role.
    println!("Generating certificates (before either side knows active/passive)...");
    let uac_identity = generate_identity()?;
    let uas_identity = generate_identity()?;
    println!(
        "UAC fingerprint: {}",
        fingerprint_hex(&uac_identity.fingerprint_sha256)
    );
    println!(
        "UAS fingerprint: {}",
        fingerprint_hex(&uas_identity.fingerprint_sha256)
    );

    // Step 2: simulate the SDP offer/answer that would normally travel over
    // the SIP-over-WS/WSS signaling connection. The UAC (caller) offers
    // actpass, per RFC 5763 §5's recommendation for the offerer; the UAS
    // (callee) picks a concrete role in its answer.
    let offer = SdpMediaDescription {
        media_addr: uac_addr,
        fingerprint_sha256: uac_identity.fingerprint_sha256,
        setup: SetupRole::ActPass,
    };
    println!(
        "\nUAC -> UAS SDP offer: m=audio {} ... a=fingerprint sha-256 {} / a=setup:actpass",
        offer.media_addr.port(),
        fingerprint_hex(&offer.fingerprint_sha256)
    );

    // The UAS answers active, meaning it will be the DTLS client and
    // initiate the handshake toward the UAC; the UAC is then passive
    // (the DTLS server).
    let answer = SdpMediaDescription {
        media_addr: uas_addr,
        fingerprint_sha256: uas_identity.fingerprint_sha256,
        setup: SetupRole::Active,
    };
    println!(
        "UAS -> UAC SDP answer: m=audio {} ... a=fingerprint sha-256 {} / a=setup:active",
        answer.media_addr.port(),
        fingerprint_hex(&answer.fingerprint_sha256)
    );

    let uac_role = match answer.setup {
        SetupRole::Active => SetupRole::Passive,
        SetupRole::Passive => SetupRole::Active,
        SetupRole::ActPass => panic!("an answer must pick a concrete role, not actpass"),
    };
    println!(
        "\nResolved roles: UAC is DTLS {:?} (server), UAS is DTLS {:?} (client)",
        uac_role, answer.setup
    );

    // Step 3: only now does the handshake run, over the same media sockets
    // whose addresses were just exchanged in the SDP above.
    let uac_task = tokio::spawn(handshake_server(
        uac_socket,
        uac_identity,
        default_srtp_profiles(),
    ));
    let uas_task = tokio::spawn(handshake_client(
        uas_socket,
        uas_identity,
        default_srtp_profiles(),
    ));

    let (uac_result, uas_result) = tokio::join!(uac_task, uas_task);
    let uac_result = uac_result??;
    let uas_result = uas_result??;

    println!(
        "\nHandshake complete. Negotiated profile: {:?}",
        uac_result.profile
    );

    // Confirm the fingerprints exchanged in step 2 match what the handshake
    // actually saw: this is the check a real UA must do against an on-path
    // attacker swapping in a different certificate.
    assert_eq!(
        uac_result.remote_fingerprint_sha256,
        answer.fingerprint_sha256
    );
    assert_eq!(
        uas_result.remote_fingerprint_sha256,
        offer.fingerprint_sha256
    );
    println!("Remote fingerprints matched what the SDP promised.");

    assert_eq!(
        uac_result.client_write_key.key(),
        uas_result.client_write_key.key()
    );
    assert_eq!(
        uac_result.server_write_key.key(),
        uas_result.server_write_key.key()
    );
    println!("Both sides derived identical SRTP keys, ready to key an SRTP session.");

    Ok(())
}

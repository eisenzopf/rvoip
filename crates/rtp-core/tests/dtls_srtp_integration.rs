//! Cross-module integration tests verifying the DTLS -> SRTP -> RTP encryption pipeline
//! using the production adapter types (`DtlsConnectionAdapter`, `SrtpContextAdapter`).
//!
//! These tests validate:
//! 1. DTLS handshake over loopback with SRTP key extraction
//! 2. SRTP encrypt/decrypt roundtrip via adapters
//! 3. Full pipeline: DTLS handshake -> SRTP context -> RTP packet encrypt/decrypt over UDP
//! 4. SRTCP protect/unprotect roundtrip

use std::sync::Arc;
use std::time::Duration;

use rvoip_rtp_core::dtls::adapter::{
    DtlsAdapterConfig, DtlsConnectionAdapter, DtlsRole, SrtpKeyMaterial,
};
use rvoip_rtp_core::srtp::adapter::SrtpContextAdapter;
use webrtc_srtp::protection_profile::ProtectionProfile;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a minimal valid RTP packet (V=2, PT=0, seq=seq_num, ts=160, ssrc=1)
/// followed by the given payload bytes.
fn build_rtp_packet(seq_num: u16, payload: &[u8]) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(12 + payload.len());
    // V=2, P=0, X=0, CC=0
    pkt.push(0x80);
    // M=0, PT=0
    pkt.push(0x00);
    // Sequence number (big-endian)
    pkt.extend_from_slice(&seq_num.to_be_bytes());
    // Timestamp = 160 * seq_num (big-endian)
    let ts = 160u32.wrapping_mul(seq_num as u32);
    pkt.extend_from_slice(&ts.to_be_bytes());
    // SSRC = 1
    pkt.extend_from_slice(&1u32.to_be_bytes());
    // Payload
    pkt.extend_from_slice(payload);
    pkt
}

/// Build a minimal valid RTCP Sender Report (V=2, PT=200, length=6, SSRC=1).
fn build_rtcp_sender_report() -> Vec<u8> {
    vec![
        0x80, 0xC8, // V=2, P=0, RC=0, PT=200 (SR)
        0x00, 0x06, // Length = 6 (28 bytes total)
        0x00, 0x00, 0x00, 0x01, // SSRC = 1
        // NTP timestamp (8 bytes)
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        // RTP timestamp (4 bytes)
        0x00, 0x00, 0x00, 0xA0,
        // Sender packet count (4 bytes)
        0x00, 0x00, 0x00, 0x0A,
        // Sender octet count (4 bytes)
        0x00, 0x00, 0x01, 0x00,
    ]
}

/// Perform a DTLS handshake between a client and server adapter over connected
/// loopback UDP sockets. Returns the key material from both sides.
async fn dtls_handshake_loopback() -> Result<(SrtpKeyMaterial, SrtpKeyMaterial), rvoip_rtp_core::Error> {
    use tokio::net::UdpSocket;

    // Bind two sockets on loopback with OS-assigned ports.
    let sock_a = UdpSocket::bind("127.0.0.1:0").await
        .map_err(|e| rvoip_rtp_core::Error::IoError(e.to_string()))?;
    let sock_b = UdpSocket::bind("127.0.0.1:0").await
        .map_err(|e| rvoip_rtp_core::Error::IoError(e.to_string()))?;

    let addr_a = sock_a.local_addr()
        .map_err(|e| rvoip_rtp_core::Error::IoError(e.to_string()))?;
    let addr_b = sock_b.local_addr()
        .map_err(|e| rvoip_rtp_core::Error::IoError(e.to_string()))?;

    // Connect each socket to the other's address so send/recv work without
    // specifying the peer address (required by webrtc-dtls).
    sock_a.connect(addr_b).await
        .map_err(|e| rvoip_rtp_core::Error::IoError(e.to_string()))?;
    sock_b.connect(addr_a).await
        .map_err(|e| rvoip_rtp_core::Error::IoError(e.to_string()))?;

    let conn_a: Arc<dyn webrtc_util_dtls::Conn + Send + Sync> = Arc::new(sock_a);
    let conn_b: Arc<dyn webrtc_util_dtls::Conn + Send + Sync> = Arc::new(sock_b);

    let config = DtlsAdapterConfig {
        insecure_skip_verify: true,
        ..DtlsAdapterConfig::default()
    };

    let mut client = DtlsConnectionAdapter::new(DtlsRole::Client).await?;
    let mut server = DtlsConnectionAdapter::new(DtlsRole::Server).await?;

    // Run both handshakes concurrently — they need each other to complete.
    let config_c = config.clone();
    let client_handle = tokio::spawn(async move {
        client.handshake(conn_a, &config_c).await?;
        client.get_srtp_keys().await
    });

    let server_handle = tokio::spawn(async move {
        server.handshake(conn_b, &config).await?;
        server.get_srtp_keys().await
    });

    let client_keys = client_handle
        .await
        .map_err(|e| rvoip_rtp_core::Error::IoError(format!("Client task panicked: {e}")))??;
    let server_keys = server_handle
        .await
        .map_err(|e| rvoip_rtp_core::Error::IoError(format!("Server task panicked: {e}")))??;

    Ok((client_keys, server_keys))
}

// ---------------------------------------------------------------------------
// Test 1: DTLS handshake -> SRTP key extraction
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_dtls_handshake_srtp_key_extraction() -> Result<(), rvoip_rtp_core::Error> {
    tokio::time::timeout(Duration::from_secs(15), async {
    let (client_keys, server_keys) = dtls_handshake_loopback().await?;

    // The client's local (tx) key must equal the server's remote (rx) key —
    // this is how SRTP symmetry works.
    assert_eq!(
        client_keys.local_key, server_keys.remote_key,
        "Client tx key must match server rx key"
    );
    assert_eq!(
        client_keys.local_salt, server_keys.remote_salt,
        "Client tx salt must match server rx salt"
    );

    // And vice versa.
    assert_eq!(
        server_keys.local_key, client_keys.remote_key,
        "Server tx key must match client rx key"
    );
    assert_eq!(
        server_keys.local_salt, client_keys.remote_salt,
        "Server tx salt must match client rx salt"
    );

    // Both sides must agree on the negotiated SRTP profile.
    assert_eq!(
        client_keys.profile, server_keys.profile,
        "Both sides must negotiate the same SRTP profile"
    );

    // Keys must not be all zeros (sanity check).
    assert!(
        client_keys.local_key.iter().any(|&b| b != 0),
        "Extracted key must not be all zeros"
    );

    Ok(())
    }).await.map_err(|_| rvoip_rtp_core::Error::IoError("Test timed out".to_string()))?
}

// ---------------------------------------------------------------------------
// Test 2: SRTP encrypt -> decrypt roundtrip via adapters
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_srtp_encrypt_decrypt_roundtrip() -> Result<(), rvoip_rtp_core::Error> {
    tokio::time::timeout(Duration::from_secs(15), async {
    // Use fixed known keys (AES-128-CM-HMAC-SHA1-80: 16-byte key, 14-byte salt).
    let key_a: Vec<u8> = (0..16).collect();
    let salt_a: Vec<u8> = (16..30).collect();
    let key_b: Vec<u8> = (32..48).collect();
    let salt_b: Vec<u8> = (48..62).collect();

    let mut sender = SrtpContextAdapter::new(
        &key_a, &salt_a,
        &key_b, &salt_b,
        ProtectionProfile::Aes128CmHmacSha1_80,
    )?;

    let mut receiver = SrtpContextAdapter::new(
        &key_b, &salt_b,
        &key_a, &salt_a,
        ProtectionProfile::Aes128CmHmacSha1_80,
    )?;

    let payload = b"Hello, SRTP world!";
    let rtp_packet = build_rtp_packet(1, payload);

    // Protect (encrypt + authenticate)
    let protected = sender.protect_rtp(&rtp_packet)?;
    assert_ne!(
        protected.as_ref(),
        &rtp_packet[..],
        "Protected packet must differ from plaintext"
    );

    // Unprotect (verify + decrypt)
    let unprotected = receiver.unprotect_rtp(&protected)?;
    assert_eq!(
        &unprotected[..],
        &rtp_packet[..],
        "Decrypted packet must match the original"
    );

    Ok(())
    }).await.map_err(|_| rvoip_rtp_core::Error::IoError("Test timed out".to_string()))?
}

// ---------------------------------------------------------------------------
// Test 3: Full pipeline -- DTLS handshake -> SRTP -> RTP packet over UDP
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_full_pipeline_dtls_srtp_rtp() -> Result<(), rvoip_rtp_core::Error> {
    tokio::time::timeout(Duration::from_secs(15), async {
    use tokio::net::UdpSocket;

    // --- Step 1: DTLS handshake ---
    let (client_keys, server_keys) = dtls_handshake_loopback().await?;

    // --- Step 2: Create SRTP contexts from the extracted keys ---
    let mut client_srtp = SrtpContextAdapter::from_key_material(&client_keys)?;
    let mut server_srtp = SrtpContextAdapter::from_key_material(&server_keys)?;

    // --- Step 3: Set up a separate UDP channel for media transport ---
    let media_tx_sock = UdpSocket::bind("127.0.0.1:0").await
        .map_err(|e| rvoip_rtp_core::Error::IoError(e.to_string()))?;
    let media_rx_sock = UdpSocket::bind("127.0.0.1:0").await
        .map_err(|e| rvoip_rtp_core::Error::IoError(e.to_string()))?;

    let rx_addr = media_rx_sock.local_addr()
        .map_err(|e| rvoip_rtp_core::Error::IoError(e.to_string()))?;

    // --- Step 4: Client creates RTP, protects, and sends ---
    let payload = b"audio frame data 1234567890";
    let rtp_packet = build_rtp_packet(42, payload);
    let srtp_packet = client_srtp.protect_rtp(&rtp_packet)?;

    media_tx_sock.send_to(&srtp_packet, rx_addr).await
        .map_err(|e| rvoip_rtp_core::Error::IoError(e.to_string()))?;

    // --- Step 5: Server receives, unprotects, and verifies ---
    let mut recv_buf = vec![0u8; 2048];
    let (n, _from) = media_rx_sock.recv_from(&mut recv_buf).await
        .map_err(|e| rvoip_rtp_core::Error::IoError(e.to_string()))?;

    let decrypted = server_srtp.unprotect_rtp(&recv_buf[..n])?;
    assert_eq!(
        &decrypted[..],
        &rtp_packet[..],
        "Received and decrypted packet must match original RTP"
    );

    // Verify the payload portion specifically.
    // RTP header is 12 bytes (no CSRC, no extensions).
    assert_eq!(
        &decrypted[12..],
        payload,
        "Decrypted payload must match original"
    );

    Ok(())
    }).await.map_err(|_| rvoip_rtp_core::Error::IoError("Test timed out".to_string()))?
}

// ---------------------------------------------------------------------------
// Test 4: SRTCP protect/unprotect roundtrip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_srtcp_protect_unprotect_roundtrip() -> Result<(), rvoip_rtp_core::Error> {
    tokio::time::timeout(Duration::from_secs(15), async {
    let key_a: Vec<u8> = (0..16).collect();
    let salt_a: Vec<u8> = (16..30).collect();
    let key_b: Vec<u8> = (32..48).collect();
    let salt_b: Vec<u8> = (48..62).collect();

    let mut sender = SrtpContextAdapter::new(
        &key_a, &salt_a,
        &key_b, &salt_b,
        ProtectionProfile::Aes128CmHmacSha1_80,
    )?;

    let mut receiver = SrtpContextAdapter::new(
        &key_b, &salt_b,
        &key_a, &salt_a,
        ProtectionProfile::Aes128CmHmacSha1_80,
    )?;

    let rtcp_sr = build_rtcp_sender_report();

    // Protect
    let protected = sender.protect_rtcp(&rtcp_sr)?;
    assert_ne!(
        protected.as_ref(),
        &rtcp_sr[..],
        "Protected RTCP must differ from plaintext"
    );

    // Unprotect
    let unprotected = receiver.unprotect_rtcp(&protected)?;
    assert_eq!(
        &unprotected[..],
        &rtcp_sr[..],
        "Decrypted RTCP must match original Sender Report"
    );

    Ok(())
    }).await.map_err(|_| rvoip_rtp_core::Error::IoError("Test timed out".to_string()))?
}

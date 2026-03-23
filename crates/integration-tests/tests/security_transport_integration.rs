//! Cross-crate integration tests for SRTP transport security.
//!
//! Tests the SecurityRtpTransport wrapper around UdpRtpTransport, verifying
//! that SRTP-enabled transports correctly enforce encryption requirements
//! and refuse to send/receive unencrypted packets when SRTP is enabled.
//!
//! RFC 5764: When SRTP is negotiated, all RTP/RTCP packets MUST be protected.

use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use serial_test::serial;

use rvoip_rtp_core::transport::{RtpTransport, RtpTransportConfig, UdpRtpTransport};
use rvoip_rtp_core::transport::SecurityRtpTransport;
use rvoip_rtp_core::RtpPacket;

/// Create a UdpRtpTransport bound to an ephemeral port on localhost.
async fn create_udp_transport() -> (UdpRtpTransport, SocketAddr) {
    let config = RtpTransportConfig {
        local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
        local_rtcp_addr: None,
        symmetric_rtp: true,
        rtcp_mux: true,
        session_id: None,
        use_port_allocator: false,
    };

    let transport = UdpRtpTransport::new(config)
        .await
        .expect("UdpRtpTransport::new should succeed");
    let addr = transport
        .local_rtp_addr()
        .expect("should have local address");

    (transport, addr)
}

/// Build a minimal RTP packet for testing.
fn build_test_rtp_packet(seq: u16, ts: u32) -> RtpPacket {
    let payload = Bytes::from(vec![0x80u8; 160]); // 160 bytes of audio
    RtpPacket::new_with_payload(
        0,    // payload type 0 = PCMU
        seq,
        ts,
        0x12345678, // SSRC
        payload,
    )
}

// =============================================================================
// Test 1: SecurityRtpTransport wraps UdpRtpTransport and preserves address
// =============================================================================

#[tokio::test]
#[serial]
async fn test_security_transport_wraps_udp_transport() {
    let (transport, addr) = create_udp_transport().await;
    let inner = Arc::new(transport);

    // Create security transport with SRTP disabled
    let security_transport = SecurityRtpTransport::new(inner.clone(), false)
        .await
        .expect("SecurityRtpTransport::new should succeed");

    // Local address should be the same as the inner transport
    let security_addr = security_transport
        .local_rtp_addr()
        .expect("should have local address");
    assert_eq!(
        security_addr, addr,
        "Security transport should delegate local_rtp_addr to inner transport"
    );

    // Inner transport should be accessible
    let inner_ref = security_transport.inner_transport();
    let inner_addr = inner_ref
        .local_rtp_addr()
        .expect("inner should have address");
    assert_eq!(inner_addr, addr);
}

// =============================================================================
// Test 2: SRTP-enabled transport refuses to send without SRTP context (RFC 5764)
// =============================================================================

#[tokio::test]
#[serial]
async fn test_srtp_enabled_refuses_send_without_context() {
    let (transport, _addr) = create_udp_transport().await;
    let inner = Arc::new(transport);

    // Create security transport with SRTP ENABLED but no context set
    let security_transport = SecurityRtpTransport::new(inner, true)
        .await
        .expect("SecurityRtpTransport::new should succeed");

    // SRTP should not be "ready" (no context)
    assert!(
        !security_transport.is_srtp_ready().await,
        "SRTP should not be ready without a context"
    );

    // Attempt to send an RTP packet should fail (per RFC 5764: MUST NOT send unencrypted)
    let packet = build_test_rtp_packet(1, 160);
    let dest: SocketAddr = "127.0.0.1:9999".parse().unwrap();
    let result = security_transport.send_rtp(&packet, dest).await;

    assert!(
        result.is_err(),
        "Sending RTP with SRTP enabled but no context should fail"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("SRTP") || err_msg.contains("srtp") || err_msg.contains("context"),
        "Error should mention SRTP context issue, got: {}",
        err_msg
    );
}

// =============================================================================
// Test 3: SRTP-enabled transport refuses raw bytes send
// =============================================================================

#[tokio::test]
#[serial]
async fn test_srtp_enabled_refuses_raw_bytes_send() {
    let (transport, _addr) = create_udp_transport().await;
    let inner = Arc::new(transport);

    let security_transport = SecurityRtpTransport::new(inner, true)
        .await
        .expect("SecurityRtpTransport::new should succeed");

    // Attempt to send raw bytes with SRTP enabled should fail
    let raw_bytes = vec![0x80, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0xA0, 0x12, 0x34, 0x56, 0x78];
    let dest: SocketAddr = "127.0.0.1:9999".parse().unwrap();
    let result = security_transport.send_rtp_bytes(&raw_bytes, dest).await;

    assert!(
        result.is_err(),
        "Sending raw RTP bytes with SRTP enabled should fail"
    );
}

// =============================================================================
// Test 4: SRTP-disabled transport allows plain RTP send
// =============================================================================

#[tokio::test]
#[serial]
async fn test_srtp_disabled_allows_plain_send() {
    let (sender_transport, _sender_addr) = create_udp_transport().await;
    let (receiver_transport, receiver_addr) = create_udp_transport().await;
    let sender_inner = Arc::new(sender_transport);

    // Create security transport with SRTP DISABLED
    let security_transport = SecurityRtpTransport::new(sender_inner, false)
        .await
        .expect("SecurityRtpTransport::new should succeed");

    // Send should succeed when SRTP is disabled (plain RTP is allowed)
    let packet = build_test_rtp_packet(1, 160);
    let result = security_transport.send_rtp(&packet, receiver_addr).await;

    assert!(
        result.is_ok(),
        "Sending plain RTP with SRTP disabled should succeed, got: {:?}",
        result.err()
    );

    // Clean up
    receiver_transport
        .close()
        .await
        .expect("close receiver");
}

// =============================================================================
// Test 5: SRTP-enabled transport refuses RTCP without context
// =============================================================================

#[tokio::test]
#[serial]
async fn test_srtp_enabled_refuses_rtcp_without_context() {
    let (transport, _addr) = create_udp_transport().await;
    let inner = Arc::new(transport);

    let security_transport = SecurityRtpTransport::new(inner, true)
        .await
        .expect("SecurityRtpTransport::new should succeed");

    // Attempt to send raw RTCP bytes with SRTP enabled but no context
    let rtcp_bytes = vec![0x80, 0xC8, 0x00, 0x06]; // SR packet stub
    let dest: SocketAddr = "127.0.0.1:9999".parse().unwrap();
    let result = security_transport.send_rtcp_bytes(&rtcp_bytes, dest).await;

    assert!(
        result.is_err(),
        "Sending RTCP with SRTP enabled but no context should fail"
    );
}

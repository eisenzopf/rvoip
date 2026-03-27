//! Cross-crate integration tests for SIP-over-SCTP (RFC 4168)
//!
//! These tests validate SCTP transport integration with the SIP stack,
//! including transport creation, message formatting, and transport type
//! detection.

#![cfg(feature = "sctp")]

use std::net::SocketAddr;
use std::time::Duration;

use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::{Message, Method};
use rvoip_sip_transport::transport::sctp::{SctpConfig, SctpTransport};
use rvoip_sip_transport::transport::{Transport, TransportEvent, TransportType};

/// Validates that the SCTP transport type integrates with the transport
/// type enum and display formatting.
#[tokio::test]
async fn test_sctp_transport_type_display() {
    let sctp_type = TransportType::Sctp;
    assert_eq!(format!("{}", sctp_type), "SCTP");
}

/// Validates that SCTP transport can be created and reports correct
/// transport type capabilities.
#[tokio::test]
async fn test_sctp_transport_capabilities_integration() {
    let (transport, _rx) = SctpTransport::bind(
        "127.0.0.1:0"
            .parse()
            .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 0))),
        None,
        None,
    )
    .await
    .unwrap_or_else(|e| panic!("bind failed: {}", e));

    // SCTP transport should only support SCTP
    assert!(transport.supports_transport(TransportType::Sctp));
    assert!(!transport.supports_transport(TransportType::Udp));
    assert!(!transport.supports_transport(TransportType::Tcp));
    assert!(!transport.supports_transport(TransportType::Tls));
    assert!(!transport.supports_transport(TransportType::Ws));
    assert!(!transport.supports_transport(TransportType::Wss));

    // Connection status
    assert!(!transport.is_closed());
    assert!(transport.is_transport_connected(TransportType::Sctp));

    transport
        .close()
        .await
        .unwrap_or_else(|e| panic!("close failed: {}", e));

    assert!(transport.is_closed());
    assert!(!transport.is_transport_connected(TransportType::Sctp));
}

/// Validates that SIP messages with SCTP Via headers are properly
/// constructed and can be serialized.
#[tokio::test]
async fn test_sip_message_with_sctp_via_header() {
    // Build a SIP INVITE with an SCTP Via header
    let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
        .unwrap_or_else(|e| panic!("Failed to create request: {}", e))
        .from("alice", "sip:alice@example.com", Some("tag1"))
        .to("bob", "sip:bob@example.com", None)
        .call_id("sctp-integration-1@example.com")
        .cseq(1)
        .via("192.168.1.1:5060", "SCTP", Some("z9hG4bK-sctp-test"))
        .build();

    let message: Message = request.into();
    let bytes = message.to_bytes();
    let message_str = String::from_utf8_lossy(&bytes);

    // Verify the Via header contains SCTP transport
    assert!(
        message_str.contains("SIP/2.0/SCTP"),
        "Via header should contain SCTP transport. Message:\n{}",
        message_str
    );
}

/// Validates that SCTP transport properly handles the close lifecycle.
#[tokio::test]
async fn test_sctp_transport_lifecycle() {
    let (transport, _rx) = SctpTransport::bind(
        "127.0.0.1:0"
            .parse()
            .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 0))),
        Some(10),
        Some(SctpConfig {
            max_receive_buffer_size: 512 * 1024,
            max_message_size: 32768,
        }),
    )
    .await
    .unwrap_or_else(|e| panic!("bind failed: {}", e));

    let addr = transport
        .local_addr()
        .unwrap_or_else(|e| panic!("local_addr failed: {}", e));
    assert!(addr.port() > 0);
    assert!(!transport.is_closed());

    // Close
    transport
        .close()
        .await
        .unwrap_or_else(|e| panic!("close failed: {}", e));
    assert!(transport.is_closed());

    // Sending after close should fail
    let request = SimpleRequestBuilder::new(Method::Options, "sip:example.com")
        .unwrap_or_else(|e| panic!("Failed to create request: {}", e))
        .from("alice", "sip:alice@example.com", Some("tag1"))
        .to("bob", "sip:bob@example.com", None)
        .call_id("lifecycle-test@example.com")
        .cseq(1)
        .build();

    let result = transport
        .send_message(
            request.into(),
            "127.0.0.1:5060"
                .parse()
                .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 5060))),
        )
        .await;
    assert!(result.is_err());
}

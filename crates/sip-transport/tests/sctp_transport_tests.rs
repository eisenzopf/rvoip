//! Integration tests for SCTP transport (RFC 4168)
//!
//! These tests validate the SIP-over-SCTP transport implementation using
//! user-space SCTP (webrtc-sctp) over UDP loopback.

#![cfg(feature = "sctp")]

use std::net::SocketAddr;
use std::time::Duration;

use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::{Message, Method};
use rvoip_sip_transport::transport::sctp::{SctpConfig, SctpTransport};
use rvoip_sip_transport::transport::{Transport, TransportEvent, TransportType};

/// Helper to create a bound SCTP transport on loopback with a random port
async fn create_transport() -> (SctpTransport, tokio::sync::mpsc::Receiver<TransportEvent>) {
    SctpTransport::bind(
        "127.0.0.1:0"
            .parse()
            .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 0))),
        Some(50),
        None,
    )
    .await
    .unwrap_or_else(|e| panic!("Failed to create SCTP transport: {}", e))
}

/// Helper to build a SIP INVITE request
fn build_invite(call_id: &str) -> Message {
    let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
        .unwrap_or_else(|e| panic!("Failed to create request: {}", e))
        .from("alice", "sip:alice@example.com", Some("tag1"))
        .to("bob", "sip:bob@example.com", None)
        .call_id(call_id)
        .cseq(1)
        .build();
    request.into()
}

/// Helper to build a SIP REGISTER request
fn build_register(call_id: &str) -> Message {
    let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com")
        .unwrap_or_else(|e| panic!("Failed to create request: {}", e))
        .from("alice", "sip:alice@example.com", Some("tag1"))
        .to("bob", "sip:bob@example.com", None)
        .call_id(call_id)
        .cseq(1)
        .build();
    request.into()
}

#[tokio::test]
async fn test_sctp_transport_bind_and_local_addr() {
    let (transport, _rx) = create_transport().await;

    let addr = transport
        .local_addr()
        .unwrap_or_else(|e| panic!("local_addr failed: {}", e));
    assert!(addr.port() > 0);
    assert_eq!(addr.ip(), "127.0.0.1".parse::<std::net::IpAddr>().unwrap_or_else(|e| panic!("parse failed: {}", e)));

    transport
        .close()
        .await
        .unwrap_or_else(|e| panic!("close failed: {}", e));
}

#[tokio::test]
async fn test_sctp_transport_type_support() {
    let (transport, _rx) = create_transport().await;

    assert!(transport.supports_sctp());
    assert!(!transport.supports_udp());
    assert!(!transport.supports_tcp());
    assert!(!transport.supports_tls());
    assert!(!transport.supports_ws());
    assert!(!transport.supports_wss());
    assert_eq!(transport.default_transport_type(), TransportType::Sctp);
    assert!(transport.supports_transport(TransportType::Sctp));

    transport
        .close()
        .await
        .unwrap_or_else(|e| panic!("close failed: {}", e));
}

#[tokio::test]
async fn test_sctp_transport_close_idempotent() {
    let (transport, _rx) = create_transport().await;

    assert!(!transport.is_closed());

    // First close
    transport
        .close()
        .await
        .unwrap_or_else(|e| panic!("first close failed: {}", e));
    assert!(transport.is_closed());

    // Second close should also succeed
    transport
        .close()
        .await
        .unwrap_or_else(|e| panic!("second close failed: {}", e));
    assert!(transport.is_closed());
}

#[tokio::test]
async fn test_sctp_transport_send_after_close_fails() {
    let (transport, _rx) = create_transport().await;

    transport
        .close()
        .await
        .unwrap_or_else(|e| panic!("close failed: {}", e));

    let message = build_invite("closed-test@example.com");
    let result = transport
        .send_message(
            message,
            "127.0.0.1:5060"
                .parse()
                .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 5060))),
        )
        .await;

    assert!(result.is_err(), "Expected error when sending after close");
}

#[tokio::test]
async fn test_sctp_transport_custom_config() {
    let config = SctpConfig {
        max_receive_buffer_size: 2 * 1024 * 1024,
        max_message_size: 128000,
    };

    let (transport, _rx) = SctpTransport::bind(
        "127.0.0.1:0"
            .parse()
            .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 0))),
        Some(20),
        Some(config),
    )
    .await
    .unwrap_or_else(|e| panic!("Failed to create SCTP transport with config: {}", e));

    assert!(!transport.is_closed());

    transport
        .close()
        .await
        .unwrap_or_else(|e| panic!("close failed: {}", e));
}

#[tokio::test]
async fn test_sctp_transport_debug_format() {
    let (transport, _rx) = create_transport().await;

    let debug_str = format!("{:?}", transport);
    assert!(
        debug_str.contains("SctpTransport"),
        "Debug format should contain 'SctpTransport', got: {}",
        debug_str
    );
    assert!(
        debug_str.contains("127.0.0.1"),
        "Debug format should contain address, got: {}",
        debug_str
    );

    transport
        .close()
        .await
        .unwrap_or_else(|e| panic!("close failed: {}", e));
}

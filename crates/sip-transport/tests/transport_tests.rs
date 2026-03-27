//! Comprehensive transport tests for the sip-transport crate.
//!
//! Tests cover UDP send/receive, TCP send/receive, transport events,
//! concurrent sends, large messages, and transport shutdown behavior.

use std::net::SocketAddr;
use std::time::Duration;

use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::types::{Method, StatusCode};
use rvoip_sip_core::Message;
use rvoip_sip_transport::prelude::*;

/// Helper: build a minimal SIP INVITE request message.
fn build_invite_message() -> Message {
    let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
        .expect("valid URI")
        .from("alice", "sip:alice@example.com", Some("tag-alice-1"))
        .to("bob", "sip:bob@example.com", None)
        .call_id("test-call-001@transport-test")
        .cseq(1)
        .build();
    request.into()
}

/// Helper: build a minimal SIP 200 OK response message.
fn build_200_ok_response() -> Message {
    let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
        .from("alice", "sip:alice@example.com", Some("tag-alice-1"))
        .to("bob", "sip:bob@example.com", Some("tag-bob-1"))
        .call_id("test-call-002@transport-test")
        .cseq(1, Method::Invite)
        .build();
    response.into()
}

/// Helper: build a SIP REGISTER request.
fn build_register_message() -> Message {
    let request = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com")
        .expect("valid URI")
        .from("alice", "sip:alice@example.com", Some("reg-tag"))
        .to("alice", "sip:alice@example.com", None)
        .call_id("register-001@transport-test")
        .cseq(1)
        .build();
    request.into()
}

// =============================================================================
// Test 1: UDP send/receive SIP message
// =============================================================================
#[tokio::test]
async fn test_udp_send_receive_invite() {
    let addr_a: SocketAddr = "127.0.0.1:0".parse().expect("valid addr");
    let addr_b: SocketAddr = "127.0.0.1:0".parse().expect("valid addr");

    let (transport_a, _rx_a) = UdpTransport::bind(addr_a, None)
        .await
        .expect("bind A failed");
    let (transport_b, mut rx_b) = UdpTransport::bind(addr_b, None)
        .await
        .expect("bind B failed");

    let local_b = transport_b.local_addr().expect("local_addr B");

    let message = build_invite_message();
    transport_a
        .send_message(message.clone(), local_b)
        .await
        .expect("send failed");

    let event = tokio::time::timeout(Duration::from_secs(5), rx_b.recv())
        .await
        .expect("timeout waiting for message")
        .expect("channel closed");

    match event {
        TransportEvent::MessageReceived {
            message: received,
            destination,
            ..
        } => {
            assert_eq!(destination, local_b, "destination should match B's address");
            assert!(received.is_request(), "expected a request message");
            if let Message::Request(req) = &received {
                assert_eq!(req.method(), Method::Invite, "method should be INVITE");
            }
        }
        other => panic!("expected MessageReceived, got {:?}", other),
    }

    transport_a.close().await.expect("close A");
    transport_b.close().await.expect("close B");
}

// =============================================================================
// Test 2: UDP send/receive SIP response
// =============================================================================
#[tokio::test]
async fn test_udp_send_receive_response() {
    let addr_a: SocketAddr = "127.0.0.1:0".parse().expect("valid addr");
    let addr_b: SocketAddr = "127.0.0.1:0".parse().expect("valid addr");

    let (transport_a, _rx_a) = UdpTransport::bind(addr_a, None)
        .await
        .expect("bind A");
    let (transport_b, mut rx_b) = UdpTransport::bind(addr_b, None)
        .await
        .expect("bind B");

    let local_b = transport_b.local_addr().expect("local_addr B");

    let response_msg = build_200_ok_response();
    transport_a
        .send_message(response_msg, local_b)
        .await
        .expect("send failed");

    let event = tokio::time::timeout(Duration::from_secs(5), rx_b.recv())
        .await
        .expect("timeout")
        .expect("channel closed");

    match event {
        TransportEvent::MessageReceived {
            message: received, ..
        } => {
            assert!(received.is_response(), "expected a response message");
            if let Message::Response(resp) = &received {
                assert_eq!(
                    resp.status(),
                    StatusCode::Ok,
                    "status should be 200 OK"
                );
            }
        }
        other => panic!("expected MessageReceived, got {:?}", other),
    }

    transport_a.close().await.expect("close A");
    transport_b.close().await.expect("close B");
}

// =============================================================================
// Test 3: TCP transport send/receive
// =============================================================================
#[tokio::test]
async fn test_tcp_send_receive() {
    let (server_transport, mut server_rx) = TcpTransport::bind(
        "127.0.0.1:0".parse().expect("valid addr"),
        Some(10),
        None,
    )
    .await
    .expect("TCP server bind failed");

    let server_addr = server_transport.local_addr().expect("server local_addr");

    let (client_transport, _client_rx) = TcpTransport::bind(
        "127.0.0.1:0".parse().expect("valid addr"),
        Some(10),
        None,
    )
    .await
    .expect("TCP client bind failed");

    let message = build_register_message();
    client_transport
        .send_message(message, server_addr)
        .await
        .expect("TCP send failed");

    let event = tokio::time::timeout(Duration::from_secs(5), server_rx.recv())
        .await
        .expect("timeout waiting for TCP message")
        .expect("channel closed");

    match event {
        TransportEvent::MessageReceived {
            message: received,
            destination,
            ..
        } => {
            assert_eq!(destination, server_addr, "destination should be server addr");
            assert!(received.is_request(), "expected a request message");
            if let Message::Request(req) = &received {
                assert_eq!(
                    req.method(),
                    Method::Register,
                    "method should be REGISTER"
                );
            }
        }
        other => panic!("expected MessageReceived, got {:?}", other),
    }

    client_transport.close().await.expect("close client");
    server_transport.close().await.expect("close server");
}

// =============================================================================
// Test 4: Transport event handling -- verify Closed event on close
// =============================================================================
#[tokio::test]
async fn test_transport_closed_event() {
    let (transport, mut rx) = UdpTransport::bind(
        "127.0.0.1:0".parse().expect("valid addr"),
        None,
    )
    .await
    .expect("bind failed");

    assert!(!transport.is_closed(), "should not be closed initially");

    transport.close().await.expect("close failed");
    assert!(transport.is_closed(), "should be closed after close()");

    // Drain events and look for at least one Closed event
    let mut found_closed = false;
    loop {
        match tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
            Ok(Some(TransportEvent::Closed)) => {
                found_closed = true;
                break;
            }
            Ok(Some(_)) => {
                // Keep draining other events
                continue;
            }
            Ok(None) | Err(_) => break,
        }
    }

    assert!(found_closed, "expected to receive a Closed event");
}

// =============================================================================
// Test 5: Multiple concurrent UDP sends
// =============================================================================
#[tokio::test]
async fn test_multiple_concurrent_udp_sends() {
    let (sender_transport, _sender_rx) = UdpTransport::bind(
        "127.0.0.1:0".parse().expect("valid addr"),
        Some(200),
    )
    .await
    .expect("bind sender");

    let (receiver_transport, mut receiver_rx) = UdpTransport::bind(
        "127.0.0.1:0".parse().expect("valid addr"),
        Some(200),
    )
    .await
    .expect("bind receiver");

    let receiver_addr = receiver_transport.local_addr().expect("receiver addr");

    let message_count = 100usize;

    // Send 100 messages rapidly
    for _ in 0..message_count {
        let msg = build_invite_message();
        // Fire-and-forget style: just verify no panics
        if let Err(e) = sender_transport.send_message(msg, receiver_addr).await {
            // UDP send failures are acceptable under load but should not panic
            eprintln!("send error (acceptable under load): {}", e);
        }
    }

    // Collect received messages with a reasonable timeout
    let mut received_count = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, receiver_rx.recv()).await {
            Ok(Some(TransportEvent::MessageReceived { .. })) => {
                received_count += 1;
            }
            Ok(Some(_)) => {
                // Ignore non-message events
            }
            Ok(None) | Err(_) => break,
        }
    }

    // UDP may lose some packets but we should receive a reasonable amount
    assert!(
        received_count > 0,
        "should have received at least some messages, got 0"
    );
    // On localhost, most should arrive
    assert!(
        received_count >= message_count / 2,
        "expected at least half the messages on localhost, got {}/{}",
        received_count,
        message_count
    );

    sender_transport.close().await.expect("close sender");
    receiver_transport.close().await.expect("close receiver");
}

// =============================================================================
// Test 6: Large SIP message (> MTU) via TCP
// =============================================================================
#[tokio::test]
async fn test_large_sip_message_tcp() {
    let (server_transport, mut server_rx) = TcpTransport::bind(
        "127.0.0.1:0".parse().expect("valid addr"),
        Some(10),
        None,
    )
    .await
    .expect("TCP server bind");

    let server_addr = server_transport.local_addr().expect("server addr");

    let (client_transport, _client_rx) = TcpTransport::bind(
        "127.0.0.1:0".parse().expect("valid addr"),
        Some(10),
        None,
    )
    .await
    .expect("TCP client bind");

    // Build a message with a large SDP body (> 1400 bytes to exceed typical MTU)
    let mut large_sdp = String::from("v=0\r\no=alice 12345 12345 IN IP4 127.0.0.1\r\ns=Large Test Session\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n");
    // Add many media attributes to exceed MTU
    for i in 0..100 {
        large_sdp.push_str(&format!(
            "a=rtpmap:{} opus/48000/2\r\n",
            (96 + (i % 32)) as u8
        ));
    }

    let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
        .expect("valid URI")
        .from("alice", "sip:alice@example.com", Some("tag-large"))
        .to("bob", "sip:bob@example.com", None)
        .call_id("large-msg-001@transport-test")
        .cseq(1)
        .content_type("application/sdp")
        .body(large_sdp.as_bytes().to_vec())
        .build();

    let message: Message = request.into();
    let serialized_len = message.to_bytes().len();
    assert!(
        serialized_len > 1400,
        "message should be larger than typical MTU, got {} bytes",
        serialized_len
    );

    client_transport
        .send_message(message, server_addr)
        .await
        .expect("TCP send of large message failed");

    let event = tokio::time::timeout(Duration::from_secs(5), server_rx.recv())
        .await
        .expect("timeout waiting for large message")
        .expect("channel closed");

    match event {
        TransportEvent::MessageReceived {
            message: received, ..
        } => {
            assert!(received.is_request(), "expected a request");
            if let Message::Request(req) = &received {
                assert_eq!(req.method(), Method::Invite);
                // Verify the body was received intact
                let body = req.body();
                assert!(
                    body.len() > 1400,
                    "body should be large, got {} bytes",
                    body.len()
                );
            }
        }
        other => panic!("expected MessageReceived, got {:?}", other),
    }

    client_transport.close().await.expect("close client");
    server_transport.close().await.expect("close server");
}

// =============================================================================
// Test 7: Transport close/shutdown -- send after close returns error
// =============================================================================
#[tokio::test]
async fn test_send_after_close_returns_error() {
    let (transport, _rx) = UdpTransport::bind(
        "127.0.0.1:0".parse().expect("valid addr"),
        None,
    )
    .await
    .expect("bind failed");

    let target: SocketAddr = "127.0.0.1:9999".parse().expect("valid addr");

    // Verify send works before close
    let msg = build_invite_message();
    let result = transport.send_message(msg, target).await;
    assert!(result.is_ok(), "send before close should succeed");

    // Close the transport
    transport.close().await.expect("close failed");
    assert!(transport.is_closed(), "transport should be closed");

    // Verify send after close returns error
    let msg2 = build_invite_message();
    let result = transport.send_message(msg2, target).await;
    assert!(
        result.is_err(),
        "send after close should return an error"
    );

    // Verify the error indicates closed transport
    if let Err(e) = result {
        assert!(
            e.is_connection_closed(),
            "error should indicate transport is closed, got: {}",
            e
        );
    }
}

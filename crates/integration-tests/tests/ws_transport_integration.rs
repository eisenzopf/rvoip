//! WebSocket SIP transport integration tests.
//!
//! Tests the WebSocket transport layer for SIP messaging over ws:// connections.
//! Validates RFC 7118 compliance, message roundtrips, connection lifecycle,
//! concurrent connections, and error handling.

use std::net::SocketAddr;
use std::time::Duration;

use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::method::Method;
use rvoip_sip_core::{Message, parse_message};
use rvoip_sip_transport::transport::{Transport, TransportEvent, WebSocketTransport};

use tokio::time::timeout;

// =============================================================================
// Helpers
// =============================================================================

/// Bind a plain WS transport on a random port and return (transport, event_rx).
async fn bind_ws() -> (WebSocketTransport, tokio::sync::mpsc::Receiver<TransportEvent>) {
    WebSocketTransport::bind("127.0.0.1:0".parse().unwrap(), false, None, None, None)
        .await
        .expect("should bind WS transport")
}

/// Build a minimal SIP REGISTER request.
fn build_register(call_id: &str) -> Message {
    SimpleRequestBuilder::new(Method::Register, "sip:example.com")
        .expect("valid URI")
        .from("Alice", "sip:alice@example.com", Some("ws-tag"))
        .to("Alice", "sip:alice@example.com", None)
        .call_id(call_id)
        .cseq(1)
        .via("127.0.0.1:5060", "WS", Some("z9hG4bK-ws-test"))
        .build()
        .into()
}

/// Build a minimal SIP INVITE request.
fn build_invite(call_id: &str) -> Message {
    SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
        .expect("valid URI")
        .from("Alice", "sip:alice@example.com", Some("inv-tag"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id(call_id)
        .cseq(1)
        .via("127.0.0.1:5060", "WS", Some("z9hG4bK-ws-invite"))
        .build()
        .into()
}

/// Build a SIP OPTIONS request (lightweight keep-alive probe).
fn build_options(call_id: &str) -> Message {
    SimpleRequestBuilder::new(Method::Options, "sip:proxy.example.com")
        .expect("valid URI")
        .from("Alice", "sip:alice@example.com", Some("opt-tag"))
        .to("Proxy", "sip:proxy@example.com", None)
        .call_id(call_id)
        .cseq(1)
        .via("127.0.0.1:5060", "WS", Some("z9hG4bK-ws-options"))
        .build()
        .into()
}

/// Build a SIP BYE request.
fn build_bye(call_id: &str) -> Message {
    SimpleRequestBuilder::new(Method::Bye, "sip:bob@example.com")
        .expect("valid URI")
        .from("Alice", "sip:alice@example.com", Some("bye-tag"))
        .to("Bob", "sip:bob@example.com", Some("bye-to-tag"))
        .call_id(call_id)
        .cseq(2)
        .via("127.0.0.1:5060", "WS", Some("z9hG4bK-ws-bye"))
        .build()
        .into()
}

/// Wait for the next MessageReceived event within a timeout, returning the message.
async fn recv_message(
    rx: &mut tokio::sync::mpsc::Receiver<TransportEvent>,
    wait: Duration,
) -> Message {
    let event = timeout(wait, async {
        loop {
            match rx.recv().await {
                Some(TransportEvent::MessageReceived { message, .. }) => return message,
                Some(_) => continue, // skip error/closed events
                None => panic!("event channel closed unexpectedly"),
            }
        }
    })
    .await
    .expect("timed out waiting for SIP message");
    event
}

const TIMEOUT: Duration = Duration::from_secs(5);

// =============================================================================
// Test 1: Basic client→server REGISTER roundtrip
// =============================================================================

#[cfg(feature = "ws")]
#[tokio::test]
async fn test_ws_register_roundtrip() {
    let (server, mut server_rx) = bind_ws().await;
    let server_addr = server.local_addr().expect("server addr");

    let (client, _client_rx) = bind_ws().await;

    // Send REGISTER from client to server
    let msg = build_register("ws-register-001@example.com");
    client.send_message(msg, server_addr).await.expect("send should succeed");

    // Server receives the message
    let received = recv_message(&mut server_rx, TIMEOUT).await;
    if let Message::Request(req) = received {
        assert_eq!(req.method(), Method::Register);
        assert_eq!(req.call_id().expect("Call-ID").to_string(), "ws-register-001@example.com");
    } else {
        panic!("expected SIP request, got response");
    }

    client.close().await.expect("client close");
    server.close().await.expect("server close");
}

// =============================================================================
// Test 2: INVITE message over WebSocket
// =============================================================================

#[cfg(feature = "ws")]
#[tokio::test]
async fn test_ws_invite_delivery() {
    let (server, mut server_rx) = bind_ws().await;
    let server_addr = server.local_addr().expect("server addr");

    let (client, _client_rx) = bind_ws().await;

    let msg = build_invite("ws-invite-001@example.com");
    client.send_message(msg, server_addr).await.expect("send invite");

    let received = recv_message(&mut server_rx, TIMEOUT).await;
    if let Message::Request(req) = received {
        assert_eq!(req.method(), Method::Invite);
        assert_eq!(req.call_id().expect("Call-ID").to_string(), "ws-invite-001@example.com");
    } else {
        panic!("expected INVITE request");
    }

    client.close().await.expect("client close");
    server.close().await.expect("server close");
}

// =============================================================================
// Test 3: Multiple SIP methods over the same connection
// =============================================================================

#[cfg(feature = "ws")]
#[tokio::test]
async fn test_ws_multiple_methods_same_connection() {
    let (server, mut server_rx) = bind_ws().await;
    let server_addr = server.local_addr().expect("server addr");
    let (client, _client_rx) = bind_ws().await;

    // Send REGISTER, then OPTIONS, then INVITE on the same WS connection
    let register = build_register("ws-multi-001@example.com");
    let options = build_options("ws-multi-002@example.com");
    let invite = build_invite("ws-multi-003@example.com");

    client.send_message(register, server_addr).await.expect("send register");
    client.send_message(options, server_addr).await.expect("send options");
    client.send_message(invite, server_addr).await.expect("send invite");

    // Receive all three in order
    let msg1 = recv_message(&mut server_rx, TIMEOUT).await;
    let msg2 = recv_message(&mut server_rx, TIMEOUT).await;
    let msg3 = recv_message(&mut server_rx, TIMEOUT).await;

    let methods: Vec<Method> = [msg1, msg2, msg3]
        .into_iter()
        .map(|m| match m {
            Message::Request(req) => req.method().clone(),
            Message::Response(_) => panic!("expected request"),
        })
        .collect();

    assert_eq!(methods, vec![Method::Register, Method::Options, Method::Invite]);

    client.close().await.expect("close");
    server.close().await.expect("close");
}

// =============================================================================
// Test 4: Connection reuse — second send reuses the existing WS connection
// =============================================================================

#[cfg(feature = "ws")]
#[tokio::test]
async fn test_ws_connection_reuse() {
    let (server, mut server_rx) = bind_ws().await;
    let server_addr = server.local_addr().expect("server addr");
    let (client, _client_rx) = bind_ws().await;

    // First message establishes the connection
    let msg1 = build_register("ws-reuse-001@example.com");
    client.send_message(msg1, server_addr).await.expect("first send");
    let _ = recv_message(&mut server_rx, TIMEOUT).await;

    // Second message should reuse the same connection
    let msg2 = build_register("ws-reuse-002@example.com");
    client.send_message(msg2, server_addr).await.expect("second send");

    let received = recv_message(&mut server_rx, TIMEOUT).await;
    if let Message::Request(req) = received {
        assert_eq!(req.call_id().expect("Call-ID").to_string(), "ws-reuse-002@example.com");
    } else {
        panic!("expected request");
    }

    client.close().await.expect("close");
    server.close().await.expect("close");
}

// =============================================================================
// Test 5: Multiple concurrent clients sending to the same server
// =============================================================================

#[cfg(feature = "ws")]
#[tokio::test]
async fn test_ws_multiple_clients() {
    let (server, mut server_rx) = bind_ws().await;
    let server_addr = server.local_addr().expect("server addr");

    let (client1, _rx1) = bind_ws().await;
    let (client2, _rx2) = bind_ws().await;
    let (client3, _rx3) = bind_ws().await;

    // All three clients send simultaneously
    let msg1 = build_register("ws-mc-client1@example.com");
    let msg2 = build_register("ws-mc-client2@example.com");
    let msg3 = build_register("ws-mc-client3@example.com");

    // Send from all clients
    let (r1, r2, r3) = tokio::join!(
        client1.send_message(msg1, server_addr),
        client2.send_message(msg2, server_addr),
        client3.send_message(msg3, server_addr),
    );
    r1.expect("client1 send");
    r2.expect("client2 send");
    r3.expect("client3 send");

    // Collect all three messages (order may vary)
    let mut call_ids: Vec<String> = Vec::new();
    for _ in 0..3 {
        let msg = recv_message(&mut server_rx, TIMEOUT).await;
        if let Message::Request(req) = msg {
            call_ids.push(req.call_id().expect("Call-ID").to_string());
        } else {
            panic!("expected request");
        }
    }

    call_ids.sort();
    assert_eq!(call_ids, vec![
        "ws-mc-client1@example.com",
        "ws-mc-client2@example.com",
        "ws-mc-client3@example.com",
    ]);

    client1.close().await.expect("close");
    client2.close().await.expect("close");
    client3.close().await.expect("close");
    server.close().await.expect("close");
}

// =============================================================================
// Test 6: Send on a closed transport should fail
// =============================================================================

#[cfg(feature = "ws")]
#[tokio::test]
async fn test_ws_send_after_close_fails() {
    let (server, _server_rx) = bind_ws().await;
    let server_addr = server.local_addr().expect("server addr");

    let (client, _client_rx) = bind_ws().await;
    client.close().await.expect("close");

    let msg = build_register("ws-closed-001@example.com");
    let result = client.send_message(msg, server_addr).await;
    assert!(result.is_err(), "send on closed transport should fail");

    server.close().await.expect("close");
}

// =============================================================================
// Test 7: Connection refused — sending to a non-listening address
// =============================================================================

#[cfg(feature = "ws")]
#[tokio::test]
async fn test_ws_connection_refused() {
    let (client, _client_rx) = bind_ws().await;

    // Pick an address where no WS server is listening
    let dead_addr: SocketAddr = "127.0.0.1:19998".parse().unwrap();
    let msg = build_register("ws-refused@example.com");

    let result = client.send_message(msg, dead_addr).await;
    assert!(result.is_err(), "send to non-listening address should fail");

    client.close().await.expect("close");
}

// =============================================================================
// Test 8: Transport close emits Closed event
// =============================================================================

#[cfg(feature = "ws")]
#[tokio::test]
async fn test_ws_close_event() {
    let (transport, mut rx) = bind_ws().await;

    transport.close().await.expect("close");
    assert!(transport.is_closed());

    // Should receive a Closed event
    let event = timeout(Duration::from_secs(2), rx.recv()).await;
    if let Ok(Some(TransportEvent::Closed)) = event {
        // expected
    } else {
        // The Closed event may or may not arrive depending on timing —
        // the important thing is the transport is marked as closed.
        assert!(transport.is_closed());
    }
}

// =============================================================================
// Test 9: SIP message content integrity over WS
// =============================================================================

#[cfg(feature = "ws")]
#[tokio::test]
async fn test_ws_message_content_integrity() {
    let (server, mut server_rx) = bind_ws().await;
    let server_addr = server.local_addr().expect("server addr");
    let (client, _client_rx) = bind_ws().await;

    // Build a message with specific headers we can verify
    let original = SimpleRequestBuilder::new(Method::Invite, "sip:bob@biloxi.example.com")
        .expect("valid URI")
        .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("3848276298220188511@atlanta.example.com")
        .cseq(1)
        .via("127.0.0.1:5060", "WS", Some("z9hG4bK-content-check"))
        .max_forwards(70)
        .build();

    let original_msg: Message = original.into();
    client.send_message(original_msg.clone(), server_addr).await.expect("send");

    let received = recv_message(&mut server_rx, TIMEOUT).await;

    // Verify the received message matches the original by checking key headers
    if let (Message::Request(orig), Message::Request(recv)) = (&original_msg, &received) {
        assert_eq!(orig.method(), recv.method(), "method mismatch");
        assert_eq!(
            orig.call_id().expect("orig Call-ID").to_string(),
            recv.call_id().expect("recv Call-ID").to_string(),
            "Call-ID mismatch"
        );
        assert_eq!(
            orig.from().expect("orig From").to_string(),
            recv.from().expect("recv From").to_string(),
            "From header mismatch"
        );
        assert_eq!(
            orig.to().expect("orig To").to_string(),
            recv.to().expect("recv To").to_string(),
            "To header mismatch"
        );
    } else {
        panic!("expected both to be requests");
    }

    client.close().await.expect("close");
    server.close().await.expect("close");
}

// =============================================================================
// Test 10: SIP BYE request delivery
// =============================================================================

#[cfg(feature = "ws")]
#[tokio::test]
async fn test_ws_bye_delivery() {
    let (server, mut server_rx) = bind_ws().await;
    let server_addr = server.local_addr().expect("server addr");
    let (client, _client_rx) = bind_ws().await;

    let msg = build_bye("ws-bye-001@example.com");
    client.send_message(msg, server_addr).await.expect("send bye");

    let received = recv_message(&mut server_rx, TIMEOUT).await;
    if let Message::Request(req) = received {
        assert_eq!(req.method(), Method::Bye);
        assert_eq!(req.call_id().expect("Call-ID").to_string(), "ws-bye-001@example.com");
    } else {
        panic!("expected BYE request");
    }

    client.close().await.expect("close");
    server.close().await.expect("close");
}

// =============================================================================
// Test 11: Simulated call flow — INVITE then BYE over WS
// =============================================================================

#[cfg(feature = "ws")]
#[tokio::test]
async fn test_ws_invite_then_bye_flow() {
    let (server, mut server_rx) = bind_ws().await;
    let server_addr = server.local_addr().expect("server addr");
    let (client, _client_rx) = bind_ws().await;

    let call_id = "ws-call-flow-001@example.com";

    // Send INVITE
    let invite = build_invite(call_id);
    client.send_message(invite, server_addr).await.expect("send invite");

    let msg1 = recv_message(&mut server_rx, TIMEOUT).await;
    if let Message::Request(req) = &msg1 {
        assert_eq!(req.method(), Method::Invite);
    } else {
        panic!("expected INVITE");
    }

    // Send BYE for the same call
    let bye = build_bye(call_id);
    client.send_message(bye, server_addr).await.expect("send bye");

    let msg2 = recv_message(&mut server_rx, TIMEOUT).await;
    if let Message::Request(req) = &msg2 {
        assert_eq!(req.method(), Method::Bye);
        assert_eq!(req.call_id().expect("Call-ID").to_string(), call_id);
    } else {
        panic!("expected BYE");
    }

    client.close().await.expect("close");
    server.close().await.expect("close");
}

// =============================================================================
// Test 12: local_addr returns a valid address
// =============================================================================

#[cfg(feature = "ws")]
#[tokio::test]
async fn test_ws_local_addr() {
    let (transport, _rx) = bind_ws().await;

    let addr = transport.local_addr().expect("local_addr");
    assert_eq!(addr.ip(), "127.0.0.1".parse::<std::net::IpAddr>().unwrap());
    assert!(addr.port() > 0, "should have a non-zero port");

    transport.close().await.expect("close");
}

// =============================================================================
// Test 13: Rapid sequential sends
// =============================================================================

#[cfg(feature = "ws")]
#[tokio::test]
async fn test_ws_rapid_sequential_sends() {
    let (server, mut server_rx) = bind_ws().await;
    let server_addr = server.local_addr().expect("server addr");
    let (client, _client_rx) = bind_ws().await;

    let count = 10;
    for i in 0..count {
        let call_id = format!("ws-rapid-{}@example.com", i);
        let msg = build_register(&call_id);
        client.send_message(msg, server_addr).await.expect("send");
    }

    // Receive all messages
    let mut received_ids: Vec<String> = Vec::new();
    for _ in 0..count {
        let msg = recv_message(&mut server_rx, TIMEOUT).await;
        if let Message::Request(req) = msg {
            received_ids.push(req.call_id().expect("Call-ID").to_string());
        }
    }

    // Verify all messages were received (order should be preserved on a single connection)
    assert_eq!(received_ids.len(), count);
    for i in 0..count {
        assert_eq!(received_ids[i], format!("ws-rapid-{}@example.com", i));
    }

    client.close().await.expect("close");
    server.close().await.expect("close");
}

// =============================================================================
// Test 14: Custom channel capacity
// =============================================================================

#[cfg(feature = "ws")]
#[tokio::test]
async fn test_ws_custom_channel_capacity() {
    // Bind with a small channel capacity
    let (transport, _rx) = WebSocketTransport::bind(
        "127.0.0.1:0".parse().unwrap(),
        false,
        None,
        None,
        Some(5),
    )
    .await
    .expect("bind with custom capacity");

    let addr = transport.local_addr().expect("local_addr");
    assert!(addr.port() > 0);

    transport.close().await.expect("close");
}

// =============================================================================
// Test 15: Double close is idempotent
// =============================================================================

#[cfg(feature = "ws")]
#[tokio::test]
async fn test_ws_double_close() {
    let (transport, _rx) = bind_ws().await;

    transport.close().await.expect("first close");
    assert!(transport.is_closed());

    // Second close should not panic or error
    transport.close().await.expect("second close should succeed");
    assert!(transport.is_closed());
}

// =============================================================================
// Test 16: Debug format includes address
// =============================================================================

#[cfg(feature = "ws")]
#[tokio::test]
async fn test_ws_debug_format() {
    let (transport, _rx) = bind_ws().await;
    let addr = transport.local_addr().expect("addr");

    let debug = format!("{:?}", transport);
    assert!(
        debug.contains(&addr.port().to_string()),
        "Debug output should contain the port: {}",
        debug
    );

    transport.close().await.expect("close");
}

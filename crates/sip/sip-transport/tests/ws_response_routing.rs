//! Response-routing tests for `WebSocketTransport`.
//!
//! These tests verify that inbound SIP requests received over a WebSocket
//! connection have their responses sent back through the same WebSocket
//! connection, not via a UDP fallback.
//!
//! The root cause of the original bug was that `WebSocketTransport` did not
//! override `Transport::has_connection_to()`, causing
//! `MultiplexedTransport::pick_transport()` to fall through to UDP for every
//! WS/WSS response.
//!
//! # Tests
//!
//! | Test | What it proves |
//! |---|---|
//! | `has_connection_to_false_before_any_connection` | Default: no peers registered |
//! | `has_connection_to_true_after_client_connects` | Pool is populated on accept |
//! | `has_connection_to_false_after_connection_closed` | Pool is cleaned up on close |
//! | `server_sends_ringing_back_through_ws_connection` | 180 arrives on client WS bus |
//! | `server_sends_ok_back_through_ws_connection` | 200 OK arrives on client WS bus |
//! | `parallel_connections_each_receive_only_their_response` | Two clients, no cross-routing |
//! | `ws_transport_declares_correct_capabilities` | `supports_ws`, `default_transport_type` |

#![cfg(feature = "ws")]

use std::net::SocketAddr;
use std::time::Duration;

use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::types::Method;
use rvoip_sip_core::Message;
use rvoip_sip_transport::transport::ws::WebSocketTransport;
use rvoip_sip_transport::transport::TransportType;
use rvoip_sip_transport::{Transport, TransportEvent};

fn loopback(port: u16) -> SocketAddr {
    format!("127.0.0.1:{port}").parse().unwrap()
}

fn build_invite(call_id: &str) -> Message {
    Message::Request(
        SimpleRequestBuilder::new(Method::Invite, "sip:bob@server.example.com")
            .unwrap()
            .from("alice", "sip:alice@client.example.com", Some("tag-alice"))
            .to("bob", "sip:bob@server.example.com", None)
            .call_id(call_id)
            .cseq(1)
            .build(),
    )
}

fn build_ringing(call_id: &str) -> Message {
    Message::Response(
        SimpleResponseBuilder::ringing()
            .from("alice", "sip:alice@client.example.com", Some("tag-alice"))
            .to("bob", "sip:bob@server.example.com", Some("tag-bob"))
            .call_id(call_id)
            .cseq(1, Method::Invite)
            .via("server.example.com", "WS", Some("z9hG4bKtest"))
            .build(),
    )
}

fn build_ok(call_id: &str) -> Message {
    Message::Response(
        SimpleResponseBuilder::ok()
            .from("alice", "sip:alice@client.example.com", Some("tag-alice"))
            .to("bob", "sip:bob@server.example.com", Some("tag-bob"))
            .call_id(call_id)
            .cseq(1, Method::Invite)
            .via("server.example.com", "WS", Some("z9hG4bKtest"))
            .build(),
    )
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Bind a plain-WS server on an ephemeral port and give the accept loop a
/// moment to start.
async fn bind_ws_server() -> (
    WebSocketTransport,
    tokio::sync::mpsc::Receiver<TransportEvent>,
) {
    let (t, rx) = WebSocketTransport::bind(loopback(0), false, None, None, None)
        .await
        .expect("server bind");
    tokio::time::sleep(Duration::from_millis(50)).await;
    (t, rx)
}

/// Bind a plain-WS client on an ephemeral port.
async fn bind_ws_client() -> (
    WebSocketTransport,
    tokio::sync::mpsc::Receiver<TransportEvent>,
) {
    WebSocketTransport::bind(loopback(0), false, None, None, None)
        .await
        .expect("client bind")
}

/// Drain events from `rx` until `predicate` returns `Some(T)` or the timeout
/// fires. Skips `Closed` / `Error` events silently so the tests only see
/// the events they care about.
async fn next_matching<T, F>(
    rx: &mut tokio::sync::mpsc::Receiver<TransportEvent>,
    predicate: F,
    timeout: Duration,
) -> T
where
    F: Fn(TransportEvent) -> Option<T>,
{
    tokio::time::timeout(timeout, async {
        loop {
            let ev = rx.recv().await.expect("channel closed");
            if let Some(result) = predicate(ev) {
                return result;
            }
        }
    })
    .await
    .expect("timed out waiting for expected transport event")
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// A freshly-bound transport has no connections — `has_connection_to` must
/// return `false` for an arbitrary address.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn has_connection_to_false_before_any_connection() {
    let (server, _rx) = bind_ws_server().await;

    let random_addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
    assert!(
        !server.has_connection_to(random_addr),
        "has_connection_to must be false when no client has connected"
    );
}

/// After a client connects and sends a message, the server's connection pool
/// must contain the client's TCP source address (as seen by the server).
///
/// Note: the TCP source address used for the WS dial is the OS-assigned
/// ephemeral port, not the WS listener port reported by `client.local_addr()`.
/// The test therefore extracts the source from the server's `MessageReceived`
/// event rather than from `client.local_addr()`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn has_connection_to_true_after_client_connects() {
    let _ = tracing_subscriber::fmt::try_init();

    let (server, mut server_rx) = bind_ws_server().await;
    let server_addr = server.local_addr().unwrap();

    let (client, _client_rx) = bind_ws_client().await;

    // Sending establishes the WS connection from client → server.
    client
        .send_message(build_invite("has-conn-true"), server_addr)
        .await
        .expect("client send");

    // The server sees the client's TCP source address in the event.
    let client_tcp_src = next_matching(
        &mut server_rx,
        |ev| match ev {
            TransportEvent::MessageReceived { source, .. } => Some(source),
            _ => None,
        },
        Duration::from_secs(3),
    )
    .await;

    // `has_connection_to` must return true for the TCP source address.
    assert!(
        server.has_connection_to(client_tcp_src),
        "server must report has_connection_to == true for the connected client \
         (source: {client_tcp_src})"
    );

    // An unrelated address must still return false.
    let other: SocketAddr = "127.0.0.1:1".parse().unwrap();
    assert!(
        !server.has_connection_to(other),
        "unrelated address must not be reported as connected"
    );
}

/// After the client closes its transport, the server-side reader loop detects
/// the Close frame / EOF and removes the peer from the pool.
/// `has_connection_to` must then return `false`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn has_connection_to_false_after_connection_closed() {
    let _ = tracing_subscriber::fmt::try_init();

    let (server, mut server_rx) = bind_ws_server().await;
    let server_addr = server.local_addr().unwrap();

    let (client, _client_rx) = bind_ws_client().await;

    client
        .send_message(build_invite("has-conn-close"), server_addr)
        .await
        .expect("client send");

    // Wait for server to see the message so we have the connection's source.
    let client_tcp_src = next_matching(
        &mut server_rx,
        |ev| match ev {
            TransportEvent::MessageReceived { source, .. } => Some(source),
            _ => None,
        },
        Duration::from_secs(3),
    )
    .await;

    assert!(
        server.has_connection_to(client_tcp_src),
        "connected before close (source: {client_tcp_src})"
    );

    // Close the client: `close()` sends a WS Close frame then shuts the TCP
    // socket. The server's reader task sees the Close frame, sets
    // `conn.is_closed() = true`, breaks out of the read loop, and removes
    // the peer from `connections`.
    client.close().await.expect("client close");

    // Give the server's reader task time to process the Close frame and update
    // the connection pool.
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(
        !server.has_connection_to(client_tcp_src),
        "has_connection_to must be false after the client closes the connection \
         (source: {client_tcp_src})"
    );
}

/// The server receives a SIP INVITE over a WebSocket connection, then sends
/// a `180 Ringing` response back using `server.send_message(ringing, source)`.
///
/// The response must arrive on the *client's* event bus with
/// `transport_type == Ws` — proving it was routed through the existing
/// WebSocket connection and not via a new dial or a UDP fallback.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_sends_ringing_back_through_ws_connection() {
    let _ = tracing_subscriber::fmt::try_init();

    let (server, mut server_rx) = bind_ws_server().await;
    let server_addr = server.local_addr().unwrap();

    let (client, mut client_rx) = bind_ws_client().await;

    // Client sends INVITE → server.
    client
        .send_message(build_invite("ringing-ws"), server_addr)
        .await
        .expect("client send INVITE");

    // Wait for the server to receive the INVITE. The `source` is the TCP
    // address the server must reply to.
    let client_tcp_src = next_matching(
        &mut server_rx,
        |ev| match ev {
            TransportEvent::MessageReceived {
                message: Message::Request(ref req),
                source,
                ..
            } if req.method() == Method::Invite => Some(source),
            _ => None,
        },
        Duration::from_secs(3),
    )
    .await;

    // Server sends 180 Ringing back. `send_message` finds the existing
    // connection in the pool (via `connect_to`'s pool-hit branch) and
    // writes to it — no new TCP dial.
    server
        .send_message(build_ringing("ringing-ws"), client_tcp_src)
        .await
        .expect("server send 180");

    // The response must arrive on the client's event bus.
    let (msg, ttype) = next_matching(
        &mut client_rx,
        |ev| match ev {
            TransportEvent::MessageReceived {
                message,
                transport_type,
                ..
            } => Some((message, transport_type)),
            _ => None,
        },
        Duration::from_secs(3),
    )
    .await;

    assert_eq!(
        ttype,
        TransportType::Ws,
        "180 Ringing must arrive with TransportType::Ws, got {ttype:?}"
    );
    match msg {
        Message::Response(resp) => {
            assert_eq!(resp.status_code(), 180, "expected 180 Ringing");
        }
        _ => panic!("expected a SIP response, got a request"),
    }
}

/// Same as `server_sends_ringing_back_through_ws_connection` but for the
/// final `200 OK`. Ensures the fix covers both provisional and final responses.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_sends_ok_back_through_ws_connection() {
    let _ = tracing_subscriber::fmt::try_init();

    let (server, mut server_rx) = bind_ws_server().await;
    let server_addr = server.local_addr().unwrap();

    let (client, mut client_rx) = bind_ws_client().await;

    client
        .send_message(build_invite("ok-ws"), server_addr)
        .await
        .expect("client send INVITE");

    let client_tcp_src = next_matching(
        &mut server_rx,
        |ev| match ev {
            TransportEvent::MessageReceived {
                message: Message::Request(ref req),
                source,
                ..
            } if req.method() == Method::Invite => Some(source),
            _ => None,
        },
        Duration::from_secs(3),
    )
    .await;

    server
        .send_message(build_ok("ok-ws"), client_tcp_src)
        .await
        .expect("server send 200 OK");

    let (msg, ttype) = next_matching(
        &mut client_rx,
        |ev| match ev {
            TransportEvent::MessageReceived {
                message,
                transport_type,
                ..
            } => Some((message, transport_type)),
            _ => None,
        },
        Duration::from_secs(3),
    )
    .await;

    assert_eq!(
        ttype,
        TransportType::Ws,
        "200 OK must arrive with TransportType::Ws"
    );
    match msg {
        Message::Response(resp) => {
            assert_eq!(resp.status_code(), 200, "expected 200 OK");
        }
        _ => panic!("expected a SIP response"),
    }
}

/// Two clients connect to the same server. The server collects both source
/// addresses, sends a `180 Ringing` to one and a `200 OK` to the other, and
/// verifies that:
///
/// - each client receives exactly one response;
/// - the responses arrive with `transport_type == Ws`;
/// - the two status codes are 180 and 200 (one each, no duplicates).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_connections_each_receive_only_their_response() {
    let _ = tracing_subscriber::fmt::try_init();

    let (server, mut server_rx) = bind_ws_server().await;
    let server_addr = server.local_addr().unwrap();

    let (client_a, mut rx_a) = bind_ws_client().await;
    let (client_b, mut rx_b) = bind_ws_client().await;

    // Both clients send an INVITE.
    client_a
        .send_message(build_invite("parallel-a"), server_addr)
        .await
        .expect("client_a send");
    client_b
        .send_message(build_invite("parallel-b"), server_addr)
        .await
        .expect("client_b send");

    // Drain two INVITE events from the server and collect their TCP sources.
    let mut sources: Vec<SocketAddr> = Vec::new();
    for _ in 0..2 {
        let src = next_matching(
            &mut server_rx,
            |ev| match ev {
                TransportEvent::MessageReceived {
                    message: Message::Request(ref req),
                    source,
                    ..
                } if req.method() == Method::Invite => Some(source),
                _ => None,
            },
            Duration::from_secs(3),
        )
        .await;
        sources.push(src);
    }

    assert_eq!(sources.len(), 2, "server must receive from both clients");
    assert_ne!(
        sources[0], sources[1],
        "the two clients must have distinct source addresses"
    );

    let (src0, src1) = (sources[0], sources[1]);

    // Server sends 180 → src0, 200 → src1 (arbitrary split — we verify
    // no cross-routing by checking which client gets which code).
    server
        .send_message(build_ringing("parallel-a"), src0)
        .await
        .expect("server -> src0 180");
    server
        .send_message(build_ok("parallel-b"), src1)
        .await
        .expect("server -> src1 200");

    // Run both collects concurrently so neither blocks the other.
    let predicate = |ev: TransportEvent| match ev {
        TransportEvent::MessageReceived {
            message: Message::Response(resp),
            transport_type,
            ..
        } => Some((resp.status_code(), transport_type)),
        _ => None,
    };
    let (result_a, result_b) = tokio::join!(
        next_matching(&mut rx_a, predicate, Duration::from_secs(3)),
        next_matching(&mut rx_b, predicate, Duration::from_secs(3)),
    );

    let (code_a, ttype_a) = result_a;
    let (code_b, ttype_b) = result_b;

    assert_eq!(
        ttype_a,
        TransportType::Ws,
        "client_a: expected Ws transport"
    );
    assert_eq!(
        ttype_b,
        TransportType::Ws,
        "client_b: expected Ws transport"
    );

    // The two responses must be 180 and 200 — one each, no duplicates.
    let mut codes = [code_a, code_b];
    codes.sort_unstable();
    assert_eq!(
        codes,
        [180, 200],
        "expected one 180 and one 200 across the two clients, got {code_a} and {code_b}"
    );
}

/// Verify the capability-declaration methods added alongside `has_connection_to`.
/// A plain-WS transport must declare `supports_ws = true`, `supports_wss = false`,
/// and `default_transport_type = Ws`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ws_transport_declares_correct_capabilities() {
    let (ws, _rx) = bind_ws_server().await;

    assert!(
        ws.supports_ws(),
        "plain-WS transport: supports_ws must be true"
    );
    assert!(
        !ws.supports_wss(),
        "plain-WS transport: supports_wss must be false"
    );
    assert_eq!(
        ws.default_transport_type(),
        TransportType::Ws,
        "plain-WS transport: default_transport_type must be Ws"
    );
}

/// WSS transport must declare `supports_wss = true`, `supports_ws = false`,
/// and `default_transport_type = Wss`.
#[cfg(feature = "wss")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wss_transport_declares_correct_capabilities() {
    use std::io::Write;

    let dir = tempfile::tempdir().unwrap();
    let cert_path = dir.path().join("s.crt");
    let key_path = dir.path().join("s.key");
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    std::fs::File::create(&cert_path)
        .and_then(|mut f| f.write_all(cert.cert.pem().as_bytes()))
        .unwrap();
    std::fs::File::create(&key_path)
        .and_then(|mut f| f.write_all(cert.signing_key.serialize_pem().as_bytes()))
        .unwrap();

    let (wss, _rx) = WebSocketTransport::bind(
        loopback(0),
        true,
        Some(cert_path.to_str().unwrap()),
        Some(key_path.to_str().unwrap()),
        None,
    )
    .await
    .unwrap();

    assert!(
        !wss.supports_ws(),
        "WSS transport: supports_ws must be false"
    );
    assert!(
        wss.supports_wss(),
        "WSS transport: supports_wss must be true"
    );
    assert_eq!(
        wss.default_transport_type(),
        TransportType::Wss,
        "WSS transport: default_transport_type must be Wss"
    );
}

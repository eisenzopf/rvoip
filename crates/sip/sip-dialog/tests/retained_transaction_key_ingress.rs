//! An inbound request whose transaction key is still held by post-transaction
//! retention must never be dropped in silence.
//!
//! After a server INVITE transaction terminates, its key stays reserved for a
//! few seconds by the INVITE 2xx response cache and by the server-INVITE ACK
//! index. A request that arrives inside that window cannot allocate a new
//! transaction. Before the fix it produced nothing at all on the wire — the
//! peer waited out its own timeout — and the behaviour alternated between
//! working and hanging as attempts fell inside or outside the window.
//!
//! RFC 3261 §17.2.1 requires the retained final response to be replayed, and
//! §18.2.1 does not stop requiring it just because a connection-oriented peer
//! reconnected under a new flow.

use rvoip_sip_core::builder::SimpleResponseBuilder;
use rvoip_sip_core::StatusCode;
use rvoip_sip_dialog::transaction::{TransactionEvent, TransactionManager};
use rvoip_sip_transport::{TcpTransport, Transport, UdpTransport};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpSocket, UdpSocket};

/// Every attempt reuses this branch, so every attempt maps to the same server
/// transaction key. That is what puts a retained key on the ingress path.
const BRANCH: &str = "z9hG4bK.retained-key-regression";
const ATTEMPTS: usize = 4;

/// Long enough to outlive the retention a cached 2xx gets after its ACK, short
/// enough to stay inside the ACK index retention that keeps the key reserved.
/// In that gap the key is reserved with nothing left to replay — the case that
/// went silent on every transport.
const PAST_CACHED_2XX_RETENTION: Duration = Duration::from_millis(4200);

fn invite(server: SocketAddr, client: SocketAddr, proto: &str) -> Vec<u8> {
    format!(
        "INVITE sip:bob@{server} SIP/2.0\r\n\
         Via: SIP/2.0/{proto} {client};branch={BRANCH}\r\n\
         From: <sip:alice@example.test>;tag=alicetag\r\n\
         To: <sip:bob@example.test>\r\n\
         Call-ID: retained-key-regression\r\n\
         CSeq: 1 INVITE\r\n\
         Contact: <sip:alice@{client}>\r\n\
         Max-Forwards: 70\r\n\
         Content-Length: 0\r\n\
         \r\n"
    )
    .into_bytes()
}

fn ack(server: SocketAddr, client: SocketAddr, proto: &str) -> Vec<u8> {
    format!(
        "ACK sip:bob@{server} SIP/2.0\r\n\
         Via: SIP/2.0/{proto} {client};branch={BRANCH}.ack\r\n\
         From: <sip:alice@example.test>;tag=alicetag\r\n\
         To: <sip:bob@example.test>;tag=local-tag-value\r\n\
         Call-ID: retained-key-regression\r\n\
         CSeq: 1 ACK\r\n\
         Max-Forwards: 70\r\n\
         Content-Length: 0\r\n\
         \r\n"
    )
    .into_bytes()
}

/// Answers every new INVITE with 180 then 200, and counts how many distinct
/// INVITE transactions actually reached the transaction user.
fn spawn_uas(
    manager: Arc<TransactionManager>,
    mut events: tokio::sync::mpsc::Receiver<TransactionEvent>,
) -> Arc<AtomicUsize> {
    let invites = Arc::new(AtomicUsize::new(0));
    let counter = invites.clone();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if let TransactionEvent::InviteRequest {
                transaction_id,
                request,
                ..
            } = event
            {
                counter.fetch_add(1, Ordering::Relaxed);
                let ringing =
                    SimpleResponseBuilder::dialog_response(&request, StatusCode::Ringing, None)
                        .build();
                let _ = manager.send_response(&transaction_id, ringing).await;
                let ok = SimpleResponseBuilder::dialog_response(&request, StatusCode::Ok, None)
                    .contact("sip:bob@127.0.0.1:5060", None)
                    .build();
                let _ = manager.send_response(&transaction_id, ok).await;
            }
        }
    });
    invites
}

fn assert_sip_response(bytes: &[u8], label: &str) {
    assert!(
        String::from_utf8_lossy(bytes).starts_with("SIP/2.0 "),
        "{label} did not receive a SIP response"
    );
}

/// One INVITE over a brand new TCP connection, answered and ACKed.
async fn tcp_exchange(server: SocketAddr, label: &str) {
    let socket = TcpSocket::new_v4().expect("client socket");
    socket
        .bind("127.0.0.1:0".parse().unwrap())
        .expect("bind client");
    let client = socket.local_addr().expect("client addr");
    let mut stream = socket.connect(server).await.expect("connect");
    stream
        .write_all(&invite(server, client, "TCP"))
        .await
        .expect("write INVITE");

    let mut buf = vec![0u8; 8192];
    let read = tokio::time::timeout(Duration::from_secs(3), stream.read(&mut buf)).await;
    let bytes = match read {
        Ok(Ok(bytes)) if bytes > 0 => bytes,
        other => panic!(
            "{label} received no response over TCP ({other:?}); \
             a retained transaction key must never swallow an inbound request"
        ),
    };
    assert_sip_response(&buf[..bytes], label);

    let _ = stream.write_all(&ack(server, client, "TCP")).await;
}

async fn udp_exchange(socket: &UdpSocket, server: SocketAddr, client: SocketAddr, label: &str) {
    socket
        .send_to(&invite(server, client, "UDP"), server)
        .await
        .expect("send INVITE");

    let mut buf = vec![0u8; 8192];
    let read = tokio::time::timeout(Duration::from_secs(3), socket.recv_from(&mut buf)).await;
    let bytes = match read {
        Ok(Ok((bytes, _))) if bytes > 0 => bytes,
        other => panic!(
            "{label} received no response over UDP ({other:?}); \
             a retained transaction key must never swallow an inbound request"
        ),
    };
    assert_sip_response(&buf[..bytes], label);

    let _ = socket.send_to(&ack(server, client, "UDP"), server).await;
}

async fn tcp_uas() -> (SocketAddr, Arc<TransactionManager>, Arc<AtomicUsize>) {
    let (transport, transport_rx) =
        TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(64), None)
            .await
            .expect("bind TCP transport");
    let server = transport.local_addr().expect("local addr");
    let (manager, events) = TransactionManager::new(Arc::new(transport), transport_rx, Some(64))
        .await
        .expect("start transaction manager");
    let manager = Arc::new(manager);
    let invites = spawn_uas(manager.clone(), events);
    (server, manager, invites)
}

async fn udp_uas() -> (SocketAddr, Arc<TransactionManager>, Arc<AtomicUsize>) {
    let (transport, transport_rx) = UdpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(64))
        .await
        .expect("bind UDP transport");
    let server = transport.local_addr().expect("local addr");
    let (manager, events) = TransactionManager::new(Arc::new(transport), transport_rx, Some(64))
        .await
        .expect("start transaction manager");
    let manager = Arc::new(manager);
    let invites = spawn_uas(manager.clone(), events);
    (server, manager, invites)
}

/// Four sequential TCP connections, each carrying an INVITE with the same
/// branch. Every one of them has to be answered.
///
/// The connections deliberately come from a fresh source port each time. The
/// original report tied the failure to a repeated source address, but the
/// discriminator is the reused branch — keeping the ports distinct keeps this
/// honest about that and sidesteps TIME_WAIT entirely, so no abortive close is
/// needed to make it run. The same-peer reconnect case is covered directly by
/// `cached_invite_2xx_replays_onto_a_reconnected_tcp_flow` in the unit tests,
/// where flow identity can be controlled without socket games.
#[tokio::test(flavor = "multi_thread")]
async fn retained_key_answers_every_sequential_tcp_invite() {
    let (server, manager, _invites) = tcp_uas().await;

    for attempt in 0..ATTEMPTS {
        tcp_exchange(server, &format!("attempt {attempt}")).await;
        // Stay well inside the retention window so every attempt after the
        // first lands on the reserved key.
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    manager.shutdown().await;
}

/// The same reserved key, but with the cached 2xx already gone. Nothing is
/// left to replay, so this is where the ingress path went quiet.
#[tokio::test(flavor = "multi_thread")]
async fn retained_key_answers_tcp_invite_after_cached_2xx_expires() {
    let (server, manager, _invites) = tcp_uas().await;

    tcp_exchange(server, "first call").await;
    tokio::time::sleep(PAST_CACHED_2XX_RETENTION).await;
    tcp_exchange(server, "post-cache attempt").await;

    manager.shutdown().await;
}

/// UDP inside the 2xx cache window. The bug was masked here: the cached 200 OK
/// is replayed on the matching route, so the peer sees "a response" while the
/// transaction user never sees a second call. Assert both halves.
#[tokio::test(flavor = "multi_thread")]
async fn retained_key_replays_cached_2xx_to_udp_duplicates() {
    let (server, manager, invites) = udp_uas().await;
    let socket = UdpSocket::bind("127.0.0.1:0").await.expect("client socket");
    let client = socket.local_addr().expect("client addr");

    for attempt in 0..ATTEMPTS {
        udp_exchange(&socket, server, client, &format!("attempt {attempt}")).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // A reused branch is one transaction by RFC 3261 §17.2.3, so the replays
    // must not fabricate extra calls for the application either.
    assert_eq!(
        invites.load(Ordering::Relaxed),
        1,
        "retransmissions of one branch must reach the transaction user exactly once"
    );

    manager.shutdown().await;
}

/// UDP past the cached 2xx retention, still inside the reservation.
#[tokio::test(flavor = "multi_thread")]
async fn retained_key_answers_udp_invite_after_cached_2xx_expires() {
    let (server, manager, _invites) = udp_uas().await;
    let socket = UdpSocket::bind("127.0.0.1:0").await.expect("client socket");
    let client = socket.local_addr().expect("client addr");

    udp_exchange(&socket, server, client, "first call").await;
    tokio::time::sleep(PAST_CACHED_2XX_RETENTION).await;
    udp_exchange(&socket, server, client, "post-cache attempt").await;

    manager.shutdown().await;
}

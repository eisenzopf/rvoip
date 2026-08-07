//! Throughput of many concurrent senders sharing one TCP connection.
//!
//! This is the shape the per-connection write path is built for: a B2BUA
//! or proxy carrying many calls over a single TCP flow to one peer. The
//! peer drains continuously, so the socket buffer never becomes the
//! limit and what is being measured is the cost of getting N concurrent
//! senders through one write half.
//!
//! Two numbers are reported, because they answer different questions:
//!
//!   * end to end, wall time until the peer has received every byte.
//!     This is throughput, and it is fair to compare across designs.
//!   * caller visible, how long `send_message` itself takes. Under a
//!     write mutex this includes the socket work of everyone ahead of
//!     you; behind a writer task it is a queue push.
//!
//! Ignored by default: this is a measurement tool, not a correctness
//! check, and it only produces comparable numbers in a release build.
//!
//! Run with: cargo test -p rvoip-sip-transport --release
//!           --test tcp_write_contention -- --ignored --nocapture

use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::{Message, Method};
use rvoip_sip_transport::transport::tcp::TcpConnection;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};

const SENDERS: usize = 64;
const PER_SENDER: usize = 4_000;

/// The index is zero padded so every message serialises to the same
/// number of bytes. The peer counts bytes rather than parsing, so a
/// Call-ID that grew a digit partway through would make the expected
/// total wrong and stop the drain early.
fn sample_message(index: usize) -> Message {
    SimpleRequestBuilder::new(Method::Register, "sip:example.com")
        .unwrap()
        .from("alice", "sip:alice@example.com", Some("tag1"))
        .to("bob", "sip:bob@example.com", None)
        .call_id(&format!("contention-{index:08}@example.com"))
        .cseq(1)
        .build()
        .into()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "measurement tool; run explicitly with --release --ignored"]
async fn concurrent_senders_on_one_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_addr = listener.local_addr().unwrap();
    let accept = tokio::spawn(async move { listener.accept().await.unwrap().0 });
    let connection = Arc::new(TcpConnection::connect(server_addr).await.unwrap());
    let mut peer: TcpStream = accept.await.unwrap();

    // One message is enough to learn the wire size; every sender emits
    // the same shape, so the expected byte total is exact.
    let message_bytes = sample_message(0).to_bytes().len();
    let total_messages = SENDERS * PER_SENDER;
    let expected_bytes = message_bytes * total_messages;

    // The peer drains as fast as it can and reports when the last byte
    // lands. Nothing here parses, so the reader is never the limit.
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let drain = tokio::spawn(async move {
        let mut buffer = vec![0u8; 256 * 1024];
        let mut seen = 0usize;
        while seen < expected_bytes {
            match peer.read(&mut buffer).await {
                Ok(0) => break,
                Ok(n) => seen += n,
                Err(e) => panic!("peer read failed: {e}"),
            }
        }
        let _ = done_tx.send(Instant::now());
        seen
    });

    let send_nanos = Arc::new(AtomicU64::new(0));
    let slowest_send_nanos = Arc::new(AtomicU64::new(0));

    let started = Instant::now();
    let mut senders = Vec::with_capacity(SENDERS);
    for sender in 0..SENDERS {
        let connection = Arc::clone(&connection);
        let send_nanos = Arc::clone(&send_nanos);
        let slowest_send_nanos = Arc::clone(&slowest_send_nanos);
        senders.push(tokio::spawn(async move {
            for index in 0..PER_SENDER {
                let message = sample_message(sender * PER_SENDER + index);
                let call_started = Instant::now();
                connection.send_message(&message).await.unwrap();
                let elapsed = call_started.elapsed().as_nanos() as u64;
                send_nanos.fetch_add(elapsed, Ordering::Relaxed);
                slowest_send_nanos.fetch_max(elapsed, Ordering::Relaxed);
            }
        }));
    }
    for sender in senders {
        sender.await.unwrap();
    }
    let all_sends_returned = started.elapsed();

    let last_byte_at = done_rx.await.unwrap();
    let end_to_end = last_byte_at - started;
    let received = drain.await.unwrap();
    assert_eq!(received, expected_bytes, "peer must receive every byte");

    let mean_send_us = send_nanos.load(Ordering::Relaxed) as f64 / total_messages as f64 / 1_000.0;
    let slowest_send_us = slowest_send_nanos.load(Ordering::Relaxed) as f64 / 1_000.0;
    let throughput = total_messages as f64 / end_to_end.as_secs_f64();

    println!("--- {SENDERS} senders x {PER_SENDER} messages on one TCP connection ---");
    println!("message size          {message_bytes} B");
    println!("end to end            {:.3} s", end_to_end.as_secs_f64());
    println!("throughput            {throughput:.0} msg/s");
    println!(
        "all sends returned    {:.3} s",
        all_sends_returned.as_secs_f64()
    );
    println!("send() mean           {mean_send_us:.1} us");
    println!("send() slowest        {slowest_send_us:.1} us");

    connection.close().await.unwrap();
}

//! Throughput of many concurrent senders sharing one WebSocket connection.
//!
//! Third of the contention benchmarks, after `tcp_write_contention` and
//! `tls_write_contention`. The WebSocket writer task is fed through a
//! bounded queue like the others, but its callers then await a reply
//! oneshot, so a send still costs the socket round trip and concurrent
//! senders serialise behind it. This measures whether that costs
//! anything the whole transport can feel, before any code changes on
//! the strength of the design argument alone.
//!
//! Ignored by default: this is a measurement tool, not a correctness
//! check, and it only produces comparable numbers in a release build.
//!
//! Run with: cargo test -p rvoip-sip-transport --release
//!           --test ws_write_contention -- --ignored --nocapture

#![cfg(feature = "ws")]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::{Message, Method};
use rvoip_sip_transport::transport::ws::WebSocketTransport;
use rvoip_sip_transport::{Transport, TransportEvent};

const SENDERS: usize = 32;
const PER_SENDER: usize = 1_000;

fn loopback_addr(port: u16) -> SocketAddr {
    format!("127.0.0.1:{}", port).parse().unwrap()
}

fn sample_message(index: usize) -> Message {
    SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com")
        .unwrap()
        .from("alice", "sip:alice@example.com", Some("tag-ws"))
        .to("alice", "sip:alice@example.com", None)
        .call_id(&format!("ws-contention-{index:08}"))
        .cseq(1)
        .build()
        .into()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "measurement tool; run explicitly with --release --ignored"]
async fn concurrent_senders_on_one_ws_connection() {
    let _ = tracing_subscriber::fmt::try_init();

    let total_messages = SENDERS * PER_SENDER;

    let (server_transport, mut server_rx) =
        WebSocketTransport::bind(loopback_addr(0), false, None, None, None)
            .await
            .expect("server bind ws");
    let server_addr = server_transport.local_addr().expect("server local addr");
    tokio::time::sleep(Duration::from_millis(50)).await;

    let (client_transport, _client_rx) =
        WebSocketTransport::bind(loopback_addr(0), false, None, None, None)
            .await
            .expect("client bind ws");
    let client_transport = Arc::new(client_transport);

    // One message ahead of the measurement so the WebSocket upgrade and
    // the pool entry are not counted as send cost.
    client_transport
        .send_message(sample_message(0), server_addr)
        .await
        .expect("warmup send");
    let warmup = tokio::time::timeout(Duration::from_secs(10), server_rx.recv())
        .await
        .expect("warmup timed out");
    assert!(matches!(
        warmup,
        Some(TransportEvent::MessageReceived { .. })
    ));

    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let drain = tokio::spawn(async move {
        let mut seen = 0usize;
        while seen < total_messages {
            match server_rx.recv().await {
                Some(TransportEvent::MessageReceived { .. }) => seen += 1,
                Some(_) => continue,
                None => break,
            }
        }
        let _ = done_tx.send(Instant::now());
        seen
    });

    let started = Instant::now();
    let mut senders = Vec::with_capacity(SENDERS);
    for sender in 0..SENDERS {
        let transport = Arc::clone(&client_transport);
        senders.push(tokio::spawn(async move {
            for index in 0..PER_SENDER {
                let message = sample_message(1 + sender * PER_SENDER + index);
                transport.send_message(message, server_addr).await.unwrap();
            }
        }));
    }
    for sender in senders {
        sender.await.unwrap();
    }
    let all_sends_returned = started.elapsed();

    let last_at = tokio::time::timeout(Duration::from_secs(120), done_rx)
        .await
        .expect("receive side timed out")
        .unwrap();
    let end_to_end = last_at - started;
    assert_eq!(drain.await.unwrap(), total_messages);

    println!("--- {SENDERS} senders x {PER_SENDER} messages on one WS connection ---");
    println!("end to end            {:.3} s", end_to_end.as_secs_f64());
    println!(
        "throughput            {:.0} msg/s",
        total_messages as f64 / end_to_end.as_secs_f64()
    );
    println!(
        "all sends returned    {:.3} s",
        all_sends_returned.as_secs_f64()
    );
}

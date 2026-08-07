//! Throughput of many concurrent senders sharing one TLS connection.
//!
//! Companion to `tcp_write_contention`, and the measurement that decides
//! whether the TLS writer task is worth changing. The TLS path already
//! hands bytes to a writer task and returns, so the caller is not
//! waiting on the socket; what it does not do is coalesce queued
//! messages into one write. This measures the whole transport, receive
//! side included, because a gain that only exists in isolation is not a
//! gain.
//!
//! Ignored by default: this is a measurement tool, not a correctness
//! check, and it only produces comparable numbers in a release build.
//! In debug the writer task falls far enough behind that the send path
//! starts returning `BufferCapacityExceeded`, since TLS answers a full
//! writer queue with an error rather than waiting.
//!
//! Run with: cargo test -p rvoip-sip-transport --release
//!           --test tls_write_contention -- --ignored --nocapture

use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::Message;
use rvoip_sip_core::Method;
use rvoip_sip_transport::transport::tls::{TlsClientConfig, TlsTransport};
use rvoip_sip_transport::{Transport, TransportEvent};
use tempfile::tempdir;

const SENDERS: usize = 32;
const PER_SENDER: usize = 1_000;

fn write_self_signed_cert() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempdir().expect("tempdir");
    let cert_path = dir.path().join("server.crt");
    let key_path = dir.path().join("server.key");

    let cert = rcgen::generate_simple_self_signed(vec![
        "localhost".to_string(),
        "registrar.example.com".to_string(),
    ])
    .expect("rcgen self-signed");

    std::fs::File::create(&cert_path)
        .and_then(|mut f| f.write_all(cert.cert.pem().as_bytes()))
        .expect("write cert");
    std::fs::File::create(&key_path)
        .and_then(|mut f| f.write_all(cert.signing_key.serialize_pem().as_bytes()))
        .expect("write key");

    (dir, cert_path, key_path)
}

fn sample_message(index: usize) -> Message {
    SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com")
        .unwrap()
        .from("alice", "sip:alice@example.com", Some("tag-tls"))
        .to("alice", "sip:alice@example.com", None)
        .call_id(&format!("tls-contention-{index:08}"))
        .cseq(1)
        .build()
        .into()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "measurement tool; run explicitly with --release --ignored"]
async fn concurrent_senders_on_one_tls_connection() {
    let _ = tracing_subscriber::fmt::try_init();
    let (_dir, cert_path, key_path) = write_self_signed_cert();

    let total_messages = SENDERS * PER_SENDER;

    let (server_tx, mut server_events) = tokio::sync::mpsc::channel(4_096);
    let (server_transport, _server_rx) = TlsTransport::bind(
        "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        &cert_path,
        &key_path,
        Some(server_tx),
    )
    .await
    .expect("server bind");
    let server_addr = server_transport.local_addr().expect("server local addr");
    tokio::time::sleep(Duration::from_millis(100)).await;

    let (client_transport, _client_rx) = TlsTransport::client_only(
        "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        None,
        TlsClientConfig {
            extra_ca_path: Some(cert_path.clone()),
            insecure_skip_verify: false,
            ..Default::default()
        },
    )
    .await
    .expect("client bind");
    let client_transport = Arc::new(client_transport);

    // One message ahead of the measurement so the handshake and the
    // connection pool entry are not counted as send cost.
    client_transport
        .send_message(sample_message(0), server_addr)
        .await
        .expect("warmup send");
    let warmup = tokio::time::timeout(Duration::from_secs(10), server_events.recv())
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
            match server_events.recv().await {
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

    println!("--- {SENDERS} senders x {PER_SENDER} messages on one TLS connection ---");
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

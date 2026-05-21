//! UDP transport loopback benchmark.
//!
//! Measures the cost of one round-trip through `UdpListener::receive` —
//! the function that allocates a fresh 8 KiB `BytesMut` per packet
//! (see `transport/udp/listener.rs:48`). If you change the buffer
//! handling (e.g. add pooling) this bench is where the win shows up.
//!
//! Two harnesses:
//!
//! - `transport_udp_listener` — raw `UdpListener` only, packet sizes
//!   from 200 B to 8 KiB. Highlights the per-packet allocation cost
//!   without any parsing in the loop.
//! - `transport_udp_full_stack` — `UdpTransport` with its background
//!   receive task + parser + `TransportEvent` channel. Closer to what
//!   a SIP stack sees in production. Driven by a real INVITE message
//!   so the inbound parser succeeds.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rvoip_sip_transport::transport::udp::UdpListener;
use rvoip_sip_transport::{Transport, TransportEvent, UdpTransport};
use std::time::Instant;
use tokio::net::UdpSocket;
use tokio::runtime::Builder;

const PACKET_SIZES: [usize; 4] = [200, 1024, 4096, 8000];
const LOOPBACK: &str = "127.0.0.1:0";

/// Real INVITE used to drive the full-stack bench; the transport parses
/// inbound bytes before emitting `TransportEvent::MessageReceived`.
const SAMPLE_INVITE: &[u8] = b"INVITE sip:bob@biloxi.example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP pc33.atlanta.example.com:5060;branch=z9hG4bKudpbench\r\n\
Max-Forwards: 70\r\n\
To: Bob <sip:bob@biloxi.example.com>\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=udpbench\r\n\
Call-ID: udpbench@pc33.atlanta.example.com\r\n\
CSeq: 1 INVITE\r\n\
Contact: <sip:alice@pc33.atlanta.example.com>\r\n\
Content-Length: 0\r\n\r\n";

fn make_payload(size: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(size);
    for i in 0..size {
        v.push(b'A' + ((i % 26) as u8));
    }
    v
}

fn bench_listener_roundtrip(c: &mut Criterion) {
    let rt = Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime");

    let mut group = c.benchmark_group("transport_udp_listener");
    for &size in &PACKET_SIZES {
        let payload = make_payload(size);
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &payload, |b, payload| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let listener = UdpListener::bind(LOOPBACK.parse().unwrap())
                        .await
                        .expect("bind listener");
                    let listener_addr = listener.local_addr().expect("local addr");
                    let sender = UdpSocket::bind(LOOPBACK).await.expect("bind sender");

                    let start = Instant::now();
                    for _ in 0..iters {
                        sender
                            .send_to(payload, listener_addr)
                            .await
                            .expect("send");
                        let (bytes, _src, _local) =
                            listener.receive().await.expect("receive");
                        black_box(bytes);
                    }
                    start.elapsed()
                })
            });
        });
    }
    group.finish();
}

fn bench_full_stack_roundtrip(c: &mut Criterion) {
    let rt = Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("runtime");

    let mut group = c.benchmark_group("transport_udp_full_stack");
    group.throughput(Throughput::Elements(1));
    group.bench_function("invite_minimal", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let (transport, mut events) =
                    UdpTransport::bind(LOOPBACK.parse().unwrap(), Some(1024))
                        .await
                        .expect("bind transport");
                let transport_addr = transport.local_addr().expect("local addr");
                let sender = UdpSocket::bind(LOOPBACK).await.expect("bind sender");

                let start = Instant::now();
                for _ in 0..iters {
                    sender
                        .send_to(SAMPLE_INVITE, transport_addr)
                        .await
                        .expect("send");
                    loop {
                        match events.recv().await {
                            Some(TransportEvent::MessageReceived { message, .. }) => {
                                black_box(message);
                                break;
                            }
                            Some(_other) => continue,
                            None => panic!("transport channel closed"),
                        }
                    }
                }
                let elapsed = start.elapsed();
                transport.close().await.ok();
                elapsed
            })
        });
    });
    group.finish();
}

criterion_group!(benches, bench_listener_roundtrip, bench_full_stack_roundtrip);
criterion_main!(benches);

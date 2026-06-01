//! UDP RTP transport loopback benchmark.
//!
//! Measures the cost of one round-trip through `UdpRtpTransport` —
//! the full per-packet path including SRTP unprotect, header parse, and
//! the broadcast event dispatch (see `transport/udp.rs:269`). If you
//! tighten the locks in the receive loop or remove allocations, this
//! bench is where the win shows up.
//!
//! Two harnesses, mirroring `rvoip-sip-transport/benches/udp_loopback.rs`:
//!
//! - `transport_rtp_full_stack_plain` — `UdpRtpTransport` with no SRTP.
//!   Isolates the recv loop's tokio::Mutex `active` flag + broadcast
//!   dispatch from the crypto.
//! - `transport_rtp_full_stack_srtp` — same path with SRTP unprotect.
//!   Isolates the `srtp_recv` `tokio::Mutex` cost on top of the crypto.
//!
//! Driven from a sender `UdpSocket` that emits a pre-serialised RTP
//! packet; the transport's receive task parses inbound bytes and emits
//! `RtpEvent::MediaReceived`.

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rvoip_rtp_core::srtp::{SrtpContext, SrtpCryptoKey, SRTP_AES128_CM_SHA1_80};
use rvoip_rtp_core::traits::RtpEvent;
use rvoip_rtp_core::transport::{RtpTransport, RtpTransportConfig, UdpRtpTransport};
use rvoip_rtp_core::{RtpHeader, RtpPacket};
use std::time::Instant;
use tokio::net::UdpSocket;
use tokio::runtime::Builder;

const PAYLOAD_SIZES: [(&str, usize); 4] = [
    ("opus_80", 80),
    ("g711_160", 160),
    ("opus_200", 200),
    ("video_1200", 1200),
];

const LOOPBACK: &str = "127.0.0.1:0";

fn make_payload(size: usize) -> Bytes {
    let mut v = Vec::with_capacity(size);
    for i in 0..size {
        v.push((i & 0xff) as u8);
    }
    Bytes::from(v)
}

fn make_packet_wire(size: usize, seq: u16) -> Bytes {
    let header = RtpHeader::new(0, seq, 0x1234_5678, 0xdead_beef);
    let packet = RtpPacket::new(header, make_payload(size));
    packet.serialize().expect("serialize")
}

fn make_srtp() -> SrtpContext {
    SrtpContext::new(
        SRTP_AES128_CM_SHA1_80,
        SrtpCryptoKey::new(vec![0x42; 16], vec![0x37; 14]),
    )
    .expect("srtp context")
}

fn bench_full_stack_plain(c: &mut Criterion) {
    let rt = Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime");

    let mut group = c.benchmark_group("transport_rtp_full_stack_plain");
    for (name, size) in PAYLOAD_SIZES {
        let wire = make_packet_wire(size, 0);
        group.throughput(Throughput::Bytes(wire.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &wire, |b, wire| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let cfg = RtpTransportConfig {
                        local_rtp_addr: LOOPBACK.parse().unwrap(),
                        ..Default::default()
                    };
                    let transport = UdpRtpTransport::new(cfg).await.expect("transport");
                    let dest = transport.local_rtp_addr().expect("local_rtp_addr");
                    let mut rx = transport.subscribe();
                    let sender = UdpSocket::bind(LOOPBACK).await.expect("bind sender");

                    let start = Instant::now();
                    for _ in 0..iters {
                        sender.send_to(wire, dest).await.expect("send");
                        loop {
                            match rx.recv().await {
                                Ok(RtpEvent::MediaReceived { payload, .. }) => {
                                    black_box(payload);
                                    break;
                                }
                                Ok(_) => continue,
                                Err(e) => panic!("recv: {e}"),
                            }
                        }
                    }
                    let elapsed = start.elapsed();
                    transport.close().await.ok();
                    elapsed
                })
            });
        });
    }
    group.finish();
}

fn bench_full_stack_srtp(c: &mut Criterion) {
    let rt = Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime");

    let mut group = c.benchmark_group("transport_rtp_full_stack_srtp");
    for (name, size) in PAYLOAD_SIZES {
        // Pre-encrypt one packet using a sender-side SRTP context that
        // matches the receive side; the loopback re-uses the same wire
        // bytes for every iteration. We pre-build a window of N packets
        // with distinct sequence numbers so the receive-side replay
        // protection doesn't reject duplicates within a measurement
        // batch. iters is unknown ahead of time, so we cycle through a
        // 4096-packet window.
        const WINDOW: u16 = 4096;
        let mut tx_ctx = make_srtp();
        let plain = RtpPacket::new(
            RtpHeader::new(0, 0, 0x1234_5678, 0xdead_beef),
            make_payload(size),
        );
        let _ = tx_ctx.protect(&plain).expect("warm-up protect"); // burn seq 0
        let mut wires: Vec<Bytes> = Vec::with_capacity(WINDOW as usize);
        let mut tx_ctx = make_srtp(); // fresh ctx so the window starts at seq 0
        for seq in 0..WINDOW {
            let mut p = plain.clone();
            p.header.sequence_number = seq;
            let protected = tx_ctx.protect(&p).expect("protect");
            wires.push(protected.serialize().expect("serialize protected"));
        }

        let approx_wire_len = wires[0].len();
        group.throughput(Throughput::Bytes(approx_wire_len as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &wires, |b, wires| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let cfg = RtpTransportConfig {
                        local_rtp_addr: LOOPBACK.parse().unwrap(),
                        ..Default::default()
                    };
                    let transport = UdpRtpTransport::new(cfg).await.expect("transport");
                    let dest = transport.local_rtp_addr().expect("local_rtp_addr");
                    // Both directions need a context per the current API;
                    // the send side is unused (this bench drives raw UDP
                    // from a sender socket), so we install a no-op send.
                    transport.set_srtp_contexts(make_srtp(), make_srtp()).await;
                    let mut rx = transport.subscribe();
                    let sender = UdpSocket::bind(LOOPBACK).await.expect("bind sender");

                    let start = Instant::now();
                    for i in 0..iters {
                        let wire = &wires[(i as usize) % wires.len()];
                        sender.send_to(wire, dest).await.expect("send");
                        loop {
                            match rx.recv().await {
                                Ok(RtpEvent::MediaReceived { payload, .. }) => {
                                    black_box(payload);
                                    break;
                                }
                                Ok(_) => continue,
                                Err(e) => panic!("recv: {e}"),
                            }
                        }
                    }
                    let elapsed = start.elapsed();
                    transport.close().await.ok();
                    elapsed
                })
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_full_stack_plain, bench_full_stack_srtp);
criterion_main!(benches);

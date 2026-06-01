//! SRTP protect / unprotect micro-benchmark.
//!
//! `SrtpContext::protect` and `SrtpContext::unprotect` are the dominant
//! per-packet CPU cost on encrypted media. Today they live behind a
//! per-direction `tokio::sync::Mutex` in `transport/udp.rs`; the lock is
//! never held across `.await`, so the scheduler overhead is pure tax.
//! This bench measures the pure crypto work — no lock, no socket, no
//! parsing — at representative payload sizes so we have a numeric
//! ceiling for what the optimised transport hot path can reach.
//!
//! Suite under test: AES-CM-128 + HMAC-SHA1-80 (the default in this
//! stack and in WebRTC).

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rvoip_rtp_core::srtp::{SrtpContext, SrtpCryptoKey, SRTP_AES128_CM_SHA1_80};
use rvoip_rtp_core::{RtpHeader, RtpPacket};

const PAYLOAD_SIZES: [(&str, usize); 4] = [
    ("opus_80", 80),
    ("g711_160", 160),
    ("opus_200", 200),
    ("video_1200", 1200),
];

fn make_payload(size: usize) -> Bytes {
    let mut v = Vec::with_capacity(size);
    for i in 0..size {
        v.push((i & 0xff) as u8);
    }
    Bytes::from(v)
}

fn make_packet(payload_size: usize, seq: u16) -> RtpPacket {
    let header = RtpHeader::new(0, seq, 0x1234_5678, 0xdead_beef);
    RtpPacket::new(header, make_payload(payload_size))
}

fn make_context() -> SrtpContext {
    // 16-byte master key + 14-byte salt per AES-CM-128 (RFC 3711 §4.1.1).
    let key = vec![0x42; 16];
    let salt = vec![0x37; 14];
    SrtpContext::new(SRTP_AES128_CM_SHA1_80, SrtpCryptoKey::new(key, salt)).expect("srtp context")
}

fn bench_protect(c: &mut Criterion) {
    let mut group = c.benchmark_group("srtp_protect");
    for (name, size) in PAYLOAD_SIZES {
        let packet = make_packet(size, 0);
        group.throughput(Throughput::Bytes(packet.size() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &packet, |b, packet| {
            // Fresh context per benchmark to keep packet_index in a
            // realistic range and avoid measuring AES re-keying.
            let mut ctx = make_context();
            b.iter(|| {
                let protected = ctx.protect(black_box(packet)).expect("protect");
                black_box(protected);
            });
        });
    }
    group.finish();
}

fn bench_unprotect(c: &mut Criterion) {
    let mut group = c.benchmark_group("srtp_unprotect");
    for (name, size) in PAYLOAD_SIZES {
        // Pre-protect one packet so we have realistic ciphertext.
        let mut tx_ctx = make_context();
        let packet = make_packet(size, 0);
        let protected = tx_ctx.protect(&packet).expect("protect");
        let wire = protected.serialize().expect("serialize protected");

        group.throughput(Throughput::Bytes(wire.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &wire, |b, wire| {
            // Each iteration unprotects the same ciphertext. A fresh
            // context every iteration avoids the replay window
            // rejecting subsequent unprotects with the same seq.
            b.iter_batched(
                make_context,
                |mut ctx| {
                    let plain = ctx.unprotect(black_box(wire)).expect("unprotect");
                    black_box(plain);
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_protect_rtcp(c: &mut Criterion) {
    let mut group = c.benchmark_group("srtp_protect_rtcp");
    // Typical compound RTCP report ~60–100 bytes.
    let rtcp_data: Vec<u8> = (0..96).map(|i| (i & 0xff) as u8).collect();
    group.throughput(Throughput::Bytes(rtcp_data.len() as u64));
    group.bench_function("compound_96", |b| {
        let mut ctx = make_context();
        b.iter(|| {
            let out = ctx
                .protect_rtcp(black_box(&rtcp_data))
                .expect("protect_rtcp");
            black_box(out);
        });
    });
    group.finish();
}

criterion_group!(benches, bench_protect, bench_unprotect, bench_protect_rtcp);
criterion_main!(benches);

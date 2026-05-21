//! RTP packet parse / serialize micro-benchmarks.
//!
//! Measures `RtpPacket::parse` and `RtpPacket::serialize` at representative
//! payload sizes:
//!
//! - 160 B — G.711 µ-law/A-law, 20 ms ptime at 8 kHz (the dominant VoIP
//!   carrier in this stack)
//! - 80 B — Opus, 20 ms at 6 kbps (low end)
//! - 200 B — Opus, 20 ms at 48 kbps (typical)
//! - 1200 B — video / RTP-fragmented payload near the MTU floor
//!
//! Throughput is reported in bytes/sec so per-byte regressions (e.g. an
//! added copy in the parse path) show up directly. See
//! `crates/rvoip-sip/docs/PROFILING.md` for the broader workflow.

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
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

fn make_packet(payload_size: usize) -> RtpPacket {
    let header = RtpHeader::new(
        /* payload_type */ 0,
        /* sequence_number */ 1234,
        /* timestamp */ 0x1234_5678,
        /* ssrc */ 0xdead_beef,
    );
    RtpPacket::new(header, make_payload(payload_size))
}

fn bench_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("rtp_packet_serialize");
    for (name, size) in PAYLOAD_SIZES {
        let packet = make_packet(size);
        // Throughput == total packet size (header + payload), since that's
        // what the serializer actually writes.
        group.throughput(Throughput::Bytes(packet.size() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &packet, |b, packet| {
            b.iter(|| {
                let bytes = packet.serialize().expect("serialize");
                black_box(bytes);
            });
        });
    }
    group.finish();
}

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("rtp_packet_parse");
    for (name, size) in PAYLOAD_SIZES {
        let packet = make_packet(size);
        let wire = packet.serialize().expect("serialize");
        group.throughput(Throughput::Bytes(wire.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &wire, |b, wire| {
            b.iter(|| {
                let parsed = RtpPacket::parse(black_box(wire)).expect("parse");
                black_box(parsed);
            });
        });
    }
    group.finish();
}

/// Parse → re-serialize roundtrip. Real receivers do both per packet on
/// the bridge / mixer path, so the combined number is the load-bearing
/// one. Diverging the two single benches lets us see which side regressed.
fn bench_parse_serialize_roundtrip(c: &mut Criterion) {
    let mut group = c.benchmark_group("rtp_packet_roundtrip");
    for (name, size) in PAYLOAD_SIZES {
        let packet = make_packet(size);
        let wire = packet.serialize().expect("serialize");
        group.throughput(Throughput::Bytes(wire.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &wire, |b, wire| {
            b.iter(|| {
                let parsed = RtpPacket::parse(black_box(wire)).expect("parse");
                let out = parsed.serialize().expect("serialize");
                black_box(out);
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_serialize,
    bench_parse,
    bench_parse_serialize_roundtrip
);
criterion_main!(benches);

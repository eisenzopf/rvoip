//! Serializer micro-benchmarks for `rvoip-sip-core`.
//!
//! Measures `Message::to_bytes()` over the canonical corpus. The parser
//! is invoked once during setup so each iteration only times the
//! serialization path (Vec allocation + per-header `format!`).

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rvoip_sip_core::parse_message;

#[path = "common/fixtures.rs"]
mod fixtures;

fn bench_to_bytes(c: &mut Criterion) {
    let mut group = c.benchmark_group("core_serialize_message");
    for fx in fixtures::corpus() {
        let msg = parse_message(fx.bytes).expect("fixture parses");
        group.throughput(Throughput::Bytes(fx.bytes.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(fx.name), &msg, |b, msg| {
            b.iter(|| {
                let out = black_box(msg).to_bytes();
                black_box(out);
            });
        });
    }
    group.finish();
}

fn bench_roundtrip(c: &mut Criterion) {
    let mut group = c.benchmark_group("core_roundtrip_message");
    for fx in fixtures::corpus() {
        group.throughput(Throughput::Bytes(fx.bytes.len() as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(fx.name),
            fx.bytes,
            |b, bytes| {
                b.iter(|| {
                    let msg = parse_message(black_box(bytes)).expect("parse");
                    let out = msg.to_bytes();
                    black_box(out);
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_to_bytes, bench_roundtrip);
criterion_main!(benches);

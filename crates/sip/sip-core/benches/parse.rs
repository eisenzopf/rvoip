//! Parser micro-benchmarks for `rvoip-sip-core`.
//!
//! Measures `parse_message` (lenient + strict) across the canonical
//! corpus. Throughput is reported in bytes so degradation from added
//! per-byte work (e.g. UTF-8 validation) shows up cleanly.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rvoip_sip_core::parser::message::ParseMode;
use rvoip_sip_core::{parse_message, parse_message_with_mode};

#[path = "common/fixtures.rs"]
mod fixtures;

fn bench_parse_lenient(c: &mut Criterion) {
    let mut group = c.benchmark_group("core_parse_message");
    for fx in fixtures::corpus() {
        group.throughput(Throughput::Bytes(fx.bytes.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("lenient", fx.name),
            fx.bytes,
            |b, bytes| {
                b.iter(|| {
                    let msg = parse_message(black_box(bytes)).expect("parse");
                    black_box(msg);
                });
            },
        );
    }
    group.finish();
}

fn bench_parse_strict(c: &mut Criterion) {
    let mut group = c.benchmark_group("core_parse_message");
    for fx in fixtures::corpus() {
        group.throughput(Throughput::Bytes(fx.bytes.len() as u64));
        group.bench_with_input(BenchmarkId::new("strict", fx.name), fx.bytes, |b, bytes| {
            b.iter(|| {
                let msg =
                    parse_message_with_mode(black_box(bytes), ParseMode::Strict).expect("parse");
                black_box(msg);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_parse_lenient, bench_parse_strict);
criterion_main!(benches);

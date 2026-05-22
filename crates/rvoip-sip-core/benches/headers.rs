//! Per-header parser micro-benchmarks.
//!
//! Isolates the hot per-header parsers (Via, From, Contact, CSeq,
//! Route, Call-ID, Max-Forwards) so per-header regressions don't get
//! hidden by the full `parse_message` envelope.
//!
//! Inputs match each parser's expected scope: `parse_via` and
//! `parse_call_id` consume the header name + HCOLON; other parsers
//! receive just the header value. Neither group expects a trailing
//! CRLF — that's stripped by `header_value_better` upstream.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rvoip_sip_core::parser::headers::via::parse_via;
use rvoip_sip_core::parser::headers::{
    parse_call_id, parse_contact, parse_cseq, parse_from, parse_max_forwards, parse_route,
};

// Via includes the header name + HCOLON; no trailing CRLF.
const VIA_SIMPLE: &[u8] = b"Via: SIP/2.0/UDP pc33.atlanta.example.com:5060;branch=z9hG4bK776asdhds";
const VIA_PARAMS: &[u8] = b"Via: SIP/2.0/UDP pc33.atlanta.example.com:5060;branch=z9hG4bK776asdhds;received=192.0.2.101;rport=5060;ttl=64";
const VIA_STACK: &[u8] = b"Via: SIP/2.0/UDP proxy1.example.com:5060;branch=z9hG4bKa, SIP/2.0/UDP proxy2.example.com:5060;branch=z9hG4bKb, SIP/2.0/UDP pc33.atlanta.example.com:5060;branch=z9hG4bKc";

// From / To address values (no header name, no CRLF).
const FROM_SIMPLE: &[u8] = b"Alice <sip:alice@atlanta.example.com>;tag=1928301774";
const TO_SIMPLE: &[u8] = b"Bob <sip:bob@biloxi.example.com>;tag=8321234356";

// Contact values — `parse_contact` consumes the value only.
const CONTACT_SIMPLE: &[u8] = b"<sip:alice@pc33.atlanta.example.com>";
const CONTACT_PARAMS: &[u8] =
    b"\"Alice\" <sip:alice@pc33.atlanta.example.com;transport=tcp>;expires=3600;q=0.7";

// Call-ID includes the header name + HCOLON.
const CALL_ID_SIMPLE: &[u8] = b"Call-ID: a84b4c76e66710@pc33.atlanta.example.com";

const CSEQ_SIMPLE: &[u8] = b"314159 INVITE";
const MAX_FORWARDS_SIMPLE: &[u8] = b"70";
const ROUTE_SIMPLE: &[u8] = b"<sip:proxy.example.com;lr>";

fn bench_via(c: &mut Criterion) {
    let mut group = c.benchmark_group("core_parse_header_via");
    for (name, bytes) in [
        ("simple", VIA_SIMPLE),
        ("params", VIA_PARAMS),
        ("stack_3hop", VIA_STACK),
    ] {
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), bytes, |b, bytes| {
            b.iter(|| {
                let (_, v) = parse_via(black_box(bytes)).expect("via");
                black_box(v);
            });
        });
    }
    group.finish();
}

fn bench_address_headers(c: &mut Criterion) {
    let mut group = c.benchmark_group("core_parse_header_address");

    group.throughput(Throughput::Bytes(FROM_SIMPLE.len() as u64));
    group.bench_function(BenchmarkId::from_parameter("from"), |b| {
        b.iter(|| {
            let (_, v) = parse_from(black_box(FROM_SIMPLE)).expect("from");
            black_box(v);
        });
    });

    // To uses the same address parser as From; bench via parse_from since
    // there's no separately exported parse_to entry point.
    group.throughput(Throughput::Bytes(TO_SIMPLE.len() as u64));
    group.bench_function(BenchmarkId::from_parameter("to"), |b| {
        b.iter(|| {
            let (_, v) = parse_from(black_box(TO_SIMPLE)).expect("to");
            black_box(v);
        });
    });

    group.throughput(Throughput::Bytes(CONTACT_SIMPLE.len() as u64));
    group.bench_function(BenchmarkId::from_parameter("contact_simple"), |b| {
        b.iter(|| {
            let (_, v) = parse_contact(black_box(CONTACT_SIMPLE)).expect("contact");
            black_box(v);
        });
    });

    group.throughput(Throughput::Bytes(CONTACT_PARAMS.len() as u64));
    group.bench_function(BenchmarkId::from_parameter("contact_params"), |b| {
        b.iter(|| {
            let (_, v) = parse_contact(black_box(CONTACT_PARAMS)).expect("contact");
            black_box(v);
        });
    });

    group.throughput(Throughput::Bytes(ROUTE_SIMPLE.len() as u64));
    group.bench_function(BenchmarkId::from_parameter("route"), |b| {
        b.iter(|| {
            let (_, v) = parse_route(black_box(ROUTE_SIMPLE)).expect("route");
            black_box(v);
        });
    });

    group.finish();
}

fn bench_scalar_headers(c: &mut Criterion) {
    let mut group = c.benchmark_group("core_parse_header_scalar");

    group.throughput(Throughput::Bytes(CALL_ID_SIMPLE.len() as u64));
    group.bench_function(BenchmarkId::from_parameter("call_id"), |b| {
        b.iter(|| {
            let (_, v) = parse_call_id(black_box(CALL_ID_SIMPLE)).expect("call_id");
            black_box(v);
        });
    });

    group.throughput(Throughput::Bytes(CSEQ_SIMPLE.len() as u64));
    group.bench_function(BenchmarkId::from_parameter("cseq"), |b| {
        b.iter(|| {
            let (_, v) = parse_cseq(black_box(CSEQ_SIMPLE)).expect("cseq");
            black_box(v);
        });
    });

    group.throughput(Throughput::Bytes(MAX_FORWARDS_SIMPLE.len() as u64));
    group.bench_function(BenchmarkId::from_parameter("max_forwards"), |b| {
        b.iter(|| {
            let (_, v) = parse_max_forwards(black_box(MAX_FORWARDS_SIMPLE)).expect("max_fwd");
            black_box(v);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_via,
    bench_address_headers,
    bench_scalar_headers
);
criterion_main!(benches);

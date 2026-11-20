//! G.711 codec hot-path micro-benchmarks.
//!
//! These benches compare the public codec buffer APIs against scalar reference
//! helpers and the existing cross-platform SIMD dispatcher. On AArch64 today
//! the NEON dispatcher still falls back to scalar, so this bench is the guardrail
//! before promoting any future NEON or encode-table experiment.

use codec_core::codecs::g711::{alaw_expand, ulaw_expand, G711Codec};
use codec_core::types::{AudioCodecExt, CodecConfig, CodecType, SampleRate};
use codec_core::utils::simd::{encode_alaw_optimized, encode_mulaw_optimized};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

const FRAME_SIZES: [(&str, usize); 3] =
    [("ptime_10ms", 80), ("ptime_20ms", 160), ("batch_1s", 8000)];

fn pcm_samples(size: usize) -> Vec<i16> {
    (0..size)
        .map(|i| {
            let phase = (i as f32 * 440.0 * std::f32::consts::TAU / 8_000.0).sin();
            (phase * 12_000.0) as i16
        })
        .collect()
}

fn encoded_bytes(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i & 0xff) as u8).collect()
}

fn pcmu_codec() -> G711Codec {
    G711Codec::new_pcmu(
        CodecConfig::new(CodecType::G711Pcmu)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(1),
    )
    .expect("PCMU codec")
}

fn pcma_codec() -> G711Codec {
    G711Codec::new_pcma(
        CodecConfig::new(CodecType::G711Pcma)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(1),
    )
    .expect("PCMA codec")
}

fn bench_decode_to_buffer(c: &mut Criterion) {
    let mut group = c.benchmark_group("g711_decode_to_buffer");
    for (name, size) in FRAME_SIZES {
        let encoded = encoded_bytes(size);
        let mut output = vec![0i16; size];
        group.throughput(Throughput::Elements(size as u64));

        group.bench_with_input(
            BenchmarkId::new("pcmu_codec_scalar", name),
            &size,
            |b, _| {
                let mut codec = pcmu_codec();
                b.iter(|| {
                    let decoded = codec
                        .decode_to_buffer(black_box(&encoded), black_box(&mut output))
                        .expect("decode");
                    black_box(decoded);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("pcmu_reference_scalar", name),
            &size,
            |b, _| {
                b.iter(|| {
                    for (out, &byte) in output.iter_mut().zip(encoded.iter()) {
                        *out = ulaw_expand(byte);
                    }
                    black_box(&output);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("pcma_codec_scalar", name),
            &size,
            |b, _| {
                let mut codec = pcma_codec();
                b.iter(|| {
                    let decoded = codec
                        .decode_to_buffer(black_box(&encoded), black_box(&mut output))
                        .expect("decode");
                    black_box(decoded);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("pcma_reference_scalar", name),
            &size,
            |b, _| {
                b.iter(|| {
                    for (out, &byte) in output.iter_mut().zip(encoded.iter()) {
                        *out = alaw_expand(byte);
                    }
                    black_box(&output);
                });
            },
        );
    }
    group.finish();
}

fn bench_encode_to_buffer(c: &mut Criterion) {
    let mut group = c.benchmark_group("g711_encode_to_buffer");
    for (name, size) in FRAME_SIZES {
        let samples = pcm_samples(size);
        let mut output = vec![0u8; size];
        group.throughput(Throughput::Elements(size as u64));

        group.bench_with_input(
            BenchmarkId::new("pcmu_codec_scalar", name),
            &size,
            |b, _| {
                let mut codec = pcmu_codec();
                b.iter(|| {
                    let encoded = codec
                        .encode_to_buffer(black_box(&samples), black_box(&mut output))
                        .expect("encode");
                    black_box(encoded);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("pcmu_optimized_dispatch", name),
            &size,
            |b, _| {
                b.iter(|| {
                    encode_mulaw_optimized(black_box(&samples), black_box(&mut output));
                    black_box(&output);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("pcma_codec_scalar", name),
            &size,
            |b, _| {
                let mut codec = pcma_codec();
                b.iter(|| {
                    let encoded = codec
                        .encode_to_buffer(black_box(&samples), black_box(&mut output))
                        .expect("encode");
                    black_box(encoded);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("pcma_optimized_dispatch", name),
            &size,
            |b, _| {
                b.iter(|| {
                    encode_alaw_optimized(black_box(&samples), black_box(&mut output));
                    black_box(&output);
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_decode_to_buffer, bench_encode_to_buffer);
criterion_main!(benches);

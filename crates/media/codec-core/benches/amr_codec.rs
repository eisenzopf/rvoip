//! AMR encode, decode and concealment throughput, per variant per rate.
//!
//! # What may and may not be changed to make these numbers better
//!
//! Correctness here *is* exact agreement with the 3GPP reference. Every rate
//! of both variants is bit-exact against TS 26.073 and TS 26.173, and that is
//! not a quality target that can be traded against speed — a single differing
//! LSB is a conformance failure.
//!
//! **Safe:** allocation, memory layout, bounds-check elimination, inlining,
//! reusing scratch buffers, avoiding recomputation of a value that is already
//! in hand.
//!
//! **Forbidden:** reassociating a sum, widening an accumulator, skipping a
//! saturation, replacing a table lookup with a computed value, or eliding a
//! "redundant" saturating operation. The last is the subtle one: the ETSI
//! basic operators set an overflow flag, that flag is *read* in seven places
//! in this codec, and a computation whose result is discarded can still change
//! control flow through it.
//!
//! Every optimisation lands with the bit-exactness suite green or it does not
//! land. `cargo test -p rvoip-codec-core --all-features` is the gate.
//!
//! # Reading the results
//!
//! Each benchmark encodes or decodes one 20 ms frame. A frame's real-time
//! budget is therefore 20 ms, and the real-time factor is `time / 20 ms`. The
//! `real_time_factor` test in `src/codecs/amr/mod.rs` asserts the measured
//! figure rather than leaving it to a chart.

use codec_core::codecs::amr::mode::{AmrMode, AmrVariant};
use codec_core::types::{CodecConfig, VariableRateCodec};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;

/// A deterministic voiced-ish signal, one frame long.
///
/// Not silence and not white noise: silence makes the codebook searches
/// degenerate and noise makes them worst-case, and neither is what a call
/// looks like. Two harmonics plus a little noise keeps the pitch predictor
/// and the algebraic search both doing real work.
fn frame(variant: AmrVariant, index: usize) -> Vec<i16> {
    let rate = match variant {
        AmrVariant::NarrowBand => 8_000.0f64,
        AmrVariant::WideBand => 16_000.0,
    };
    let samples = variant.frame_samples();
    let mut seed = 0x1234_5678u32;
    (0..samples)
        .map(|i| {
            let t = ((index * samples + i) as f64) / rate;
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = f64::from((seed >> 20) as i16 % 256) * 2.0;
            let tone = (t * 220.0 * std::f64::consts::TAU).sin() * 6000.0
                + (t * 660.0 * std::f64::consts::TAU).sin() * 2500.0;
            (tone + noise).clamp(-32000.0, 32000.0) as i16
        })
        .collect()
}

fn config_for(variant: AmrVariant, mode: AmrMode) -> CodecConfig {
    let mut config = match variant {
        AmrVariant::NarrowBand => CodecConfig::amr_nb(),
        AmrVariant::WideBand => CodecConfig::amr_wb(),
    };
    config.parameters.amr.mode_set = 1u16 << mode.index();
    config
}

fn variants() -> Vec<(AmrVariant, &'static str)> {
    let mut out = Vec::new();
    #[cfg(feature = "amr-nb")]
    out.push((AmrVariant::NarrowBand, "amr-nb"));
    #[cfg(feature = "amr-wb")]
    out.push((AmrVariant::WideBand, "amr-wb"));
    out
}

fn bench_encode(c: &mut Criterion) {
    let mut group = c.benchmark_group("amr/encode");
    for (variant, label) in variants() {
        group.throughput(Throughput::Elements(1));
        for mode in AmrMode::all(variant) {
            let config = config_for(variant, mode);
            let mut codec = codec_core::codecs::amr::AmrCodec::new(&config).expect("constructs");
            let frames: Vec<Vec<i16>> = (0..8).map(|i| frame(variant, i)).collect();
            let mut at = 0usize;

            group.bench_with_input(
                BenchmarkId::new(label, mode.bitrate()),
                &mode,
                |b, _| {
                    b.iter(|| {
                        let pcm = &frames[at % frames.len()];
                        at += 1;
                        black_box(codec.encode_frame(black_box(pcm)).expect("encodes"))
                    });
                },
            );
        }
    }
    group.finish();
}

fn bench_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("amr/decode");
    for (variant, label) in variants() {
        group.throughput(Throughput::Elements(1));
        for mode in AmrMode::all(variant) {
            let config = config_for(variant, mode);
            let mut encoder = codec_core::codecs::amr::AmrCodec::new(&config).expect("constructs");
            let payloads: Vec<_> = (0..8)
                .map(|i| encoder.encode_frame(&frame(variant, i)).expect("encodes"))
                .collect();

            let mut codec = codec_core::codecs::amr::AmrCodec::new(&config).expect("constructs");
            let mut at = 0usize;
            group.bench_with_input(
                BenchmarkId::new(label, mode.bitrate()),
                &mode,
                |b, _| {
                    b.iter(|| {
                        let coded = &payloads[at % payloads.len()];
                        at += 1;
                        black_box(codec.decode_frame(black_box(coded)).expect("decodes"))
                    });
                },
            );
        }
    }
    group.finish();
}

/// Concealment, which runs on every lost frame and is therefore on the hot
/// path of a lossy call rather than an exceptional one.
fn bench_conceal(c: &mut Criterion) {
    let mut group = c.benchmark_group("amr/conceal");
    for (variant, label) in variants() {
        let mode = AmrMode::all(variant).into_iter().last().expect("a mode");
        let config = config_for(variant, mode);
        let mut encoder = codec_core::codecs::amr::AmrCodec::new(&config).expect("constructs");
        let payload = encoder.encode_frame(&frame(variant, 0)).expect("encodes");

        let mut codec = codec_core::codecs::amr::AmrCodec::new(&config).expect("constructs");
        // The decoder refuses to conceal before it has decoded anything, which
        // is the reference's own behaviour -- so prime it first.
        codec.decode_frame(&payload).expect("decodes");
        let lost = codec_core::types::CodedFrame {
            kind: codec_core::types::FrameKind::Lost,
            mode: mode.index(),
            data: Vec::new(),
            quality_ok: false,
        };

        group.throughput(Throughput::Elements(1));
        group.bench_function(BenchmarkId::new(label, "lost"), |b| {
            b.iter(|| black_box(codec.decode_frame(black_box(&lost)).expect("conceals")));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_encode, bench_decode, bench_conceal);
criterion_main!(benches);

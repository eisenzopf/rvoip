use std::env;
use std::time::Instant;

use criterion::{Criterion, black_box};
#[cfg(feature = "annex_b")]
use g729::FrameType;
use g729::{DecoderConfig, EncoderConfig, G729Decoder, G729Encoder};

const ITERATIONS_JSON: usize = 200_000;
const FRAME_SAMPLES: usize = 80;
const SPEECH_BYTES: usize = 10;

fn sample_pcm_frame() -> [i16; FRAME_SAMPLES] {
    let mut frame = [0i16; FRAME_SAMPLES];
    for (i, sample) in frame.iter_mut().enumerate() {
        *sample = ((i as i16) - 40) * 90;
    }
    frame
}

fn sample_speech_bits() -> [u8; SPEECH_BYTES] {
    [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
}

fn bench_encode_once(iterations: usize) -> u64 {
    let mut enc = G729Encoder::new(EncoderConfig::default());
    let frame = sample_pcm_frame();
    let mut out = [0u8; SPEECH_BYTES];

    let start = Instant::now();
    for _ in 0..iterations {
        let _ = enc.encode(black_box(&frame), black_box(&mut out));
    }
    let elapsed = start.elapsed();
    (elapsed.as_nanos() / iterations as u128) as u64
}

fn bench_decode_once(iterations: usize) -> u64 {
    let mut dec = G729Decoder::new(DecoderConfig::default());
    let bits = sample_speech_bits();
    let mut out = [0i16; FRAME_SAMPLES];

    let start = Instant::now();
    for _ in 0..iterations {
        dec.decode(black_box(&bits), black_box(&mut out));
    }
    let elapsed = start.elapsed();
    (elapsed.as_nanos() / iterations as u128) as u64
}

fn print_json_result(id: &str, ns: u64) {
    println!(
        "{{\"reason\":\"benchmark-complete\",\"id\":\"{}\",\"mean\":{{\"estimate\":{}}}}}",
        id, ns
    );
}

fn wants_json_output() -> bool {
    let mut args = env::args().skip(1).peekable();
    while let Some(arg) = args.next() {
        if arg == "--output-format=json" {
            return true;
        }
        if arg == "--output-format" {
            if let Some(next) = args.peek() {
                if next == "json" {
                    return true;
                }
            }
        }
    }
    false
}

fn run_json_mode() {
    let encode_ns = bench_encode_once(ITERATIONS_JSON);
    let decode_ns = bench_decode_once(ITERATIONS_JSON);
    print_json_result("encode/frame", encode_ns);
    print_json_result("decode/frame", decode_ns);
}

fn run_criterion_mode() {
    let mut criterion = Criterion::default();
    let mut group = criterion.benchmark_group("codec");

    group.bench_function("encode/frame", |b| {
        let mut enc = G729Encoder::new(EncoderConfig::default());
        let frame = sample_pcm_frame();
        let mut out = [0u8; SPEECH_BYTES];
        b.iter(|| {
            let _ = enc.encode(black_box(&frame), black_box(&mut out));
        });
    });

    group.bench_function("decode/frame", |b| {
        let mut dec = G729Decoder::new(DecoderConfig::default());
        let bits = sample_speech_bits();
        let mut out = [0i16; FRAME_SAMPLES];
        b.iter(|| {
            dec.decode(black_box(&bits), black_box(&mut out));
        });
    });

    #[cfg(feature = "annex_b")]
    group.bench_function("annexb_encode/frame", |b| {
        let mut enc = G729Encoder::new(EncoderConfig { annex_b: true });
        let frame = sample_pcm_frame();
        let mut out = [0u8; SPEECH_BYTES];
        b.iter(|| {
            let _ = enc.encode(black_box(&frame), black_box(&mut out));
        });
    });

    #[cfg(feature = "annex_b")]
    group.bench_function("annexb_decode/frame", |b| {
        let mut dec = G729Decoder::new(DecoderConfig {
            annex_b: true,
            ..DecoderConfig::default()
        });
        let bits = sample_speech_bits();
        let mut out = [0i16; FRAME_SAMPLES];
        b.iter(|| {
            dec.decode_with_type(
                black_box(&bits),
                black_box(FrameType::Speech),
                black_box(&mut out),
            );
        });
    });

    group.finish();
    criterion.final_summary();
}

fn main() {
    if wants_json_output() {
        run_json_mode();
        return;
    }
    run_criterion_mode();
}

//! Conference audio mixer micro-benchmark.
//!
//! `AudioMixer` (processing/audio/mixer.rs) holds five separate
//! `tokio::sync::Mutex` instances on its hot path: `stats`,
//! `output_cache`, `frame_pool`, `format_converter`, `vad`. Each one is
//! grabbed (with `.await`) on every `mix_participants` cycle, so the
//! per-cycle scheduler overhead grows with the number of participants.
//!
//! Two scenarios:
//!
//! - `mixer_add_stream` — `add_audio_stream` cost at N participants.
//!   Touches `stats` Mutex and the stream-manager DashMap.
//! - `mixer_mix_cycle` — full `process_audio_frame` × N inputs →
//!   `mix_participants` round at N = {2, 4, 8, 16} participants. The
//!   load-bearing number for conference scalability.

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use rvoip_media_core::processing::audio::AudioMixer;
use rvoip_media_core::types::conference::{
    AudioStream, ConferenceMixingConfig, ParticipantId,
};
use rvoip_media_core::types::AudioFrame;
use std::time::Instant;
use tokio::runtime::Builder;

const SAMPLES_PER_FRAME: usize = 160; // 20 ms @ 8 kHz
const PARTICIPANT_COUNTS: [usize; 4] = [2, 4, 8, 16];

fn make_frame(seed: i16) -> AudioFrame {
    let samples: Vec<i16> = (0..SAMPLES_PER_FRAME)
        .map(|i| (seed.wrapping_add(i as i16)) as i16 / 32)
        .collect();
    AudioFrame::new(samples, 8000, 1, 0)
}

fn participant(i: usize) -> ParticipantId {
    ParticipantId(format!("p-{i:04}"))
}

async fn build_mixer(n: usize) -> AudioMixer {
    let cfg = ConferenceMixingConfig {
        max_participants: n.max(16),
        output_sample_rate: 8_000,
        output_channels: 1,
        output_samples_per_frame: SAMPLES_PER_FRAME as u32,
        ..Default::default()
    };
    AudioMixer::new(cfg).await.expect("mixer")
}

fn bench_add_stream(c: &mut Criterion) {
    let rt = Builder::new_current_thread().enable_all().build().unwrap();
    let mut group = c.benchmark_group("mixer_add_stream");
    group.throughput(Throughput::Elements(1));
    for &n in &PARTICIPANT_COUNTS {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let mut total = std::time::Duration::ZERO;
                    for it in 0..iters {
                        // Fresh mixer per iteration so we measure
                        // first-add cost from a known starting depth (n).
                        let mixer = build_mixer(n * 2).await;
                        for i in 0..n {
                            let p = participant(i);
                            let _ = mixer
                                .add_audio_stream(p.clone(), AudioStream::new(p, 8_000, 1))
                                .await;
                        }
                        // Time the (n+1)-th add — the steady-state cost.
                        let p_new = participant(n + (it as usize));
                        let start = Instant::now();
                        let _ = mixer
                            .add_audio_stream(p_new.clone(), AudioStream::new(p_new, 8_000, 1))
                            .await;
                        total += start.elapsed();
                    }
                    total
                })
            });
        });
    }
    group.finish();
}

fn bench_mix_cycle(c: &mut Criterion) {
    let rt = Builder::new_current_thread().enable_all().build().unwrap();
    let mut group = c.benchmark_group("mixer_mix_cycle");
    for &n in &PARTICIPANT_COUNTS {
        // One full mix cycle ingests N frames and produces N mixed outputs.
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let mixer = build_mixer(n).await;
                    for i in 0..n {
                        let p = participant(i);
                        let _ = mixer
                            .add_audio_stream(p.clone(), AudioStream::new(p, 8_000, 1))
                            .await;
                    }
                    // Pre-build the frames so per-iteration overhead is
                    // dominated by the mix path, not Vec<i16> allocation.
                    let frames: Vec<(ParticipantId, AudioFrame)> = (0..n)
                        .map(|i| (participant(i), make_frame((i * 37) as i16)))
                        .collect();
                    let inputs: Vec<AudioFrame> =
                        frames.iter().map(|(_, f)| f.clone()).collect();

                    let start = Instant::now();
                    for _ in 0..iters {
                        // 1. Push N frames in.
                        for (pid, f) in &frames {
                            let _ = mixer.process_audio_frame(pid, f.clone()).await;
                        }
                        // 2. Run the N-way mix → N outputs.
                        let out = mixer.mix_participants(&inputs).await.expect("mix");
                        black_box(out);
                    }
                    start.elapsed()
                })
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_add_stream, bench_mix_cycle);
criterion_main!(benches);

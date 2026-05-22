//! media-core jitter-buffer micro-benchmarks.
//!
//! Today `buffer/jitter::JitterBuffer::add_frame` takes seven separate
//! `tokio::sync::RwLock.write().await` calls per inserted frame (one
//! each for buffer, stats counters, next_sequence, last_playout_time,
//! jitter_state, target_depth — see `buffer/jitter.rs:152–189`). That's
//! seven scheduler dips per frame on a hot path that fires every 20 ms
//! per active call.
//!
//! This bench gives us the pre-refactor baseline so Phase C6's
//! collapse-to-one-parking-lot-mutex-plus-atomics is measurable.
//!
//! Three scenarios mirroring `rtp-core/benches/jitter_buffer.rs`:
//!
//! - `add_in_order` — strictly monotonic sequence numbers, the
//!   dominant production path.
//! - `add_out_of_order` — small reorder window (±4).
//! - `add_get_steady` — alternating add + drain at depth N.
//!
//! Depth sweep: {0, 10, 100, 1000}.

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rvoip_media_core::buffer::{JitterBuffer, JitterBufferConfig};
use rvoip_media_core::types::{AudioFrame, MediaPacket};
use std::time::{Duration, Instant};
use tokio::runtime::Builder;

const DEPTHS: [usize; 4] = [0, 10, 100, 1000];
const SAMPLES_PER_FRAME: usize = 160; // 20 ms @ 8 kHz

fn make_packet_and_frame(seq: u16) -> (MediaPacket, AudioFrame) {
    let payload: Vec<u8> = (0..SAMPLES_PER_FRAME).map(|i| (i & 0xff) as u8).collect();
    let packet = MediaPacket {
        payload: Bytes::from(payload),
        payload_type: 0,
        timestamp: (seq as u32) * SAMPLES_PER_FRAME as u32,
        sequence_number: seq,
        ssrc: 0xdead_beef,
        received_at: Instant::now(),
    };
    let samples = vec![0i16; SAMPLES_PER_FRAME];
    let frame = AudioFrame::new(samples, 8000, 1, packet.timestamp);
    (packet, frame)
}

fn make_buffer() -> JitterBuffer {
    JitterBuffer::new(JitterBufferConfig {
        // Generous depths so pre-population doesn't trip overflow before
        // the timed loop starts.
        initial_depth: 4,
        min_depth: 2,
        max_depth: 4096,
        max_late_packet_age_ms: 60_000,
        ..Default::default()
    })
}

fn bench_add_in_order(c: &mut Criterion) {
    let rt = Builder::new_current_thread().enable_all().build().unwrap();
    let mut group = c.benchmark_group("media_jitter_add_in_order");
    group.throughput(Throughput::Elements(1));
    for depth in DEPTHS {
        group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, &depth| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let buf = make_buffer();
                    for i in 1..=depth as u16 {
                        let (p, f) = make_packet_and_frame(i);
                        let _ = buf.add_frame(p, f).await;
                    }
                    let mut seq = depth as u16 + 1;
                    let start = Instant::now();
                    for _ in 0..iters {
                        let (p, f) = make_packet_and_frame(seq);
                        let _ = buf.add_frame(p, f).await;
                        seq = seq.wrapping_add(1);
                    }
                    start.elapsed()
                })
            });
        });
    }
    group.finish();
}

fn bench_add_out_of_order(c: &mut Criterion) {
    let rt = Builder::new_current_thread().enable_all().build().unwrap();
    let mut group = c.benchmark_group("media_jitter_add_out_of_order");
    group.throughput(Throughput::Elements(1));
    for depth in DEPTHS {
        group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, &depth| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let buf = make_buffer();
                    // Pre-populate with small reorder swaps every 8 frames.
                    for chunk in (1..=depth as u16).collect::<Vec<_>>().chunks(8) {
                        let mut g = chunk.to_vec();
                        if g.len() >= 8 {
                            g.swap(0, 7);
                            g.swap(2, 5);
                        }
                        for s in g {
                            let (p, f) = make_packet_and_frame(s);
                            let _ = buf.add_frame(p, f).await;
                        }
                    }
                    let mut probe = depth as u16 + 1;
                    let start = Instant::now();
                    for i in 0..iters {
                        let seq = if i & 1 == 0 {
                            probe.wrapping_sub(2)
                        } else {
                            let s = probe;
                            probe = probe.wrapping_add(1);
                            s
                        };
                        let (p, f) = make_packet_and_frame(seq);
                        let _ = buf.add_frame(p, f).await;
                    }
                    start.elapsed()
                })
            });
        });
    }
    group.finish();
}

fn bench_add_get_steady(c: &mut Criterion) {
    let rt = Builder::new_current_thread().enable_all().build().unwrap();
    let mut group = c.benchmark_group("media_jitter_add_get_steady");
    group.throughput(Throughput::Elements(1));
    for depth in DEPTHS {
        group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, &depth| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let buf = make_buffer();
                    for i in 1..=depth as u16 {
                        let (p, f) = make_packet_and_frame(i);
                        let _ = buf.add_frame(p, f).await;
                    }
                    let mut seq = depth as u16 + 1;
                    let start = Instant::now();
                    for _ in 0..iters {
                        let (p, f) = make_packet_and_frame(seq);
                        let _ = buf.add_frame(p, f).await;
                        seq = seq.wrapping_add(1);
                        let popped = buf.get_next_frame().await.ok().flatten();
                        black_box(popped);
                    }
                    start.elapsed()
                })
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_add_in_order,
    bench_add_out_of_order,
    bench_add_get_steady
);
criterion_main!(benches);
// silence unused warning for `Duration` if optimization removes it
#[allow(dead_code)]
fn _keep_duration() -> Duration {
    Duration::from_millis(0)
}

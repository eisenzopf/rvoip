//! Adaptive jitter buffer micro-benchmarks.
//!
//! `AdaptiveJitterBuffer` uses a single-task `&mut self` API with
//! synchronous methods (Phase C23b stripped the now-unnecessary async
//! locking from `buffer/jitter.rs`). Each `add_packet` /
//! `get_next_packet` is on the receive hot path of every RTP session,
//! so the per-op cost matters more than throughput.
//!
//! Three scenarios:
//!
//! - `add_in_order` — sequence numbers strictly increasing, no
//!   reordering. The dominant production path.
//! - `add_out_of_order` — small reorder window (±4). Models a wifi /
//!   cellular receive path.
//! - `add_get_steady` — alternating add + drain at a target depth.
//!   Models steady-state operation under nominal load.
//!
//! Buffer depth is swept across {0, 10, 100, 1000}. Above 1000 packets
//! you're well outside reasonable jitter-buffer territory — but the
//! curve at 100 → 1000 still tells us how the BTreeMap insertion cost
//! grows with depth.

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rvoip_rtp_core::buffer::{AdaptiveJitterBuffer, JitterBufferConfig};
use rvoip_rtp_core::{RtpHeader, RtpPacket};
use tokio::runtime::Builder;

const DEPTHS: [usize; 4] = [0, 10, 100, 1000];

fn make_packet(seq: u16) -> RtpPacket {
    let header = RtpHeader::new(0, seq, (seq as u32) * 160, 0xdead_beef);
    // 160 B = G.711 20 ms; representative.
    let mut payload = vec![0u8; 160];
    payload[0] = (seq & 0xff) as u8;
    RtpPacket::new(header, Bytes::from(payload))
}

fn make_buffer() -> AdaptiveJitterBuffer {
    AdaptiveJitterBuffer::new(JitterBufferConfig {
        // Generous limits so the bench's pre-population doesn't trip
        // overflow / max_packet_age before we measure.
        initial_size_ms: 200,
        min_size_ms: 40,
        max_size_ms: 4000,
        max_out_of_order: 4096,
        max_packet_age_ms: 60_000,
        ..Default::default()
    })
}

fn bench_add_in_order(c: &mut Criterion) {
    let rt = Builder::new_current_thread().enable_all().build().unwrap();
    let mut group = c.benchmark_group("jitter_add_in_order");
    group.throughput(Throughput::Elements(1));
    for depth in DEPTHS {
        group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, &depth| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let mut buf = make_buffer();
                    // Pre-populate to target depth with strictly monotonic
                    // sequence numbers starting at 1 (next is `depth+1`).
                    for i in 1..=depth as u16 {
                        let _ = buf.add_packet(make_packet(i));
                    }
                    let start = std::time::Instant::now();
                    let mut seq = depth as u16 + 1;
                    for _ in 0..iters {
                        let _ = buf.add_packet(make_packet(seq));
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
    let mut group = c.benchmark_group("jitter_add_out_of_order");
    group.throughput(Throughput::Elements(1));
    for depth in DEPTHS {
        group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, &depth| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let mut buf = make_buffer();
                    // Same pre-population as the in-order variant but
                    // insertion order is shuffled in groups of 8 with a
                    // ±4 reorder distance — a realistic worst-case mild
                    // reordering pattern.
                    for chunk in (1..=depth as u16).collect::<Vec<_>>().chunks(8) {
                        let mut group = chunk.to_vec();
                        if group.len() >= 8 {
                            group.swap(0, 7);
                            group.swap(2, 5);
                        }
                        for s in group {
                            let _ = buf.add_packet(make_packet(s));
                        }
                    }
                    let mut probe = depth as u16 + 1;
                    let start = std::time::Instant::now();
                    for i in 0..iters {
                        // Alternate insertion of a "late" packet (probe-2)
                        // and a "new" packet (probe). The buffer's BTreeMap
                        // sees inserts on both sides of the window.
                        let seq = if i & 1 == 0 {
                            probe.wrapping_sub(2)
                        } else {
                            let s = probe;
                            probe = probe.wrapping_add(1);
                            s
                        };
                        let _ = buf.add_packet(make_packet(seq));
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
    let mut group = c.benchmark_group("jitter_add_get_steady");
    group.throughput(Throughput::Elements(1));
    for depth in DEPTHS {
        group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, &depth| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let mut buf = make_buffer();
                    for i in 1..=depth as u16 {
                        let _ = buf.add_packet(make_packet(i));
                    }
                    let mut seq = depth as u16 + 1;
                    // Drain enough to reach playout-ready state. Internal
                    // playout delay logic may return None initially; we
                    // tolerate that and only count timed cycles.
                    let start = std::time::Instant::now();
                    for _ in 0..iters {
                        let _ = buf.add_packet(make_packet(seq));
                        seq = seq.wrapping_add(1);
                        let popped = buf.get_next_packet();
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

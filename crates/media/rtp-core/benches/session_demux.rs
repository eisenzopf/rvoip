//! Session SSRC-demultiplex micro-benchmarks.
//!
//! The real `RtpSession::streams` is an `Arc<std::sync::Mutex<HashMap<RtpSsrc, RtpStream>>>`
//! (see `session/mod.rs:223`), looked up on every received packet to
//! find the owning stream. Constructing a real session in a tight bench
//! loop is noisy and apples-to-oranges across runs (it spins up a
//! transport, broadcast channels, timer state). Instead — mirroring
//! `rvoip-sip-dialog/benches/transaction_manager.rs` — this bench
//! isolates the lookup structure:
//!
//! 1. Uncontended `HashMap` lookup at realistic stream counts.
//! 2. Contended `Arc<Mutex<HashMap>>` vs `Arc<DashMap>` under N
//!    concurrent tasks — the structure used verbatim today vs the
//!    proposed Phase C2 replacement.
//!
//! Pair the deltas here with the end-to-end deltas in `udp_loopback`
//! to size up the SSRC-demux contribution.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use dashmap::DashMap;
use rvoip_rtp_core::{RtpSsrc, RtpStream};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::runtime::Builder;

const STREAM_COUNTS: [usize; 4] = [1, 10, 100, 1_000];
const CONTENDED_STREAM_COUNT: usize = 1_000;
const CONTENDED_OPS_PER_TASK: u64 = 200;
const THREAD_COUNTS: [usize; 4] = [1, 4, 8, 16];

fn ssrc(i: usize) -> RtpSsrc {
    // Spread SSRCs so the hash function isn't doing trivial work; the
    // production demultiplexer sees a sparse 32-bit keyspace.
    (i as u32).wrapping_mul(0x9E37_79B9)
}

fn populated_hashmap(n: usize) -> HashMap<RtpSsrc, RtpStream> {
    let mut map = HashMap::with_capacity(n);
    for i in 0..n {
        let s = ssrc(i);
        map.insert(s, RtpStream::new(s, 8_000));
    }
    map
}

fn bench_uncontended_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("rtp_session_lookup");
    for &n in &STREAM_COUNTS {
        let map = populated_hashmap(n);
        // 256 probes spread across the keyspace so we don't accidentally
        // benchmark a single cache line.
        let probe: Vec<RtpSsrc> = (0..256).map(|i| ssrc((i * 31) % n.max(1))).collect();
        group.throughput(Throughput::Elements(probe.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                for k in &probe {
                    black_box(map.get(black_box(k)));
                }
            });
        });
    }
    group.finish();
}

fn bench_contended_lookup(c: &mut Criterion) {
    let rt = Builder::new_multi_thread()
        .worker_threads(16)
        .enable_all()
        .build()
        .expect("runtime");

    let mut group = c.benchmark_group("rtp_session_contended");
    for &threads in &THREAD_COUNTS {
        let total_ops = (threads as u64) * CONTENDED_OPS_PER_TASK;
        group.throughput(Throughput::Elements(total_ops));

        // Variant 1: `Arc<std::sync::Mutex<HashMap>>` — exactly the shape
        // of `RtpSession::streams` today.
        group.bench_with_input(
            BenchmarkId::new("std_mutex", threads),
            &threads,
            |b, &threads| {
                b.iter_custom(|iters| {
                    rt.block_on(async move {
                        let map =
                            Arc::new(StdMutex::new(populated_hashmap(CONTENDED_STREAM_COUNT)));
                        let mut total = Duration::ZERO;
                        for _ in 0..iters {
                            let start = Instant::now();
                            let mut handles = Vec::with_capacity(threads);
                            for t in 0..threads {
                                let map = Arc::clone(&map);
                                handles.push(tokio::spawn(async move {
                                    for op in 0..CONTENDED_OPS_PER_TASK as usize {
                                        let idx = (t * 7919 + op * 17) % CONTENDED_STREAM_COUNT;
                                        let key = ssrc(idx);
                                        let guard = map.lock().expect("poisoned");
                                        let v = guard.get(&key).map(|s| s.ssrc);
                                        drop(guard);
                                        black_box(v);
                                    }
                                }));
                            }
                            for h in handles {
                                h.await.expect("task");
                            }
                            total += start.elapsed();
                        }
                        total
                    })
                });
            },
        );

        // Variant 2: `Arc<DashMap>` — sharded, lock-free reads. Models
        // the Phase C2 refactor.
        group.bench_with_input(
            BenchmarkId::new("dashmap", threads),
            &threads,
            |b, &threads| {
                b.iter_custom(|iters| {
                    rt.block_on(async move {
                        let map: Arc<DashMap<RtpSsrc, RtpStream>> = {
                            let m = DashMap::with_capacity(CONTENDED_STREAM_COUNT);
                            for i in 0..CONTENDED_STREAM_COUNT {
                                let s = ssrc(i);
                                m.insert(s, RtpStream::new(s, 8_000));
                            }
                            Arc::new(m)
                        };
                        let mut total = Duration::ZERO;
                        for _ in 0..iters {
                            let start = Instant::now();
                            let mut handles = Vec::with_capacity(threads);
                            for t in 0..threads {
                                let map = Arc::clone(&map);
                                handles.push(tokio::spawn(async move {
                                    for op in 0..CONTENDED_OPS_PER_TASK as usize {
                                        let idx = (t * 7919 + op * 17) % CONTENDED_STREAM_COUNT;
                                        let key = ssrc(idx);
                                        let v = map.get(&key).map(|r| r.value().ssrc);
                                        black_box(v);
                                    }
                                }));
                            }
                            for h in handles {
                                h.await.expect("task");
                            }
                            total += start.elapsed();
                        }
                        total
                    })
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_uncontended_lookup, bench_contended_lookup);
criterion_main!(benches);

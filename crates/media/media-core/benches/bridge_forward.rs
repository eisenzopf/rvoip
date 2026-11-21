//! Bridge-forward lookup contention bench.
//!
//! On the bridge forward path, every inbound RTP packet looks up the
//! destination session through
//! `MediaSessionController::rtp_sessions: RwLock<HashMap<DialogId, RtpSessionWrapper>>`
//! (relay/controller/mod.rs:114) before it can call `send_packet`.
//! Today that's a single async RwLock spanning all dialogs; under N
//! concurrent bridges every forward serialises through it.
//!
//! Constructing the full bridge (two real RtpSessions + transports +
//! event loop) per iteration is too noisy to be a stable
//! micro-benchmark. Following the pattern in
//! `rvoip-sip-dialog/benches/transaction_manager.rs`, this bench
//! isolates the data-structure shape:
//!
//! 1. Uncontended lookup at N dialog counts.
//! 2. Contended `Arc<tokio::RwLock<HashMap<DialogId, _>>>` vs the
//!    proposed `Arc<DashMap<DialogId, _>>` (Phase C7) at N concurrent
//!    forwarders.
//!
//! Use the deltas here to size up the per-bridge lookup contribution;
//! end-to-end forward throughput will land on top via
//! `audio_frame_pipeline` and any future full-bridge bench.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use dashmap::DashMap;
use rvoip_media_core::types::DialogId;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Builder;
use tokio::sync::RwLock as TokioRwLock;

const DIALOG_COUNTS: [usize; 4] = [1, 16, 64, 256];
const CONTENDED_DIALOG_COUNT: usize = 256;
const OPS_PER_TASK: u64 = 200;
const TASK_COUNTS: [usize; 4] = [1, 4, 16, 64];

/// Stand-in for `RtpSessionWrapper`. A real wrapper holds an Arc<Mutex<RtpSession>>;
/// here a `u64` is enough to make the cache footprint realistic without dragging
/// in a real session.
type SessionHandle = Arc<u64>;

fn dialog(i: usize) -> DialogId {
    DialogId::new(format!("dlg-{i:06}"))
}

fn populated_hashmap(n: usize) -> HashMap<DialogId, SessionHandle> {
    let mut m = HashMap::with_capacity(n);
    for i in 0..n {
        m.insert(dialog(i), Arc::new(i as u64));
    }
    m
}

fn bench_uncontended_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("bridge_lookup");
    for &n in &DIALOG_COUNTS {
        let map = populated_hashmap(n);
        let probe: Vec<DialogId> = (0..256).map(|i| dialog((i * 31) % n.max(1))).collect();
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

    let mut group = c.benchmark_group("bridge_lookup_contended");
    for &tasks in &TASK_COUNTS {
        let total_ops = (tasks as u64) * OPS_PER_TASK;
        group.throughput(Throughput::Elements(total_ops));

        // Variant 1: `Arc<tokio::RwLock<HashMap>>` — exactly the shape
        // of `rtp_sessions` today.
        group.bench_with_input(
            BenchmarkId::new("tokio_rwlock", tasks),
            &tasks,
            |b, &tasks| {
                b.iter_custom(|iters| {
                    rt.block_on(async move {
                        let map =
                            Arc::new(TokioRwLock::new(populated_hashmap(CONTENDED_DIALOG_COUNT)));
                        let mut total = Duration::ZERO;
                        for _ in 0..iters {
                            let start = Instant::now();
                            let mut handles = Vec::with_capacity(tasks);
                            for t in 0..tasks {
                                let map = Arc::clone(&map);
                                handles.push(tokio::spawn(async move {
                                    for op in 0..OPS_PER_TASK as usize {
                                        let idx = (t * 7919 + op * 17) % CONTENDED_DIALOG_COUNT;
                                        let key = dialog(idx);
                                        let guard = map.read().await;
                                        let v = guard.get(&key).cloned();
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
        // the Phase C7 refactor.
        group.bench_with_input(BenchmarkId::new("dashmap", tasks), &tasks, |b, &tasks| {
            b.iter_custom(|iters| {
                rt.block_on(async move {
                    let map: Arc<DashMap<DialogId, SessionHandle>> = {
                        let m = DashMap::with_capacity(CONTENDED_DIALOG_COUNT);
                        for i in 0..CONTENDED_DIALOG_COUNT {
                            m.insert(dialog(i), Arc::new(i as u64));
                        }
                        Arc::new(m)
                    };
                    let mut total = Duration::ZERO;
                    for _ in 0..iters {
                        let start = Instant::now();
                        let mut handles = Vec::with_capacity(tasks);
                        for t in 0..tasks {
                            let map = Arc::clone(&map);
                            handles.push(tokio::spawn(async move {
                                for op in 0..OPS_PER_TASK as usize {
                                    let idx = (t * 7919 + op * 17) % CONTENDED_DIALOG_COUNT;
                                    let key = dialog(idx);
                                    let v = map.get(&key).map(|r| r.value().clone());
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
        });
    }
    group.finish();
}

criterion_group!(benches, bench_uncontended_lookup, bench_contended_lookup);
criterion_main!(benches);

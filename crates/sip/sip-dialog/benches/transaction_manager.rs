//! Transaction-manager hot-path micro-benchmarks.
//!
//! The real `TransactionManager` couples its `Arc<Mutex<HashMap<TransactionKey, _>>>`
//! to a `Transport`, an mpsc event loop and per-transaction timer state, which makes
//! constructing one in a tight bench loop noisy and apples-to-oranges across runs.
//! Instead, this bench isolates the two operations that dominate the hot path:
//!
//! 1. `TransactionKey` hash + `HashMap` lookup at realistic table sizes
//!    (uncontended).
//! 2. `Arc<Mutex<HashMap<TransactionKey, _>>>` contention under N concurrent
//!    tokio tasks — the structure used verbatim by `TransactionManager`.
//!
//! Use these to ground end-to-end results from
//! `crates/sip/rvoip-sip/benches/call_setup.rs` and the `dialog_steady_state`
//! profiling example. See `crates/sip/rvoip-sip/docs/PROFILING.md`.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use dashmap::DashMap;
use rvoip_sip_core::types::Method;
use rvoip_sip_dialog::transaction::TransactionKey;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Builder;
use tokio::sync::Mutex;

const TABLE_SIZES: [usize; 4] = [10, 1_000, 10_000, 100_000];
const CONTENDED_TABLE_SIZE: usize = 10_000;
const CONTENDED_OPS_PER_TASK: u64 = 200;
const THREAD_COUNTS: [usize; 4] = [1, 4, 8, 16];

fn make_key(i: usize, is_server: bool) -> TransactionKey {
    TransactionKey::new(format!("z9hG4bK{:08x}", i), Method::Invite, is_server)
}

fn populated_table(n: usize) -> HashMap<TransactionKey, u64> {
    let mut map = HashMap::with_capacity(n);
    for i in 0..n {
        map.insert(make_key(i, false), i as u64);
    }
    map
}

fn bench_key_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("dialog_txn_key_lookup");
    for &n in &TABLE_SIZES {
        let map = populated_table(n);
        // Probe a deterministic spread of keys so we don't accidentally
        // benchmark a single hot cache line.
        let probe_keys: Vec<TransactionKey> =
            (0..256).map(|i| make_key((i * 31) % n, false)).collect();
        group.throughput(Throughput::Elements(probe_keys.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                for k in &probe_keys {
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

    let mut group = c.benchmark_group("dialog_txn_contended");
    for &threads in &THREAD_COUNTS {
        let total_ops = (threads as u64) * CONTENDED_OPS_PER_TASK;
        group.throughput(Throughput::Elements(total_ops));

        // Variant 1: Mutex<HashMap> — the structure used today by
        // `TransactionManager::{client,server}_transactions` etc.
        group.bench_with_input(
            BenchmarkId::new("mutex", threads),
            &threads,
            |b, &threads| {
                b.iter_custom(|iters| {
                    rt.block_on(async move {
                        let map = Arc::new(Mutex::new(populated_table(CONTENDED_TABLE_SIZE)));
                        let mut total = Duration::ZERO;
                        for _ in 0..iters {
                            let start = Instant::now();
                            let mut handles = Vec::with_capacity(threads);
                            for t in 0..threads {
                                let map = Arc::clone(&map);
                                handles.push(tokio::spawn(async move {
                                    for op in 0..CONTENDED_OPS_PER_TASK as usize {
                                        let idx = (t * 7919 + op * 17) % CONTENDED_TABLE_SIZE;
                                        let key = make_key(idx, false);
                                        let guard = map.lock().await;
                                        let v = guard.get(&key).copied();
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

        // Variant 2: DashMap — sharded, lock-free reads. Models the
        // proposed refactor of any TransactionManager map whose locks
        // aren't held across `.await`.
        group.bench_with_input(
            BenchmarkId::new("dashmap", threads),
            &threads,
            |b, &threads| {
                b.iter_custom(|iters| {
                    rt.block_on(async move {
                        let map: Arc<DashMap<TransactionKey, u64>> = {
                            let m = DashMap::with_capacity(CONTENDED_TABLE_SIZE);
                            for i in 0..CONTENDED_TABLE_SIZE {
                                m.insert(make_key(i, false), i as u64);
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
                                        let idx = (t * 7919 + op * 17) % CONTENDED_TABLE_SIZE;
                                        let key = make_key(idx, false);
                                        let v = map.get(&key).map(|r| *r);
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

fn bench_insert_remove(c: &mut Criterion) {
    let mut group = c.benchmark_group("dialog_txn_insert_remove");
    group.throughput(Throughput::Elements(1));
    for &n in &TABLE_SIZES {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            // Pre-populate to size N, then time a single insert+remove of
            // a fresh key. Models how the real manager churns keys per
            // transaction lifecycle.
            let mut map = populated_table(n);
            let mut counter = n;
            b.iter(|| {
                let key = make_key(counter, false);
                counter += 1;
                map.insert(black_box(key.clone()), 0);
                let _ = map.remove(black_box(&key));
            });
        });
    }
    group.finish();
}

/// Latency under cross-await contention.
///
/// Models the real `TransactionManager` pathology: one task holds the
/// outer lock while doing async work (e.g. `client_tx.initiate().await`),
/// blocking every other task that just wants a brief lookup. Measures
/// the median-and-tail completion time of the brief lookups while a
/// slow holder periodically grabs the lock for ~1 ms.
///
/// Two variants:
/// - `mutex_hold`  — `Arc<Mutex<HashMap>>`, slow holder grabs the
///   guard, awaits a sleep, drops the guard. Matches today's
///   `send_request` shape exactly.
/// - `dashmap_extract` — `Arc<DashMap<_, Arc<TxLike>>>`, slow holder
///   clones the per-key `Arc<TxLike>` out, *drops the shard guard*,
///   then awaits its own per-tx `Mutex`. Brief lookups never see the
///   slow op.
fn bench_cross_await_tail(c: &mut Criterion) {
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    const BACKLOG: usize = 10_000;
    const READER_TASKS: usize = 16;
    const READS_PER_TASK: usize = 1_000;
    const SLOW_HOLD_MICROS: u64 = 1_000;
    const SLOW_EVERY_N_READS: usize = 100;

    /// Fake per-transaction state. Production code awaits inside
    /// `client_tx.initiate()` while the map lock is held; we model the
    /// same shape with a tokio sleep gated behind the per-tx Mutex.
    struct TxLike {
        inner: tokio::sync::Mutex<u64>,
    }

    let rt = Builder::new_multi_thread()
        .worker_threads(16)
        .enable_all()
        .build()
        .expect("runtime");

    let mut group = c.benchmark_group("dialog_txn_p99");
    group.sample_size(20);
    group.throughput(Throughput::Elements((READER_TASKS * READS_PER_TASK) as u64));

    // ---- Variant: Mutex<HashMap>, slow holder awaits inside the guard.
    group.bench_function("mutex_hold", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let map: Arc<Mutex<HashMap<TransactionKey, Arc<TxLike>>>> = {
                    let mut m = HashMap::with_capacity(BACKLOG);
                    for i in 0..BACKLOG {
                        m.insert(
                            make_key(i, false),
                            Arc::new(TxLike {
                                inner: tokio::sync::Mutex::new(i as u64),
                            }),
                        );
                    }
                    Arc::new(Mutex::new(m))
                };
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let total_us = Arc::new(AtomicU64::new(0));

                    let start = Instant::now();
                    let mut handles = Vec::with_capacity(READER_TASKS + 1);

                    // Slow holder: holds the outer map lock across a sleep,
                    // exactly like `send_request` holds it across
                    // `initiate().await`.
                    {
                        let map = Arc::clone(&map);
                        handles.push(tokio::spawn(async move {
                            for _ in 0..(READS_PER_TASK / SLOW_EVERY_N_READS) {
                                let guard = map.lock().await;
                                let _entry = guard.get(&make_key(0, false));
                                tokio::time::sleep(Duration::from_micros(SLOW_HOLD_MICROS)).await;
                                drop(guard);
                            }
                        }));
                    }

                    for t in 0..READER_TASKS {
                        let map = Arc::clone(&map);
                        let total_us = Arc::clone(&total_us);
                        handles.push(tokio::spawn(async move {
                            for op in 0..READS_PER_TASK {
                                let idx = (t * 7919 + op * 17) % BACKLOG;
                                let key = make_key(idx, false);
                                let op_start = Instant::now();
                                let guard = map.lock().await;
                                let _ = guard.get(&key);
                                drop(guard);
                                total_us.fetch_add(
                                    op_start.elapsed().as_micros() as u64,
                                    AtomicOrdering::Relaxed,
                                );
                            }
                        }));
                    }
                    for h in handles {
                        h.await.expect("task");
                    }
                    total += start.elapsed();
                    // The mean per-op latency is total_us / (READER_TASKS * READS_PER_TASK).
                    // We report wall time so concurrent slowdown shows up as
                    // throughput drop. The atomic is kept so the optimiser
                    // can't elide the timing.
                    black_box(total_us.load(AtomicOrdering::Relaxed));
                }
                total
            })
        });
    });

    // ---- Variant: DashMap<_, Arc<TxLike>>, slow holder extracts and
    // drops the shard guard before awaiting.
    group.bench_function("dashmap_extract", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let map: Arc<DashMap<TransactionKey, Arc<TxLike>>> = {
                    let m = DashMap::with_capacity(BACKLOG);
                    for i in 0..BACKLOG {
                        m.insert(
                            make_key(i, false),
                            Arc::new(TxLike {
                                inner: tokio::sync::Mutex::new(i as u64),
                            }),
                        );
                    }
                    Arc::new(m)
                };
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let total_us = Arc::new(AtomicU64::new(0));

                    let start = Instant::now();
                    let mut handles = Vec::with_capacity(READER_TASKS + 1);

                    // Slow holder: clone Arc<TxLike> out of the shard,
                    // drop the shard guard, then await on the per-tx mutex.
                    // This is the pattern the real TransactionManager
                    // refactor will use at every cross-await site.
                    {
                        let map = Arc::clone(&map);
                        handles.push(tokio::spawn(async move {
                            for _ in 0..(READS_PER_TASK / SLOW_EVERY_N_READS) {
                                let tx_arc =
                                    map.get(&make_key(0, false)).map(|r| r.value().clone());
                                if let Some(tx) = tx_arc {
                                    let _g = tx.inner.lock().await;
                                    tokio::time::sleep(Duration::from_micros(SLOW_HOLD_MICROS))
                                        .await;
                                }
                            }
                        }));
                    }

                    for t in 0..READER_TASKS {
                        let map = Arc::clone(&map);
                        let total_us = Arc::clone(&total_us);
                        handles.push(tokio::spawn(async move {
                            for op in 0..READS_PER_TASK {
                                let idx = (t * 7919 + op * 17) % BACKLOG;
                                let key = make_key(idx, false);
                                let op_start = Instant::now();
                                let _ = map.get(&key).map(|r| r.value().clone());
                                total_us.fetch_add(
                                    op_start.elapsed().as_micros() as u64,
                                    AtomicOrdering::Relaxed,
                                );
                            }
                        }));
                    }
                    for h in handles {
                        h.await.expect("task");
                    }
                    total += start.elapsed();
                    black_box(total_us.load(AtomicOrdering::Relaxed));
                }
                total
            })
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_key_lookup,
    bench_contended_lookup,
    bench_insert_remove,
    bench_cross_await_tail
);
criterion_main!(benches);

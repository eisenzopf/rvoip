//! Create / find hot-path micro-benchmarks for `rvoip-sip`.
//!
//! Modeled on `crates/media/rtp-core/benches/session_demux.rs`: each hot path
//! is benchmarked in isolation rather than end-to-end. Where there is a
//! concrete candidate optimization, the bench measures today's structure
//! side-by-side with the replacement at thread counts {1, 4, 8, 16} so
//! the swap-or-skip decision is data-driven.
//!
//! Five groups:
//!
//! 1. `session_store_uncontended` — single-task `get_session` vs the
//!    two-hop `find_by_call_id` at varied population sizes.
//! 2. `session_store_contended` — N concurrent tasks looking up sessions
//!    by id and by Call-ID against a 1 000-session store.
//! 3. `session_store_create` — N concurrent tasks creating then removing
//!    sessions, to catch DashMap shard contention on the insert path.
//! 4. `session_registry_lookup` — `tokio::sync::RwLock<Option<_>>` vs
//!    `arc_swap::ArcSwapOption<_>` for the single-session registry. This
//!    is the candidate the bench was built to validate.
//! 5. `dialog_callid_key_type` — `DashMap<String, _>` vs
//!    `DashMap<Arc<str>, _>` for the per-message Call-ID lookup the
//!    DialogAdapter performs. Measures the String hashing cost flagged
//!    in `docs/PROFILING.md` so a follow-up interning effort can be
//!    sized against the win.
//!
//! Pair the deltas here with the end-to-end deltas in `call_setup` and
//! `dialog_steady_state` to size the contribution to whole-call cost.

use arc_swap::ArcSwapOption;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use dashmap::DashMap;
use rvoip_sip::session_store::{SessionState, SessionStore};
use rvoip_sip::state_table::{DialogId, Role, SessionId};
use rvoip_sip::types::CallState;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::RwLock;

const POPULATION_SIZES: [usize; 4] = [1, 10, 100, 1_000];
const CONTENDED_POPULATION: usize = 1_000;
const CONTENDED_OPS_PER_TASK: u64 = 200;
const THREAD_COUNTS: [usize; 4] = [1, 4, 8, 16];
const PROBE_COUNT: usize = 256;

fn make_runtime() -> Runtime {
    Builder::new_multi_thread()
        .worker_threads(16)
        .enable_all()
        .build()
        .expect("bench runtime")
}

fn session_key(i: usize) -> SessionId {
    SessionId(format!("session-{i:010}"))
}

fn call_id_key(i: usize) -> String {
    // Realistic RFC 3261 Call-ID length: ~30–50 ASCII chars including @host.
    format!("{i:016x}-bench@host.example.com")
}

/// Populate the store with `n` sessions, each carrying a unique Call-ID
/// so `find_by_call_id` actually exercises the two-hop index lookup.
async fn populated_store(n: usize) -> Arc<SessionStore> {
    let store = Arc::new(SessionStore::new());
    for i in 0..n {
        let id = session_key(i);
        let mut session = store
            .create_session(id.clone(), Role::UAC, false)
            .await
            .expect("create");
        session.call_id = Some(call_id_key(i));
        store.update_session(session).await.expect("update");
    }
    store
}

fn bench_uncontended(c: &mut Criterion) {
    let rt = make_runtime();
    let mut group = c.benchmark_group("session_store_uncontended");
    group.measurement_time(Duration::from_secs(3));

    for &n in &POPULATION_SIZES {
        let store = rt.block_on(populated_store(n));
        // 256 probes spread across the keyspace so we don't accidentally
        // benchmark a single cache line / shard.
        let id_probes: Vec<SessionId> = (0..PROBE_COUNT)
            .map(|i| session_key((i * 31) % n.max(1)))
            .collect();
        let call_probes: Vec<String> = (0..PROBE_COUNT)
            .map(|i| call_id_key((i * 31) % n.max(1)))
            .collect();

        group.throughput(Throughput::Elements(PROBE_COUNT as u64));

        group.bench_with_input(BenchmarkId::new("get_session", n), &n, |b, _| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let start = Instant::now();
                    for _ in 0..iters {
                        for k in &id_probes {
                            black_box(store.get_session(black_box(k)).await.ok());
                        }
                    }
                    start.elapsed()
                })
            });
        });

        group.bench_with_input(BenchmarkId::new("find_by_call_id", n), &n, |b, _| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let start = Instant::now();
                    for _ in 0..iters {
                        for k in &call_probes {
                            black_box(store.find_by_call_id(black_box(k)).await);
                        }
                    }
                    start.elapsed()
                })
            });
        });
    }
    group.finish();
}

fn bench_contended_lookup(c: &mut Criterion) {
    let rt = make_runtime();
    let mut group = c.benchmark_group("session_store_contended");
    group.measurement_time(Duration::from_secs(4));

    let store = rt.block_on(populated_store(CONTENDED_POPULATION));

    for &threads in &THREAD_COUNTS {
        let total_ops = (threads as u64) * CONTENDED_OPS_PER_TASK;
        group.throughput(Throughput::Elements(total_ops));

        group.bench_with_input(
            BenchmarkId::new("get_session", threads),
            &threads,
            |b, &threads| {
                b.iter_custom(|iters| {
                    rt.block_on(async {
                        let mut total = Duration::ZERO;
                        for _ in 0..iters {
                            let start = Instant::now();
                            let mut handles = Vec::with_capacity(threads);
                            for t in 0..threads {
                                let store = Arc::clone(&store);
                                handles.push(tokio::spawn(async move {
                                    for op in 0..CONTENDED_OPS_PER_TASK as usize {
                                        let idx = (t * 7919 + op * 17) % CONTENDED_POPULATION;
                                        let key = session_key(idx);
                                        black_box(store.get_session(&key).await.ok());
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

        group.bench_with_input(
            BenchmarkId::new("find_by_call_id", threads),
            &threads,
            |b, &threads| {
                b.iter_custom(|iters| {
                    rt.block_on(async {
                        let mut total = Duration::ZERO;
                        for _ in 0..iters {
                            let start = Instant::now();
                            let mut handles = Vec::with_capacity(threads);
                            for t in 0..threads {
                                let store = Arc::clone(&store);
                                handles.push(tokio::spawn(async move {
                                    for op in 0..CONTENDED_OPS_PER_TASK as usize {
                                        let idx = (t * 7919 + op * 17) % CONTENDED_POPULATION;
                                        let key = call_id_key(idx);
                                        black_box(store.find_by_call_id(&key).await);
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

fn bench_create(c: &mut Criterion) {
    let rt = make_runtime();
    let mut group = c.benchmark_group("session_store_create");
    group.measurement_time(Duration::from_secs(4));
    // Session creation allocates `SessionState`, which is heavy. Keep the
    // per-task count modest so the bench finishes in reasonable time.
    let ops_per_task: u64 = 100;

    for &threads in &THREAD_COUNTS {
        let total_ops = (threads as u64) * ops_per_task;
        group.throughput(Throughput::Elements(total_ops));

        group.bench_with_input(
            BenchmarkId::new("create_remove", threads),
            &threads,
            |b, &threads| {
                b.iter_custom(|iters| {
                    rt.block_on(async {
                        let mut total = Duration::ZERO;
                        for iter in 0..iters {
                            let store = Arc::new(SessionStore::new());
                            let start = Instant::now();
                            let mut handles = Vec::with_capacity(threads);
                            for t in 0..threads {
                                let store = Arc::clone(&store);
                                let base = (iter as usize)
                                    .wrapping_mul(1024)
                                    .wrapping_add(t * (ops_per_task as usize));
                                handles.push(tokio::spawn(async move {
                                    for op in 0..ops_per_task as usize {
                                        let id = session_key(base + op);
                                        store
                                            .create_session(id.clone(), Role::UAC, false)
                                            .await
                                            .expect("create");
                                        store.remove_session(&id).await.expect("remove");
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

/// Track the allocation boundary introduced by the COW cold-state split.
/// The normal call path changes hot signaling/media fields and should retain
/// the cold block; authentication/registration mutations deliberately pay one
/// copy so previously published revisions remain immutable.
fn bench_session_state_clone(c: &mut Criterion) {
    let mut base = SessionState::new(session_key(0), Role::UAC);
    base.registration_contact = Some("sip:bench@example.test".into());

    let mut group = c.benchmark_group("session_state_clone");
    group.bench_function("hot_revision", |b| {
        b.iter(|| {
            let mut revision = black_box(&base).clone();
            revision.call_state = CallState::Active;
            black_box(revision)
        });
    });
    group.bench_function("cold_revision", |b| {
        b.iter(|| {
            let mut revision = black_box(&base).clone();
            revision.registration_contact = Some("sip:changed@example.test".into());
            black_box(revision)
        });
    });
    group.finish();
}

/// Variant 1: today's shape — `Arc<RwLock<Option<SessionId>>>` × 2,
/// looked up with two `read().await` acquisitions.
fn lookup_via_rwlock(
    current_session: &Arc<RwLock<Option<SessionId>>>,
    current_dialog: &Arc<RwLock<Option<DialogId>>>,
) -> impl std::future::Future<Output = Option<SessionId>> + Send {
    let current_session = Arc::clone(current_session);
    let current_dialog = Arc::clone(current_dialog);
    async move {
        let dialog_guard = current_dialog.read().await;
        // Realistic: the registry compares against a target dialog id.
        // Here we just observe presence — the cost being measured is the
        // lock acquisition, not the comparison.
        if dialog_guard.is_some() {
            current_session.read().await.clone()
        } else {
            None
        }
    }
}

/// Variant 2: candidate shape — `ArcSwapOption<SessionId>` × 2, looked
/// up with two atomic loads (zero contention on the read path).
fn lookup_via_arcswap(
    current_session: &Arc<ArcSwapOption<SessionId>>,
    current_dialog: &Arc<ArcSwapOption<DialogId>>,
) -> Option<SessionId> {
    if current_dialog.load().is_some() {
        current_session.load().as_deref().cloned()
    } else {
        None
    }
}

fn bench_session_registry_lookup(c: &mut Criterion) {
    let rt = make_runtime();
    let mut group = c.benchmark_group("session_registry_lookup");
    group.measurement_time(Duration::from_secs(4));

    for &threads in &THREAD_COUNTS {
        let total_ops = (threads as u64) * CONTENDED_OPS_PER_TASK;
        group.throughput(Throughput::Elements(total_ops));

        // Variant 1: RwLock<Option<_>>
        group.bench_with_input(
            BenchmarkId::new("rwlock", threads),
            &threads,
            |b, &threads| {
                b.iter_custom(|iters| {
                    rt.block_on(async move {
                        let current_session: Arc<RwLock<Option<SessionId>>> =
                            Arc::new(RwLock::new(Some(SessionId::new())));
                        let current_dialog: Arc<RwLock<Option<DialogId>>> =
                            Arc::new(RwLock::new(Some(DialogId::new())));
                        let mut total = Duration::ZERO;
                        for _ in 0..iters {
                            let start = Instant::now();
                            let mut handles = Vec::with_capacity(threads);
                            for _ in 0..threads {
                                let cs = Arc::clone(&current_session);
                                let cd = Arc::clone(&current_dialog);
                                handles.push(tokio::spawn(async move {
                                    for _ in 0..CONTENDED_OPS_PER_TASK {
                                        let v = lookup_via_rwlock(&cs, &cd).await;
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

        // Variant 2: ArcSwapOption<_>
        group.bench_with_input(
            BenchmarkId::new("arcswap", threads),
            &threads,
            |b, &threads| {
                b.iter_custom(|iters| {
                    rt.block_on(async move {
                        let current_session: Arc<ArcSwapOption<SessionId>> =
                            Arc::new(ArcSwapOption::from_pointee(SessionId::new()));
                        let current_dialog: Arc<ArcSwapOption<DialogId>> =
                            Arc::new(ArcSwapOption::from_pointee(DialogId::new()));
                        let mut total = Duration::ZERO;
                        for _ in 0..iters {
                            let start = Instant::now();
                            let mut handles = Vec::with_capacity(threads);
                            for _ in 0..threads {
                                let cs = Arc::clone(&current_session);
                                let cd = Arc::clone(&current_dialog);
                                handles.push(tokio::spawn(async move {
                                    for _ in 0..CONTENDED_OPS_PER_TASK {
                                        let v = lookup_via_arcswap(&cs, &cd);
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

fn bench_dialog_callid_key_type(c: &mut Criterion) {
    let rt = make_runtime();
    let mut group = c.benchmark_group("dialog_callid_key_type");
    group.measurement_time(Duration::from_secs(4));

    let string_map: Arc<DashMap<String, SessionId>> = {
        let m = DashMap::with_capacity(CONTENDED_POPULATION);
        for i in 0..CONTENDED_POPULATION {
            m.insert(call_id_key(i), session_key(i));
        }
        Arc::new(m)
    };

    let arc_str_map: Arc<DashMap<Arc<str>, SessionId>> = {
        let m = DashMap::with_capacity(CONTENDED_POPULATION);
        for i in 0..CONTENDED_POPULATION {
            m.insert(Arc::<str>::from(call_id_key(i).as_str()), session_key(i));
        }
        Arc::new(m)
    };

    for &threads in &THREAD_COUNTS {
        let total_ops = (threads as u64) * CONTENDED_OPS_PER_TASK;
        group.throughput(Throughput::Elements(total_ops));

        // Variant 1: DashMap<String, _>. Each lookup hashes the probe
        // String fresh — the cost flagged in `docs/PROFILING.md:124`.
        group.bench_with_input(
            BenchmarkId::new("dashmap_string", threads),
            &threads,
            |b, &threads| {
                b.iter_custom(|iters| {
                    rt.block_on(async {
                        let mut total = Duration::ZERO;
                        for _ in 0..iters {
                            let start = Instant::now();
                            let mut handles = Vec::with_capacity(threads);
                            for t in 0..threads {
                                let map = Arc::clone(&string_map);
                                handles.push(tokio::spawn(async move {
                                    for op in 0..CONTENDED_OPS_PER_TASK as usize {
                                        let idx = (t * 7919 + op * 17) % CONTENDED_POPULATION;
                                        let key = call_id_key(idx);
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
            },
        );

        // Variant 2: DashMap<Arc<str>, _>. Hashing is identical to
        // String (str hash), but value clones cost an atomic refcount
        // bump instead of a heap copy. Probes still build a fresh
        // `Arc<str>` per lookup — that's the realistic cost if the
        // parser interns Call-IDs at parse time, since each inbound
        // message still needs its own Arc<str> until the intern table
        // sees it.
        group.bench_with_input(
            BenchmarkId::new("dashmap_arc_str", threads),
            &threads,
            |b, &threads| {
                b.iter_custom(|iters| {
                    rt.block_on(async {
                        let mut total = Duration::ZERO;
                        for _ in 0..iters {
                            let start = Instant::now();
                            let mut handles = Vec::with_capacity(threads);
                            for t in 0..threads {
                                let map = Arc::clone(&arc_str_map);
                                handles.push(tokio::spawn(async move {
                                    for op in 0..CONTENDED_OPS_PER_TASK as usize {
                                        let idx = (t * 7919 + op * 17) % CONTENDED_POPULATION;
                                        let key: Arc<str> = Arc::from(call_id_key(idx).as_str());
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
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_uncontended,
    bench_contended_lookup,
    bench_create,
    bench_session_state_clone,
    bench_session_registry_lookup,
    bench_dialog_callid_key_type,
);
criterion_main!(benches);

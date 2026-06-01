//! End-to-end call-setup throughput benchmark.
//!
//! Spins up two `StreamPeer`s on loopback (one auto-answering server,
//! one client) and measures the cost of INVITE → 200 OK → ACK → BYE →
//! 200 OK sequences. Reports throughput in calls/sec.
//!
//! The peers are constructed *outside* the timed window so the bench
//! measures call setup, not socket binding. If you need the per-peer
//! construction cost, run the `profiling_call_setup_loop` example
//! under `samply` instead — see `crates/sip/rvoip-sip/docs/PROFILING.md`.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rvoip_sip::{Config, StreamPeer};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[path = "common/mod.rs"]
mod common;

/// How many sequential INVITE-BYE cycles each `iter_custom` batch runs.
/// Smaller batch sizes give criterion finer-grained samples; too small
/// and per-batch peer creation dominates.
const CALLS_PER_BATCH: u64 = 4;

/// Concurrent call fan-out for the parallel variant.
const CONCURRENCY: [usize; 3] = [1, 4, 16];

async fn build_server(port: u16) -> StreamPeer {
    let (media_start, media_end) = common::next_media_window();
    let cfg = Config {
        media_port_start: media_start,
        media_port_end: media_end,
        ..Config::local("bench-server", port)
    };
    StreamPeer::with_config(cfg).await.expect("server peer")
}

async fn build_client(port: u16) -> StreamPeer {
    let (media_start, media_end) = common::next_media_window();
    let cfg = Config {
        media_port_start: media_start,
        media_port_end: media_end,
        ..Config::local("bench-client", port)
    };
    StreamPeer::with_config(cfg).await.expect("client peer")
}

/// Background task that loops `wait_for_incoming` → `accept` until the
/// peer is dropped. Stops cleanly when `wait_for_incoming` errors after
/// the peer shuts down.
fn spawn_auto_answer(mut peer: StreamPeer) -> tokio::task::JoinHandle<StreamPeer> {
    tokio::spawn(async move {
        loop {
            match tokio::time::timeout(Duration::from_secs(30), peer.wait_for_incoming()).await {
                Ok(Ok(incoming)) => {
                    if let Ok(handle) = incoming.accept().await {
                        // Fire-and-forget: the client drives hangup.
                        tokio::spawn(async move {
                            let _ = handle.wait_for_end(Some(Duration::from_secs(30))).await;
                        });
                    }
                }
                Ok(Err(_)) | Err(_) => break,
            }
        }
        peer
    })
}

fn bench_call_setup_sequential(c: &mut Criterion) {
    let rt = common::build_runtime();

    let mut group = c.benchmark_group("e2e_call_setup");
    group.throughput(Throughput::Elements(CALLS_PER_BATCH));
    group.sample_size(20);
    group.bench_function(BenchmarkId::from_parameter("sequential"), |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let server_port = common::next_sip_port();
                let client_port = common::next_sip_port();
                let server = build_server(server_port).await;
                let server_task = spawn_auto_answer(server);
                let mut client = build_client(client_port).await;
                let target = format!("sip:bench-server@127.0.0.1:{}", server_port);

                let start = Instant::now();
                for _ in 0..iters {
                    for _ in 0..CALLS_PER_BATCH {
                        let call_id = client.invite(&target).send().await.expect("invite send");
                        let handle = client.coordinator().session(&call_id);
                        client
                            .wait_for_answered(handle.id())
                            .await
                            .expect("wait answered");
                        handle.hangup().await.expect("hangup");
                        client
                            .wait_for_ended(handle.id())
                            .await
                            .expect("wait ended");
                        black_box(call_id);
                    }
                }
                let elapsed = start.elapsed();

                client.shutdown().await.ok();
                server_task.abort();
                let _ = server_task.await;
                elapsed
            })
        });
    });
    group.finish();
}

fn bench_call_setup_concurrent(c: &mut Criterion) {
    let rt = common::build_runtime();

    let mut group = c.benchmark_group("e2e_call_setup");
    group.sample_size(15);
    for &concurrency in &CONCURRENCY {
        let total_calls = (concurrency as u64) * CALLS_PER_BATCH;
        group.throughput(Throughput::Elements(total_calls));
        group.bench_with_input(
            BenchmarkId::new("concurrent", concurrency),
            &concurrency,
            |b, &concurrency| {
                b.iter_custom(|iters| {
                    rt.block_on(async {
                        let server_port = common::next_sip_port();
                        let server = build_server(server_port).await;
                        let server_task = spawn_auto_answer(server);

                        // One client peer per concurrent stream.
                        let mut clients = Vec::with_capacity(concurrency);
                        for _ in 0..concurrency {
                            let p = common::next_sip_port();
                            clients.push(Arc::new(Mutex::new(build_client(p).await)));
                        }
                        let target =
                            Arc::new(format!("sip:bench-server@127.0.0.1:{}", server_port));

                        let start = Instant::now();
                        for _ in 0..iters {
                            let mut handles = Vec::with_capacity(concurrency);
                            for client in &clients {
                                let client = Arc::clone(client);
                                let target = Arc::clone(&target);
                                handles.push(tokio::spawn(async move {
                                    let mut guard = client.lock().await;
                                    for _ in 0..CALLS_PER_BATCH {
                                        let call_id = guard
                                            .invite(target.as_str())
                                            .send()
                                            .await
                                            .expect("invite send");
                                        let handle = guard.coordinator().session(&call_id);
                                        guard
                                            .wait_for_answered(handle.id())
                                            .await
                                            .expect("wait answered");
                                        handle.hangup().await.expect("hangup");
                                        guard
                                            .wait_for_ended(handle.id())
                                            .await
                                            .expect("wait ended");
                                    }
                                }));
                            }
                            for h in handles {
                                h.await.expect("client task");
                            }
                        }
                        let elapsed = start.elapsed();

                        for client in clients {
                            let mut g = client.lock().await;
                            // StreamPeer::shutdown takes `self` by value; we hold an
                            // Arc<Mutex<StreamPeer>> here so we can't move it out.
                            // Leaving the peer to drop is acceptable for bench teardown.
                            let _ = &mut *g;
                        }
                        server_task.abort();
                        let _ = server_task.await;
                        elapsed
                    })
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_call_setup_sequential,
    bench_call_setup_concurrent
);
criterion_main!(benches);

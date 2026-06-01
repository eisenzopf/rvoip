//! Steady-state dialog count benchmark.
//!
//! Establishes N persistent calls between two `StreamPeer`s, then
//! measures how long it takes to set up *one more* call against that
//! backlog. The latency curve as N grows exposes the cost of carrying
//! large dialog tables — DashMap shard contention in the dialog
//! adapter, lock pressure on the transaction manager's
//! `Arc<Mutex<HashMap<TransactionKey, _>>>`, etc.
//!
//! For heap-side measurement, run the `profiling_dhat_dialog` example
//! under the `dhat` feature; see `crates/sip/rvoip-sip/docs/PROFILING.md`.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use rvoip_sip::{Config, StreamPeer};
use std::time::{Duration, Instant};

#[path = "common/mod.rs"]
mod common;

/// Number of persistent dialogs held in the background while the bench
/// times its measured call. Keep the upper bound modest — each entry
/// burns a UDP socket pair, RTP port window, and dialog/session state.
const BACKLOG: [usize; 4] = [0, 50, 250, 1000];

async fn build_server(port: u16) -> StreamPeer {
    let (m0, m1) = common::next_media_window();
    let cfg = Config {
        media_port_start: m0,
        media_port_end: m1,
        ..Config::local("bench-steady-server", port)
    };
    StreamPeer::with_config(cfg).await.expect("server peer")
}

async fn build_client(port: u16) -> StreamPeer {
    let (m0, m1) = common::next_media_window();
    let cfg = Config {
        media_port_start: m0,
        media_port_end: m1,
        ..Config::local("bench-steady-client", port)
    };
    StreamPeer::with_config(cfg).await.expect("client peer")
}

fn spawn_auto_answer(mut peer: StreamPeer) -> tokio::task::JoinHandle<StreamPeer> {
    tokio::spawn(async move {
        loop {
            match tokio::time::timeout(Duration::from_secs(30), peer.wait_for_incoming()).await {
                Ok(Ok(incoming)) => {
                    if let Ok(handle) = incoming.accept().await {
                        tokio::spawn(async move {
                            let _ = handle.wait_for_end(Some(Duration::from_secs(300))).await;
                        });
                    }
                }
                Ok(Err(_)) | Err(_) => break,
            }
        }
        peer
    })
}

fn bench_steady_state(c: &mut Criterion) {
    let rt = common::build_runtime();

    let mut group = c.benchmark_group("e2e_dialog_steady_state");
    group.sample_size(10);
    for &n in &BACKLOG {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let server_port = common::next_sip_port();
                    let client_port = common::next_sip_port();
                    let server = build_server(server_port).await;
                    let server_task = spawn_auto_answer(server);
                    let mut client = build_client(client_port).await;
                    let target = format!("sip:bench-steady-server@127.0.0.1:{}", server_port);

                    // Pre-establish N persistent calls; hold the handles
                    // for the duration of the timed loop so the dialog
                    // table stays at size N.
                    let mut backlog_handles = Vec::with_capacity(n);
                    for _ in 0..n {
                        let call_id = client.invite(&target).send().await.expect("invite send");
                        let handle = client.coordinator().session(&call_id);
                        client
                            .wait_for_answered(handle.id())
                            .await
                            .expect("wait answered");
                        backlog_handles.push(handle);
                    }

                    // Measured loop: each iter sets up one additional
                    // call (above the steady-state backlog) and tears
                    // it down. iters is controlled by criterion.
                    let start = Instant::now();
                    for _ in 0..iters {
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
                    let elapsed = start.elapsed();

                    // Tear down the backlog calls.
                    for h in &backlog_handles {
                        let _ = h.hangup().await;
                    }
                    drop(backlog_handles);

                    client.shutdown().await.ok();
                    server_task.abort();
                    let _ = server_task.await;
                    elapsed
                })
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_steady_state);
criterion_main!(benches);

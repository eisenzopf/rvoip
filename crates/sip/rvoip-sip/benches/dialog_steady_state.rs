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

// Keep real RTP allocation in this benchmark: the steady-state cost under
// test includes the media resources attached to every dialog.  The previous
// shared 200-port window could not represent the declared 250- and
// 1,000-dialog cases, so those cases measured allocator exhaustion instead
// of dialog-table behavior.  Each peer gets a disjoint, reusable range large
// enough for the maximum backlog plus the one measured call.  A Criterion
// iteration fully shuts its peers down before the next iteration reuses it.
const STEADY_MEDIA_CAPACITY: usize = 2048;
const SERVER_MEDIA_START: u16 = 20_000;
const CLIENT_MEDIA_START: u16 = 24_000;

const _: () = assert!(STEADY_MEDIA_CAPACITY > BACKLOG[BACKLOG.len() - 1]);

async fn build_server(port: u16) -> StreamPeer {
    let cfg = Config::local("bench-steady-server", port)
        .with_media_port_capacity(SERVER_MEDIA_START, STEADY_MEDIA_CAPACITY);
    StreamPeer::with_config(cfg).await.expect("server peer")
}

async fn build_client(port: u16) -> StreamPeer {
    let cfg = Config::local("bench-steady-client", port)
        .with_media_port_capacity(CLIENT_MEDIA_START, STEADY_MEDIA_CAPACITY);
    StreamPeer::with_config(cfg).await.expect("client peer")
}

fn spawn_auto_answer(mut peer: StreamPeer) -> tokio::task::JoinHandle<StreamPeer> {
    tokio::spawn(async move {
        while let Ok(Ok(incoming)) =
            tokio::time::timeout(Duration::from_secs(30), peer.wait_for_incoming()).await
        {
            if let Ok(handle) = incoming.accept().await {
                tokio::spawn(async move {
                    let _ = handle.wait_for_end(Some(Duration::from_secs(300))).await;
                });
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

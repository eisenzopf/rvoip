//! Registration-storm benchmark.
//!
//! Boots a `UnifiedCoordinator` configured as an authenticating
//! registrar and times batches of `StreamPeer` REGISTER → 200 OK cycles
//! against it. Measures the per-REGISTER cost the registrar pays at
//! varying client fan-out — the classic "all phones come back after a
//! WAN outage" scenario.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rvoip_sip::{Config, StreamPeer, UnifiedCoordinator};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[path = "common/mod.rs"]
mod common;

const REGISTRAR_REALM: &str = "bench.local";
const REGISTRAR_USER: &str = "alice";
const REGISTRAR_PASS: &str = "password123";

const FANOUT: [usize; 3] = [1, 8, 32];
const REGISTRATIONS_PER_CLIENT: u64 = 4;

async fn build_registrar(port: u16) -> Arc<UnifiedCoordinator> {
    let coordinator = UnifiedCoordinator::new(Config::local("bench-registrar", port))
        .await
        .expect("registrar coordinator");
    let mut users = HashMap::new();
    users.insert(REGISTRAR_USER.to_string(), REGISTRAR_PASS.to_string());
    // The handle is kept alive by holding the coordinator; the registrar
    // task stays running until the coordinator is dropped.
    let _registrar = coordinator
        .start_registration_server(REGISTRAR_REALM, users)
        .await
        .expect("start registrar");
    coordinator
}

async fn build_client(port: u16) -> StreamPeer {
    let (media_start, media_end) = common::next_media_window();
    let cfg = Config {
        media_port_start: media_start,
        media_port_end: media_end,
        ..Config::local("bench-reg-client", port)
    };
    StreamPeer::with_config(cfg).await.expect("client peer")
}

fn bench_register_storm(c: &mut Criterion) {
    let rt = common::build_runtime();

    let mut group = c.benchmark_group("e2e_register_storm");
    group.sample_size(15);
    for &fanout in &FANOUT {
        let total_regs = (fanout as u64) * REGISTRATIONS_PER_CLIENT;
        group.throughput(Throughput::Elements(total_regs));
        group.bench_with_input(
            BenchmarkId::from_parameter(fanout),
            &fanout,
            |b, &fanout| {
                b.iter_custom(|iters| {
                    rt.block_on(async {
                        let registrar_port = common::next_sip_port();
                        let _registrar = build_registrar(registrar_port).await;

                        // Each client peer reuses its socket across all
                        // REGISTER cycles in the batch. Per-cycle work
                        // is REGISTER + 200 OK + unregister round-trip.
                        let mut clients = Vec::with_capacity(fanout);
                        for _ in 0..fanout {
                            let p = common::next_sip_port();
                            clients.push(build_client(p).await);
                        }
                        let target = format!("sip:127.0.0.1:{}", registrar_port);

                        let start = Instant::now();
                        for _ in 0..iters {
                            let mut handles = Vec::with_capacity(fanout);
                            for client in &clients {
                                let target = target.clone();
                                // We can't move &StreamPeer into tokio::spawn;
                                // run each client's batch sequentially in a
                                // local async block instead. Concurrency comes
                                // from awaiting many such blocks via join_all.
                                handles.push(async move {
                                    for _ in 0..REGISTRATIONS_PER_CLIENT {
                                        let handle = client
                                            .register(
                                                target.as_str(),
                                                REGISTRAR_USER,
                                                REGISTRAR_PASS,
                                            )
                                            .send()
                                            .await
                                            .expect("register");
                                        // Give the registrar a beat to
                                        // process; without this the next
                                        // unregister can race the 200 OK
                                        // and surface a transient error.
                                        for _ in 0..20 {
                                            if client.is_registered(&handle).await.unwrap_or(false)
                                            {
                                                break;
                                            }
                                            tokio::time::sleep(Duration::from_millis(5)).await;
                                        }
                                        client.unregister(&handle).await.ok();
                                        black_box(&handle);
                                    }
                                });
                            }
                            futures::future::join_all(handles).await;
                        }
                        let elapsed = start.elapsed();

                        // Tear-down: dropping StreamPeer Arcs closes
                        // sockets; the coordinator drops with _registrar.
                        drop(clients);
                        elapsed
                    })
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_register_storm);
criterion_main!(benches);

//! Audio frame pipeline benchmark.
//!
//! Drives `MediaSessionController::process_rtp_packet_zero_copy` and
//! `process_rtp_packet_traditional` (see
//! `relay/controller/zero_copy.rs:25` and `:91`) — the two flavours of
//! the decode → process → encode loop. Both flavours fight the same
//! contended `Arc<tokio::sync::Mutex<G711Codec>>` on
//! `MediaSessionController::g711_codec` (mod.rs:150). Sweeping N
//! concurrent calls against one shared controller exposes that lock
//! exactly the way production conferences do.
//!
//! Two harnesses:
//!
//! - `pipeline_single` — one packet per iteration through the
//!   zero-copy and traditional paths. Isolates the per-packet CPU
//!   cost.
//! - `pipeline_concurrent` — N tasks each processing M packets against
//!   one shared `MediaSessionController`. Throughput is total ops/sec
//!   across all tasks; we expect a steep cliff at N>1 today and a
//!   nearly linear curve after the Phase C5 per-session-codec
//!   refactor.

use bytes::Bytes;
use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use rvoip_media_core::relay::controller::MediaSessionController;
use rvoip_rtp_core::{RtpHeader, RtpPacket};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Builder;

const PAYLOAD_SIZE: usize = 160; // G.711 µ-law, 20 ms @ 8 kHz
const OPS_PER_TASK: u64 = 200;
const CONCURRENT_TASK_COUNTS: [usize; 4] = [1, 4, 8, 16];

fn make_packet(seq: u16) -> RtpPacket {
    let header = RtpHeader::new(0 /* µ-law PT */, seq, (seq as u32) * 160, 0xdead_beef);
    // µ-law silence byte is 0xFF; pre-fill so decode produces near-zero
    // samples (representative of a hold/comfort path).
    let payload: Vec<u8> = (0..PAYLOAD_SIZE).map(|i| (i & 0xff) as u8).collect();
    RtpPacket::new(header, Bytes::from(payload))
}

fn bench_single(c: &mut Criterion) {
    let rt = Builder::new_current_thread().enable_all().build().unwrap();
    let mut group = c.benchmark_group("pipeline_single");
    group.throughput(Throughput::Elements(1));

    let packet = make_packet(0);
    let ctrl = Arc::new(MediaSessionController::new());

    group.bench_function("zero_copy", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let start = Instant::now();
                for _ in 0..iters {
                    let out = ctrl
                        .process_rtp_packet_zero_copy(&packet)
                        .await
                        .expect("zero_copy");
                    black_box(out);
                }
                start.elapsed()
            })
        });
    });

    group.bench_function("traditional", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let start = Instant::now();
                for _ in 0..iters {
                    let out = ctrl
                        .process_rtp_packet_traditional(&packet)
                        .await
                        .expect("traditional");
                    black_box(out);
                }
                start.elapsed()
            })
        });
    });

    group.finish();
}

fn bench_concurrent(c: &mut Criterion) {
    let rt = Builder::new_multi_thread()
        .worker_threads(16)
        .enable_all()
        .build()
        .expect("runtime");

    let mut group = c.benchmark_group("pipeline_concurrent");
    for &tasks in &CONCURRENT_TASK_COUNTS {
        let total_ops = (tasks as u64) * OPS_PER_TASK;
        group.throughput(Throughput::Elements(total_ops));
        group.bench_with_input(
            BenchmarkId::new("zero_copy", tasks),
            &tasks,
            |b, &tasks| {
                b.iter_custom(|iters| {
                    rt.block_on(async move {
                        let ctrl = Arc::new(MediaSessionController::new());
                        let mut total = Duration::ZERO;
                        for _ in 0..iters {
                            let start = Instant::now();
                            let mut handles = Vec::with_capacity(tasks);
                            for t in 0..tasks {
                                let ctrl = Arc::clone(&ctrl);
                                handles.push(tokio::spawn(async move {
                                    let packet = make_packet(t as u16);
                                    for _ in 0..OPS_PER_TASK {
                                        let out = ctrl
                                            .process_rtp_packet_zero_copy(&packet)
                                            .await
                                            .expect("zero_copy");
                                        black_box(out);
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

criterion_group!(benches, bench_single, bench_concurrent);
criterion_main!(benches);

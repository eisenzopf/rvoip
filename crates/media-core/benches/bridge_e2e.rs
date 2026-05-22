//! End-to-end bridge throughput bench.
//!
//! Drives real `MediaSessionController::bridge_sessions` setups
//! through real UDP loopback so the C16 production wins
//! (`RtpSendHandle` on the bridge forwarder), the C7+ DashMap
//! conversions (`sessions` / `rtp_sessions`), and the C19/C20 atomic
//! counter refactors get measured concretely. The earlier
//! `audio_frame_pipeline` bench drives `process_rtp_packet_zero_copy`
//! and never exercises the bridge forward path; this bench fills
//! that gap.
//!
//! Topology per bridge:
//!
//! ```text
//!   sender_udp ──► [session_a]──(bridge forwarder)──►[session_b] ──► sink_udp
//! ```
//!
//! Each `iter` sends one RTP packet per bridge in parallel and waits
//! for all forwarded packets to land on the sink sockets. The
//! per-iter time is therefore one forward round-trip across N
//! concurrent bridges.

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rvoip_media_core::relay::controller::{
    bridge::BridgeHandle, MediaConfig, MediaSessionController,
};
use rvoip_media_core::types::DialogId;
use rvoip_rtp_core::{RtpHeader, RtpPacket};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::runtime::{Builder, Runtime};

/// Bridge counts driven concurrently. `/64` was originally on the
/// list but trips the rtp-core `GlobalPortAllocator` retry budget
/// (each bridge consumes 2 sessions × ≥1 port plus the sender +
/// receiver sockets — at 64 bridges that's ~256+ ports against an
/// allocator range tuned for typical SIP loads). `/16` is the
/// upper bound that fits cleanly.
const BRIDGE_COUNTS: [usize; 3] = [1, 4, 16];

/// One full bridge endpoint pair: a UDP sender that injects RTP into
/// session A's port, and a UDP receiver bound to the address session B
/// is configured to send to. End-to-end round trip = one bridged
/// packet.
struct BridgeEndpoints {
    sender: UdpSocket,
    a_local_addr: SocketAddr,
    receiver: UdpSocket,
}

/// Fixture: holds the controller, every endpoint pair, and every
/// `BridgeHandle` so the forwarder tasks stay alive for the duration
/// of the bench. Dropping `_handles` tears the bridges down.
struct BridgeFixture {
    _controller: Arc<MediaSessionController>,
    bridges: Vec<BridgeEndpoints>,
    _handles: Vec<BridgeHandle>,
    payload: Bytes,
}

async fn build_fixture(n_bridges: usize) -> BridgeFixture {
    let controller = Arc::new(MediaSessionController::new());
    let mut bridges = Vec::with_capacity(n_bridges);
    let mut handles = Vec::with_capacity(n_bridges);

    for i in 0..n_bridges {
        let dialog_a = DialogId::new(format!("bench-bridge-{i}-a"));
        let dialog_b = DialogId::new(format!("bench-bridge-{i}-b"));

        // Sink socket first so we know the address session B will
        // forward to.
        let receiver = UdpSocket::bind("127.0.0.1:0").await.expect("bind sink");
        let sink_addr = receiver.local_addr().expect("sink local_addr");

        // Session A: any local port; remote_addr is a placeholder
        // (bridge precondition just checks `is_some`). Symmetric RTP
        // will overwrite it on receive anyway.
        let config_a = MediaConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: Some("127.0.0.1:1".parse().unwrap()),
            preferred_codec: Some("PCMU".to_string()),
            parameters: HashMap::new(),
        };
        // Session B: bridged outbound goes here. Remote_addr is the
        // sink we'll receive on.
        let config_b = MediaConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: Some(sink_addr),
            preferred_codec: Some("PCMU".to_string()),
            parameters: HashMap::new(),
        };

        controller
            .start_media(dialog_a.clone(), config_a)
            .await
            .expect("start_media A");
        controller
            .start_media(dialog_b.clone(), config_b)
            .await
            .expect("start_media B");

        // The RtpSession's send task reads the destination from
        // `UdpRtpTransport::remote_rtp_addr()`, which `start_media`
        // does *not* populate from MediaConfig. Without this call,
        // session B's send_task would drop every bridged packet
        // with "No destination address". Explicitly install B's
        // remote_addr so outbound forwards land on our sink socket.
        controller
            .update_rtp_remote_addr(&dialog_b, sink_addr)
            .await
            .expect("update remote B");
        // A also needs its transport's remote_addr set for the bridge
        // precondition check (`wrapper.remote_addr.is_some()`); the
        // value is irrelevant for the receive-only side, but the
        // session's symmetric_rtp will overwrite it on the first
        // received packet anyway.
        controller
            .update_rtp_remote_addr(&dialog_a, "127.0.0.1:1".parse().unwrap())
            .await
            .expect("update remote A");

        // Session A's allocated local port (where we send into).
        let a_info = controller
            .get_session_info(&dialog_a)
            .await
            .expect("session A info");
        let a_local_addr: SocketAddr =
            format!("127.0.0.1:{}", a_info.rtp_port.expect("A rtp_port"))
                .parse()
                .unwrap();

        let handle = controller
            .bridge_sessions(dialog_a, dialog_b)
            .await
            .expect("bridge_sessions");
        handles.push(handle);

        let sender = UdpSocket::bind("127.0.0.1:0").await.expect("bind sender");

        bridges.push(BridgeEndpoints {
            sender,
            a_local_addr,
            receiver,
        });
    }

    // Pre-build one RTP wire frame (160 B G.711 = 20 ms ptime). The
    // same bytes are sent through every bridge; sequence numbers
    // don't matter for the throughput measurement.
    let header = RtpHeader::new(0, 0, 0, 0xdead_beef);
    let payload: Vec<u8> = (0..160).map(|i| (i & 0xff) as u8).collect();
    let packet = RtpPacket::new(header, Bytes::from(payload));
    let wire = packet.serialize().expect("serialize");

    BridgeFixture {
        _controller: controller,
        bridges,
        _handles: handles,
        payload: wire,
    }
}

/// One full round trip: send one packet per bridge in parallel, wait
/// for all of them to land on the sink sockets. We `recv_from` with a
/// generous timeout — UDP loopback occasionally drops under heavy load
/// and we'd rather log + abort the bench than hang.
async fn drive_one_round(fix: &BridgeFixture) {
    // Phase 1: fire all sends concurrently.
    let send_futs = fix.bridges.iter().map(|ep| {
        let payload = fix.payload.clone();
        async move {
            ep.sender
                .send_to(&payload, ep.a_local_addr)
                .await
                .expect("send_to");
        }
    });
    futures::future::join_all(send_futs).await;

    // Phase 2: collect all receives concurrently.
    let recv_futs = fix.bridges.iter().enumerate().map(|(idx, ep)| async move {
        let mut buf = [0u8; 2048];
        match tokio::time::timeout(Duration::from_millis(2000), ep.receiver.recv_from(&mut buf))
            .await
        {
            Ok(Ok((n, _src))) => black_box(n),
            Ok(Err(e)) => panic!("recv_from err: {e}"),
            Err(_) => panic!("recv_from timeout — bridge {idx} forward path stalled?"),
        }
    });
    futures::future::join_all(recv_futs).await;
}

fn bench_bridge_e2e(c: &mut Criterion) {
    let rt = Builder::new_multi_thread()
        .worker_threads(8)
        .enable_all()
        .build()
        .expect("runtime");

    let mut group = c.benchmark_group("bridge_e2e_roundtrip");
    for &n in &BRIDGE_COUNTS {
        // Throughput == bridged packets per round trip.
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            // Build the fixture once outside the timed loop.
            let mut fix_opt = Some(rt.block_on(build_fixture(n)));

            // Let the bridge forwarder tasks actually start polling
            // their broadcast receivers (they're tokio::spawn'd
            // inside bridge_sessions; the spawn returns before the
            // task has been scheduled).
            std::thread::sleep(Duration::from_millis(100));

            // Warm-up: send one round so socket buffers / forwarder
            // tasks are paged in before measurement starts.
            rt.block_on(drive_one_round(fix_opt.as_ref().unwrap()));

            b.iter_custom(|iters| {
                rt.block_on(async {
                    let fix = fix_opt.as_ref().unwrap();
                    let start = Instant::now();
                    for _ in 0..iters {
                        drive_one_round(fix).await;
                    }
                    start.elapsed()
                })
            });

            // `BridgeHandle::Drop` spawns a tokio task to abort the
            // forwarder, which panics if invoked outside a runtime
            // context. Drop the fixture inside `rt.block_on` so the
            // current runtime is available for that spawn.
            rt.block_on(async move {
                drop(fix_opt.take());
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_bridge_e2e);
criterion_main!(benches);

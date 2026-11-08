//! The lane table on a real dedicated media lane, through the public API
//! only, as an application uses it.
//!
//! The media runtime and the executor are process-wide and configured once,
//! so this binary holds a single test: it installs one dedicated lane and
//! `MediaExecutorConfig::v2()`, then runs calls whose 20 ms TX is a
//! [`LaneTxSource`] installed by a supervisor that
//! `executor::spawn_on_media_lane` hosts on the call's lane.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use rvoip_rtp_core::executor::{
    install_media_executor_config, spawn_on_media_lane, MediaExecutorConfig,
};
use rvoip_rtp_core::lane_table::{
    self, callback_lane_counters, Escalation, LaneNext, LaneTxReturn, LaneTxSource,
};
use rvoip_rtp_core::session::{RtpFrameNow, RtpSendHandle, RtpSession, RtpSessionConfig};
use rvoip_rtp_core::task_runtime::{install_media_runtime_config, MediaLanes, MediaRuntimeConfig};
use rvoip_rtp_core::RtpPacket;
use tokio::net::UdpSocket;
use tokio::time::Instant;

const FRAME: Duration = Duration::from_millis(20);
const FRAMES: usize = 25;
const CALLS: usize = 8;

/// An application's paced sender: one 160-byte PCMU frame per 20 ms.
struct Pacer {
    sender: RtpSendHandle,
    seed: u8,
    next: usize,
    start: Instant,
    /// (deadline, pass reading) of every fire.
    fires: Arc<parking_lot::Mutex<Vec<(Instant, Instant)>>>,
}

impl Pacer {
    fn deadline(&self, frame: usize) -> Instant {
        self.start + FRAME * frame as u32
    }

    fn payload(&self, frame: usize) -> [u8; 160] {
        [self.seed ^ frame as u8; 160]
    }
}

impl LaneTxSource for Pacer {
    fn on_due(&mut self, now: Instant) -> LaneNext {
        self.fires.lock().push((self.deadline(self.next), now));
        let payload = self.payload(self.next);
        let frame = RtpFrameNow {
            timestamp: (self.next as u32) * 160,
            payload: &payload,
            marker: self.next == 0,
            payload_type: 0,
        };
        match self.sender.try_send_now(&[frame], false) {
            Ok(report) if report.sent == 1 => {}
            Ok(_) => return LaneNext::Escalate(Escalation::SendDeferred),
            Err(_) => return LaneNext::Done,
        }
        self.next += 1;
        if self.next == FRAMES {
            LaneNext::Done
        } else {
            LaneNext::At(self.deadline(self.next))
        }
    }
}

/// Installs the pacer on the lane, completes escalated frames with the
/// awaited send, and reports how many installs the call took (0: no table).
async fn supervise(mut pacer: Box<Pacer>) -> usize {
    let mut installs = 0;
    loop {
        let first = pacer.deadline(pacer.next);
        let mut handle = match lane_table::install_tx(pacer, first) {
            Ok(handle) => handle,
            Err(_) => return installs,
        };
        installs += 1;
        match handle.finished().await {
            Some(LaneTxReturn::Escalated(back, _)) => {
                pacer = back.downcast::<Pacer>().ok().expect("our pacer");
                let frame = pacer.next;
                pacer
                    .sender
                    .send_packet_with_pt(
                        (frame as u32) * 160,
                        Bytes::copy_from_slice(&pacer.payload(frame)),
                        frame == 0,
                        0,
                    )
                    .await
                    .expect("awaited send");
                pacer.next += 1;
                lane_table::record_resume(pacer.next < FRAMES);
                if pacer.next == FRAMES {
                    return installs;
                }
            }
            _ => return installs,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tx_sources_fire_from_the_table_of_a_dedicated_lane_without_a_task_per_call() {
    install_media_runtime_config(MediaRuntimeConfig::new(MediaLanes::dedicated(1))).unwrap();
    install_media_executor_config(MediaExecutorConfig::v2()).unwrap();
    assert!(lane_table::callback_send_enabled());

    let mut calls = Vec::new();
    for _ in 0..CALLS {
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let session = RtpSession::new_event_driven(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: Some(peer.local_addr().unwrap()),
            payload_type: 0,
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        let sender = session.send_handle().expect("send handle");
        calls.push((session, sender, peer));
    }
    let lane = calls[0]
        .1
        .media_runtime_handle()
        .expect("the transport is placed on the dedicated lane");
    // Let the lane's poller and every receive loop start first.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let lane_tasks = lane.metrics().num_alive_tasks();
    let before = callback_lane_counters();

    let fires = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let start = Instant::now() + Duration::from_millis(40);
    let mut installs = Vec::new();
    for (index, (_, sender, _)) in calls.iter().enumerate() {
        let pacer = Box::new(Pacer {
            sender: sender.clone(),
            seed: index as u8,
            next: 0,
            start: start + Duration::from_micros(2_300 * index as u64),
            fires: Arc::clone(&fires),
        });
        let (tx, rx) = tokio::sync::oneshot::channel();
        installs.push(rx);
        spawn_on_media_lane(&lane, async move {
            let _ = tx.send(supervise(pacer).await);
        });
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        lane.metrics().num_alive_tasks(),
        lane_tasks,
        "the supervisors are hosted on the lane poller: no task per call"
    );

    for (index, (_, _, peer)) in calls.iter().enumerate() {
        let mut buffer = [0u8; 1500];
        for frame in 0..FRAMES {
            let (size, _) =
                tokio::time::timeout(Duration::from_secs(5), peer.recv_from(&mut buffer))
                    .await
                    .expect("every frame arrives")
                    .unwrap();
            let packet = RtpPacket::parse(&buffer[..size]).unwrap();
            assert_eq!(
                packet.header.timestamp,
                (frame as u32) * 160,
                "call {index}"
            );
            assert_eq!(packet.header.marker, frame == 0);
            assert_eq!(
                packet.payload.as_ref(),
                &[index as u8 ^ frame as u8; 160][..]
            );
        }
    }
    for rx in installs {
        assert!(rx.await.unwrap() >= 1, "installed in the lane's table");
    }

    let fires = fires.lock().clone();
    assert!(
        fires.iter().all(|(deadline, now)| now >= deadline),
        "never early"
    );
    let worst = fires
        .iter()
        .map(|(deadline, now)| now.saturating_duration_since(*deadline))
        .max()
        .unwrap();
    assert!(
        worst < Duration::from_millis(30),
        "worst lateness {worst:?}"
    );

    // Counters are published per table every 64 passes; by now (hundreds of
    // 2 ms ticks) the lane's table has flushed.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let after = callback_lane_counters();
    assert!(after.table_lanes >= 1, "the dedicated lane keeps a table");
    assert_eq!(after.unsupported_lanes, before.unsupported_lanes);
    assert!(after.installs >= before.installs + CALLS as u64);
    assert!(after.fires >= before.fires + (CALLS * (FRAMES - 1)) as u64);

    for (mut session, _, _) in calls {
        session.close().await.unwrap();
    }
}
